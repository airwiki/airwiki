//! Device trust, blocking and collection-scoped grants.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use airwiki_types::{DisclosureGate, DisclosureLease, SearchAuthorization, SearchPurpose};
use libp2p::PeerId;
use parking_lot::RwLock;
use thiserror::Error;
use uuid::Uuid;

/// Mutable access state is intentionally independent from discovery state.
/// Discovering a peer never grants it any information.
#[derive(Debug, Clone)]
pub struct AccessControl {
    inner: Arc<RwLock<HashMap<PeerId, PeerAccess>>>,
    disclosure_gate: DisclosureGate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerAccess {
    pub trusted: bool,
    pub blocked: bool,
    pub grants: HashSet<Uuid>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AccessError {
    #[error("peer is not trusted")]
    NotTrusted,
    #[error("peer is blocked")]
    Blocked,
    #[error("peer has no collection grants")]
    NoGrants,
}

impl AccessControl {
    pub fn with_disclosure_gate(disclosure_gate: DisclosureGate) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            disclosure_gate,
        }
    }

    pub(crate) fn disclosure_lease_is_current(&self, lease: &DisclosureLease) -> bool {
        self.disclosure_gate.owns(lease)
    }

    /// Restore a peer from durable storage during application startup.
    pub fn restore(&self, peer: PeerId, access: PeerAccess) {
        let _mutation = self.disclosure_gate.acquire_mutation();
        self.inner.write().insert(peer, access);
    }

    pub fn mark_trusted(&self, peer: PeerId) {
        let _mutation = self.disclosure_gate.acquire_mutation();
        let mut peers = self.inner.write();
        let access = peers.entry(peer).or_default();
        access.trusted = true;
        access.blocked = false;
    }

    pub fn block(&self, peer: PeerId) {
        let _mutation = self.disclosure_gate.acquire_mutation();
        let mut peers = self.inner.write();
        let access = peers.entry(peer).or_default();
        access.blocked = true;
        access.trusted = false;
        access.grants.clear();
    }

    pub fn unblock(&self, peer: PeerId) {
        let _mutation = self.disclosure_gate.acquire_mutation();
        if let Some(access) = self.inner.write().get_mut(&peer) {
            access.blocked = false;
        }
    }

    pub fn grant(&self, peer: PeerId, collection: Uuid) -> Result<(), AccessError> {
        let _mutation = self.disclosure_gate.acquire_mutation();
        let mut peers = self.inner.write();
        let access = peers.entry(peer).or_default();
        if access.blocked {
            return Err(AccessError::Blocked);
        }
        if !access.trusted {
            return Err(AccessError::NotTrusted);
        }
        access.grants.insert(collection);
        Ok(())
    }

    pub fn remove_grant(&self, peer: PeerId, collection: Uuid) {
        let _mutation = self.disclosure_gate.acquire_mutation();
        if let Some(access) = self.inner.write().get_mut(&peer) {
            access.grants.remove(&collection);
        }
    }

    /// Revocation is fail-closed: all grants disappear and the device is blocked.
    pub fn revoke_and_block(&self, peer: PeerId) {
        self.block(peer);
    }

    pub fn state(&self, peer: &PeerId) -> PeerAccess {
        self.inner.read().get(peer).cloned().unwrap_or_default()
    }

    pub fn is_blocked(&self, peer: &PeerId) -> bool {
        self.inner
            .read()
            .get(peer)
            .is_some_and(|access| access.blocked)
    }

    pub fn trusted_peers(&self) -> Vec<PeerId> {
        self.inner
            .read()
            .iter()
            .filter_map(|(peer, access)| (access.trusted && !access.blocked).then_some(*peer))
            .collect()
    }

    /// Construct the only authorization object passed to the local search backend.
    /// No API exposes the complete collection catalog to a remote peer.
    pub fn authorize(
        &self,
        peer: &PeerId,
        purpose: SearchPurpose,
    ) -> Result<SearchAuthorization, AccessError> {
        let peers = self.inner.read();
        let access = peers.get(peer).ok_or(AccessError::NotTrusted)?;
        if access.blocked {
            return Err(AccessError::Blocked);
        }
        if !access.trusted {
            return Err(AccessError::NotTrusted);
        }
        if access.grants.is_empty() {
            return Err(AccessError::NoGrants);
        }
        Ok(SearchAuthorization::new(
            peer.to_string(),
            access.grants.iter().copied().collect(),
            purpose,
            self.disclosure_gate.clone(),
        ))
    }
}

impl Default for AccessControl {
    fn default() -> Self {
        Self::with_disclosure_gate(DisclosureGate::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_require_trust_and_revocation_is_fail_closed() {
        let acl = AccessControl::default();
        let peer = PeerId::random();
        let collection = Uuid::new_v4();

        assert_eq!(acl.grant(peer, collection), Err(AccessError::NotTrusted));
        acl.mark_trusted(peer);
        acl.grant(peer, collection).unwrap();
        let auth = acl.authorize(&peer, SearchPurpose::LocalAssistant).unwrap();
        assert_eq!(auth.allowed_collections, vec![collection]);
        let external = acl.authorize(&peer, SearchPurpose::ExternalAi).unwrap();
        assert_eq!(external.purpose, SearchPurpose::ExternalAi);
        assert_eq!(external.allowed_collections, vec![collection]);

        acl.revoke_and_block(peer);
        assert!(matches!(
            acl.authorize(&peer, SearchPurpose::LocalAssistant),
            Err(AccessError::Blocked)
        ));
        assert!(acl.state(&peer).grants.is_empty());
    }

    #[test]
    fn authorization_lease_delays_revocation_completion() {
        use std::sync::mpsc;

        let acl = AccessControl::default();
        let peer = PeerId::random();
        let collection = Uuid::new_v4();
        acl.mark_trusted(peer);
        acl.grant(peer, collection).unwrap();
        let authorization = acl.authorize(&peer, SearchPurpose::LocalAssistant).unwrap();
        let lease = authorization.acquire_disclosure_lease();
        let (finished_tx, finished_rx) = mpsc::channel();
        let revoking_acl = acl.clone();
        let revocation = std::thread::spawn(move || {
            revoking_acl.revoke_and_block(peer);
            finished_tx.send(()).ok();
        });

        assert!(
            finished_rx
                .recv_timeout(std::time::Duration::from_millis(25))
                .is_err()
        );
        drop(lease);
        finished_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        revocation.join().unwrap();
        assert!(acl.is_blocked(&peer));
    }
}
