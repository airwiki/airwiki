use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::Duration;

use airwiki_types::{
    PUBLIC_BROWSE_PROTOCOL, PUBLIC_CATALOG_PROTOCOL, PUBLIC_SEARCH_PROTOCOL, PublicBrowsePage,
    PublicBrowseRequest, PublicCatalogQuery, PublicCollectionSummary, PublicCollectionTarget,
    PublicSearchRequest, SearchContractError, SearchHit, SearchRequest, SearchResponse,
    SignedPublicCollectionManifest, SignedPublicCollectionTombstone,
};
use libp2p::identity::Keypair;
use libp2p::request_response::{self, OutboundRequestId, ProtocolSupport};
use libp2p::swarm::{ConnectionId, NetworkBehaviour, SwarmEvent};
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder};
use tokio::sync::{Semaphore, mpsc};
use tokio::time::{Instant, timeout_at};

use crate::{
    CatalogWireRequest, CatalogWireResponse, NetworkError, PublicBrowseWireResponse,
    PublicSearchWireResponse, PublicSourceRejection, verify_manifest,
};

const INDEX_DEADLINE: Duration = Duration::from_millis(1_000);
const OWNER_CONNECT_BUDGET: Duration = Duration::from_secs(3);
const OWNER_RESPONSE_BUDGET: Duration = Duration::from_millis(800);
const MAX_INDEXES: usize = 3;
const MAX_PUBLIC_PEERS: usize = 12;
const RRF_K: f64 = 60.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicIndexEndpoint {
    pub peer_id: PeerId,
    pub address: Multiaddr,
}

#[derive(Debug)]
pub struct PublicReader {
    identity: Keypair,
    searches: Semaphore,
    manifests: tokio::sync::RwLock<HashMap<(String, uuid::Uuid), SignedPublicCollectionManifest>>,
    blocked_publishers: tokio::sync::RwLock<HashSet<String>>,
}

/// Reachability observed for the owner of a public collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicRouteKind {
    /// No successful public route has been observed for the current request.
    Offline,
    /// The owner answered through a circuit relay.
    Relay,
    /// The owner answered over a direct transport.
    Direct,
}

/// Public search response and the route used by an owner that returned a
/// protocol-valid response.
#[derive(Debug, Clone)]
pub struct PublicSearchResult {
    pub response: SearchResponse,
    pub route_kind: PublicRouteKind,
}

#[derive(Debug, Clone, Copy)]
struct OwnerDeadlines {
    connect: Instant,
    finish: Instant,
}

/// Current availability of a federated public collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCollectionAvailability {
    /// The owner answered and the route class is known.
    Available(PublicRouteKind),
    /// The signed collection manifest has expired.
    Expired,
    /// The owner could not be reached before the public deadline.
    Offline,
}

/// Public collection metadata, optional page content, and current availability.
#[derive(Debug, Clone)]
pub struct PublicBrowseResult {
    /// Signed collection profile selected by the routing indexes.
    pub summary: PublicCollectionSummary,
    /// Page returned by the owner; absent for expired or offline collections.
    pub page: Option<PublicBrowsePage>,
    /// Availability observed while resolving this page.
    pub availability: PublicCollectionAvailability,
}

impl Default for PublicReader {
    fn default() -> Self {
        Self::new()
    }
}

impl PublicReader {
    pub fn new() -> Self {
        Self {
            identity: Keypair::generate_ed25519(),
            searches: Semaphore::new(2),
            manifests: tokio::sync::RwLock::new(HashMap::new()),
            blocked_publishers: tokio::sync::RwLock::new(HashSet::new()),
        }
    }

    pub async fn set_publisher_blocked(&self, publisher_id: String, blocked: bool) {
        let mut publishers = self.blocked_publishers.write().await;
        if blocked {
            publishers.insert(publisher_id);
        } else {
            publishers.remove(&publisher_id);
        }
    }

    pub async fn search(
        &self,
        indexes: &[PublicIndexEndpoint],
        request: SearchRequest,
    ) -> Result<SearchResponse, SearchContractError> {
        Ok(self.search_inner(indexes, request, None).await?.response)
    }

    pub async fn search_with_route(
        &self,
        indexes: &[PublicIndexEndpoint],
        request: SearchRequest,
    ) -> Result<PublicSearchResult, SearchContractError> {
        self.search_inner(indexes, request, None).await
    }

    pub async fn search_with_partials(
        &self,
        indexes: &[PublicIndexEndpoint],
        request: SearchRequest,
        partials: mpsc::Sender<SearchResponse>,
    ) -> Result<SearchResponse, SearchContractError> {
        Ok(self
            .search_inner(indexes, request, Some(&partials))
            .await?
            .response)
    }

    pub async fn search_with_route_and_partials(
        &self,
        indexes: &[PublicIndexEndpoint],
        request: SearchRequest,
        partials: mpsc::Sender<SearchResponse>,
    ) -> Result<PublicSearchResult, SearchContractError> {
        self.search_inner(indexes, request, Some(&partials)).await
    }

