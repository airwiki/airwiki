use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use airwiki_federation_index::{CatalogBackend, CatalogStore};
use airwiki_network::{
    MemorySecretStore, Multiaddr, NodeIdentity, PublicBrowseDelivery, PublicBrowseOptions,
    PublicCatalogBackend, PublicCatalogBackendError, PublicIndexEndpoint, PublicReader,
    PublicRouteKind, PublicSearchDelivery, PublicSourceBackend, PublicSourceBackendError,
    PublicSourceServerConfig, relay_circuit_address, relayed_peer_address,
    run_public_catalog_server, run_public_source_server, sign_manifest,
};
use airwiki_types::{
    ConceptType, DisclosureGate, PUBLIC_BROWSE_PROTOCOL, PUBLIC_CATALOG_PROTOCOL,
    PUBLIC_SEARCH_PROTOCOL, PublicBrowsePage, PublicBrowseRequest, PublicCatalogQuery,
    PublicCollectionManifest, PublicCollectionRevision, PublicSearchRequest, PublicSearchResponse,
    SearchHit, SearchPurpose, SearchRequest, SearchResponse, SignedPublicCollectionManifest,
    SignedPublicCollectionTombstone,
};
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug)]
struct PublicFixtureBackend {
    gate: DisclosureGate,
    publisher_id: String,
    manifest_sequence: u64,
}

#[async_trait]
impl PublicSourceBackend for PublicFixtureBackend {
    async fn search(
        &self,
        request: PublicSearchRequest,
    ) -> Result<PublicSearchDelivery, PublicSourceBackendError> {
        let collection = request
            .collections
            .first()
            .ok_or(PublicSourceBackendError::Invalid)?;
        let now = Utc::now();
        let hit = SearchHit {
            concept_id: collection.collection_id,
            collection_id: collection.collection_id,
            chunk_id: Uuid::new_v4(),
            title: "Atlas recovery".to_owned(),
            snippet: "Restart the synthetic Atlas queue.".to_owned(),
            heading_or_page: "Recovery".to_owned(),
            logical_resource_uri: "urn:airwiki:atlas:recovery".to_owned(),
            source_revision: 1,
            source_sha256: "b".repeat(64),
            updated_at: now,
            rank: 1,
            node_id: "replaced-by-transport".to_owned(),
            collection_presentation: None,
            assurance: None,
            lifecycle_status: Some("stable".to_owned()),
        };
        Ok(PublicSearchDelivery::new(
            PublicSearchResponse {
                protocol_version: PUBLIC_SEARCH_PROTOCOL.to_owned(),
                manifest_sequences: vec![PublicCollectionRevision {
                    collection_id: collection.collection_id,
                    manifest_sequence: collection.manifest_sequence,
                }],
                response: SearchResponse {
                    request_id: request.request_id,
                    hits: vec![hit],
                    authorized_candidates: Vec::new(),
                    offline_nodes: Vec::new(),
                    warnings: Vec::new(),
                    partial: false,
                },
            },
            self.gate.acquire_disclosure(),
        ))
    }

    async fn browse(
        &self,
        request: PublicBrowseRequest,
    ) -> Result<PublicBrowseDelivery, PublicSourceBackendError> {
        let now = Utc::now();
        Ok(PublicBrowseDelivery::new(
            PublicBrowsePage {
                protocol_version: PUBLIC_BROWSE_PROTOCOL.to_owned(),
                request_id: request.request_id,
                manifest_sequence: self.manifest_sequence,
                concepts: vec![airwiki_types::PublicConceptSummary {
                    publisher_id: self.publisher_id.clone(),
                    collection_id: request.collection_id,
                    concept_id: request.collection_id,
                    concept_type: ConceptType::Procedure,
                    title: "Atlas recovery".to_owned(),
                    description: "Synthetic procedure".to_owned(),
                    language: "en".to_owned(),
                    tags: vec!["atlas".to_owned()],
                    summary: "Restart the synthetic queue.".to_owned(),
                    logical_resource_uri: "urn:airwiki:atlas:recovery".to_owned(),
                    source_revision: 1,
                    updated_at: now,
                    lifecycle_status: Some("stable".to_owned()),
                    assurance: None,
                }],
                next_cursor: None,
                workspace: Some(airwiki_types::PublishedWikiWorkspacePage {
                    workspace_fingerprint: "b".repeat(64),
                    reserved_pages: Vec::new(),
                    documents: vec![airwiki_types::PublishedWikiPageDescriptor {
                        page: airwiki_types::PublishedWikiPageId::Concept {
                            concept_id: request.collection_id,
                        },
                        logical_path: format!("concepts/{}.md", request.collection_id),
                        title: "Atlas recovery".to_owned(),
                        fingerprint: "c".repeat(64),
                    }],
                    links: Vec::new(),
                    next_graph_cursor: None,
                }),
                document: None,
            },
            self.gate.acquire_disclosure(),
        ))
    }
}

