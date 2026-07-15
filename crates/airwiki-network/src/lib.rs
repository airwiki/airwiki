//! Authenticated, collection-scoped AirWiki knowledge search over a local network.
//!
//! The crate deliberately implements a narrow libp2p stack: mDNS discovery, TCP,
//! Noise, Yamux and bounded request/response CBOR. It has no DHT, gossip, relay,
//! UPnP, QUIC or document replication.

use std::time::Duration;

use thiserror::Error;

pub mod access;
pub mod address;
pub mod codec;
pub mod coordinator;
pub mod identity;
pub mod pairing;
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
pub use libp2p::{Multiaddr, PeerId};
pub use pairing::{PAIRING_PROTOCOL, PAIRING_TTL, PairingCode, PairingNonce, derive_sas};
pub use runtime::{
    AuthorizedSearchBackend, AuthorizedSearchResult, NetworkConfig, NetworkEvent, NetworkHandle,
    NetworkWarningKind, PairingFailureReason, spawn_network,
};

pub const SEARCH_DEADLINE: Duration = Duration::from_secs(3);

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