    async fn search_inner(
        &self,
        indexes: &[PublicIndexEndpoint],
        request: SearchRequest,
        partials: Option<&mpsc::Sender<SearchResponse>>,
    ) -> Result<PublicSearchResult, SearchContractError> {
        request.validate()?;
        let _permit = self.searches.acquire().await.map_err(|_| {
            SearchContractError::Unavailable("public reader is shutting down".to_owned())
        })?;
        let started = Instant::now();
        let mut swarm = reader_swarm(self.identity.clone())
            .map_err(|error| SearchContractError::Unavailable(error.to_string()))?;
        let catalog_query = PublicCatalogQuery {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            request_id: request.request_id,
            query: request.query.clone(),
            languages: Vec::new(),
            limit: airwiki_types::MAX_PUBLIC_CANDIDATES,
        };
        let mut pending_catalog = HashSet::new();
        for endpoint in bounded_indexes(indexes) {
            swarm.add_peer_address(endpoint.peer_id, endpoint.address.clone());
            pending_catalog.insert(swarm.behaviour_mut().catalog.send_request(
                &endpoint.peer_id,
                CatalogWireRequest::Query(catalog_query.clone()),
            ));
        }
        if pending_catalog.is_empty() {
            return Err(SearchContractError::Unavailable(
                "no public federation index is configured".to_owned(),
            ));
        }
        let mut manifests = Vec::new();
        let mut catalog_state = CatalogQueryState::default();
        let index_deadline = public_index_deadline(started);
        while !pending_catalog.is_empty() {
            let event = match timeout_at(
                index_deadline,
                futures::StreamExt::select_next_some(&mut swarm),
            )
            .await
            {
                Ok(event) => event,
                Err(_) => break,
            };
            collect_catalog_event(
                event,
                &mut pending_catalog,
                &mut manifests,
                &mut catalog_state,
            );
        }
        catalog_state.failed = catalog_state.failed.saturating_add(pending_catalog.len());
        let catalog_partial = catalog_query_is_partial(catalog_state)?;
        let candidates = {
            let blocked = self.blocked_publishers.read().await;
            select_candidates(manifests)
                .into_iter()
                .filter(|candidate| !blocked.contains(&candidate.manifest.publisher_id))
                .collect::<Vec<_>>()
        };
        if candidates.is_empty() {
            return Ok(PublicSearchResult {
                response: public_search_response(request.request_id, Vec::new(), catalog_partial),
                route_kind: PublicRouteKind::Offline,
            });
        }
        {
            let mut cache = self.manifests.write().await;
            for candidate in &candidates {
                cache.insert(
                    (
                        candidate.manifest.publisher_id.clone(),
                        candidate.manifest.collection_id,
                    ),
                    candidate.clone(),
                );
            }
            cache.retain(|_, manifest| manifest.manifest.expires_at > chrono::Utc::now());
        }
        let groups = group_candidates_by_peer(candidates);
        let owner_peers = groups.iter().map(|(peer, _)| *peer).collect::<HashSet<_>>();
        let mut pending_search = HashMap::<OutboundRequestId, PendingOwnerSearch>::new();
        for (peer, collections) in groups {
            let blocked = self.blocked_publishers.read().await;
            if collections
                .first()
                .is_some_and(|manifest| blocked.contains(&manifest.manifest.publisher_id))
            {
                continue;
            }
            for manifest in &collections {
                for route in &manifest.manifest.routes {
                    if let Ok(address) = Multiaddr::from_str(route) {
                        swarm.add_peer_address(peer, address);
                    }
                }
            }
            let public_request = PublicSearchRequest {
                protocol_version: PUBLIC_SEARCH_PROTOCOL.to_owned(),
                request_id: request.request_id,
                query: request.query.clone(),
                purpose: request.purpose,
                collections: collections
                    .iter()
                    .map(|manifest| PublicCollectionTarget {
                        collection_id: manifest.manifest.collection_id,
                        manifest_sequence: manifest.manifest.sequence,
                        publication_fingerprint: manifest.manifest.publication_fingerprint.clone(),
                    })
                    .collect(),
                top_k: request.top_k,
            };
            let request_id = swarm
                .behaviour_mut()
                .search
                .send_request(&peer, public_request);
            pending_search.insert(
                request_id,
                PendingOwnerSearch {
                    peer,
                    manifests: collections,
                },
            );
        }
        let owner_deadlines = public_owner_deadlines(Instant::now());
        let mut route_tracker = OwnerRouteTracker::new(owner_peers, owner_deadlines.connect);
        let mut accepted_routes = HashMap::new();
        let mut sources = Vec::new();
        let mut partial = catalog_partial;
        while !pending_search.is_empty() {
            let event = match timeout_at(
                owner_deadlines.finish,
                futures::StreamExt::select_next_some(&mut swarm),
            )
            .await
            {
                Ok(event) => event,
                Err(_) => {
                    let pending_peers = pending_owner_peers(&pending_search);
                    tracing::warn!(
                        error_kind = route_tracker.timeout_error_kind(&pending_peers),
                        pending_owner_count = pending_search.len(),
                        connected_owner_count = route_tracker.connected_owner_count(&pending_peers),
                        "public owner stage timed out"
                    );
                    partial = true;
                    break;
                }
            };
            let observed_at = Instant::now();
            route_tracker.observe_swarm_event(&event, observed_at);
            let blocked = self.blocked_publishers.read().await;
            let accepted_route = collect_search_event(
                ObservedReaderEvent {
                    event,
                    at: observed_at,
                },
                ExpectedSearchResponse {
                    request_id: request.request_id,
                    top_k: request.top_k,
                },
                &mut pending_search,
                &mut sources,
                &mut partial,
                &route_tracker,
                &blocked,
            );
            let accepted_source = accepted_route.is_some();
            retain_unblocked_sources(&mut sources, &blocked);
            accepted_routes.retain(|publisher_id, _| !blocked.contains(publisher_id));
            if let Some(accepted_route) = accepted_route {
                accepted_routes.insert(accepted_route.publisher_id, accepted_route.route_kind);
            }
            if accepted_source && let Some(partials) = partials {
                emit_partial(partials, request.request_id, request.top_k, &sources);
            }
            if !pending_search.is_empty()
                && pending_cannot_change_top_k(
                    &sources,
                    pending_search.len(),
                    usize::from(request.top_k),
                )
            {
                pending_search.clear();
                break;
            }
        }
        partial |= !pending_search.is_empty();
        let blocked = self.blocked_publishers.read().await;
        retain_unblocked_sources(&mut sources, &blocked);
        accepted_routes.retain(|publisher_id, _| !blocked.contains(publisher_id));
        let route_kind = accepted_routes
            .into_values()
            .fold(PublicRouteKind::Offline, merge_route_kind);
        let mut hits = fuse_rankings(sources);
        hits.truncate(usize::from(request.top_k));
        for (position, hit) in hits.iter_mut().enumerate() {
            hit.rank = u32::try_from(position + 1).unwrap_or(u32::MAX);
        }
        Ok(PublicSearchResult {
            response: public_search_response(request.request_id, hits, partial),
            route_kind,
        })
    }

    pub async fn browse(
        &self,
        manifest: &SignedPublicCollectionManifest,
        cursor: Option<String>,
        limit: u8,
    ) -> Result<PublicBrowsePage, SearchContractError> {
        let result = self.browse_with_route(manifest, cursor, limit).await?;
        let blocked = self.blocked_publishers.read().await;
        if blocked.contains(&manifest.manifest.publisher_id) {
            return Err(SearchContractError::Unauthorized);
        }
        Ok(result.page)
    }

