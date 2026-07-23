use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use airwiki_federation_index::{CatalogBackend, CatalogStore};
use airwiki_network::{
    MemorySecretStore, Multiaddr, NodeIdentity, PublicBrowseDelivery, PublicIndexEndpoint,
    PublicReader, PublicRouteKind, PublicSearchDelivery, PublicSourceBackend,
    PublicSourceBackendError, PublicSourceServerConfig, relay_circuit_address,
    relayed_peer_address, run_public_catalog_server, run_public_source_server, sign_manifest,
};
use airwiki_types::{
    ConceptType, DisclosureGate, PUBLIC_BROWSE_PROTOCOL, PUBLIC_CATALOG_PROTOCOL,
    PUBLIC_SEARCH_PROTOCOL, PublicBrowsePage, PublicBrowseRequest, PublicCollectionManifest,
    PublicCollectionRevision, PublicSearchRequest, PublicSearchResponse, SearchHit, SearchPurpose,
    SearchRequest, SearchResponse,
};
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug)]
struct PublicFixtureBackend {
    gate: DisclosureGate,
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
            concept_id: Uuid::new_v4(),
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
                manifest_sequence: 1,
                concepts: vec![airwiki_types::PublicConceptSummary {
                    publisher_id: "replaced-by-transport".to_owned(),
                    collection_id: request.collection_id,
                    concept_id: Uuid::new_v4(),
                    concept_type: ConceptType::Procedure,
                    title: "Atlas recovery".to_owned(),
                    description: "Synthetic procedure".to_owned(),
                    language: "en".to_owned(),
                    tags: vec!["atlas".to_owned()],
                    summary: "Restart the synthetic queue.".to_owned(),
                    logical_resource_uri: "urn:airwiki:atlas:recovery".to_owned(),
                    source_revision: 1,
                    updated_at: now,
                }],
                next_cursor: None,
            },
            self.gate.acquire_disclosure(),
        ))
    }
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
    let response = reader
        .search(
            &[endpoint],
            SearchRequest::new("atlas recovery", SearchPurpose::LocalAssistant, 5),
        )
        .await
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert_eq!(response.hits[0].collection_id, collection_id);
    let page = reader.browse(&manifest, None, 50).await.unwrap();
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
                format!("/dns4/relay.invalid/udp/{index_port}/quic-v1")
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
    });
    let source_task = tokio::spawn(run_public_source_server(
        source_identity.clone(),
        source_config,
        source_backend.clone(),
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

    let response = reader
        .search(
            std::slice::from_ref(&endpoint),
            SearchRequest::new("atlas recovery", SearchPurpose::LocalAssistant, 5),
        )
        .await
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert_eq!(reader.route_kind(), PublicRouteKind::Relay);
    let page = reader.browse(&manifest, None, 50).await.unwrap();
    assert_eq!(page.concepts.len(), 1);
    assert_eq!(reader.route_kind(), PublicRouteKind::Relay);

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
        source_backend,
        restarted_cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(250)).await;

    let response = reader
        .search(
            std::slice::from_ref(&endpoint),
            SearchRequest::new("atlas recovery", SearchPurpose::LocalAssistant, 5),
        )
        .await
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert_eq!(reader.route_kind(), PublicRouteKind::Relay);

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

fn available_udp_ports() -> (u16, u16) {
    let first = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let second = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    (
        first.local_addr().unwrap().port(),
        second.local_addr().unwrap().port(),
    )
}
