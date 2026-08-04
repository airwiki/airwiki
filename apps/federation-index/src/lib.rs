use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use airwiki_network::{PublicCatalogBackend, PublicCatalogBackendError};
use airwiki_network::{verify_manifest, verify_tombstone};
use airwiki_types::{
    MAX_PUBLIC_CANDIDATES, MAX_PUBLIC_MANIFEST_LIFETIME_SECONDS, PublicCatalogQuery,
    SignedPublicCollectionManifest, SignedPublicCollectionTombstone,
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

const MAX_COLLECTIONS_PER_PUBLISHER: u32 = 1_000;
const MAX_CATALOG_ENTRIES: u32 = 100_000;

#[derive(Debug, Error)]
pub enum CatalogStoreError {
    #[error("public catalog persistence failed")]
    Persistence(#[from] rusqlite::Error),
    #[error("public catalog payload encoding failed")]
    Encoding,
    #[error("public catalog manifest verification failed")]
    Verification,
    #[error("public catalog update is stale or replayed")]
    StaleSequence,
    #[error("public catalog query is invalid")]
    InvalidQuery,
    #[error("public catalog publisher registration limit reached")]
    PublisherLimit,
    #[error("public catalog capacity limit reached")]
    CapacityLimit,
    #[error("public catalog lock is unavailable")]
    Lock,
}

#[derive(Debug)]
pub struct CatalogStore {
    connection: Mutex<Connection>,
    limits: CatalogLimits,
}

#[derive(Debug, Clone, Copy)]
struct CatalogLimits {
    max_entries: u32,
    max_collections_per_publisher: u32,
}

impl Default for CatalogLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_CATALOG_ENTRIES,
            max_collections_per_publisher: MAX_COLLECTIONS_PER_PUBLISHER,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CatalogBackend {
    store: std::sync::Arc<CatalogStore>,
}

impl CatalogBackend {
    pub fn new(store: std::sync::Arc<CatalogStore>) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl PublicCatalogBackend for CatalogBackend {
    async fn register(
        &self,
        manifest: SignedPublicCollectionManifest,
    ) -> Result<(), PublicCatalogBackendError> {
        let store = std::sync::Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || store.register(&manifest, Utc::now()))
            .await
            .map_err(map_catalog_join_error)?
            .map_err(map_backend_error)
    }

    async fn withdraw(
        &self,
        tombstone: SignedPublicCollectionTombstone,
    ) -> Result<(), PublicCatalogBackendError> {
        let store = std::sync::Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || store.withdraw(&tombstone))
            .await
            .map_err(map_catalog_join_error)?
            .map_err(map_backend_error)
    }

    async fn query(
        &self,
        query: PublicCatalogQuery,
    ) -> Result<Vec<SignedPublicCollectionManifest>, PublicCatalogBackendError> {
        let store = std::sync::Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            let now = Utc::now();
            let _ = store.purge_expired(now)?;
            store.query(&query, now)
        })
        .await
        .map_err(map_catalog_join_error)?
        .map_err(map_backend_error)
    }
}

fn map_catalog_join_error(_: tokio::task::JoinError) -> PublicCatalogBackendError {
    tracing::error!(
        error_kind = "catalog_worker_failed",
        "public catalog blocking worker failed"
    );
    PublicCatalogBackendError::Internal
}

fn catalog_error_kind(error: &CatalogStoreError) -> &'static str {
    match error {
        CatalogStoreError::Persistence(_) => "catalog_persistence_failed",
        CatalogStoreError::Encoding => "catalog_encoding_failed",
        CatalogStoreError::Verification => "catalog_verification_rejected",
        CatalogStoreError::StaleSequence => "catalog_stale_rejected",
        CatalogStoreError::InvalidQuery => "catalog_query_rejected",
        CatalogStoreError::PublisherLimit | CatalogStoreError::CapacityLimit => {
            "catalog_capacity_rejected"
        }
        CatalogStoreError::Lock => "catalog_busy",
    }
}

fn map_backend_error(error: CatalogStoreError) -> PublicCatalogBackendError {
    let error_kind = catalog_error_kind(&error);
    tracing::warn!(error_kind, "public catalog operation failed");
    match error {
        CatalogStoreError::Verification
        | CatalogStoreError::InvalidQuery
        | CatalogStoreError::PublisherLimit
        | CatalogStoreError::CapacityLimit => PublicCatalogBackendError::Invalid,
        CatalogStoreError::StaleSequence => PublicCatalogBackendError::Stale,
        CatalogStoreError::Lock => PublicCatalogBackendError::Busy,
        CatalogStoreError::Persistence(_) | CatalogStoreError::Encoding => {
            PublicCatalogBackendError::Internal
        }
    }
}