    async fn browse_with_route(
        &self,
        manifest: &SignedPublicCollectionManifest,
        cursor: Option<String>,
        limit: u8,
    ) -> Result<RoutedPublicBrowsePage, SearchContractError> {
        if self
            .blocked_publishers
            .read()
            .await
            .contains(&manifest.manifest.publisher_id)
        {
            return Err(SearchContractError::Unauthorized);
        }
        verify_manifest(manifest, chrono::Utc::now())
            .map_err(|_| SearchContractError::Unauthorized)?;
        let peer = PeerId::from_str(&manifest.manifest.publisher_id)
            .map_err(|_| SearchContractError::Unauthorized)?;
        let mut swarm = reader_swarm(self.identity.clone())
            .map_err(|error| SearchContractError::Unavailable(error.to_string()))?;
        for route in &manifest.manifest.routes {
            if let Ok(address) = Multiaddr::from_str(route) {
                swarm.add_peer_address(peer, address);
            }
        }
        let request = PublicBrowseRequest {
            protocol_version: PUBLIC_BROWSE_PROTOCOL.to_owned(),
            request_id: uuid::Uuid::new_v4(),
            collection_id: manifest.manifest.collection_id,
            cursor,
            limit,
        };
        request
            .validate()
            .map_err(|error| SearchContractError::Backend(error.to_string()))?;
        let outbound = swarm
            .behaviour_mut()
            .browse
            .send_request(&peer, request.clone());
        let deadlines = public_owner_deadlines(Instant::now());
        let owner_peers = HashSet::from([peer]);
        let mut route_tracker = OwnerRouteTracker::new(owner_peers.clone(), deadlines.connect);
        loop {
            let event = timeout_at(
                deadlines.finish,
                futures::StreamExt::select_next_some(&mut swarm),
            )
            .await
            .map_err(|_| {
                tracing::warn!(
                    error_kind = route_tracker.timeout_error_kind(&owner_peers),
                    connected_owner_count = route_tracker.connected_owner_count(&owner_peers),
                    "public owner browse stage timed out"
                );
                SearchContractError::Unavailable("public browse timed out".to_owned())
            })?;
            let observed_at = Instant::now();
            route_tracker.observe_swarm_event(&event, observed_at);
            match event {
                SwarmEvent::Behaviour(ReaderBehaviourEvent::Browse(
                    request_response::Event::Message {
                        peer: response_peer,
                        connection_id,
                        message:
                            request_response::Message::Response {
                                request_id,
                                response,
                            },
                        ..
                    },
                )) if request_id == outbound => {
                    return match response {
                        PublicBrowseWireResponse::Success(page) => {
                            let blocked = self.blocked_publishers.read().await;
                            if blocked.contains(&manifest.manifest.publisher_id) {
                                return Err(SearchContractError::Unauthorized);
                            }
                            if page.manifest_sequence < manifest.manifest.sequence
                                || page
                                    .validate_for(&request, &manifest.manifest.publisher_id)
                                    .is_err()
                                || response_peer != peer
                            {
                                return Err(SearchContractError::Unauthorized);
                            }
                            let route_kind = match route_tracker.route_for_response(
                                response_peer,
                                connection_id,
                                observed_at,
                            ) {
                                Ok(route_kind) => route_kind,
                                Err(error_kind) => {
                                    tracing::warn!(
                                        error_kind,
                                        "public owner response could not be tied to its connection"
                                    );
                                    return Err(SearchContractError::Unavailable(
                                        "public browse route is unavailable".to_owned(),
                                    ));
                                }
                            };
                            Ok(RoutedPublicBrowsePage { page, route_kind })
                        }
                        PublicBrowseWireResponse::Rejected(
                            PublicSourceRejection::Invalid | PublicSourceRejection::NotPublic,
                        ) => Err(SearchContractError::Unauthorized),
                        PublicBrowseWireResponse::Rejected(
                            PublicSourceRejection::Busy | PublicSourceRejection::Unavailable,
                        ) => Err(SearchContractError::Unavailable(
                            "public browse source is unavailable".to_owned(),
                        )),
                    };
                }
                SwarmEvent::Behaviour(ReaderBehaviourEvent::Browse(
                    request_response::Event::OutboundFailure {
                        request_id, error, ..
                    },
                )) if request_id == outbound => {
                    log_public_owner_outbound_failure(&error);
                    return Err(SearchContractError::Unavailable(
                        "public browse source is unavailable".to_owned(),
                    ));
                }
                _ => {}
            }
        }
    }

    pub async fn browse_collection(
        &self,
        publisher_id: &str,
        collection_id: uuid::Uuid,
        cursor: Option<String>,
        limit: u8,
    ) -> Result<PublicBrowseResult, SearchContractError> {
        let manifest = self
            .manifests
            .read()
            .await
            .get(&(publisher_id.to_owned(), collection_id))
            .cloned()
            .ok_or_else(|| {
                SearchContractError::Unavailable(
                    "public collection route is no longer available".to_owned(),
                )
            })?;
        if self
            .blocked_publishers
            .read()
            .await
            .contains(&manifest.manifest.publisher_id)
        {
            return Err(SearchContractError::Unauthorized);
        }
        let summary = manifest.manifest.summary();
        if manifest.manifest.expires_at <= chrono::Utc::now() {
            let blocked = self.blocked_publishers.read().await;
            if blocked.contains(&manifest.manifest.publisher_id) {
                return Err(SearchContractError::Unauthorized);
            }
            return Ok(PublicBrowseResult {
                summary,
                page: None,
                availability: PublicCollectionAvailability::Expired,
            });
        }
        let result = self.browse_with_route(&manifest, cursor, limit).await;
        let blocked = self.blocked_publishers.read().await;
        if blocked.contains(&manifest.manifest.publisher_id) {
            return Err(SearchContractError::Unauthorized);
        }
        match result {
            Ok(result) => Ok(PublicBrowseResult {
                summary,
                page: Some(result.page),
                availability: PublicCollectionAvailability::Available(result.route_kind),
            }),
            Err(SearchContractError::Unavailable(_)) => Ok(PublicBrowseResult {
                summary,
                page: None,
                availability: PublicCollectionAvailability::Offline,
            }),
            Err(error) => Err(error),
        }
    }

    pub async fn register_manifest(
        &self,
        indexes: &[PublicIndexEndpoint],
        manifest: SignedPublicCollectionManifest,
    ) -> Result<usize, SearchContractError> {
        self.catalog_update(indexes, CatalogWireRequest::Register(manifest))
            .await
    }

    pub async fn withdraw_manifest(
        &self,
        indexes: &[PublicIndexEndpoint],
        tombstone: SignedPublicCollectionTombstone,
    ) -> Result<usize, SearchContractError> {
        self.catalog_update(indexes, CatalogWireRequest::Withdraw(tombstone))
            .await
    }

