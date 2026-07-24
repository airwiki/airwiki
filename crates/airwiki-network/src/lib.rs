//! Authenticated, collection-scoped AirWiki knowledge search over a local network.
//!
//! LAN and opt-in public federation use separate protocols and identities. The
//! public stack adds QUIC and relay traversal without widening LAN grants or
//! replicating documents.

use std::time::Duration;

use thiserror::Error;

pub mod access;
pub mod address;
pub mod codec;
pub mod coordinator;
pub mod identity;
pub mod pairing;
pub mod public_catalog;
pub mod public_manifest;
pub mod public_reader;
pub mod public_source;
pub mod rate_limit;
pub mod runtime;

pub use access::{AccessControl, AccessError, PeerAccess};
pub use address::{
    LanAddressError, MAX_MANUAL_LAN_ADDRESS_BYTES, MAX_MDNS_ADDRESSES_PER_PEER,
    MAX_VOLATILE_LAN_PEERS, ManualLanAddress, PeerAddressBook,
};
pub use coordinator::FederatedCoordinator;
#[cfg(feature = "os-keyring")]
pub use identity::KeyringSecretStore;
pub use identity::{FileSecretStore, IdentityError, MemorySecretStore, NodeIdentity, SecretStore};
pub use libp2p::identity::Keypair;
pub use libp2p::{Multiaddr, PeerId};
pub use pairing::{PAIRING_PROTOCOL, PAIRING_TTL, PairingCode, PairingNonce, derive_sas};
pub use public_catalog::{
    CatalogRejection, CatalogWireRequest, CatalogWireResponse, PublicCatalogBackend,
    PublicCatalogBackendError, PublicCatalogServerConfig, run_public_catalog_server,
    validate_public_relay_external_address,
};
pub use public_manifest::{
    PublicManifestError, sign_manifest, sign_tombstone, verify_manifest, verify_tombstone,
};
pub use public_reader::{
    PublicBrowseResult, PublicCollectionAvailability, PublicIndexEndpoint, PublicReader,
    PublicRouteKind,
};
pub use public_source::{
    PublicBrowseDelivery, PublicBrowseWireResponse, PublicSearchDelivery, PublicSearchWireResponse,
    PublicSourceBackend, PublicSourceBackendError, PublicSourceRejection, PublicSourceServerConfig,
    run_public_source_server,
};
pub use rate_limit::PeerRateLimiter;
pub use runtime::{
    AuthorizedSearchBackend, AuthorizedSearchResult, NetworkConfig, NetworkEvent, NetworkHandle,
    NetworkWarningKind, PairingFailureReason, spawn_network,
};

pub const SEARCH_DEADLINE: Duration = Duration::from_secs(3);

/// Converts a relay endpoint into the circuit-listen address used by a
/// publisher. Keeping this detail here avoids leaking libp2p protocol internals
/// into application composition code.
pub fn relay_circuit_address(mut address: Multiaddr, relay_peer: PeerId) -> Multiaddr {
    if !address
        .iter()
        .any(|protocol| matches!(protocol, libp2p::multiaddr::Protocol::P2p(_)))
    {
        address.push(libp2p::multiaddr::Protocol::P2p(relay_peer));
    }
    address.push(libp2p::multiaddr::Protocol::P2pCircuit);
    address
}

/// Builds the routable address advertised for a peer listening through a
/// circuit relay.
pub fn relayed_peer_address(
    address: Multiaddr,
    relay_peer: PeerId,
    target_peer: PeerId,
) -> Multiaddr {
    let mut address = relay_circuit_address(address, relay_peer);
    address.push(libp2p::multiaddr::Protocol::P2p(target_peer));
    address
}

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error(transparent)]
    Address(#[from] LanAddressError),
    #[error(transparent)]
    Contract(#[from] airwiki_types::SearchContractError),
    #[error("LAN runtime is not running")]
    RuntimeStopped,
    #[error("transport setup failed: {0}")]
    Transport(String),
    #[error("mDNS setup failed: {0}")]
    Mdns(String),
    #[error("could not listen on LAN: {0}")]
    Listen(String),
    #[error("peer access denied: {0}")]
    Access(String),
}
