//! Local + LAN result coordination for desktop search and MCP.

use std::collections::HashMap;
use std::sync::Arc;

use airwiki_types::{
    FederatedSearch, SearchContractError, SearchHit, SearchRequest, SearchResponse,
};
use async_trait::async_trait;

use crate::NetworkHandle;

const RRF_K: f64 = 60.0;

/// Searches the local node and all trusted LAN peers concurrently, then applies a
/// second reciprocal-rank fusion because scores from different nodes are not comparable.
#[derive(Clone)]
pub struct FederatedCoordinator {
    local: Arc<dyn FederatedSearch>,
    peers: Arc<dyn FederatedSearch>,
}

impl std::fmt::Debug for FederatedCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FederatedCoordinator")
            .finish_non_exhaustive()
    }
}

impl FederatedCoordinator {
    pub fn new(local: Arc<dyn FederatedSearch>, network: NetworkHandle) -> Self {
        Self {
            local,
            peers: Arc::new(network),
        }
    }

    #[cfg(test)]
    fn with_backends(local: Arc<dyn FederatedSearch>, peers: Arc<dyn FederatedSearch>) -> Self {
        Self { local, peers }
    }
}

#[async_trait]
impl FederatedSearch for FederatedCoordinator {
    async fn search(&self, request: SearchRequest) -> Result<SearchResponse, SearchContractError> {
        request.validate()?;
        let (local, peers) = tokio::join!(
            self.local.search(request.clone()),
            self.peers.search(request.clone())
        );

        if local.is_err() && peers.is_err() {
            return Err(SearchContractError::Unavailable(
                "local and LAN search unavailable".to_owned(),
            ));
        }

        Ok(fuse_local_and_peers(request, local, peers))
    }
}