    async fn catalog_update(
        &self,
        indexes: &[PublicIndexEndpoint],
        update: CatalogWireRequest,
    ) -> Result<usize, SearchContractError> {
        let mut swarm = reader_swarm(self.identity.clone())
            .map_err(|error| SearchContractError::Unavailable(error.to_string()))?;
        let mut pending = HashSet::new();
        for endpoint in bounded_indexes(indexes) {
            swarm.add_peer_address(endpoint.peer_id, endpoint.address.clone());
            pending.insert(
                swarm
                    .behaviour_mut()
                    .catalog
                    .send_request(&endpoint.peer_id, update.clone()),
            );
        }
        if pending.is_empty() {
            return Err(SearchContractError::Unavailable(
                "no public federation index is configured".to_owned(),
            ));
        }
        let deadline = Instant::now() + INDEX_DEADLINE;
        let mut accepted = 0_usize;
        while !pending.is_empty() {
            let event = match timeout_at(deadline, futures::StreamExt::select_next_some(&mut swarm))
                .await
            {
                Ok(event) => event,
                Err(_) => break,
            };
            match event {
                SwarmEvent::Behaviour(ReaderBehaviourEvent::Catalog(
                    request_response::Event::Message {
                        message:
                            request_response::Message::Response {
                                request_id,
                                response,
                            },
                        ..
                    },
                )) => {
                    pending.remove(&request_id);
                    if matches!(response, CatalogWireResponse::Accepted) {
                        accepted = accepted.saturating_add(1);
                    }
                }
                SwarmEvent::Behaviour(ReaderBehaviourEvent::Catalog(
                    request_response::Event::OutboundFailure { request_id, .. },
                )) => {
                    pending.remove(&request_id);
                }
                _ => {}
            }
        }
        if accepted == 0 {
            return Err(SearchContractError::Unavailable(
                "no public federation index accepted the update".to_owned(),
            ));
        }
        Ok(accepted)
    }
}

struct RoutedPublicBrowsePage {
    page: PublicBrowsePage,
    route_kind: PublicRouteKind,
}

struct OwnerRouteTracker {
    expected_peers: HashSet<PeerId>,
    connect_deadline: Instant,
    routes: HashMap<ConnectionId, OwnerConnectionRoute>,
}

#[derive(Debug, Clone, Copy)]
struct OwnerConnectionRoute {
    peer: PeerId,
    route_kind: PublicRouteKind,
    response_deadline: Instant,
}

impl OwnerRouteTracker {
    fn new(expected_peers: HashSet<PeerId>, connect_deadline: Instant) -> Self {
        Self {
            expected_peers,
            connect_deadline,
            routes: HashMap::new(),
        }
    }

    fn observe_swarm_event(
        &mut self,
        event: &SwarmEvent<ReaderBehaviourEvent>,
        observed_at: Instant,
    ) {
        match event {
            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            } if self.expected_peers.contains(peer_id) => {
                let route_kind = if endpoint.is_relayed() {
                    PublicRouteKind::Relay
                } else {
                    PublicRouteKind::Direct
                };
                self.record_connection(*peer_id, *connection_id, route_kind, observed_at);
            }
            SwarmEvent::ConnectionClosed { connection_id, .. } => {
                self.remove_connection(*connection_id);
            }
            _ => {}
        }
    }

    fn record_connection(
        &mut self,
        peer: PeerId,
        connection_id: ConnectionId,
        route_kind: PublicRouteKind,
        observed_at: Instant,
    ) {
        if self.expected_peers.contains(&peer) && observed_at <= self.connect_deadline {
            self.routes.insert(
                connection_id,
                OwnerConnectionRoute {
                    peer,
                    route_kind,
                    response_deadline: observed_at + OWNER_RESPONSE_BUDGET,
                },
            );
        }
    }

    fn remove_connection(&mut self, connection_id: ConnectionId) {
        self.routes.remove(&connection_id);
    }

    fn route_for_response(
        &self,
        peer: PeerId,
        connection_id: ConnectionId,
        observed_at: Instant,
    ) -> Result<PublicRouteKind, &'static str> {
        let route = self
            .routes
            .get(&connection_id)
            .filter(|route| route.peer == peer)
            .ok_or("public_owner_route_unavailable")?;
        if observed_at > route.response_deadline {
            return Err("public_owner_response_timeout");
        }
        Ok(route.route_kind)
    }

    fn connected_owner_count(&self, peers: &HashSet<PeerId>) -> usize {
        self.routes
            .values()
            .filter(|route| peers.contains(&route.peer))
            .map(|route| route.peer)
            .collect::<HashSet<_>>()
            .len()
    }

    fn timeout_error_kind(&self, peers: &HashSet<PeerId>) -> &'static str {
        match self.connected_owner_count(peers) {
            0 => "public_owner_connect_timeout",
            connected if connected < peers.len() => "public_owner_mixed_timeout",
            _ => "public_owner_response_timeout",
        }
    }
}

#[derive(NetworkBehaviour)]
struct ReaderBehaviour {
    catalog: request_response::cbor::Behaviour<CatalogWireRequest, CatalogWireResponse>,
    search: request_response::cbor::Behaviour<PublicSearchRequest, PublicSearchWireResponse>,
    browse: request_response::cbor::Behaviour<PublicBrowseRequest, PublicBrowseWireResponse>,
    relay: libp2p::relay::client::Behaviour,
    dcutr: libp2p::dcutr::Behaviour,
    limits: libp2p::connection_limits::Behaviour,
}

fn reader_swarm(identity: Keypair) -> Result<Swarm<ReaderBehaviour>, NetworkError> {
    let local_peer = identity.public().to_peer_id();
    let catalog = outbound_behaviour(
        PUBLIC_CATALOG_PROTOCOL,
        128 * 1024,
        512 * 1024,
        INDEX_DEADLINE,
    )?;
    let search = outbound_behaviour(
        PUBLIC_SEARCH_PROTOCOL,
        16 * 1024,
        256 * 1024,
        OWNER_RESPONSE_BUDGET,
    )?;
    let browse = outbound_behaviour(
        PUBLIC_BROWSE_PROTOCOL,
        16 * 1024,
        256 * 1024,
        OWNER_RESPONSE_BUDGET,
    )?;
    SwarmBuilder::with_existing_identity(identity)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .with_quic()
        .with_dns()
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .with_relay_client(libp2p::noise::Config::new, libp2p::yamux::Config::default)
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .with_behaviour(move |_, relay| ReaderBehaviour {
            catalog,
            search,
            browse,
            relay,
            dcutr: libp2p::dcutr::Behaviour::new(local_peer),
            limits: libp2p::connection_limits::Behaviour::new(
                libp2p::connection_limits::ConnectionLimits::default()
                    .with_max_pending_outgoing(Some(24))
                    .with_max_established_outgoing(Some(24))
                    .with_max_established(Some(24))
                    .with_max_established_per_peer(Some(2)),
            ),
        })
        .map_err(|error| NetworkError::Transport(error.to_string()))
        .map(|builder| builder.build())
}

