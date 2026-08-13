use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use airwiki_network::{
    MemorySecretStore, Multiaddr, NodeIdentity, PublicBrowseDelivery, PublicCatalogBackend,
    PublicCatalogBackendError, PublicIndexEndpoint, PublicReader, PublicRouteKind,
    PublicSearchDelivery, PublicSourceBackend, PublicSourceBackendError, PublicSourceServerConfig,
    run_public_catalog_server, run_public_source_server, sign_manifest,
};
use airwiki_types::{
    ConceptType, DisclosureGate, PUBLIC_BROWSE_PROTOCOL, PUBLIC_CATALOG_PROTOCOL,
    PUBLIC_SEARCH_PROTOCOL, PublicBrowsePage, PublicBrowseRequest, PublicCatalogQuery,
    PublicCollectionManifest, PublicCollectionRevision, PublicSearchRequest, PublicSearchResponse,
    SearchContractError, SearchHit, SearchPurpose, SearchRequest, SearchResponse,
    SignedPublicCollectionManifest, SignedPublicCollectionTombstone,
};
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct StaticCatalogBackend {
    manifest: SignedPublicCollectionManifest,
}

#[async_trait]
impl PublicCatalogBackend for StaticCatalogBackend {
    async fn register(
        &self,
        _manifest: SignedPublicCollectionManifest,
    ) -> Result<(), PublicCatalogBackendError> {
        Ok(())
    }

    async fn withdraw(
        &self,
        _tombstone: SignedPublicCollectionTombstone,
    ) -> Result<(), PublicCatalogBackendError> {
        Ok(())
    }

    async fn query(
        &self,
        _query: PublicCatalogQuery,
    ) -> Result<Vec<SignedPublicCollectionManifest>, PublicCatalogBackendError> {
        Ok(vec![self.manifest.clone()])
    }
}

struct RequestGate {
    started: Mutex<Option<oneshot::Sender<()>>>,
    release: Notify,
}

impl RequestGate {
    fn new() -> (Arc<Self>, oneshot::Receiver<()>) {
        let (started, receiver) = oneshot::channel();
        (
            Arc::new(Self {
                started: Mutex::new(Some(started)),
                release: Notify::new(),
            }),
            receiver,
        )
    }