fn fuse_local_and_peers(
    request: SearchRequest,
    local: Result<SearchResponse, SearchContractError>,
    peers: Result<SearchResponse, SearchContractError>,
) -> SearchResponse {
    let mut sources = Vec::with_capacity(2);
    let mut warnings = Vec::new();
    let mut offline_nodes = Vec::new();
    let mut partial = false;

    collect_source(
        "local",
        request.request_id,
        local,
        &mut sources,
        &mut warnings,
        &mut offline_nodes,
        &mut partial,
    );
    collect_source(
        "LAN",
        request.request_id,
        peers,
        &mut sources,
        &mut warnings,
        &mut offline_nodes,
        &mut partial,
    );

    let mut fused: HashMap<(String, uuid::Uuid), (SearchHit, f64)> = HashMap::new();
    for hits in sources {
        for (position, mut hit) in hits.into_iter().enumerate() {
            hit.sanitize_for_wire();
            let rank = if hit.rank == 0 {
                (position + 1) as u32
            } else {
                hit.rank
            };
            let contribution = 1.0 / (RRF_K + f64::from(rank));
            let key = (hit.source_sha256.clone(), hit.chunk_id);
            fused
                .entry(key)
                .and_modify(|(_, score)| *score += contribution)
                .or_insert((hit, contribution));
        }
    }

    let mut ranked: Vec<_> = fused.into_values().collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.title.cmp(&right.0.title))
    });
    let mut hits: Vec<_> = ranked
        .into_iter()
        .take(usize::from(request.top_k))
        .map(|(hit, _)| hit)
        .collect();
    for (position, hit) in hits.iter_mut().enumerate() {
        hit.rank = (position + 1) as u32;
    }

    offline_nodes.sort();
    offline_nodes.dedup();
    warnings.sort();
    warnings.dedup();
    partial |= !offline_nodes.is_empty() || !warnings.is_empty();
    SearchResponse {
        request_id: request.request_id,
        hits,
        offline_nodes,
        warnings,
        partial,
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_source(
    label: &str,
    expected_request_id: uuid::Uuid,
    source: Result<SearchResponse, SearchContractError>,
    hits: &mut Vec<Vec<SearchHit>>,
    warnings: &mut Vec<String>,
    offline_nodes: &mut Vec<String>,
    partial: &mut bool,
) {
    match source {
        Ok(mut response) if response.request_id == expected_request_id => {
            let source_reported_warning = !response.warnings.is_empty();
            *partial |= response.partial || source_reported_warning;
            offline_nodes.append(&mut response.offline_nodes);
            if source_reported_warning {
                warnings.push(format!("{label} search reported incomplete results"));
            }
            hits.push(response.hits);
        }
        Ok(_) => {
            *partial = true;
            warnings.push(format!("{label}: mismatched request id"));
        }
        Err(_) => {
            *partial = true;
            warnings.push(format!("{label} search unavailable"));
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use parking_lot::Mutex;

    use super::*;

    struct FakeSearch {
        result: Mutex<Option<Result<SearchResponse, SearchContractError>>>,
    }

    impl FakeSearch {
        fn returns(result: Result<SearchResponse, SearchContractError>) -> Arc<Self> {
            Arc::new(Self {
                result: Mutex::new(Some(result)),
            })
        }
    }

    #[async_trait]
    impl FederatedSearch for FakeSearch {
        async fn search(
            &self,
            _request: SearchRequest,
        ) -> Result<SearchResponse, SearchContractError> {
            self.result.lock().take().expect("fake called only once")
        }
    }

    fn hit(title: &str, rank: u32, hash: &str, chunk_id: uuid::Uuid) -> SearchHit {
        SearchHit {
            concept_id: uuid::Uuid::new_v4(),
            collection_id: uuid::Uuid::new_v4(),
            chunk_id,
            title: title.to_owned(),
            snippet: "evidence".to_owned(),
            heading_or_page: "section".to_owned(),
            logical_resource_uri: "urn:test".to_owned(),
            source_revision: 1,
            source_sha256: hash.to_owned(),
            updated_at: Utc::now(),
            rank,
            node_id: title.to_owned(),
        }
    }

    #[tokio::test]
    async fn local_results_survive_when_lan_fails() {
        let request = SearchRequest::new("pagos", airwiki_types::SearchPurpose::LocalAssistant, 5);
        let mut local = SearchResponse::empty(request.request_id);
        local
            .hits
            .push(hit("local", 1, "local-hash", uuid::Uuid::new_v4()));
        let coordinator = FederatedCoordinator::with_backends(
            FakeSearch::returns(Ok(local)),
            FakeSearch::returns(Err(SearchContractError::Unavailable(
                "CANARY-REASON at C:\\private\\peer.db".to_owned(),
            ))),
        );

        let response = coordinator.search(request).await.unwrap();
        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].title, "local");
        assert!(response.partial);
        assert_eq!(response.warnings, ["LAN search unavailable"]);
        assert!(!response.warnings.join(" ").contains("CANARY-REASON"));
        assert!(!response.warnings.join(" ").contains("C:\\private"));
    }

    #[tokio::test]
    async fn second_rrf_deduplicates_local_and_peer_evidence() {
        let request = SearchRequest::new("pagos", airwiki_types::SearchPurpose::LocalAssistant, 5);
        let duplicate = uuid::Uuid::new_v4();
        let mut local = SearchResponse::empty(request.request_id);
        local.hits.push(hit("local copy", 1, "same", duplicate));
        let mut remote = SearchResponse::empty(request.request_id);
        remote.hits.push(hit("remote copy", 2, "same", duplicate));
        remote
            .hits
            .push(hit("remote only", 1, "other", uuid::Uuid::new_v4()));
        let coordinator = FederatedCoordinator::with_backends(
            FakeSearch::returns(Ok(local)),
            FakeSearch::returns(Ok(remote)),
        );

        let response = coordinator.search(request).await.unwrap();
        assert_eq!(response.hits.len(), 2);
        assert_eq!(response.hits[0].source_sha256, "same");
        assert_eq!(response.hits[0].rank, 1);
    }

    #[tokio::test]
    async fn both_failures_are_explicit() {
        let request = SearchRequest::new("pagos", airwiki_types::SearchPurpose::LocalAssistant, 5);
        let coordinator = FederatedCoordinator::with_backends(
            FakeSearch::returns(Err(SearchContractError::Backend(
                "/Users/alice/private/CANARY-LOCAL".to_owned(),
            ))),
            FakeSearch::returns(Err(SearchContractError::Unavailable(
                "C:\\secrets\\CANARY-LAN".to_owned(),
            ))),
        );
        let error = coordinator.search(request).await.unwrap_err();
        let text = error.to_string();
        assert!(text.contains("local and LAN search unavailable"));
        assert!(!text.contains("CANARY"));
        assert!(!text.contains("/Users/alice"));
        assert!(!text.contains("C:\\secrets"));
    }

    #[tokio::test]
    async fn backend_warnings_are_replaced_with_stable_local_messages() {
        let request = SearchRequest::new("pagos", airwiki_types::SearchPurpose::ExternalAi, 5);
        let mut local = SearchResponse::empty(request.request_id);
        local
            .warnings
            .push("CANARY-LOCAL at /Users/alice/private/notes.md".to_owned());
        let mut peers = SearchResponse::empty(request.request_id);
        peers
            .warnings
            .push("CANARY-LAN at C:\\secrets\\payroll.txt".to_owned());
        let coordinator = FederatedCoordinator::with_backends(
            FakeSearch::returns(Ok(local)),
            FakeSearch::returns(Ok(peers)),
        );

        let response = coordinator.search(request).await.unwrap();

        assert!(response.partial);
        assert_eq!(
            response.warnings,
            [
                "LAN search reported incomplete results",
                "local search reported incomplete results",
            ]
        );
        let visible = response.warnings.join(" ");
        assert!(!visible.contains("CANARY"));
        assert!(!visible.contains("/Users/alice"));
        assert!(!visible.contains("C:\\secrets"));
    }
}