fn outbound_behaviour<Request, Response>(
    protocol: &'static str,
    request_bytes: u64,
    response_bytes: u64,
    request_timeout: Duration,
) -> Result<request_response::cbor::Behaviour<Request, Response>, NetworkError>
where
    Request: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    Response: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
{
    let protocol = StreamProtocol::try_from_owned(protocol.to_owned())
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    let codec = request_response::cbor::codec::Codec::<Request, Response>::default()
        .set_request_size_maximum(request_bytes)
        .set_response_size_maximum(response_bytes);
    Ok(request_response::Behaviour::with_codec(
        codec,
        [(protocol, ProtocolSupport::Outbound)],
        request_response::Config::default()
            .with_request_timeout(request_timeout)
            .with_max_concurrent_streams(32),
    ))
}

fn collect_catalog_event(
    event: SwarmEvent<ReaderBehaviourEvent>,
    pending: &mut HashSet<OutboundRequestId>,
    manifests: &mut Vec<SignedPublicCollectionManifest>,
    state: &mut CatalogQueryState,
) {
    match event {
        SwarmEvent::Behaviour(ReaderBehaviourEvent::Catalog(
            request_response::Event::Message {
                message:
                    request_response::Message::Response {
                        request_id,
                        response,
                    },
                ..
            },
        )) => {
            if !pending.remove(&request_id) {
                return;
            }
            if let CatalogWireResponse::Results(results) = response {
                state.successful = state.successful.saturating_add(1);
                for manifest in results {
                    if verify_manifest(&manifest, chrono::Utc::now()).is_ok() {
                        manifests.push(manifest);
                    } else {
                        state.invalid_manifest = true;
                    }
                }
            } else {
                state.failed = state.failed.saturating_add(1);
            }
        }
        SwarmEvent::Behaviour(ReaderBehaviourEvent::Catalog(
            request_response::Event::OutboundFailure { request_id, .. },
        )) if pending.remove(&request_id) => {
            state.failed = state.failed.saturating_add(1);
        }
        _ => {}
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CatalogQueryState {
    successful: usize,
    failed: usize,
    invalid_manifest: bool,
}

fn bounded_indexes(indexes: &[PublicIndexEndpoint]) -> impl Iterator<Item = &PublicIndexEndpoint> {
    indexes.iter().take(MAX_INDEXES)
}

fn catalog_query_is_partial(state: CatalogQueryState) -> Result<bool, SearchContractError> {
    if state.successful == 0 {
        return Err(SearchContractError::Unavailable(
            "public federation indexes are offline".to_owned(),
        ));
    }
    Ok(state.failed > 0 || state.invalid_manifest)
}

fn public_search_response(
    request_id: uuid::Uuid,
    hits: Vec<SearchHit>,
    partial: bool,
) -> SearchResponse {
    SearchResponse {
        request_id,
        hits,
        authorized_candidates: Vec::new(),
        offline_nodes: Vec::new(),
        warnings: if partial {
            vec!["public search returned partial results".to_owned()]
        } else {
            Vec::new()
        },
        partial,
    }
}

struct ObservedReaderEvent {
    event: SwarmEvent<ReaderBehaviourEvent>,
    at: Instant,
}

#[derive(Clone, Copy)]
struct ExpectedSearchResponse {
    request_id: uuid::Uuid,
    top_k: u8,
}

struct PendingOwnerSearch {
    peer: PeerId,
    manifests: Vec<SignedPublicCollectionManifest>,
}

struct AcceptedOwnerRoute {
    publisher_id: String,
    route_kind: PublicRouteKind,
}

fn pending_owner_peers(
    pending: &HashMap<OutboundRequestId, PendingOwnerSearch>,
) -> HashSet<PeerId> {
    pending.values().map(|owner| owner.peer).collect()
}

fn collect_search_event(
    observed: ObservedReaderEvent,
    expected: ExpectedSearchResponse,
    pending: &mut HashMap<OutboundRequestId, PendingOwnerSearch>,
    sources: &mut Vec<Vec<SearchHit>>,
    partial: &mut bool,
    route_tracker: &OwnerRouteTracker,
    blocked_publishers: &HashSet<String>,
) -> Option<AcceptedOwnerRoute> {
    match observed.event {
        SwarmEvent::Behaviour(ReaderBehaviourEvent::Search(request_response::Event::Message {
            peer,
            connection_id,
            message,
        })) => {
            if let request_response::Message::Response {
                request_id,
                response,
            } = message
                && let Some(pending_owner) = pending.remove(&request_id)
            {
                let manifests = pending_owner.manifests;
                match response {
                    PublicSearchWireResponse::Success(mut response)
                        if response.protocol_version == PUBLIC_SEARCH_PROTOCOL
                            && response.response.request_id == expected.request_id
                            && peer == pending_owner.peer
                            && revisions_are_current(&response.manifest_sequences, &manifests)
                            && public_search_hits_are_valid(
                                &response.response.hits,
                                &manifests,
                                expected.top_k,
                            ) =>
                    {
                        let Some(publisher_id) = manifests
                            .first()
                            .map(|manifest| manifest.manifest.publisher_id.clone())
                        else {
                            *partial = true;
                            return None;
                        };
                        if publisher_id != peer.to_string() {
                            *partial = true;
                            return None;
                        }
                        if blocked_publishers.contains(&publisher_id) {
                            return None;
                        }
                        let route_kind = match route_tracker.route_for_response(
                            peer,
                            connection_id,
                            observed.at,
                        ) {
                            Ok(route_kind) => route_kind,
                            Err(error_kind) => {
                                tracing::warn!(
                                    error_kind,
                                    "public owner response could not be tied to its connection"
                                );
                                *partial = true;
                                return None;
                            }
                        };
                        for hit in &mut response.response.hits {
                            hit.node_id.clone_from(&publisher_id);
                        }
                        sources.push(response.response.hits);
                        return Some(AcceptedOwnerRoute {
                            publisher_id,
                            route_kind,
                        });
                    }
                    _ => *partial = true,
                }
            }
        }
        SwarmEvent::Behaviour(ReaderBehaviourEvent::Search(
            request_response::Event::OutboundFailure {
                request_id, error, ..
            },
        )) if pending.remove(&request_id).is_some() => {
            log_public_owner_outbound_failure(&error);
            *partial = true;
        }
        _ => {}
    }
    None
}

fn log_public_owner_outbound_failure(error: &request_response::OutboundFailure) {
    let error_kind = match error {
        request_response::OutboundFailure::DialFailure => "public_owner_dial_failed",
        request_response::OutboundFailure::Timeout => "public_owner_response_timeout",
        request_response::OutboundFailure::ConnectionClosed => "public_owner_connection_closed",
        request_response::OutboundFailure::UnsupportedProtocols => {
            "public_owner_protocol_unsupported"
        }
        request_response::OutboundFailure::Io(_) => "public_owner_io_failed",
    };
    tracing::warn!(error_kind, "public owner request failed");
}

fn merge_route_kind(current: PublicRouteKind, observed: PublicRouteKind) -> PublicRouteKind {
    match (current, observed) {
        (PublicRouteKind::Relay, _) | (_, PublicRouteKind::Relay) => PublicRouteKind::Relay,
        (PublicRouteKind::Direct, _) | (_, PublicRouteKind::Direct) => PublicRouteKind::Direct,
        _ => PublicRouteKind::Offline,
    }
}

fn public_search_hits_are_valid(
    hits: &[SearchHit],
    manifests: &[SignedPublicCollectionManifest],
    top_k: u8,
) -> bool {
    if hits.len() > usize::from(top_k) {
        return false;
    }
    let collections = manifests
        .iter()
        .map(|manifest| manifest.manifest.collection_id)
        .collect::<HashSet<_>>();
    let mut identities = HashSet::with_capacity(hits.len());
    hits.iter().enumerate().all(|(position, hit)| {
        hit.rank == u32::try_from(position + 1).unwrap_or(u32::MAX)
            && collections.contains(&hit.collection_id)
            && identities.insert((hit.source_sha256.clone(), hit.chunk_id))
    })
}

fn revisions_are_current(
    revisions: &[airwiki_types::PublicCollectionRevision],
    manifests: &[SignedPublicCollectionManifest],
) -> bool {
    manifests.iter().all(|manifest| {
        revisions.iter().any(|revision| {
            revision.collection_id == manifest.manifest.collection_id
                && revision.manifest_sequence >= manifest.manifest.sequence
        })
    })
}

fn select_candidates(
    manifests: Vec<SignedPublicCollectionManifest>,
) -> Vec<SignedPublicCollectionManifest> {
    let mut by_collection = HashMap::new();
    for manifest in manifests {
        let key = (
            manifest.manifest.publisher_id.clone(),
            manifest.manifest.collection_id,
        );
        let replace =
            by_collection
                .get(&key)
                .is_none_or(|known: &SignedPublicCollectionManifest| {
                    manifest.manifest.sequence > known.manifest.sequence
                });
        if replace {
            by_collection.insert(key, manifest);
        }
    }
    let mut candidates = by_collection.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .manifest
            .updated_at
            .cmp(&left.manifest.updated_at)
            .then_with(|| left.manifest.publisher_id.cmp(&right.manifest.publisher_id))
            .then_with(|| {
                left.manifest
                    .collection_id
                    .cmp(&right.manifest.collection_id)
            })
    });
    candidates.truncate(usize::from(airwiki_types::MAX_PUBLIC_CANDIDATES));
    candidates
}