    async fn wait_once(&self) {
        let started = self.started.lock().await.take();
        if let Some(started) = started {
            let _ = started.send(());
            self.release.notified().await;
        }
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

struct ControlledPublicSourceBackend {
    disclosure: DisclosureGate,
    publisher_id: String,
    search_gate: Arc<RequestGate>,
    successful_browse_gate: Arc<RequestGate>,
    unavailable_browse_gate: Arc<RequestGate>,
    browse_attempt: AtomicUsize,
}

#[async_trait]
impl PublicSourceBackend for ControlledPublicSourceBackend {
    async fn search(
        &self,
        request: PublicSearchRequest,
    ) -> Result<PublicSearchDelivery, PublicSourceBackendError> {
        self.search_gate.wait_once().await;
        let collection = request
            .collections
            .first()
            .ok_or(PublicSourceBackendError::Invalid)?;
        let now = Utc::now();
        Ok(PublicSearchDelivery::new(
            PublicSearchResponse {
                protocol_version: PUBLIC_SEARCH_PROTOCOL.to_owned(),
                manifest_sequences: vec![PublicCollectionRevision {
                    collection_id: collection.collection_id,
                    manifest_sequence: collection.manifest_sequence,
                }],
                response: SearchResponse {
                    request_id: request.request_id,
                    hits: vec![SearchHit {
                        concept_id: Uuid::new_v4(),
                        collection_id: collection.collection_id,
                        chunk_id: Uuid::new_v4(),
                        title: "Synthetic recovery".to_owned(),
                        snippet: "Restart the synthetic queue.".to_owned(),
                        heading_or_page: "Recovery".to_owned(),
                        logical_resource_uri: "urn:airwiki:test:recovery".to_owned(),
                        source_revision: 1,
                        source_sha256: "b".repeat(64),
                        updated_at: now,
                        rank: 1,
                        node_id: "replaced-by-transport".to_owned(),
                        assurance: None,
                        lifecycle_status: Some("stable".to_owned()),
                    }],
                    authorized_candidates: Vec::new(),
                    offline_nodes: Vec::new(),
                    warnings: Vec::new(),
                    partial: false,
                },
            },
            self.disclosure.acquire_disclosure(),
        ))
    }

    async fn browse(
        &self,
        request: PublicBrowseRequest,
    ) -> Result<PublicBrowseDelivery, PublicSourceBackendError> {
        if self.browse_attempt.fetch_add(1, Ordering::SeqCst) > 0 {
            self.unavailable_browse_gate.wait_once().await;
            return Err(PublicSourceBackendError::Unavailable);
        }
        self.successful_browse_gate.wait_once().await;
        Ok(PublicBrowseDelivery::new(
            PublicBrowsePage {
                protocol_version: PUBLIC_BROWSE_PROTOCOL.to_owned(),
                request_id: request.request_id,
                manifest_sequence: 1,
                concepts: vec![airwiki_types::PublicConceptSummary {
                    publisher_id: self.publisher_id.clone(),
                    collection_id: request.collection_id,
                    concept_id: Uuid::new_v4(),
                    concept_type: ConceptType::Procedure,
                    title: "Synthetic recovery".to_owned(),
                    description: "Synthetic procedure".to_owned(),
                    language: "en".to_owned(),
                    tags: vec!["synthetic".to_owned()],
                    summary: "Restart the synthetic queue.".to_owned(),
                    logical_resource_uri: "urn:airwiki:test:recovery".to_owned(),
                    source_revision: 1,
                    updated_at: Utc::now(),
                    lifecycle_status: Some("stable".to_owned()),
                    assurance: None,
                }],
                next_cursor: None,
            },
            self.disclosure.acquire_disclosure(),
        ))
    }
}

#[tokio::test]
async fn publisher_block_linearizes_in_flight_search_and_cached_browse() {
    let (index_port, source_port) = available_tcp_ports();
    let index_identity = identity();
    let source_identity = identity();
    let publisher_id = source_identity.peer_id().to_string();
    let collection_id = Uuid::new_v4();
    let index_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{index_port}")
        .parse()
        .expect("synthetic index address is valid");
    let source_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{source_port}")
        .parse()
        .expect("synthetic source address is valid");
    let now = Utc::now();
    let manifest = sign_manifest(
        source_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: publisher_id.clone(),
            collection_id,
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Synthetic public runbooks".to_owned(),
            description: "Synthetic public collection".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["synthetic".to_owned(), "recovery".to_owned()],
            routes: vec![source_address.to_string()],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .expect("synthetic manifest can be signed");
    let (search_gate, search_started) = RequestGate::new();
    let (successful_browse_gate, successful_browse_started) = RequestGate::new();
    let (unavailable_browse_gate, unavailable_browse_started) = RequestGate::new();
    let source_backend = Arc::new(ControlledPublicSourceBackend {
        disclosure: DisclosureGate::default(),
        publisher_id: publisher_id.clone(),
        search_gate: Arc::clone(&search_gate),
        successful_browse_gate: Arc::clone(&successful_browse_gate),
        unavailable_browse_gate: Arc::clone(&unavailable_browse_gate),
        browse_attempt: AtomicUsize::new(0),
    });
    let catalog_cancellation = CancellationToken::new();
    let source_cancellation = CancellationToken::new();
    let catalog_task = tokio::spawn(run_public_catalog_server(
        index_identity.clone(),
        airwiki_network::PublicCatalogServerConfig::new(vec![index_address.clone()]),
        Arc::new(StaticCatalogBackend { manifest }),
        catalog_cancellation.clone(),
    ));
    let source_task = tokio::spawn(run_public_source_server(
        source_identity,
        PublicSourceServerConfig::new(vec![source_address]),
        source_backend,
        source_cancellation.clone(),
    ));
    wait_for_tcp_listener(index_port).await;
    wait_for_tcp_listener(source_port).await;

    let endpoint = PublicIndexEndpoint {
        peer_id: index_identity.peer_id(),
        address: index_address,
    };
    let reader = Arc::new(PublicReader::new());
    let (partial_sender, mut partial_receiver) = mpsc::channel(4);
    let search_task = {
        let reader = Arc::clone(&reader);
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            reader
                .search_with_route_and_partials(
                    &[endpoint],
                    SearchRequest::new("synthetic recovery", SearchPurpose::LocalAssistant, 5),
                    partial_sender,
                )
                .await
        })
    };
    search_started
        .await
        .expect("the controlled public search reaches the source");
    reader
        .set_publisher_blocked(publisher_id.clone(), true)
        .await;
    search_gate.release();
    let blocked_search = tokio::time::timeout(Duration::from_secs(5), search_task)
        .await
        .expect("the blocked public search completes within its budget")
        .expect("the public search task does not panic")
        .expect("the public search returns a bounded result");
    let mut late_partial_hits = 0;
    while let Ok(partial) = partial_receiver.try_recv() {
        late_partial_hits += partial.hits.len();
    }

    reader
        .set_publisher_blocked(publisher_id.clone(), false)
        .await;
    let cache_warming_search = reader
        .search(
            std::slice::from_ref(&endpoint),
            SearchRequest::new("synthetic recovery", SearchPurpose::LocalAssistant, 5),
        )
        .await
        .expect("an unblocked synthetic search warms the browse route cache");
    assert_eq!(
        cache_warming_search.hits.len(),
        1,
        "the cache setup must return the synthetic source"
    );
    let browse_task = {
        let reader = Arc::clone(&reader);
        let publisher_id = publisher_id.clone();
        tokio::spawn(async move {
            reader
                .browse_collection(&publisher_id, collection_id, None, 10)
                .await
        })
    };
    successful_browse_started
        .await
        .expect("the cached public browse reaches the source");
    reader
        .set_publisher_blocked(publisher_id.clone(), true)
        .await;
    successful_browse_gate.release();
    let successful_in_flight_browse = tokio::time::timeout(Duration::from_secs(5), browse_task)
        .await
        .expect("the blocked public browse completes within its budget")
        .expect("the public browse task does not panic");

    reader
        .set_publisher_blocked(publisher_id.clone(), false)
        .await;
    let unavailable_browse_task = {
        let reader = Arc::clone(&reader);
        let publisher_id = publisher_id.clone();
        tokio::spawn(async move {
            reader
                .browse_collection(&publisher_id, collection_id, None, 10)
                .await
        })
    };
    unavailable_browse_started
        .await
        .expect("the second cached public browse reaches the source");
    reader
        .set_publisher_blocked(publisher_id.clone(), true)
        .await;
    unavailable_browse_gate.release();
    let unavailable_in_flight_browse =
        tokio::time::timeout(Duration::from_secs(5), unavailable_browse_task)
            .await
            .expect("the unavailable blocked browse completes within its budget")
            .expect("the unavailable public browse task does not panic");
    let cached_browse_after_block = reader
        .browse_collection(&publisher_id, collection_id, None, 10)
        .await;

    catalog_cancellation.cancel();
    source_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), catalog_task)
        .await
        .expect("the synthetic catalog stops")
        .expect("the synthetic catalog task does not panic")
        .expect("the synthetic catalog exits cleanly");
    tokio::time::timeout(Duration::from_secs(2), source_task)
        .await
        .expect("the synthetic source stops")
        .expect("the synthetic source task does not panic")
        .expect("the synthetic source exits cleanly");

    assert_eq!(
        (
            blocked_search.response.hits.len(),
            late_partial_hits,
            blocked_search.route_kind,
            matches!(
                successful_in_flight_browse,
                Err(SearchContractError::Unauthorized)
            ),
            matches!(
                unavailable_in_flight_browse,
                Err(SearchContractError::Unauthorized)
            ),
            matches!(
                cached_browse_after_block,
                Err(SearchContractError::Unauthorized)
            ),
        ),
        (0, 0, PublicRouteKind::Offline, true, true, true),
        "a completed publisher block must exclude late search, partial, route, and browse results"
    );
}

fn identity() -> NodeIdentity {
    NodeIdentity::load_or_create(&MemorySecretStore::default())
        .expect("a synthetic in-memory identity can be created")
}

fn available_tcp_ports() -> (u16, u16) {
    let first = TcpListener::bind(("127.0.0.1", 0)).expect("a loopback port is available");
    let second = TcpListener::bind(("127.0.0.1", 0)).expect("a second loopback port is available");
    let ports = (
        first
            .local_addr()
            .expect("the first loopback address is available")
            .port(),
        second
            .local_addr()
            .expect("the second loopback address is available")
            .port(),
    );
    drop((first, second));
    ports
}

async fn wait_for_tcp_listener(port: u16) {
    tokio::time::timeout(Duration::from_secs(2), async move {
        loop {
            if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the synthetic loopback listener becomes ready");
}
