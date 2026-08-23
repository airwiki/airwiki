//! Persistent libp2p identity without putting private key material in SQLite.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(feature = "os-keyring")]
use base64::Engine;
#[cfg(feature = "os-keyring")]
use base64::engine::general_purpose::STANDARD as BASE64;
use libp2p::PeerId;
use libp2p::identity::Keypair;
use parking_lot::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("secret store failed: {0}")]
    SecretStore(String),
    #[error("stored libp2p identity is invalid: {0}")]
    InvalidIdentity(String),
    #[error("could not serialize libp2p identity: {0}")]
    SerializeIdentity(String),
}

pub trait SecretStore: Send + Sync + 'static {
    fn get_secret(&self, key: &str) -> Result<Option<Vec<u8>>, IdentityError>;
    fn set_secret(&self, key: &str, value: &[u8]) -> Result<(), IdentityError>;
    fn delete_secret(&self, key: &str) -> Result<(), IdentityError>;
}

#[derive(Debug, Clone, Default)]
pub struct MemorySecretStore {
    inner: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl SecretStore for MemorySecretStore {
    fn get_secret(&self, key: &str) -> Result<Option<Vec<u8>>, IdentityError> {
        Ok(self.inner.lock().get(key).cloned())
    }

    fn set_secret(&self, key: &str, value: &[u8]) -> Result<(), IdentityError> {
        self.inner.lock().insert(key.to_owned(), value.to_vec());
        Ok(())
    }

    fn delete_secret(&self, key: &str) -> Result<(), IdentityError> {
        self.inner.lock().remove(key);
        Ok(())
    }
}

/// Development/test fallback. Production callers should use [`KeyringSecretStore`].
/// Files are permission-restricted but not encrypted at rest.
#[derive(Debug, Clone)]
pub struct FileSecretStore {
    directory: PathBuf,
}

impl FileSecretStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        let safe_name: String = key
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        self.directory.join(safe_name)
    }

    fn write_restricted(path: &Path, value: &[u8]) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = fs::OpenOptions::new();
            options.create(true).truncate(true).write(true).mode(0o600);
            let mut file = options.open(path)?;
            io::Write::write_all(&mut file, value)?;
            file.sync_all()
        }
        #[cfg(not(unix))]
        {
            fs::write(path, value)
        }
    }
}

impl SecretStore for FileSecretStore {
    fn get_secret(&self, key: &str) -> Result<Option<Vec<u8>>, IdentityError> {
        match fs::read(self.path_for(key)) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(IdentityError::SecretStore(error.to_string())),
        }
    }

    fn set_secret(&self, key: &str, value: &[u8]) -> Result<(), IdentityError> {
        fs::create_dir_all(&self.directory)
            .map_err(|error| IdentityError::SecretStore(error.to_string()))?;
        let destination = self.path_for(key);
        let temporary = destination.with_extension("tmp");
        Self::write_restricted(&temporary, value)
            .map_err(|error| IdentityError::SecretStore(error.to_string()))?;
        fs::rename(&temporary, &destination)
            .map_err(|error| IdentityError::SecretStore(error.to_string()))
    }

    fn delete_secret(&self, key: &str) -> Result<(), IdentityError> {
        match fs::remove_file(self.path_for(key)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(IdentityError::SecretStore(error.to_string())),
        }
    }
}

#[cfg(feature = "os-keyring")]
#[derive(Debug, Clone)]
pub struct KeyringSecretStore {
    service: String,
    account: String,
}

#[cfg(feature = "os-keyring")]
impl KeyringSecretStore {
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, IdentityError> {
        keyring::Entry::new(&self.service, &format!("{}:{key}", self.account))
            .map_err(|error| IdentityError::SecretStore(error.to_string()))
    }
}

#[cfg(feature = "os-keyring")]
impl SecretStore for KeyringSecretStore {
    fn get_secret(&self, key: &str) -> Result<Option<Vec<u8>>, IdentityError> {
        match self.entry(key)?.get_password() {
            Ok(encoded) => BASE64
                .decode(encoded)
                .map(Some)
                .map_err(|error| IdentityError::SecretStore(error.to_string())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(IdentityError::SecretStore(error.to_string())),
        }
    }

    fn set_secret(&self, key: &str, value: &[u8]) -> Result<(), IdentityError> {
        self.entry(key)?
            .set_password(&BASE64.encode(value))
            .map_err(|error| IdentityError::SecretStore(error.to_string()))
    }

    fn delete_secret(&self, key: &str) -> Result<(), IdentityError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(IdentityError::SecretStore(error.to_string())),
        }
    }
}

#[derive(Clone)]
pub struct NodeIdentity {
    keypair: Keypair,
    peer_id: PeerId,
}

impl std::fmt::Debug for NodeIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeIdentity")
            .field("peer_id", &self.peer_id)
            .finish_non_exhaustive()
    }
}

impl NodeIdentity {
    pub const DEFAULT_SECRET_KEY: &'static str = "libp2p-ed25519-v1";
    pub const PUBLIC_PUBLISHER_SECRET_KEY: &'static str = "libp2p-public-publisher-ed25519-v1";

    pub fn load_or_create(store: &dyn SecretStore) -> Result<Self, IdentityError> {
        Self::load_or_create_at(store, Self::DEFAULT_SECRET_KEY)
    }

    pub fn load_or_create_public_publisher(store: &dyn SecretStore) -> Result<Self, IdentityError> {
        Self::load_or_create_at(store, Self::PUBLIC_PUBLISHER_SECRET_KEY)
    }

    pub fn load_or_create_at(
        store: &dyn SecretStore,
        storage_key: &str,
    ) -> Result<Self, IdentityError> {
        let keypair = if let Some(encoded) = store.get_secret(storage_key)? {
            Keypair::from_protobuf_encoding(&encoded)
                .map_err(|error| IdentityError::InvalidIdentity(error.to_string()))?
        } else {
            let keypair = Keypair::generate_ed25519();
            let encoded = keypair
                .to_protobuf_encoding()
                .map_err(|error| IdentityError::SerializeIdentity(error.to_string()))?;
            store.set_secret(storage_key, &encoded)?;
            keypair
        };
        let peer_id = keypair.public().to_peer_id();
        Ok(Self { keypair, peer_id })
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_stable_in_secret_store() {
        let store = MemorySecretStore::default();
        let first = NodeIdentity::load_or_create(&store).unwrap();
        let second = NodeIdentity::load_or_create(&store).unwrap();
        assert_eq!(first.peer_id(), second.peer_id());
    }

    #[test]
    fn file_store_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let store = FileSecretStore::new(temp.path());
        store.set_secret("identity", b"private").unwrap();
        assert_eq!(
            store.get_secret("identity").unwrap(),
            Some(b"private".to_vec())
        );
        store.delete_secret("identity").unwrap();
        assert_eq!(store.get_secret("identity").unwrap(), None);
    }
}