fn group_candidates_by_peer(
    candidates: Vec<SignedPublicCollectionManifest>,
) -> Vec<(PeerId, Vec<SignedPublicCollectionManifest>)> {
    let mut groups = Vec::<(PeerId, Vec<SignedPublicCollectionManifest>)>::new();
    for candidate in candidates {
        let Ok(peer) = PeerId::from_str(&candidate.manifest.publisher_id) else {
            continue;
        };
        if let Some((_, collections)) = groups.iter_mut().find(|(known, _)| *known == peer) {
            if collections.len() < 2 {
                collections.push(candidate);
            }
        } else if groups.len() < MAX_PUBLIC_PEERS {
            groups.push((peer, vec![candidate]));
        }
    }
    groups
}

fn retain_unblocked_sources(
    sources: &mut Vec<Vec<SearchHit>>,
    blocked_publishers: &HashSet<String>,
) {
    sources.retain(|hits| {
        hits.first()
            .is_none_or(|hit| !blocked_publishers.contains(&hit.node_id))
    });
}

fn fuse_rankings(sources: Vec<Vec<SearchHit>>) -> Vec<SearchHit> {
    let mut fused = HashMap::<(String, uuid::Uuid), (SearchHit, f64)>::new();
    for hits in sources {
        for (position, hit) in hits.into_iter().enumerate() {
            let rank = if hit.rank == 0 {
                u32::try_from(position + 1).unwrap_or(u32::MAX)
            } else {
                hit.rank
            };
            let score = 1.0 / (RRF_K + f64::from(rank));
            let key = (hit.source_sha256.clone(), hit.chunk_id);
            fused
                .entry(key)
                .and_modify(|(_, total)| *total += score)
                .or_insert((hit, score));
        }
    }
    let mut values = fused.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.title.cmp(&right.0.title))
    });
    values.into_iter().map(|(hit, _)| hit).collect()
}

fn emit_partial(
    partials: &mpsc::Sender<SearchResponse>,
    request_id: uuid::Uuid,
    top_k: u8,
    sources: &[Vec<SearchHit>],
) {
    let mut hits = fuse_rankings(sources.to_vec());
    hits.truncate(usize::from(top_k));
    for (position, hit) in hits.iter_mut().enumerate() {
        hit.rank = u32::try_from(position + 1).unwrap_or(u32::MAX);
    }
    let _ = partials.try_send(SearchResponse {
        request_id,
        hits,
        authorized_candidates: Vec::new(),
        offline_nodes: Vec::new(),
        warnings: vec!["public search is still in progress".to_owned()],
        partial: true,
    });
}

fn public_index_deadline(started: Instant) -> Instant {
    started + INDEX_DEADLINE
}

fn public_owner_deadlines(started: Instant) -> OwnerDeadlines {
    let connect = started + OWNER_CONNECT_BUDGET;
    OwnerDeadlines {
        connect,
        finish: connect + OWNER_RESPONSE_BUDGET,
    }
}

fn pending_cannot_change_top_k(
    sources: &[Vec<SearchHit>],
    pending_sources: usize,
    top_k: usize,
) -> bool {
    if top_k == 0 {
        return true;
    }
    let mut scores = HashMap::<(String, uuid::Uuid), f64>::new();
    for source in sources {
        for hit in source {
            let rank = hit.rank.max(1);
            *scores
                .entry((hit.source_sha256.clone(), hit.chunk_id))
                .or_default() += 1.0 / (RRF_K + f64::from(rank));
        }
    }
    if scores.len() < top_k {
        return false;
    }
    let mut scores = scores.into_values().collect::<Vec<_>>();
    scores.sort_by(|left, right| right.total_cmp(left));
    let kth_score = scores[top_k - 1];
    let pending_upper_bound = pending_sources as f64 / (RRF_K + 1.0);
    let strongest_challenger = scores.get(top_k).copied().unwrap_or_default() + pending_upper_bound;
    kth_score > strongest_challenger.max(pending_upper_bound)
}

#[cfg(test)]
mod tests {
    use airwiki_types::PublicCollectionManifest;
    use chrono::{Duration as ChronoDuration, Utc};

    use super::*;