#[derive(Debug, Clone)]
struct DelayedCatalogBackend {
    inner: CatalogBackend,
    delay: Duration,
}

#[async_trait]
impl PublicCatalogBackend for DelayedCatalogBackend {
    async fn register(
        &self,
        manifest: SignedPublicCollectionManifest,
    ) -> Result<(), PublicCatalogBackendError> {
        tokio::time::sleep(self.delay).await;
        self.inner.register(manifest).await
    }

    async fn withdraw(
        &self,
        tombstone: SignedPublicCollectionTombstone,
    ) -> Result<(), PublicCatalogBackendError> {
        tokio::time::sleep(self.delay).await;
        self.inner.withdraw(tombstone).await
    }

    async fn query(
        &self,
        query: PublicCatalogQuery,
    ) -> Result<Vec<SignedPublicCollectionManifest>, PublicCatalogBackendError> {
        tokio::time::sleep(self.delay).await;
        self.inner.query(query).await
    }
}

#[derive(Debug)]
struct DelayedPublicSourceBackend {
    inner: Arc<PublicFixtureBackend>,
    delay: Duration,
}

#[async_trait]
impl PublicSourceBackend for DelayedPublicSourceBackend {
    async fn search(
        &self,
        request: PublicSearchRequest,
    ) -> Result<PublicSearchDelivery, PublicSourceBackendError> {
        tokio::time::sleep(self.delay).await;
        self.inner.search(request).await
    }

    async fn browse(
        &self,
        request: PublicBrowseRequest,
    ) -> Result<PublicBrowseDelivery, PublicSourceBackendError> {
        tokio::time::sleep(self.delay).await;
        self.inner.browse(request).await
    }
}

#[derive(Debug)]
struct BlockingPublicSourceBackend {
    inner: Arc<PublicFixtureBackend>,
    search_started: Arc<Notify>,
    release_search: Arc<Notify>,
}

#[async_trait]
impl PublicSourceBackend for BlockingPublicSourceBackend {
    async fn search(
        &self,
        request: PublicSearchRequest,
    ) -> Result<PublicSearchDelivery, PublicSourceBackendError> {
        self.search_started.notify_one();
        self.release_search.notified().await;
        self.inner.search(request).await
    }

    async fn browse(
        &self,
        request: PublicBrowseRequest,
    ) -> Result<PublicBrowseDelivery, PublicSourceBackendError> {
        self.inner.browse(request).await
    }
}

