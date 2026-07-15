//! Human-verifiable pairing over an already authenticated Noise channel.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bip39::Language;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const PAIRING_PROTOCOL: &str = "/airwiki/pairing/1.0.0";
pub const PAIRING_TTL: Duration = Duration::from_secs(120);
/// Maximum simultaneous, untrusted SAS sessions retained in memory.
pub const MAX_PENDING_PAIRINGS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingNonce(pub [u8; 32]);

impl PairingNonce {
    pub fn random() -> Self {
        Self(rand::random())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PairingRequest {
    Hello {
        nonce: PairingNonce,
        node_name: String,
    },
    Confirm {
        session_hash: [u8; 32],
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PairingResponse {
    Hello {
        nonce: PairingNonce,
        node_name: String,
    },
    Confirmed {
        both_confirmed: bool,
    },
    Rejected {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub words: [String; 6],
    pub session_hash: [u8; 32],
}

impl std::fmt::Display for PairingCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.words.join(" "))
    }
}

#[derive(Debug, Clone)]
pub struct PendingPairing {
    pub peer: PeerId,
    pub remote_node_name: String,
    pub local_nonce: PairingNonce,
    pub remote_nonce: PairingNonce,
    pub expires_at: Instant,
    pub local_confirmed: bool,
    pub remote_confirmed: bool,
}

impl PendingPairing {
    pub fn code(&self, local_peer: &PeerId) -> PairingCode {
        derive_sas(local_peer, self.local_nonce, &self.peer, self.remote_nonce)
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }

    pub fn is_complete(&self) -> bool {
        self.local_confirmed && self.remote_confirmed
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PairingError {
    #[error("there is no pending pairing for this peer")]
    NotPending,
    #[error("pairing expired")]
    Expired,
    #[error("pairing session hash does not match")]
    SessionMismatch,
    #[error("pairing capacity is exhausted")]
    CapacityExceeded,
}

/// Runtime-only pairing state. Durable trust begins only after both confirmations.
#[derive(Debug)]
pub struct PairingCoordinator {
    local_peer: PeerId,
    pending: HashMap<PeerId, PendingPairing>,
}

impl PairingCoordinator {
    pub fn new(local_peer: PeerId) -> Self {
        Self {
            local_peer,
            pending: HashMap::new(),
        }
    }

    pub fn begin_outgoing(&mut self, peer: PeerId) -> PairingNonce {
        let nonce = PairingNonce::random();
        // The full session is inserted after the Hello response supplies its nonce.
        self.pending.remove(&peer);
        nonce
    }

    pub fn accept_hello(
        &mut self,
        peer: PeerId,
        remote_nonce: PairingNonce,
        remote_node_name: impl Into<String>,
        now: Instant,
    ) -> Result<PairingNonce, PairingError> {
        let local_nonce = PairingNonce::random();
        self.insert_session(peer, local_nonce, remote_nonce, remote_node_name, now)?;
        Ok(local_nonce)
    }

    pub fn accept_hello_response(
        &mut self,
        peer: PeerId,
        local_nonce: PairingNonce,
        remote_nonce: PairingNonce,
        remote_node_name: impl Into<String>,
        now: Instant,
    ) -> Result<(), PairingError> {
        self.insert_session(peer, local_nonce, remote_nonce, remote_node_name, now)
    }

    fn insert_session(
        &mut self,
        peer: PeerId,
        local_nonce: PairingNonce,
        remote_nonce: PairingNonce,
        remote_node_name: impl Into<String>,
        now: Instant,
    ) -> Result<(), PairingError> {
        if !self.pending.contains_key(&peer) && self.pending.len() >= MAX_PENDING_PAIRINGS {
            return Err(PairingError::CapacityExceeded);
        }
        let remote_node_name = remote_node_name.into().trim().chars().take(128).collect();
        self.pending.insert(
            peer,
            PendingPairing {
                peer,
                remote_node_name,
                local_nonce,
                remote_nonce,
                expires_at: now + PAIRING_TTL,
                local_confirmed: false,
                remote_confirmed: false,
            },
        );
        Ok(())
    }

    pub fn pending(&self, peer: &PeerId, now: Instant) -> Result<&PendingPairing, PairingError> {
        let pending = self.pending.get(peer).ok_or(PairingError::NotPending)?;
        if pending.is_expired(now) {
            return Err(PairingError::Expired);
        }
        Ok(pending)
    }

    pub fn confirm_local(
        &mut self,
        peer: &PeerId,
        now: Instant,
    ) -> Result<PairingCode, PairingError> {
        let local_peer = self.local_peer;
        let pending = self.pending.get_mut(peer).ok_or(PairingError::NotPending)?;
        if pending.is_expired(now) {
            return Err(PairingError::Expired);
        }
        pending.local_confirmed = true;
        Ok(pending.code(&local_peer))
    }

    pub fn confirm_remote(
        &mut self,
        peer: &PeerId,
        session_hash: [u8; 32],
        now: Instant,
    ) -> Result<bool, PairingError> {
        let local_peer = self.local_peer;
        let pending = self.pending.get_mut(peer).ok_or(PairingError::NotPending)?;
        if pending.is_expired(now) {
            return Err(PairingError::Expired);
        }
        if pending.code(&local_peer).session_hash != session_hash {
            return Err(PairingError::SessionMismatch);
        }
        pending.remote_confirmed = true;
        Ok(pending.is_complete())
    }

    pub fn is_complete(&self, peer: &PeerId, now: Instant) -> bool {
        self.pending
            .get(peer)
            .is_some_and(|pending| !pending.is_expired(now) && pending.is_complete())
    }

    pub fn remove(&mut self, peer: &PeerId) {
        self.pending.remove(peer);
    }

    pub fn expire(&mut self, now: Instant) -> Vec<PeerId> {
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(peer, pending)| pending.is_expired(now).then_some(*peer))
            .collect();
        self.pending.retain(|_, pending| !pending.is_expired(now));
        expired
    }
}

/// Derive a symmetric six-word SAS while retaining which nonce belongs to each PeerId.
pub fn derive_sas(
    peer_a: &PeerId,
    nonce_a: PairingNonce,
    peer_b: &PeerId,
    nonce_b: PairingNonce,
) -> PairingCode {
    let mut sides = [
        (peer_a.to_bytes(), nonce_a.0),
        (peer_b.to_bytes(), nonce_b.0),
    ];
    sides.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    hasher.update(b"airwiki-pairing-sas-v1\0");
    for (peer, nonce) in sides {
        hasher.update((peer.len() as u16).to_be_bytes());
        hasher.update(peer);
        hasher.update(nonce);
    }
    let session_hash: [u8; 32] = hasher.finalize().into();
    let word_list = Language::English.word_list();
    let words = std::array::from_fn(|word_index| {
        let first_bit = word_index * 11;
        let mut index = 0_usize;
        for offset in 0..11 {
            let bit_index = first_bit + offset;
            let bit = (session_hash[bit_index / 8] >> (7 - (bit_index % 8))) & 1;
            index = (index << 1) | usize::from(bit);
        }
        word_list[index].to_owned()
    });
    PairingCode {
        words,
        session_hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sas_is_symmetric_and_has_six_words() {
        let left = PeerId::random();
        let right = PeerId::random();
        let left_nonce = PairingNonce([1; 32]);
        let right_nonce = PairingNonce([2; 32]);
        let first = derive_sas(&left, left_nonce, &right, right_nonce);
        let second = derive_sas(&right, right_nonce, &left, left_nonce);
        assert_eq!(first, second);
        assert_eq!(first.to_string().split_whitespace().count(), 6);
    }

    #[test]
    fn sas_binds_nonce_to_peer() {
        let left = PeerId::random();
        let right = PeerId::random();
        let first = derive_sas(&left, PairingNonce([1; 32]), &right, PairingNonce([2; 32]));
        let swapped_nonces =
            derive_sas(&left, PairingNonce([2; 32]), &right, PairingNonce([1; 32]));
        assert_ne!(first, swapped_nonces);
    }

    #[test]
    fn coordinator_rejects_new_untrusted_session_at_capacity() {
        let mut coordinator = PairingCoordinator::new(PeerId::random());
        let now = Instant::now();
        for _ in 0..MAX_PENDING_PAIRINGS {
            coordinator
                .accept_hello(
                    PeerId::random(),
                    PairingNonce([1; 32]),
                    "Nearby device",
                    now,
                )
                .unwrap();
        }

        let error = coordinator
            .accept_hello(PeerId::random(), PairingNonce([2; 32]), "Extra device", now)
            .unwrap_err();

        assert_eq!(error, PairingError::CapacityExceeded);
        assert_eq!(coordinator.pending.len(), MAX_PENDING_PAIRINGS);
    }

    #[test]
    fn coordinator_allows_existing_peer_to_restart_session_at_capacity() {
        let mut coordinator = PairingCoordinator::new(PeerId::random());
        let now = Instant::now();
        let existing = PeerId::random();
        coordinator
            .accept_hello(existing, PairingNonce([1; 32]), "Old name", now)
            .unwrap();
        for _ in 1..MAX_PENDING_PAIRINGS {
            coordinator
                .accept_hello(
                    PeerId::random(),
                    PairingNonce([1; 32]),
                    "Nearby device",
                    now,
                )
                .unwrap();
        }

        let result = coordinator.accept_hello(existing, PairingNonce([2; 32]), "Updated name", now);

        assert!(result.is_ok(), "existing peer was rejected: {result:?}");
        assert_eq!(coordinator.pending.len(), MAX_PENDING_PAIRINGS);
    }

    #[test]
    fn expiring_sessions_releases_pairing_capacity() {
        let mut coordinator = PairingCoordinator::new(PeerId::random());
        let now = Instant::now();
        for _ in 0..MAX_PENDING_PAIRINGS {
            coordinator
                .accept_hello(
                    PeerId::random(),
                    PairingNonce([1; 32]),
                    "Nearby device",
                    now,
                )
                .unwrap();
        }
        coordinator.expire(now + PAIRING_TTL);

        let result = coordinator.accept_hello(
            PeerId::random(),
            PairingNonce([2; 32]),
            "New device",
            now + PAIRING_TTL,
        );

        assert!(
            result.is_ok(),
            "released capacity was not reusable: {result:?}"
        );
    }
}