    fn hit(chunk_id: uuid::Uuid, rank: u32) -> SearchHit {
        SearchHit {
            concept_id: uuid::Uuid::new_v4(),
            collection_id: uuid::Uuid::new_v4(),
            chunk_id,
            title: "Synthetic result".to_owned(),
            snippet: "Bounded synthetic snippet".to_owned(),
            heading_or_page: "Test".to_owned(),
            logical_resource_uri: "urn:airwiki:test".to_owned(),
            source_revision: 1,
            source_sha256: "a".repeat(64),
            updated_at: Utc::now(),
            rank,
            node_id: "synthetic".to_owned(),
        }
    }

    fn manifest(publisher_id: String, collection_id: uuid::Uuid) -> SignedPublicCollectionManifest {
        let now = Utc::now();
        SignedPublicCollectionManifest {
            manifest: PublicCollectionManifest {
                protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
                publisher_id,
                collection_id,
                sequence: 1,
                publication_fingerprint: "a".repeat(64),
                name: "Synthetic collection".to_owned(),
                description: String::new(),
                languages: vec!["en".to_owned()],
                concept_count: 1,
                routing_terms: vec!["synthetic".to_owned()],
                routes: vec!["/ip4/127.0.0.1/tcp/1".to_owned()],
                updated_at: now,
                expires_at: now + ChronoDuration::minutes(15),
            },
            public_key: Vec::new(),
            signature: Vec::new(),
        }
    }

    #[test]
    fn conservative_pruning_waits_until_pending_peer_cannot_change_top_one() {
        let chunk = uuid::Uuid::new_v4();
        let one_source = vec![vec![hit(chunk, 1)]];
        assert!(!pending_cannot_change_top_k(&one_source, 1, 1));

        let two_sources = vec![vec![hit(chunk, 1)], vec![hit(chunk, 1)]];
        assert!(pending_cannot_change_top_k(&two_sources, 1, 1));
    }

    #[test]
    fn conservative_pruning_preserves_ties_and_incomplete_top_k() {
        let first = uuid::Uuid::new_v4();
        let second = uuid::Uuid::new_v4();
        let incomplete = vec![vec![hit(first, 1)]];
        assert!(!pending_cannot_change_top_k(&incomplete, 1, 2));

        let tied = vec![vec![hit(first, 1)], vec![hit(second, 1)]];
        assert!(!pending_cannot_change_top_k(&tied, 1, 2));
    }

    #[test]
    fn conservative_pruning_accounts_for_accumulated_challenger_scores() {
        let leader = uuid::Uuid::new_v4();
        let challenger = uuid::Uuid::new_v4();
        let sources = vec![
            vec![hit(leader, 1), hit(challenger, 2)],
            vec![hit(leader, 1), hit(challenger, 2)],
        ];

        assert!(!pending_cannot_change_top_k(&sources, 1, 1));
    }

    #[test]
    fn public_search_hits_reject_duplicates_excess_and_foreign_collections() {
        let collection_id = uuid::Uuid::new_v4();
        let manifests = vec![manifest("publisher".to_owned(), collection_id)];
        let mut valid = hit(uuid::Uuid::new_v4(), 1);
        valid.collection_id = collection_id;
        assert!(public_search_hits_are_valid(
            std::slice::from_ref(&valid),
            &manifests,
            1
        ));
        assert!(!public_search_hits_are_valid(
            &[valid.clone(), valid.clone()],
            &manifests,
            2
        ));

        let mut foreign = hit(uuid::Uuid::new_v4(), 1);
        foreign.collection_id = uuid::Uuid::new_v4();
        assert!(!public_search_hits_are_valid(&[foreign], &manifests, 1));

        let mut excess = hit(uuid::Uuid::new_v4(), 2);
        excess.collection_id = collection_id;
        assert!(!public_search_hits_are_valid(
            &[valid.clone(), excess],
            &manifests,
            1
        ));

        let mut zero_rank = valid;
        zero_rank.rank = 0;
        assert!(!public_search_hits_are_valid(&[zero_rank], &manifests, 1));
    }

    #[test]
    fn catalog_query_classifies_complete_partial_and_offline_states() {
        assert!(
            !catalog_query_is_partial(CatalogQueryState {
                successful: 3,
                ..CatalogQueryState::default()
            })
            .unwrap()
        );
        assert!(
            catalog_query_is_partial(CatalogQueryState {
                successful: 2,
                failed: 1,
                ..CatalogQueryState::default()
            })
            .unwrap()
        );
        assert!(catalog_query_is_partial(CatalogQueryState::default()).is_err());
    }

    #[test]
    fn index_fan_out_is_bounded_to_three() {
        let indexes = (0..5)
            .map(|ordinal| PublicIndexEndpoint {
                peer_id: PeerId::random(),
                address: format!("/ip4/127.0.0.1/tcp/{}", 42_000 + ordinal)
                    .parse()
                    .unwrap(),
            })
            .collect::<Vec<_>>();

        assert_eq!(bounded_indexes(&indexes).count(), MAX_INDEXES);
    }

    #[test]
    fn candidate_selection_is_bounded_and_keeps_publishers_distinct() {
        let now = Utc::now();
        let manifests = (0..70)
            .map(|ordinal| SignedPublicCollectionManifest {
                manifest: PublicCollectionManifest {
                    protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
                    publisher_id: format!("publisher-{ordinal}"),
                    collection_id: uuid::Uuid::from_u128(1),
                    sequence: 1,
                    publication_fingerprint: "a".repeat(64),
                    name: format!("Collection {ordinal}"),
                    description: String::new(),
                    languages: vec!["en".to_owned()],
                    concept_count: 1,
                    routing_terms: vec!["synthetic".to_owned()],
                    routes: vec!["/ip4/127.0.0.1/tcp/1".to_owned()],
                    updated_at: now,
                    expires_at: now + ChronoDuration::minutes(15),
                },
                public_key: Vec::new(),
                signature: Vec::new(),
            })
            .collect();

        assert_eq!(
            select_candidates(manifests).len(),
            usize::from(airwiki_types::MAX_PUBLIC_CANDIDATES)
        );
    }

    #[test]
    fn peer_fan_out_is_bounded_to_twelve_with_two_collections_each() {
        let candidates = (0..14)
            .flat_map(|publisher_ordinal| {
                let publisher = Keypair::generate_ed25519()
                    .public()
                    .to_peer_id()
                    .to_string();
                (0..3).map(move |collection_ordinal| {
                    manifest(
                        publisher.clone(),
                        uuid::Uuid::from_u128(publisher_ordinal * 10 + collection_ordinal + 1),
                    )
                })
            })
            .collect();

        let groups = group_candidates_by_peer(candidates);

        assert_eq!(groups.len(), MAX_PUBLIC_PEERS);
        assert!(groups.iter().all(|(_, collections)| collections.len() == 2));
    }

