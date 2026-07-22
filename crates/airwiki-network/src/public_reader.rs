use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use airwiki_types::{
    PUBLIC_BROWSE_PROTOCOL, PUBLIC_CATALOG_PROTOCOL, PUBLIC_SEARCH_PROTOCOL, PublicBrowsePage,
    PublicBrowseRequest, PublicCatalogQuery, PublicCollectionTarget, PublicSearchRequest,
    SearchContractError, SearchHit, SearchRequest, SearchResponse, SignedPublicCollectionManifest,
    SignedPublicCollectionTombstone,
};
use libp2p::identity::Keypair;
use libp2p::request_response::{self, OutboundRequestId, ProtocolSupport};
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder};
use tokio::sync::{Semaphore, mpsc};
use tokio::time::{Instant, timeout_at};

use crate::{
    CatalogWireRequest, CatalogWireResponse, NetworkError, PublicBrowseWireResponse,
    PublicSearchWireResponse, verify_manifest,
};

const INDEX_DEADLINE: Duration = Duration::from_millis(300);
const PEER_DEADLINE: Duration = Duration::from_millis(800);
const GLOBAL_DEADLINE: Duration = Duration::from_millis(1_500);
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
    route_kind: AtomicU8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicRouteKind {
    Offline,
    Relay,
    Direct,
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
            route_kind: AtomicU8::new(0),
        }
    }

    pub fn route_kind(&self) -> PublicRouteKind {
        match self.route_kind.load(Ordering::Acquire) {
            1 => PublicRouteKind::Relay,
            2 => PublicRouteKind::Direct,
            _ => PublicRouteKind::Offline,
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
        self.search_inner(indexes, request, None).await
    }

    pub async fn search_with_partials(
        &self,
        indexes: &[PublicIndexEndpoint],
        request: SearchRequest,
        partials: mpsc::Sender<SearchResponse>,
    ) -> Result<SearchResponse, SearchContractError> {
        self.search_inner(indexes, request, Some(&partials)).await
    }

    async fn search_inner(
        &self,
        indexes: &[PublicIndexEndpoint],
        request: SearchRequest,
        partials: Option<&mpsc::Sender<SearchResponse>>,
    ) -> Result<SearchResponse, SearchContractError> {
        request.validate()?;
        self.route_kind.store(0, Ordering::Release);
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
        for endpoint in indexes.iter().take(MAX_INDEXES) {
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
            collect_catalog_event(event, &mut pending_catalog, &mut manifests);
        }
        let candidates = {
            let blocked = self.blocked_publishers.read().await;
            select_candidates(manifests)
                .into_iter()
                .filter(|candidate| !blocked.contains(&candidate.manifest.publisher_id))
                .collect::<Vec<_>>()
        };
        if candidates.is_empty() {
            return Ok(SearchResponse::empty(request.request_id));
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
        let mut pending_search =
            HashMap::<OutboundRequestId, Vec<SignedPublicCollectionManifest>>::new();
        for (peer, collections) in groups {
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
            pending_search.insert(request_id, collections);
        }
        let mut sources = Vec::new();
        let mut partial = !pending_catalog.is_empty();
        while !pending_search.is_empty() {
            let peer_deadline = public_peer_deadline(started, Instant::now());
            let event = match timeout_at(
                peer_deadline,
                futures::StreamExt::select_next_some(&mut swarm),
            )
            .await
            {
                Ok(event) => event,
                Err(_) => {
                    partial = true;
                    break;
                }
            };
            let previous_source_count = sources.len();
            self.record_route(&event);
            collect_search_event(
                event,
                request.request_id,
                &mut pending_search,
                &mut sources,
                &mut partial,
            );
            if sources.len() > previous_source_count
                && let Some(partials) = partials
            {
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
        let mut hits = fuse_rankings(sources);
        hits.truncate(usize::from(request.top_k));
        for (position, hit) in hits.iter_mut().enumerate() {
            hit.rank = u32::try_from(position + 1).unwrap_or(u32::MAX);
        }
        Ok(SearchResponse {
            request_id: request.request_id,
            hits,
            authorized_candidates: Vec::new(),
            offline_nodes: Vec::new(),
            warnings: if partial {
                vec!["public search returned partial results".to_owned()]
            } else {
                Vec::new()
            },
            partial,
        })
    }

    pub async fn browse(
        &self,
        manifest: &SignedPublicCollectionManifest,
        cursor: Option<String>,
        limit: u8,
    ) -> Result<PublicBrowsePage, SearchContractError> {
        self.route_kind.store(0, Ordering::Release);
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
        let outbound = swarm.behaviour_mut().browse.send_request(&peer, request);
        let deadline = Instant::now() + GLOBAL_DEADLINE;
        loop {
            let event = timeout_at(deadline, futures::StreamExt::select_next_some(&mut swarm))
                .await
                .map_err(|_| {
                    SearchContractError::Unavailable("public browse timed out".to_owned())
                })?;
            self.record_route(&event);
            if let SwarmEvent::Behaviour(ReaderBehaviourEvent::Browse(
                request_response::Event::Message { message, .. },
            )) = event
                && let request_response::Message::Response {
                    request_id,
                    response,
                } = message
                && request_id == outbound
            {
                return match response {
                    PublicBrowseWireResponse::Success(page)
                        if page.manifest_sequence >= manifest.manifest.sequence =>
                    {
                        Ok(page)
                    }
                    PublicBrowseWireResponse::Success(_) => Err(SearchContractError::Unauthorized),
                    PublicBrowseWireResponse::Rejected(_) => Err(SearchContractError::Unavailable(
                        "public browse was rejected".to_owned(),
                    )),
                };
            }
        }
    }

    pub async fn browse_collection(
        &self,
        publisher_id: &str,
        collection_id: uuid::Uuid,
        cursor: Option<String>,
        limit: u8,
    ) -> Result<PublicBrowsePage, SearchContractError> {
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
        self.browse(&manifest, cursor, limit).await
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
        for endpoint in indexes.iter().take(MAX_INDEXES) {
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
        let deadline = Instant::now() + PEER_DEADLINE;
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

    fn record_route(&self, event: &SwarmEvent<ReaderBehaviourEvent>) {
        if let SwarmEvent::ConnectionEstablished { endpoint, .. } = event {
            self.route_kind
                .store(if endpoint.is_relayed() { 1 } else { 2 }, Ordering::Release);
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
    let catalog = outbound_behaviour(PUBLIC_CATALOG_PROTOCOL, 128 * 1024, 512 * 1024)?;
    let search = outbound_behaviour(PUBLIC_SEARCH_PROTOCOL, 16 * 1024, 256 * 1024)?;
    let browse = outbound_behaviour(PUBLIC_BROWSE_PROTOCOL, 16 * 1024, 256 * 1024)?;
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
            .with_request_timeout(PEER_DEADLINE)
            .with_max_concurrent_streams(32),
    ))
}

fn collect_catalog_event(
    event: SwarmEvent<ReaderBehaviourEvent>,
    pending: &mut HashSet<OutboundRequestId>,
    manifests: &mut Vec<SignedPublicCollectionManifest>,
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
            pending.remove(&request_id);
            if let CatalogWireResponse::Results(results) = response {
                manifests.extend(
                    results
                        .into_iter()
                        .filter(|manifest| verify_manifest(manifest, chrono::Utc::now()).is_ok()),
                );
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

fn collect_search_event(
    event: SwarmEvent<ReaderBehaviourEvent>,
    expected_request_id: uuid::Uuid,
    pending: &mut HashMap<OutboundRequestId, Vec<SignedPublicCollectionManifest>>,
    sources: &mut Vec<Vec<SearchHit>>,
    partial: &mut bool,
) {
    match event {
        SwarmEvent::Behaviour(ReaderBehaviourEvent::Search(request_response::Event::Message {
            message,
            ..
        })) => {
            if let request_response::Message::Response {
                request_id,
                response,
            } = message
                && let Some(manifests) = pending.remove(&request_id)
            {
                match response {
                    PublicSearchWireResponse::Success(response)
                        if response.protocol_version == PUBLIC_SEARCH_PROTOCOL
                            && response.response.request_id == expected_request_id
                            && revisions_are_current(&response.manifest_sequences, &manifests) =>
                    {
                        sources.push(response.response.hits);
                    }
                    _ => *partial = true,
                }
            }
        }
        SwarmEvent::Behaviour(ReaderBehaviourEvent::Search(
            request_response::Event::OutboundFailure { request_id, .. },
        )) if pending.remove(&request_id).is_some() => {
            *partial = true;
        }
        _ => {}
    }
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

fn public_global_deadline(started: Instant) -> Instant {
    started + GLOBAL_DEADLINE
}

fn public_peer_deadline(started: Instant, now: Instant) -> Instant {
    (now + PEER_DEADLINE).min(public_global_deadline(started))
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
    scores.sort_by(f64::total_cmp);
    let kth_score = scores[scores.len() - top_k];
    let pending_upper_bound = pending_sources as f64 / (RRF_K + 1.0);
    kth_score > pending_upper_bound
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
        assert_eq!(reader.route_kind(), PublicRouteKind::Offline);
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
    async fn public_deadlines_are_bounded_at_300_800_and_1500_milliseconds() {
        let started = Instant::now();
        assert_eq!(
            public_index_deadline(started).duration_since(started),
            INDEX_DEADLINE
        );
        assert_eq!(
            public_peer_deadline(started, started).duration_since(started),
            PEER_DEADLINE
        );

        tokio::time::advance(Duration::from_millis(1_000)).await;

        assert_eq!(
            public_peer_deadline(started, Instant::now()),
            public_global_deadline(started)
        );
        assert_eq!(
            public_global_deadline(started).duration_since(started),
            GLOBAL_DEADLINE
        );
    }
}