#[tokio::test]
async fn public_catalog_backfills_after_filtering_a_blocked_publisher() {
    let index_port = available_port();
    let index_identity = identity();
    let blocked_identity = identity();
    let visible_identity = identity();
    let visible_collection_id = Uuid::new_v4();
    let index_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{index_port}").parse().unwrap();
    let catalog_cancellation = CancellationToken::new();
    let catalog_task = tokio::spawn(run_public_catalog_server(
        index_identity.clone(),
        airwiki_network::PublicCatalogServerConfig::new(vec![index_address.clone()]),
        Arc::new(CatalogBackend::new(Arc::new(
            CatalogStore::in_memory().unwrap(),
        ))),
        catalog_cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;

    let endpoint = PublicIndexEndpoint {
        peer_id: index_identity.peer_id(),
        address: index_address,
    };
    let now = Utc::now();
    let blocked_manifest = sign_manifest(
        blocked_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: blocked_identity.peer_id().to_string(),
            collection_id: Uuid::new_v4(),
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Blocked public Wiki".to_owned(),
            description: "Synthetic blocked profile".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["blocked".to_owned()],
            routes: vec!["/ip4/127.0.0.1/tcp/41001".to_owned()],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(20),
        },
    )
    .unwrap();
    let visible_manifest = sign_manifest(
        visible_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: visible_identity.peer_id().to_string(),
            collection_id: visible_collection_id,
            sequence: 1,
            publication_fingerprint: "b".repeat(64),
            name: "Visible public Wiki".to_owned(),
            description: "Synthetic visible profile".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["visible".to_owned()],
            routes: vec!["/ip4/127.0.0.1/tcp/41002".to_owned()],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .unwrap();
    let reader = PublicReader::new();
    reader
        .register_manifest(std::slice::from_ref(&endpoint), visible_manifest)
        .await
        .unwrap();
    reader
        .register_manifest(std::slice::from_ref(&endpoint), blocked_manifest)
        .await
        .unwrap();
    reader
        .set_publisher_blocked(blocked_identity.peer_id().to_string(), true)
        .await;

    let catalog = reader
        .explore_catalog(std::slice::from_ref(&endpoint), 1)
        .await
        .unwrap();
    assert_eq!(catalog.collections.len(), 1);
    assert_eq!(catalog.collections[0].collection_id, visible_collection_id);

    catalog_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), catalog_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn public_search_round_trip_needs_no_lan_pairing_or_grant() {
    let index_port = available_port();
    let source_port = available_port();
    let index_identity = identity();
    let source_identity = identity();
    let collection_id = Uuid::new_v4();
    let index_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{index_port}").parse().unwrap();
    let source_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{source_port}").parse().unwrap();
    let catalog_cancellation = CancellationToken::new();
    let source_cancellation = CancellationToken::new();
    let catalog_task = tokio::spawn(run_public_catalog_server(
        index_identity.clone(),
        airwiki_network::PublicCatalogServerConfig::new(vec![index_address.clone()]),
        Arc::new(CatalogBackend::new(Arc::new(
            CatalogStore::in_memory().unwrap(),
        ))),
        catalog_cancellation.clone(),
    ));
    let source_task = tokio::spawn(run_public_source_server(
        source_identity.clone(),
        PublicSourceServerConfig::new(vec![source_address.clone()]),
        Arc::new(PublicFixtureBackend {
            gate: DisclosureGate::default(),
            publisher_id: source_identity.peer_id().to_string(),
            manifest_sequence: 1,
        }),
        source_cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;

    let endpoint = PublicIndexEndpoint {
        peer_id: index_identity.peer_id(),
        address: index_address,
    };
    let now = Utc::now();
    let manifest = sign_manifest(
        source_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: source_identity.peer_id().to_string(),
            collection_id,
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Atlas public runbooks".to_owned(),
            description: "Synthetic public collection".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["atlas".to_owned(), "recovery".to_owned()],
            routes: vec![source_address.to_string()],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .unwrap();
    let reader = PublicReader::new();
    reader
        .register_manifest(std::slice::from_ref(&endpoint), manifest.clone())
        .await
        .unwrap();
    let catalog = reader
        .explore_catalog(std::slice::from_ref(&endpoint), 24)
        .await
        .unwrap();
    assert!(!catalog.partial);
    assert_eq!(catalog.collections.len(), 1);
    assert_eq!(catalog.collections[0].collection_id, collection_id);
    assert_eq!(catalog.collections[0].name, "Atlas public runbooks");
    let response = reader
        .search(
            &[endpoint],
            SearchRequest::new("atlas recovery", SearchPurpose::LocalAssistant, 5),
        )
        .await
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert_eq!(response.hits[0].collection_id, collection_id);
    let page = reader
        .browse(
            &manifest,
            None,
            response.hits.first().map(|hit| hit.concept_id),
            None,
            None,
            50,
        )
        .await
        .unwrap();
    assert_eq!(page.concepts.len(), 1);
    assert_eq!(page.concepts[0].collection_id, collection_id);

    catalog_cancellation.cancel();
    source_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), catalog_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), source_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn public_search_preserves_owner_stage_after_slow_catalog() {
    let index_port = available_port();
    let source_port = available_port();
    let index_identity = identity();
    let source_identity = identity();
    let collection_id = Uuid::new_v4();
    let index_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{index_port}").parse().unwrap();
    let source_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{source_port}").parse().unwrap();
    let catalog_cancellation = CancellationToken::new();
    let source_cancellation = CancellationToken::new();
    let catalog_backend = CatalogBackend::new(Arc::new(CatalogStore::in_memory().unwrap()));
    let mut catalog_config =
        airwiki_network::PublicCatalogServerConfig::new(vec![index_address.clone()]);
    catalog_config.request_timeout = Duration::from_millis(950);
    let catalog_task = tokio::spawn(run_public_catalog_server(
        index_identity.clone(),
        catalog_config,
        Arc::new(DelayedCatalogBackend {
            inner: catalog_backend,
            delay: Duration::from_millis(850),
        }),
        catalog_cancellation.clone(),
    ));
    let source_task = tokio::spawn(run_public_source_server(
        source_identity.clone(),
        PublicSourceServerConfig::new(vec![source_address.clone()]),
        Arc::new(DelayedPublicSourceBackend {
            inner: Arc::new(PublicFixtureBackend {
                gate: DisclosureGate::default(),
                publisher_id: source_identity.peer_id().to_string(),
                manifest_sequence: 1,
            }),
            delay: Duration::from_millis(700),
        }),
        source_cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;

    let endpoint = PublicIndexEndpoint {
        peer_id: index_identity.peer_id(),
        address: index_address,
    };
    let now = Utc::now();
    let manifest = sign_manifest(
        source_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: source_identity.peer_id().to_string(),
            collection_id,
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Atlas public runbooks".to_owned(),
            description: "Synthetic public collection".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["atlas".to_owned(), "recovery".to_owned()],
            routes: vec![source_address.to_string()],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .unwrap();
    let reader = PublicReader::new();
    reader
        .register_manifest(std::slice::from_ref(&endpoint), manifest)
        .await
        .unwrap();
    let response = reader
        .search(
            &[endpoint],
            SearchRequest::new("atlas recovery", SearchPurpose::LocalAssistant, 5),
        )
        .await
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert!(!response.partial);

    catalog_cancellation.cancel();
    source_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), catalog_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), source_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn concurrent_public_routes_do_not_cross_between_success_and_timeout() {
    let index_port = available_port();
    let fast_source_port = available_port();
    let slow_source_port = available_port();
    let index_identity = identity();
    let fast_source_identity = identity();
    let slow_source_identity = identity();
    let fast_collection_id = Uuid::new_v4();
    let slow_collection_id = Uuid::new_v4();
    let index_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{index_port}").parse().unwrap();
    let fast_source_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{fast_source_port}")
        .parse()
        .unwrap();
    let slow_source_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{slow_source_port}")
        .parse()
        .unwrap();
    let catalog_cancellation = CancellationToken::new();
    let fast_source_cancellation = CancellationToken::new();
    let slow_source_cancellation = CancellationToken::new();
    let slow_search_started = Arc::new(Notify::new());
    let release_slow_search = Arc::new(Notify::new());
    let catalog_task = tokio::spawn(run_public_catalog_server(
        index_identity.clone(),
        airwiki_network::PublicCatalogServerConfig::new(vec![index_address.clone()]),
        Arc::new(CatalogBackend::new(Arc::new(
            CatalogStore::in_memory().unwrap(),
        ))),
        catalog_cancellation.clone(),
    ));
    let fast_source_task = tokio::spawn(run_public_source_server(
        fast_source_identity.clone(),
        PublicSourceServerConfig::new(vec![fast_source_address.clone()]),
        Arc::new(PublicFixtureBackend {
            gate: DisclosureGate::default(),
            publisher_id: fast_source_identity.peer_id().to_string(),
            manifest_sequence: 1,
        }),
        fast_source_cancellation.clone(),
    ));
    let slow_source_task = tokio::spawn(run_public_source_server(
        slow_source_identity.clone(),
        PublicSourceServerConfig::new(vec![slow_source_address.clone()]),
        Arc::new(BlockingPublicSourceBackend {
            inner: Arc::new(PublicFixtureBackend {
                gate: DisclosureGate::default(),
                publisher_id: slow_source_identity.peer_id().to_string(),
                manifest_sequence: 1,
            }),
            search_started: Arc::clone(&slow_search_started),
            release_search: Arc::clone(&release_slow_search),
        }),
        slow_source_cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;

    let endpoint = PublicIndexEndpoint {
        peer_id: index_identity.peer_id(),
        address: index_address,
    };
    let now = Utc::now();
    let fast_manifest = sign_manifest(
        fast_source_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: fast_source_identity.peer_id().to_string(),
            collection_id: fast_collection_id,
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Fast owner fixture".to_owned(),
            description: String::new(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["fastowner".to_owned()],
            routes: vec![fast_source_address.to_string()],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .unwrap();
    let slow_manifest = sign_manifest(
        slow_source_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: slow_source_identity.peer_id().to_string(),
            collection_id: slow_collection_id,
            sequence: 1,
            publication_fingerprint: "b".repeat(64),
            name: "Slow owner fixture".to_owned(),
            description: String::new(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["slowowner".to_owned()],
            routes: vec![slow_source_address.to_string()],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .unwrap();
    let reader = PublicReader::new();
    reader
        .register_manifest(std::slice::from_ref(&endpoint), fast_manifest)
        .await
        .unwrap();
    reader
        .register_manifest(std::slice::from_ref(&endpoint), slow_manifest)
        .await
        .unwrap();

    let (fast_result, slow_result) = tokio::join!(
        reader.search_with_route(
            std::slice::from_ref(&endpoint),
            SearchRequest::new("fastowner", SearchPurpose::LocalAssistant, 5),
        ),
        reader.search_with_route(
            std::slice::from_ref(&endpoint),
            SearchRequest::new("slowowner", SearchPurpose::LocalAssistant, 5),
        ),
    );
    let fast_result = fast_result.unwrap();
    let slow_result = slow_result.unwrap();

    assert_eq!(fast_result.response.hits.len(), 1);
    assert_eq!(fast_result.route_kind, PublicRouteKind::Direct);
    tokio::time::timeout(Duration::from_secs(1), slow_search_started.notified())
        .await
        .expect("slow owner received the request before its response budget elapsed");
    assert!(slow_result.response.partial);
    assert!(slow_result.response.hits.is_empty());
    assert_eq!(slow_result.route_kind, PublicRouteKind::Offline);

    release_slow_search.notify_one();
    fast_source_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), fast_source_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    slow_source_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), slow_source_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    catalog_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), catalog_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn public_search_and_browse_use_outbound_relay_reservation() {
    let (index_port, source_port) = available_udp_ports();
    let index_identity = identity();
    let source_identity = identity();
    let collection_id = Uuid::new_v4();
    let index_address: Multiaddr = format!("/ip4/127.0.0.1/udp/{index_port}/quic-v1")
        .parse()
        .unwrap();
    let source_address: Multiaddr = format!("/ip4/127.0.0.1/udp/{source_port}/quic-v1")
        .parse()
        .unwrap();
    let catalog_cancellation = CancellationToken::new();
    let source_cancellation = CancellationToken::new();
    let catalog_task = tokio::spawn(run_public_catalog_server(
        index_identity.clone(),
        airwiki_network::PublicCatalogServerConfig::new(vec![index_address.clone()])
            .with_external_addresses(vec![
                format!("/ip4/8.8.8.8/udp/{index_port}/quic-v1")
                    .parse()
                    .unwrap(),
            ]),
        Arc::new(CatalogBackend::new(Arc::new(
            CatalogStore::in_memory().unwrap(),
        ))),
        catalog_cancellation.clone(),
    ));
    let mut source_config = PublicSourceServerConfig::new(vec![source_address]);
    source_config.relay_addresses = vec![relay_circuit_address(
        index_address.clone(),
        index_identity.peer_id(),
    )];
    let source_backend = Arc::new(PublicFixtureBackend {
        gate: DisclosureGate::default(),
        publisher_id: source_identity.peer_id().to_string(),
        manifest_sequence: 1,
    });
    let delayed_source_backend = Arc::new(DelayedPublicSourceBackend {
        inner: Arc::clone(&source_backend),
        delay: Duration::from_millis(650),
    });
    let source_task = tokio::spawn(run_public_source_server(
        source_identity.clone(),
        source_config,
        delayed_source_backend.clone(),
        source_cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(250)).await;

    let endpoint = PublicIndexEndpoint {
        peer_id: index_identity.peer_id(),
        address: index_address.clone(),
    };
    let now = Utc::now();
    let relayed_route = relayed_peer_address(
        index_address,
        index_identity.peer_id(),
        source_identity.peer_id(),
    );
    let manifest = sign_manifest(
        source_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: source_identity.peer_id().to_string(),
            collection_id,
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Atlas public runbooks".to_owned(),
            description: "Synthetic public collection".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["atlas".to_owned(), "recovery".to_owned()],
            routes: vec![relayed_route.to_string()],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .unwrap();
    let reader = PublicReader::new();
    reader
        .register_manifest(std::slice::from_ref(&endpoint), manifest.clone())
        .await
        .unwrap();

    let result = reader
        .search_with_route(
            std::slice::from_ref(&endpoint),
            SearchRequest::new("atlas recovery", SearchPurpose::LocalAssistant, 5),
        )
        .await
        .unwrap();
    assert_eq!(result.response.hits.len(), 1);
    assert_eq!(result.route_kind, PublicRouteKind::Relay);
    let browsed = reader
        .browse_collection(
            &source_identity.peer_id().to_string(),
            collection_id,
            PublicBrowseOptions {
                cursor: None,
                target_concept_id: result.response.hits.first().map(|hit| hit.concept_id),
                graph_cursor: None,
                page: None,
                limit: 50,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        browsed.availability,
        airwiki_network::PublicCollectionAvailability::Available(PublicRouteKind::Relay)
    );
    assert_eq!(browsed.page.unwrap().concepts.len(), 1);

    source_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), source_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, source_port))
        .expect("relay shutdown should release the source QUIC listener before returning");

    let restarted_cancellation = CancellationToken::new();
    let mut restarted_config = PublicSourceServerConfig::new(vec![
        format!("/ip4/127.0.0.1/udp/{source_port}/quic-v1")
            .parse()
            .unwrap(),
    ]);
    restarted_config.relay_addresses = vec![relay_circuit_address(
        endpoint.address.clone(),
        index_identity.peer_id(),
    )];
    let restarted_task = tokio::spawn(run_public_source_server(
        source_identity,
        restarted_config,
        delayed_source_backend,
        restarted_cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(250)).await;

    let result = reader
        .search_with_route(
            std::slice::from_ref(&endpoint),
            SearchRequest::new("atlas recovery", SearchPurpose::LocalAssistant, 5),
        )
        .await
        .unwrap();
    assert_eq!(result.response.hits.len(), 1);
    assert_eq!(result.route_kind, PublicRouteKind::Relay);

    restarted_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), restarted_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    catalog_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), catalog_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn public_search_and_browse_fail_over_to_second_outbound_relay_reservation() {
    let (first_index_port, second_index_port) = available_tcp_ports();
    let source_port = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let first_index_identity = identity();
    let second_index_identity = identity();
    let source_identity = identity();
    let collection_id = Uuid::new_v4();
    let first_index_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{first_index_port}")
        .parse()
        .unwrap();
    let second_index_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{second_index_port}")
        .parse()
        .unwrap();
    let source_address: Multiaddr = format!("/ip4/127.0.0.1/udp/{source_port}/quic-v1")
        .parse()
        .unwrap();
    let first_catalog_cancellation = CancellationToken::new();
    let second_catalog_cancellation = CancellationToken::new();
    let source_cancellation = CancellationToken::new();
    let first_catalog_task = tokio::spawn(run_public_catalog_server(
        first_index_identity.clone(),
        airwiki_network::PublicCatalogServerConfig::new(vec![first_index_address.clone()])
            .with_external_addresses(vec![
                format!("/ip4/8.8.8.8/tcp/{first_index_port}")
                    .parse()
                    .unwrap(),
            ]),
        Arc::new(CatalogBackend::new(Arc::new(
            CatalogStore::in_memory().unwrap(),
        ))),
        first_catalog_cancellation.clone(),
    ));
    let second_catalog_task = tokio::spawn(run_public_catalog_server(
        second_index_identity.clone(),
        airwiki_network::PublicCatalogServerConfig::new(vec![second_index_address.clone()])
            .with_external_addresses(vec![
                format!("/ip4/1.1.1.1/tcp/{second_index_port}")
                    .parse()
                    .unwrap(),
            ]),
        Arc::new(CatalogBackend::new(Arc::new(
            CatalogStore::in_memory().unwrap(),
        ))),
        second_catalog_cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (readiness_sender, mut relay_readiness) = tokio::sync::watch::channel(Default::default());
    let mut source_config = PublicSourceServerConfig::new(vec![source_address]);
    source_config.relay_addresses = vec![
        relay_circuit_address(first_index_address.clone(), first_index_identity.peer_id()),
        relay_circuit_address(
            second_index_address.clone(),
            second_index_identity.peer_id(),
        ),
    ];
    source_config.relay_readiness = Some(readiness_sender);
    let source_task = tokio::spawn(run_public_source_server(
        source_identity.clone(),
        source_config,
        Arc::new(PublicFixtureBackend {
            gate: DisclosureGate::default(),
            publisher_id: source_identity.peer_id().to_string(),
            manifest_sequence: 2,
        }),
        source_cancellation.clone(),
    ));
    let readiness_result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if relay_readiness.borrow_and_update().ready_relay_count() == 2 {
                break;
            }
            relay_readiness.changed().await.unwrap();
        }
    })
    .await;
    assert!(
        readiness_result.is_ok(),
        "both outbound relay reservations should become ready; ready count was {}",
        relay_readiness.borrow().ready_relay_count()
    );
    let first_endpoint = PublicIndexEndpoint {
        peer_id: first_index_identity.peer_id(),
        address: first_index_address.clone(),
    };
    let second_endpoint = PublicIndexEndpoint {
        peer_id: second_index_identity.peer_id(),
        address: second_index_address.clone(),
    };
    let endpoints = [first_endpoint, second_endpoint];
    let first_relayed_route = relayed_peer_address(
        first_index_address,
        first_index_identity.peer_id(),
        source_identity.peer_id(),
    );
    let second_relayed_route = relayed_peer_address(
        second_index_address.clone(),
        second_index_identity.peer_id(),
        source_identity.peer_id(),
    );
    let now = Utc::now();
    let initial_manifest = sign_manifest(
        source_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: source_identity.peer_id().to_string(),
            collection_id,
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Atlas public runbooks".to_owned(),
            description: "Synthetic public collection".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["atlas".to_owned(), "recovery".to_owned()],
            routes: vec![
                first_relayed_route.to_string(),
                second_relayed_route.to_string(),
            ],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .unwrap();
    let reader = PublicReader::new();
    assert_eq!(
        reader
            .register_manifest(&endpoints, initial_manifest)
            .await
            .unwrap(),
        2
    );

    first_catalog_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), first_catalog_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let readiness = relay_readiness.borrow_and_update().clone();
            if readiness.ready_relay_count() == 1 {
                break;
            }
            relay_readiness.changed().await.unwrap();
        }
    })
    .await
    .expect("the surviving outbound relay reservation should remain ready");
    assert_eq!(
        relay_readiness.borrow().ready_relay_addresses(),
        [relay_circuit_address(
            second_index_address.clone(),
            second_index_identity.peer_id(),
        )]
    );
    let surviving_manifest = sign_manifest(
        source_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: source_identity.peer_id().to_string(),
            collection_id,
            sequence: 2,
            publication_fingerprint: "a".repeat(64),
            name: "Atlas public runbooks".to_owned(),
            description: "Synthetic public collection".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["atlas".to_owned(), "recovery".to_owned()],
            routes: vec![second_relayed_route.to_string()],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .unwrap();
    assert_eq!(
        reader
            .register_manifest(&endpoints, surviving_manifest)
            .await
            .unwrap(),
        1
    );

    let result = reader
        .search_with_route(
            &endpoints,
            SearchRequest::new("atlas recovery", SearchPurpose::LocalAssistant, 5),
        )
        .await
        .unwrap();
    assert_eq!(result.response.hits.len(), 1);
    assert!(result.response.partial);
    assert_eq!(result.route_kind, PublicRouteKind::Relay);
    let browsed = reader
        .browse_collection(
            &source_identity.peer_id().to_string(),
            collection_id,
            PublicBrowseOptions {
                cursor: None,
                target_concept_id: result.response.hits.first().map(|hit| hit.concept_id),
                graph_cursor: None,
                page: None,
                limit: 50,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        browsed.availability,
        airwiki_network::PublicCollectionAvailability::Available(PublicRouteKind::Relay)
    );
    assert_eq!(browsed.page.unwrap().concepts.len(), 1);

    source_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), source_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    second_catalog_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), second_catalog_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn public_source_retries_relay_reservation_when_relay_starts_late() {
    let (index_port, source_port) = available_udp_ports();
    let index_identity = identity();
    let source_identity = identity();
    let collection_id = Uuid::new_v4();
    let index_address: Multiaddr = format!("/ip4/127.0.0.1/udp/{index_port}/quic-v1")
        .parse()
        .unwrap();
    let source_address: Multiaddr = format!("/ip4/127.0.0.1/udp/{source_port}/quic-v1")
        .parse()
        .unwrap();
    let source_cancellation = CancellationToken::new();
    let mut source_config = PublicSourceServerConfig::new(vec![source_address]);
    source_config.relay_addresses = vec![relay_circuit_address(
        index_address.clone(),
        index_identity.peer_id(),
    )];
    let source_task = tokio::spawn(run_public_source_server(
        source_identity.clone(),
        source_config,
        Arc::new(PublicFixtureBackend {
            gate: DisclosureGate::default(),
            publisher_id: source_identity.peer_id().to_string(),
            manifest_sequence: 1,
        }),
        source_cancellation.clone(),
    ));

    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !source_task.is_finished(),
        "a transient relay outage must not stop the public source"
    );

    let catalog_cancellation = CancellationToken::new();
    let catalog_task = tokio::spawn(run_public_catalog_server(
        index_identity.clone(),
        airwiki_network::PublicCatalogServerConfig::new(vec![index_address.clone()])
            .with_external_addresses(vec![
                format!("/ip4/8.8.8.8/udp/{index_port}/quic-v1")
                    .parse()
                    .unwrap(),
            ]),
        Arc::new(CatalogBackend::new(Arc::new(
            CatalogStore::in_memory().unwrap(),
        ))),
        catalog_cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let endpoint = PublicIndexEndpoint {
        peer_id: index_identity.peer_id(),
        address: index_address.clone(),
    };
    let now = Utc::now();
    let manifest = sign_manifest(
        source_identity.keypair(),
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: source_identity.peer_id().to_string(),
            collection_id,
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Atlas public runbooks".to_owned(),
            description: "Synthetic public collection".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["atlas".to_owned(), "recovery".to_owned()],
            routes: vec![
                relayed_peer_address(
                    index_address,
                    index_identity.peer_id(),
                    source_identity.peer_id(),
                )
                .to_string(),
            ],
            updated_at: now,
            expires_at: now + ChronoDuration::minutes(15),
        },
    )
    .unwrap();
    let reader = PublicReader::new();
    reader
        .register_manifest(std::slice::from_ref(&endpoint), manifest)
        .await
        .unwrap();

    let response = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(response) = reader
                .search(
                    std::slice::from_ref(&endpoint),
                    SearchRequest::new("atlas recovery", SearchPurpose::LocalAssistant, 5),
                )
                .await
                && !response.hits.is_empty()
            {
                return response;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the source should reserve the relay after it becomes available");
    assert_eq!(response.hits.len(), 1);

    source_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), source_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    catalog_cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), catalog_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

fn identity() -> NodeIdentity {
    NodeIdentity::load_or_create(&MemorySecretStore::default()).unwrap()
}

fn available_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn available_tcp_ports() -> (u16, u16) {
    let first = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let second = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    (
        first.local_addr().unwrap().port(),
        second.local_addr().unwrap().port(),
    )
}

fn available_udp_ports() -> (u16, u16) {
    let first = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let second = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    (
        first.local_addr().unwrap().port(),
        second.local_addr().unwrap().port(),
    )
}