    #[tokio::test]
    async fn blocked_publisher_is_rejected_before_browse_dials() {
        let identity = Keypair::generate_ed25519();
        let publisher_id = identity.public().to_peer_id().to_string();
        let manifest = manifest(publisher_id.clone(), uuid::Uuid::new_v4());
        let reader = PublicReader::new();
        reader.set_publisher_blocked(publisher_id, true).await;

        assert!(matches!(
            reader.browse(&manifest, None, 1).await,
            Err(SearchContractError::Unauthorized)
        ));
    }

    #[test]
    fn owner_route_tracker_ignores_connections_to_other_peers() {
        let owner = PeerId::random();
        let owners = HashSet::from([owner]);
        let started = Instant::now();
        let mut tracker = OwnerRouteTracker::new(owners.clone(), started + OWNER_CONNECT_BUDGET);

        tracker.record_connection(
            PeerId::random(),
            ConnectionId::new_unchecked(1),
            PublicRouteKind::Direct,
            started,
        );

        assert_eq!(tracker.connected_owner_count(&owners), 0);
    }

    #[test]
    fn owner_route_tracker_enforces_connection_and_response_budgets() {
        let owner = PeerId::random();
        let connection = ConnectionId::new_unchecked(1);
        let started = Instant::now();
        let mut tracker =
            OwnerRouteTracker::new(HashSet::from([owner]), started + OWNER_CONNECT_BUDGET);
        tracker.record_connection(owner, connection, PublicRouteKind::Relay, started);

        assert_eq!(
            tracker.route_for_response(
                owner,
                connection,
                started + OWNER_RESPONSE_BUDGET - Duration::from_millis(1),
            ),
            Ok(PublicRouteKind::Relay)
        );
        assert_eq!(
            tracker.route_for_response(owner, connection, started + OWNER_RESPONSE_BUDGET),
            Ok(PublicRouteKind::Relay)
        );
        assert_eq!(
            tracker.route_for_response(
                owner,
                connection,
                started + OWNER_RESPONSE_BUDGET + Duration::from_millis(1),
            ),
            Err("public_owner_response_timeout")
        );
        assert_eq!(
            tracker.route_for_response(owner, ConnectionId::new_unchecked(2), started,),
            Err("public_owner_route_unavailable")
        );
        assert_eq!(
            tracker.route_for_response(PeerId::random(), connection, started),
            Err("public_owner_route_unavailable")
        );
    }

    #[test]
    fn owner_route_tracker_rejects_late_connections_and_removes_closed_ones() {
        let owner = PeerId::random();
        let owners = HashSet::from([owner]);
        let started = Instant::now();
        let deadline = started + OWNER_CONNECT_BUDGET;
        let mut tracker = OwnerRouteTracker::new(owners.clone(), deadline);
        let active = ConnectionId::new_unchecked(1);
        tracker.record_connection(owner, active, PublicRouteKind::Relay, started);
        assert_eq!(tracker.connected_owner_count(&owners), 1);

        tracker.remove_connection(active);
        assert_eq!(tracker.connected_owner_count(&owners), 0);
        assert_eq!(
            tracker.timeout_error_kind(&owners),
            "public_owner_connect_timeout"
        );

        let late = ConnectionId::new_unchecked(2);
        tracker.record_connection(
            owner,
            late,
            PublicRouteKind::Direct,
            deadline + Duration::from_millis(1),
        );
        assert_eq!(
            tracker.route_for_response(owner, late, deadline),
            Err("public_owner_route_unavailable")
        );
    }

    #[test]
    fn timeout_classification_considers_only_pending_owners() {
        let answered = PeerId::random();
        let pending = PeerId::random();
        let all_owners = HashSet::from([answered, pending]);
        let started = Instant::now();
        let mut tracker =
            OwnerRouteTracker::new(all_owners.clone(), started + OWNER_CONNECT_BUDGET);
        tracker.record_connection(
            answered,
            ConnectionId::new_unchecked(1),
            PublicRouteKind::Relay,
            started,
        );

        assert_eq!(
            tracker.timeout_error_kind(&all_owners),
            "public_owner_mixed_timeout"
        );
        assert_eq!(
            tracker.timeout_error_kind(&HashSet::from([pending])),
            "public_owner_connect_timeout"
        );
        assert_eq!(tracker.connected_owner_count(&HashSet::from([pending])), 0);
    }

    #[test]
    fn route_merge_preserves_successful_relay_evidence() {
        assert_eq!(
            merge_route_kind(PublicRouteKind::Relay, PublicRouteKind::Direct),
            PublicRouteKind::Relay
        );
    }

    #[tokio::test]
    async fn partial_delivery_is_deterministic_under_backpressure() {
        let request_id = uuid::Uuid::new_v4();
        let first_chunk = uuid::Uuid::new_v4();
        let second_chunk = uuid::Uuid::new_v4();
        let (sender, mut receiver) = mpsc::channel(1);

        emit_partial(&sender, request_id, 1, &[vec![hit(first_chunk, 1)]]);
        emit_partial(&sender, request_id, 1, &[vec![hit(second_chunk, 1)]]);

        let partial = receiver.recv().await.unwrap();
        assert_eq!(partial.request_id, request_id);
        assert!(partial.partial);
        assert_eq!(partial.hits.len(), 1);
        assert_eq!(partial.hits[0].chunk_id, first_chunk);
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn public_deadlines_compose_catalog_owner_connect_and_owner_response_budgets() {
        let started = Instant::now();
        assert_eq!(
            public_index_deadline(started).duration_since(started),
            INDEX_DEADLINE
        );
        tokio::time::timeout_at(
            public_index_deadline(started),
            tokio::time::sleep(Duration::from_millis(900)),
        )
        .await
        .expect("slow cross-region catalog stays within its bounded stage");

        let owner_started = Instant::now();
        let owner_deadlines = public_owner_deadlines(owner_started);
        assert_eq!(
            owner_deadlines.connect.duration_since(owner_started),
            OWNER_CONNECT_BUDGET
        );
        assert_eq!(
            owner_deadlines
                .finish
                .duration_since(owner_deadlines.connect),
            OWNER_RESPONSE_BUDGET
        );
        tokio::time::timeout_at(
            owner_deadlines.finish,
            tokio::time::sleep(OWNER_CONNECT_BUDGET + Duration::from_millis(700)),
        )
        .await
        .expect("owner receives separate connection and response budgets after catalog");
        assert_eq!(
            Instant::now().duration_since(started),
            Duration::from_millis(4_600)
        );
    }
}