impl CatalogStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CatalogStoreError> {
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        Self::initialize(connection, CatalogLimits::default())
    }

    pub fn in_memory() -> Result<Self, CatalogStoreError> {
        Self::initialize(Connection::open_in_memory()?, CatalogLimits::default())
    }

    #[cfg(test)]
    fn in_memory_with_limits(limits: CatalogLimits) -> Result<Self, CatalogStoreError> {
        Self::initialize(Connection::open_in_memory()?, limits)
    }

    fn initialize(
        connection: Connection,
        limits: CatalogLimits,
    ) -> Result<Self, CatalogStoreError> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS manifests(
                manifest_id INTEGER PRIMARY KEY,
                publisher_id TEXT NOT NULL,
                collection_id TEXT NOT NULL,
                sequence INTEGER NOT NULL CHECK(sequence >= 0),
                signed_cbor BLOB,
                expires_at TEXT,
                withdrawn INTEGER NOT NULL DEFAULT 0,
                UNIQUE(publisher_id,collection_id)
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS catalog_fts USING fts5(
                publisher_id UNINDEXED,collection_id UNINDEXED,name,description,routing_terms,
                tokenize='unicode61 remove_diacritics 2'
             );
             CREATE INDEX IF NOT EXISTS manifests_expiry_idx ON manifests(expires_at);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            limits,
        })
    }

    pub fn register(
        &self,
        signed: &SignedPublicCollectionManifest,
        now: DateTime<Utc>,
    ) -> Result<(), CatalogStoreError> {
        verify_manifest(signed, now).map_err(|_| CatalogStoreError::Verification)?;
        let manifest = &signed.manifest;
        let encoded = encode(signed)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        purge_catalog_entries(&tx, now)?;
        let known_id = tx
            .query_row(
                "SELECT manifest_id FROM manifests WHERE publisher_id=?1 AND collection_id=?2",
                params![manifest.publisher_id, manifest.collection_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if known_id.is_none() {
            ensure_publisher_capacity(
                &tx,
                &manifest.publisher_id,
                self.limits.max_collections_per_publisher,
            )?;
            ensure_catalog_capacity(&tx, self.limits.max_entries)?;
        }
        reject_stale(
            &tx,
            &manifest.publisher_id,
            &manifest.collection_id.to_string(),
            manifest.sequence,
        )?;
        let manifest_id = if let Some(manifest_id) = known_id {
            tx.execute(
                "UPDATE manifests SET sequence=?2,signed_cbor=?3,expires_at=?4,withdrawn=0
                 WHERE manifest_id=?1",
                params![
                    manifest_id,
                    manifest.sequence,
                    encoded,
                    manifest.expires_at.to_rfc3339(),
                ],
            )?;
            tx.execute("DELETE FROM catalog_fts WHERE rowid=?1", [manifest_id])?;
            manifest_id
        } else {
            tx.execute(
                "INSERT INTO manifests(publisher_id,collection_id,sequence,signed_cbor,expires_at,withdrawn)
                 VALUES (?1,?2,?3,?4,?5,0)",
                params![
                    manifest.publisher_id,
                    manifest.collection_id.to_string(),
                    manifest.sequence,
                    encoded,
                    manifest.expires_at.to_rfc3339(),
                ],
            )?;
            tx.last_insert_rowid()
        };
        tx.execute(
            "INSERT INTO catalog_fts(rowid,publisher_id,collection_id,name,description,routing_terms)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                manifest_id,
                manifest.publisher_id,
                manifest.collection_id.to_string(),
                manifest.name,
                manifest.description,
                manifest.routing_terms.join(" "),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn withdraw(
        &self,
        signed: &SignedPublicCollectionTombstone,
    ) -> Result<(), CatalogStoreError> {
        self.withdraw_at(signed, Utc::now())
    }

    fn withdraw_at(
        &self,
        signed: &SignedPublicCollectionTombstone,
        now: DateTime<Utc>,
    ) -> Result<(), CatalogStoreError> {
        verify_tombstone(signed).map_err(|_| CatalogStoreError::Verification)?;
        let tombstone = &signed.tombstone;
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        purge_catalog_entries(&tx, now)?;
        reject_stale(
            &tx,
            &tombstone.publisher_id,
            &tombstone.collection_id.to_string(),
            tombstone.sequence,
        )?;
        let known_id = tx
            .query_row(
                "SELECT manifest_id FROM manifests WHERE publisher_id=?1 AND collection_id=?2",
                params![tombstone.publisher_id, tombstone.collection_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(manifest_id) = known_id {
            tx.execute(
                "UPDATE manifests SET sequence=?2,signed_cbor=NULL,expires_at=NULL,withdrawn=1
                 WHERE manifest_id=?1",
                params![manifest_id, tombstone.sequence],
            )?;
            tx.execute("DELETE FROM catalog_fts WHERE rowid=?1", [manifest_id])?;
        } else {
            ensure_publisher_capacity(
                &tx,
                &tombstone.publisher_id,
                self.limits.max_collections_per_publisher,
            )?;
            ensure_catalog_capacity(&tx, self.limits.max_entries)?;
            tx.execute(
                "INSERT INTO manifests(publisher_id,collection_id,sequence,signed_cbor,expires_at,withdrawn)
                 VALUES (?1,?2,?3,NULL,NULL,1)",
                params![
                    tombstone.publisher_id,
                    tombstone.collection_id.to_string(),
                    tombstone.sequence,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn query(
        &self,
        query: &PublicCatalogQuery,
        now: DateTime<Utc>,
    ) -> Result<Vec<SignedPublicCollectionManifest>, CatalogStoreError> {
        query
            .validate()
            .map_err(|_| CatalogStoreError::InvalidQuery)?;
        let fts = fts_query(&query.query);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT m.signed_cbor FROM catalog_fts f
             JOIN manifests m ON m.manifest_id=f.rowid
             WHERE catalog_fts MATCH ?1 AND m.withdrawn=0 AND m.expires_at>?2
             ORDER BY bm25(catalog_fts),m.publisher_id,m.collection_id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                fts,
                now.to_rfc3339(),
                i64::from(query.limit.min(MAX_PUBLIC_CANDIDATES)),
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        let mut manifests = Vec::new();
        for row in rows {
            let signed: SignedPublicCollectionManifest = decode(&row?)?;
            if !query.languages.is_empty()
                && !signed
                    .manifest
                    .languages
                    .iter()
                    .any(|language| query.languages.contains(language))
            {
                continue;
            }
            manifests.push(signed);
        }
        Ok(manifests)
    }

    pub fn purge_expired(&self, now: DateTime<Utc>) -> Result<usize, CatalogStoreError> {
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let expired = purge_catalog_entries(&tx, now)?;
        tx.commit()?;
        Ok(expired)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, CatalogStoreError> {
        self.connection.lock().map_err(|_| CatalogStoreError::Lock)
    }
}

fn manifest_expiry_limit(now: DateTime<Utc>) -> DateTime<Utc> {
    now + chrono::Duration::seconds(MAX_PUBLIC_MANIFEST_LIFETIME_SECONDS)
}

fn purge_catalog_entries(
    connection: &Connection,
    now: DateTime<Utc>,
) -> Result<usize, CatalogStoreError> {
    let mut statement = connection.prepare(
        "SELECT manifest_id FROM manifests
         WHERE withdrawn=0 AND expires_at IS NOT NULL
           AND (expires_at<=?1 OR expires_at>?2)",
    )?;
    let expired = statement
        .query_map(
            params![now.to_rfc3339(), manifest_expiry_limit(now).to_rfc3339()],
            |row| row.get::<_, i64>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for manifest_id in &expired {
        connection.execute("DELETE FROM catalog_fts WHERE rowid=?1", [manifest_id])?;
        connection.execute("DELETE FROM manifests WHERE manifest_id=?1", [manifest_id])?;
    }
    Ok(expired.len())
}

fn ensure_catalog_capacity(
    connection: &Connection,
    max_entries: u32,
) -> Result<(), CatalogStoreError> {
    let entry_count = connection.query_row("SELECT count(*) FROM manifests", [], |row| {
        row.get::<_, u32>(0)
    })?;
    if entry_count >= max_entries {
        return Err(CatalogStoreError::CapacityLimit);
    }
    Ok(())
}

fn ensure_publisher_capacity(
    connection: &Connection,
    publisher_id: &str,
    max_collections: u32,
) -> Result<(), CatalogStoreError> {
    let publisher_count = connection.query_row(
        "SELECT count(*) FROM manifests WHERE publisher_id=?1",
        [publisher_id],
        |row| row.get::<_, u32>(0),
    )?;
    if publisher_count >= max_collections {
        return Err(CatalogStoreError::PublisherLimit);
    }
    Ok(())
}

fn reject_stale(
    connection: &Connection,
    publisher_id: &str,
    collection_id: &str,
    sequence: u64,
) -> Result<(), CatalogStoreError> {
    let previous = connection
        .query_row(
            "SELECT sequence FROM manifests WHERE publisher_id=?1 AND collection_id=?2",
            params![publisher_id, collection_id],
            |row| row.get::<_, u64>(0),
        )
        .optional()?;
    if previous.is_some_and(|previous| sequence <= previous) {
        return Err(CatalogStoreError::StaleSequence);
    }
    Ok(())
}

fn fts_query(query: &str) -> String {
    query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2)
        .take(16)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn encode(value: &impl serde::Serialize) -> Result<Vec<u8>, CatalogStoreError> {
    let mut encoded = Vec::new();
    ciborium::into_writer(value, &mut encoded).map_err(|_| CatalogStoreError::Encoding)?;
    Ok(encoded)
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, CatalogStoreError> {
    ciborium::from_reader(bytes).map_err(|_| CatalogStoreError::Encoding)
}

#[cfg(test)]
mod tests {
    use airwiki_network::{sign_manifest, sign_tombstone};
    use airwiki_types::{
        PUBLIC_CATALOG_PROTOCOL, PublicCollectionManifest, PublicCollectionTombstone,
    };
    use chrono::Duration;
    use libp2p::identity::Keypair;
    use uuid::Uuid;

    use super::*;

    fn signed_manifest(
        keypair: &Keypair,
        collection_id: Uuid,
        sequence: u64,
        now: DateTime<Utc>,
    ) -> SignedPublicCollectionManifest {
        signed_manifest_with_expiry(
            keypair,
            collection_id,
            sequence,
            now,
            now + Duration::minutes(15),
        )
    }

    fn signed_manifest_with_expiry(
        keypair: &Keypair,
        collection_id: Uuid,
        sequence: u64,
        updated_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> SignedPublicCollectionManifest {
        sign_manifest(
            keypair,
            PublicCollectionManifest {
                protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
                publisher_id: keypair.public().to_peer_id().to_string(),
                collection_id,
                sequence,
                publication_fingerprint: "a".repeat(64),
                name: "Atlas runbooks".to_owned(),
                description: "Synthetic routing fixture".to_owned(),
                languages: vec!["en".to_owned()],
                concept_count: 3,
                routing_terms: vec!["atlas".to_owned(), "recovery".to_owned()],
                routes: vec!["/ip4/127.0.0.1/tcp/42043".to_owned()],
                updated_at,
                expires_at,
            },
        )
        .unwrap()
    }

    fn query(term: &str) -> PublicCatalogQuery {
        PublicCatalogQuery {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            request_id: Uuid::new_v4(),
            query: term.to_owned(),
            languages: Vec::new(),
            limit: MAX_PUBLIC_CANDIDATES,
        }
    }

    #[test]
    fn signed_manifest_is_discoverable_and_replay_is_rejected() {
        let store = CatalogStore::in_memory().unwrap();
        let keypair = Keypair::generate_ed25519();
        let collection_id = Uuid::new_v4();
        let now = Utc::now();
        let manifest = signed_manifest(&keypair, collection_id, 1, now);

        store.register(&manifest, now).unwrap();
        assert_eq!(
            store.query(&query("atlas"), now).unwrap(),
            vec![manifest.clone()]
        );
        assert!(matches!(
            store.register(&manifest, now),
            Err(CatalogStoreError::StaleSequence)
        ));
    }

    #[test]
    fn tampering_expiry_and_tombstones_fail_closed() {
        let store = CatalogStore::in_memory().unwrap();
        let keypair = Keypair::generate_ed25519();
        let collection_id = Uuid::new_v4();
        let now = Utc::now();
        let manifest = signed_manifest(&keypair, collection_id, 1, now);
        let mut tampered = manifest.clone();
        tampered.manifest.name = "Changed".to_owned();
        assert!(matches!(
            store.register(&tampered, now),
            Err(CatalogStoreError::Verification)
        ));

        store.register(&manifest, now).unwrap();
        assert!(
            store
                .query(&query("atlas"), now + Duration::minutes(16))
                .unwrap()
                .is_empty()
        );
        let tombstone = sign_tombstone(
            &keypair,
            PublicCollectionTombstone {
                protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
                publisher_id: keypair.public().to_peer_id().to_string(),
                collection_id,
                sequence: 2,
                withdrawn_at: now,
            },
        )
        .unwrap();
        let mut tampered_tombstone = tombstone.clone();
        tampered_tombstone.tombstone.sequence = 3;
        assert!(matches!(
            store.withdraw(&tampered_tombstone),
            Err(CatalogStoreError::Verification)
        ));
        store.withdraw(&tombstone).unwrap();
        assert!(store.query(&query("atlas"), now).unwrap().is_empty());
    }

    #[test]
    fn collection_ids_are_scoped_by_publisher_identity() {
        let store = CatalogStore::in_memory().unwrap();
        let first = Keypair::generate_ed25519();
        let second = Keypair::generate_ed25519();
        let collection_id = Uuid::new_v4();
        let now = Utc::now();
        store
            .register(&signed_manifest(&first, collection_id, 1, now), now)
            .unwrap();
        store
            .register(&signed_manifest(&second, collection_id, 1, now), now)
            .unwrap();

        let results = store.query(&query("recovery"), now).unwrap();
        assert_eq!(results.len(), 2);
        assert_ne!(
            results[0].manifest.publisher_id,
            results[1].manifest.publisher_id
        );
    }

    #[test]
    fn tombstone_before_manifest_prevents_older_registration() {
        let store = CatalogStore::in_memory().unwrap();
        let keypair = Keypair::generate_ed25519();
        let collection_id = Uuid::new_v4();
        let now = Utc::now();
        let tombstone = sign_tombstone(
            &keypair,
            PublicCollectionTombstone {
                protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
                publisher_id: keypair.public().to_peer_id().to_string(),
                collection_id,
                sequence: 4,
                withdrawn_at: now,
            },
        )
        .unwrap();

        store.withdraw(&tombstone).unwrap();

        assert!(matches!(
            store.register(&signed_manifest(&keypair, collection_id, 3, now), now),
            Err(CatalogStoreError::StaleSequence)
        ));
        assert!(store.query(&query("atlas"), now).unwrap().is_empty());
    }

    #[test]
    fn publisher_registration_limit_rejects_the_next_distinct_collection() {
        let store = CatalogStore::in_memory().unwrap();
        let keypair = Keypair::generate_ed25519();
        let now = Utc::now();
        for ordinal in 0..MAX_COLLECTIONS_PER_PUBLISHER {
            store
                .register(
                    &signed_manifest(&keypair, Uuid::from_u128(u128::from(ordinal) + 1), 1, now),
                    now,
                )
                .unwrap();
        }

        assert!(matches!(
            store.register(
                &signed_manifest(
                    &keypair,
                    Uuid::from_u128(u128::from(MAX_COLLECTIONS_PER_PUBLISHER) + 1),
                    1,
                    now,
                ),
                now,
            ),
            Err(CatalogStoreError::PublisherLimit)
        ));
    }

    #[test]
    fn global_capacity_bounds_distinct_publisher_identities() {
        let store = CatalogStore::in_memory_with_limits(CatalogLimits {
            max_entries: 2,
            max_collections_per_publisher: 2,
        })
        .unwrap();
        let first = Keypair::generate_ed25519();
        let second = Keypair::generate_ed25519();
        let next = Keypair::generate_ed25519();
        let now = Utc::now();

        store
            .register(&signed_manifest(&first, Uuid::new_v4(), 1, now), now)
            .unwrap();
        store
            .register(&signed_manifest(&second, Uuid::new_v4(), 1, now), now)
            .unwrap();

        assert!(matches!(
            store.register(&signed_manifest(&next, Uuid::new_v4(), 1, now), now),
            Err(CatalogStoreError::CapacityLimit)
        ));
        assert_eq!(
            store
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM manifests", [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn global_capacity_also_bounds_tombstones_without_manifests() {
        let store = CatalogStore::in_memory_with_limits(CatalogLimits {
            max_entries: 1,
            max_collections_per_publisher: 1,
        })
        .unwrap();
        let now = Utc::now();
        let first = Keypair::generate_ed25519();
        let next = Keypair::generate_ed25519();
        let tombstone = |keypair: &Keypair| {
            sign_tombstone(
                keypair,
                PublicCollectionTombstone {
                    protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
                    publisher_id: keypair.public().to_peer_id().to_string(),
                    collection_id: Uuid::new_v4(),
                    sequence: 1,
                    withdrawn_at: now,
                },
            )
            .unwrap()
        };

        store.withdraw_at(&tombstone(&first), now).unwrap();
        assert!(matches!(
            store.withdraw_at(&tombstone(&next), now),
            Err(CatalogStoreError::CapacityLimit)
        ));
        assert_eq!(
            store
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM manifests", [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn publisher_capacity_also_bounds_tombstones_without_manifests() {
        let store = CatalogStore::in_memory_with_limits(CatalogLimits {
            max_entries: 2,
            max_collections_per_publisher: 1,
        })
        .unwrap();
        let now = Utc::now();
        let keypair = Keypair::generate_ed25519();
        let tombstone = |collection_id| {
            sign_tombstone(
                &keypair,
                PublicCollectionTombstone {
                    protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
                    publisher_id: keypair.public().to_peer_id().to_string(),
                    collection_id,
                    sequence: 1,
                    withdrawn_at: now,
                },
            )
            .unwrap()
        };

        store.withdraw_at(&tombstone(Uuid::new_v4()), now).unwrap();
        assert!(matches!(
            store.withdraw_at(&tombstone(Uuid::new_v4()), now),
            Err(CatalogStoreError::PublisherLimit)
        ));
    }

    #[test]
    fn tombstone_high_water_survives_manifest_lifetime() {
        let store = CatalogStore::in_memory_with_limits(CatalogLimits {
            max_entries: 1,
            max_collections_per_publisher: 1,
        })
        .unwrap();
        let keypair = Keypair::generate_ed25519();
        let collection_id = Uuid::new_v4();
        let now = Utc::now();
        let expiry = manifest_expiry_limit(now);
        let replay_time = expiry + Duration::seconds(1);
        let delayed_replay = signed_manifest_with_expiry(
            &keypair,
            collection_id,
            3,
            expiry,
            expiry + Duration::minutes(15),
        );
        let tombstone = sign_tombstone(
            &keypair,
            PublicCollectionTombstone {
                protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
                publisher_id: keypair.public().to_peer_id().to_string(),
                collection_id,
                sequence: 4,
                withdrawn_at: now,
            },
        )
        .unwrap();

        store.withdraw_at(&tombstone, now).unwrap();
        assert_eq!(store.purge_expired(replay_time).unwrap(), 0);
        assert!(matches!(
            store.register(&delayed_replay, replay_time),
            Err(CatalogStoreError::StaleSequence)
        ));
    }

    #[test]
    fn registration_purges_expired_rows_before_capacity_rejection() {
        let store = CatalogStore::in_memory_with_limits(CatalogLimits {
            max_entries: 1,
            max_collections_per_publisher: 1,
        })
        .unwrap();
        let first = Keypair::generate_ed25519();
        let next = Keypair::generate_ed25519();
        let now = Utc::now();
        store
            .register(&signed_manifest(&first, Uuid::new_v4(), 1, now), now)
            .unwrap();

        let later = now + Duration::minutes(16);
        store
            .register(&signed_manifest(&next, Uuid::new_v4(), 1, later), later)
            .unwrap();
        assert_eq!(
            store
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM manifests", [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn operational_error_classes_are_fixed_and_sanitized() {
        let cases = [
            (
                CatalogStoreError::Persistence(rusqlite::Error::InvalidQuery),
                "catalog_persistence_failed",
            ),
            (CatalogStoreError::Encoding, "catalog_encoding_failed"),
            (
                CatalogStoreError::Verification,
                "catalog_verification_rejected",
            ),
            (CatalogStoreError::StaleSequence, "catalog_stale_rejected"),
            (CatalogStoreError::InvalidQuery, "catalog_query_rejected"),
            (
                CatalogStoreError::PublisherLimit,
                "catalog_capacity_rejected",
            ),
            (
                CatalogStoreError::CapacityLimit,
                "catalog_capacity_rejected",
            ),
            (CatalogStoreError::Lock, "catalog_busy"),
        ];

        for (error, expected) in cases {
            assert_eq!(catalog_error_kind(&error), expected);
        }
    }
}
