//! Stable domain and wire contracts shared by the desktop, storage, LAN and MCP crates.

mod public;

pub use public::*;

use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const SEARCH_PROTOCOL: &str = "/airwiki/search/2.0.0";
pub const PUBLIC_CATALOG_PROTOCOL: &str = "/airwiki/public-catalog/1.0.0";
pub const PUBLIC_SEARCH_PROTOCOL: &str = "/airwiki/public-search/1.0.0";
pub const PUBLIC_BROWSE_PROTOCOL: &str = "/airwiki/public-browse/1.0.0";
pub const MAX_QUERY_BYTES: usize = 2 * 1024;
pub const MAX_SNIPPET_CHARS: usize = 1_200;
pub const MAX_HEADING_OR_PAGE_CHARS: usize = 300;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const MIN_TOP_K: u8 = 1;
pub const DEFAULT_TOP_K: u8 = 5;
pub const MAX_TOP_K: u8 = 10;

/// Publication state. Only `Published` may be returned outside its source node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    Detected,
    Extracted,
    Enriched,
    NeedsReview,
    /// Human-approved metadata whose durable OKF bundle is still being materialized.
    /// This state is deliberately not searchable or shareable.
    Publishing,
    Published,
    Deleted,
    Failed,
}

impl fmt::Display for DocumentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Detected => "detected",
            Self::Extracted => "extracted",
            Self::Enriched => "enriched",
            Self::NeedsReview => "needs_review",
            Self::Publishing => "publishing",
            Self::Published => "published",
            Self::Deleted => "deleted",
            Self::Failed => "failed",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConceptType {
    Document,
    Policy,
    Procedure,
    Runbook,
    Reference,
    Report,
}

impl fmt::Display for ConceptType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchPurpose {
    LocalAssistant,
    ExternalAi,
}

/// Independent collection egress controls.
///
/// `local_only` is retained in persisted and serialized data for compatibility,
/// but is derived from the three explicit opt-ins by [`Self::normalize`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionPolicy {
    #[serde(default = "default_true")]
    pub local_only: bool,
    #[serde(default)]
    pub peer_shareable: bool,
    #[serde(default)]
    pub allow_external_ai: bool,
    #[serde(default)]
    pub internet_public: bool,
}

impl Default for CollectionPolicy {
    fn default() -> Self {
        Self::local_only()
    }
}

const fn default_true() -> bool {
    true
}

impl CollectionPolicy {
    pub const fn local_only() -> Self {
        Self {
            local_only: true,
            peer_shareable: false,
            allow_external_ai: false,
            internet_public: false,
        }
    }

    pub const fn shared_with_peers() -> Self {
        Self {
            local_only: false,
            peer_shareable: true,
            allow_external_ai: false,
            internet_public: false,
        }
    }

    /// Whether neither peer sharing nor disclosure to an external AI is enabled.
    pub const fn is_local_only(self) -> bool {
        !self.peer_shareable && !self.allow_external_ai && !self.internet_public
    }

    /// Whether a search executed on this device may use the collection.
    pub const fn can_serve_locally(self, purpose: SearchPurpose) -> bool {
        match purpose {
            SearchPurpose::LocalAssistant => true,
            SearchPurpose::ExternalAi => self.allow_external_ai,
        }
    }

    /// Whether an authenticated peer may use the collection for this purpose.
    ///
    /// A grant is still required by the network authorization layer. External-AI
    /// searches require both independent egress opt-ins.
    pub const fn can_serve_peer(self, purpose: SearchPurpose) -> bool {
        self.peer_shareable && self.can_serve_locally(purpose)
    }

    /// Compatibility alias for remote-peer authorization.
    pub const fn can_serve(self, purpose: SearchPurpose) -> bool {
        self.can_serve_peer(purpose)
    }

    /// Whether this collection may be served to an unpaired Internet reader.
    ///
    /// Publication state is revalidated independently immediately before
    /// disclosure; this opt-in never publishes a draft.
    pub const fn can_serve_public(self) -> bool {
        self.internet_public
    }

    pub fn normalize(&mut self) {
        self.local_only = self.is_local_only();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedEntity {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedLink {
    pub label: String,
    pub target: String,
}

/// LLM output is a draft. It intentionally contains no policy or publication fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentDraft {
    #[serde(rename = "type")]
    pub concept_type: ConceptType,
    pub title: String,
    pub description: String,
    pub language: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub entities: Vec<SuggestedEntity>,
    #[serde(default)]
    pub links: Vec<SuggestedLink>,
    pub summary: String,
    pub classification_confidence: f32,
    pub classification_explanation: String,
}

impl EnrichmentDraft {
    pub fn sanitize(&mut self) {
        self.title = self.title.trim().chars().take(240).collect();
        self.description = self.description.trim().chars().take(1_000).collect();
        self.language = self.language.trim().chars().take(16).collect();
        self.summary = self.summary.trim().chars().take(4_000).collect();
        self.tags = self
            .tags
            .drain(..)
            .map(|tag| tag.trim().to_lowercase())
            .filter(|tag| !tag.is_empty())
            .take(10)
            .collect();
        self.tags.sort();
        self.tags.dedup();
        self.entities = self
            .entities
            .drain(..)
            .map(|entity| SuggestedEntity {
                name: entity.name.trim().chars().take(240).collect(),
                kind: entity.kind.trim().chars().take(120).collect(),
            })
            .filter(|entity| !entity.name.is_empty() && !entity.kind.is_empty())
            .take(50)
            .collect();
        self.links = self
            .links
            .drain(..)
            .map(|link| SuggestedLink {
                label: link.label.trim().chars().take(240).collect(),
                target: link.target.trim().chars().take(500).collect(),
            })
            .filter(|link| !link.label.is_empty() && !link.target.is_empty())
            .take(50)
            .collect();
        self.classification_confidence = if self.classification_confidence.is_finite() {
            self.classification_confidence.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.classification_explanation = self
            .classification_explanation
            .trim()
            .chars()
            .take(1_000)
            .collect();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub query: String,
    pub purpose: SearchPurpose,
    pub top_k: u8,
}

impl SearchRequest {
    pub fn new(query: impl Into<String>, purpose: SearchPurpose, top_k: u8) -> Self {
        Self {
            protocol_version: SEARCH_PROTOCOL.to_owned(),
            request_id: Uuid::new_v4(),
            query: query.into(),
            purpose,
            top_k,
        }
    }

    pub fn validate(&self) -> Result<(), SearchContractError> {
        if self.protocol_version != SEARCH_PROTOCOL {
            return Err(SearchContractError::UnsupportedProtocol(
                self.protocol_version.clone(),
            ));
        }
        let query = self.query.trim();
        if query.is_empty() {
            return Err(SearchContractError::EmptyQuery);
        }
        if query.len() > MAX_QUERY_BYTES {
            return Err(SearchContractError::QueryTooLarge(query.len()));
        }
        if !(MIN_TOP_K..=MAX_TOP_K).contains(&self.top_k) {
            return Err(SearchContractError::InvalidTopK(self.top_k));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub concept_id: Uuid,
    pub collection_id: Uuid,
    pub chunk_id: Uuid,
    pub title: String,
    pub snippet: String,
    pub heading_or_page: String,
    pub logical_resource_uri: String,
    pub source_revision: u32,
    pub source_sha256: String,
    pub updated_at: DateTime<Utc>,
    pub rank: u32,
    pub node_id: String,
}

impl SearchHit {
    pub fn sanitize_for_wire(&mut self) {
        self.title = self.title.chars().take(300).collect();
        self.snippet = self.snippet.chars().take(MAX_SNIPPET_CHARS).collect();
        self.heading_or_page = self
            .heading_or_page
            .chars()
            .take(MAX_HEADING_OR_PAGE_CHARS)
            .collect();
        self.logical_resource_uri = self.logical_resource_uri.chars().take(500).collect();
    }

    pub fn dedup_key(&self) -> (&str, Uuid) {
        (&self.source_sha256, self.chunk_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub request_id: Uuid,
    pub hits: Vec<SearchHit>,
    /// Authorized snippets rejected by AirWiki's lightweight answerability filter.
    ///
    /// These items passed the same publication and disclosure checks as `hits`, but
    /// callers must not treat them as verified evidence without evaluating whether
    /// they explicitly answer the question.
    #[serde(default)]
    pub authorized_candidates: Vec<SearchHit>,
    #[serde(default)]
    pub offline_nodes: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub partial: bool,
}

impl SearchResponse {
    pub fn empty(request_id: Uuid) -> Self {
        Self {
            request_id,
            hits: Vec::new(),
            authorized_candidates: Vec::new(),
            offline_nodes: Vec::new(),
            warnings: Vec::new(),
            partial: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchAuthorization {
    pub caller_node_id: String,
    pub allowed_collections: Vec<Uuid>,
    pub purpose: SearchPurpose,
    disclosure_gate: DisclosureGate,
}

impl SearchAuthorization {
    pub fn new(
        caller_node_id: String,
        allowed_collections: Vec<Uuid>,
        purpose: SearchPurpose,
        disclosure_gate: DisclosureGate,
    ) -> Self {
        Self {
            caller_node_id,
            allowed_collections,
            purpose,
            disclosure_gate,
        }
    }

    /// Prevent authorization or publication mutations while a validated
    /// response is handed to its transport.
    pub fn acquire_disclosure_lease(&self) -> DisclosureLease {
        self.disclosure_gate.acquire_disclosure()
    }
}

/// Process-local linearization barrier for evidence disclosure.
///
/// Durable authorization/publication mutations take an exclusive guard. A
/// search takes a lease only for its final revalidation and synchronous
/// transport handoff. Waiting mutations have priority over later searches.
#[derive(Debug, Clone, Default)]
pub struct DisclosureGate {
    inner: Arc<DisclosureGateInner>,
}

#[derive(Debug, Default)]
struct DisclosureGateInner {
    state: Mutex<DisclosureGateState>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct DisclosureGateState {
    active_disclosures: usize,
    mutation_active: bool,
    waiting_mutations: usize,
}

impl DisclosureGate {
    pub fn acquire_disclosure(&self) -> DisclosureLease {
        let mut state = lock_gate_state(&self.inner);
        while state.mutation_active || state.waiting_mutations > 0 {
            state = wait_for_gate_change(&self.inner, state);
        }
        state.active_disclosures = state.active_disclosures.saturating_add(1);
        DisclosureLease {
            inner: Arc::clone(&self.inner),
        }
    }

    pub fn acquire_mutation(&self) -> DisclosureMutationGuard {
        let mut state = lock_gate_state(&self.inner);
        state.waiting_mutations = state.waiting_mutations.saturating_add(1);
        self.inner.changed.notify_all();
        while state.mutation_active || state.active_disclosures > 0 {
            state = wait_for_gate_change(&self.inner, state);
        }
        state.waiting_mutations = state.waiting_mutations.saturating_sub(1);
        state.mutation_active = true;
        DisclosureMutationGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    pub fn owns(&self, lease: &DisclosureLease) -> bool {
        Arc::ptr_eq(&self.inner, &lease.inner)
    }
}

#[must_use = "the disclosure lease must be retained through transport handoff"]
#[derive(Debug)]
pub struct DisclosureLease {
    inner: Arc<DisclosureGateInner>,
}

impl Drop for DisclosureLease {
    fn drop(&mut self) {
        let mut state = lock_gate_state(&self.inner);
        state.active_disclosures = state.active_disclosures.saturating_sub(1);
        self.inner.changed.notify_all();
    }
}

#[doc(hidden)]
#[must_use = "the mutation guard must cover the complete state change"]
#[derive(Debug)]
pub struct DisclosureMutationGuard {
    inner: Arc<DisclosureGateInner>,
}

impl Drop for DisclosureMutationGuard {
    fn drop(&mut self) {
        let mut state = lock_gate_state(&self.inner);
        state.mutation_active = false;
        self.inner.changed.notify_all();
    }
}

fn lock_gate_state(inner: &DisclosureGateInner) -> MutexGuard<'_, DisclosureGateState> {
    match inner.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn wait_for_gate_change<'a>(
    inner: &'a DisclosureGateInner,
    state: MutexGuard<'a, DisclosureGateState>,
) -> MutexGuard<'a, DisclosureGateState> {
    match inner.changed.wait(state) {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Debug, Error)]
pub enum SearchContractError {
    #[error("the query must not be empty")]
    EmptyQuery,
    #[error("query is {0} bytes; the maximum is {MAX_QUERY_BYTES}")]
    QueryTooLarge(usize),
    #[error("top_k must be between {MIN_TOP_K} and {MAX_TOP_K}, got {0}")]
    InvalidTopK(u8),
    #[error("unsupported protocol version: {0}")]
    UnsupportedProtocol(String),
    #[error("search is not authorized")]
    Unauthorized,
    #[error("search backend is unavailable: {0}")]
    Unavailable(String),
    #[error("search failed: {0}")]
    Backend(String),
}

#[async_trait]
pub trait FederatedSearch: Send + Sync + 'static {
    async fn search(&self, request: SearchRequest) -> Result<SearchResponse, SearchContractError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disclosure_gate_blocks_mutation_until_the_lease_is_released() {
        use std::sync::mpsc;
        use std::time::Duration;

        let gate = DisclosureGate::default();
        let lease = gate.acquire_disclosure();
        let (mutation_acquired_tx, mutation_acquired_rx) = mpsc::channel();
        let mutation_gate = gate.clone();
        let mutation = std::thread::spawn(move || {
            let _mutation = mutation_gate.acquire_mutation();
            mutation_acquired_tx.send(()).ok();
        });

        assert!(
            mutation_acquired_rx
                .recv_timeout(Duration::from_millis(25))
                .is_err()
        );
        drop(lease);
        mutation_acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        mutation.join().unwrap();
    }

    #[test]
    fn waiting_mutation_precedes_a_later_disclosure() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let gate = DisclosureGate::default();
        let first_lease = gate.acquire_disclosure();
        let (mutation_acquired_tx, mutation_acquired_rx) = mpsc::channel();
        let (release_mutation_tx, release_mutation_rx) = mpsc::channel();
        let mutation_gate = gate.clone();
        let mutation = std::thread::spawn(move || {
            let mutation = mutation_gate.acquire_mutation();
            mutation_acquired_tx.send(()).ok();
            release_mutation_rx.recv().ok();
            drop(mutation);
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        while lock_gate_state(&gate.inner).waiting_mutations == 0 {
            assert!(Instant::now() < deadline, "mutation did not enter the gate");
            std::thread::yield_now();
        }

        let (later_acquired_tx, later_acquired_rx) = mpsc::channel();
        let later_gate = gate.clone();
        let later = std::thread::spawn(move || {
            let _lease = later_gate.acquire_disclosure();
            later_acquired_tx.send(()).ok();
        });

        drop(first_lease);
        mutation_acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert!(
            later_acquired_rx
                .recv_timeout(Duration::from_millis(25))
                .is_err()
        );
        release_mutation_tx.send(()).unwrap();
        later_acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        mutation.join().unwrap();
        later.join().unwrap();
    }

    #[test]
    fn collection_policy_egress_matrix_keeps_peer_and_external_ai_independent() {
        assert_eq!(CollectionPolicy::default(), CollectionPolicy::local_only());
        let cases = [
            (
                CollectionPolicy::local_only(),
                true,
                true,
                false,
                false,
                false,
            ),
            (
                CollectionPolicy::shared_with_peers(),
                false,
                true,
                true,
                false,
                false,
            ),
            (
                CollectionPolicy {
                    local_only: false,
                    peer_shareable: false,
                    allow_external_ai: true,
                    internet_public: false,
                },
                false,
                true,
                false,
                true,
                false,
            ),
            (
                CollectionPolicy {
                    local_only: false,
                    peer_shareable: true,
                    allow_external_ai: true,
                    internet_public: false,
                },
                false,
                true,
                true,
                true,
                true,
            ),
        ];

        for (
            policy,
            local_only,
            local_assistant,
            peer_assistant,
            local_external_ai,
            peer_external_ai,
        ) in cases
        {
            assert_eq!(policy.is_local_only(), local_only);
            assert_eq!(
                policy.can_serve_locally(SearchPurpose::LocalAssistant),
                local_assistant
            );
            assert_eq!(
                policy.can_serve_peer(SearchPurpose::LocalAssistant),
                peer_assistant
            );
            assert_eq!(
                policy.can_serve_locally(SearchPurpose::ExternalAi),
                local_external_ai
            );
            assert_eq!(
                policy.can_serve_peer(SearchPurpose::ExternalAi),
                peer_external_ai
            );
        }
    }

    #[test]
    fn normalization_only_derives_the_compatibility_flag() {
        let mut policy = CollectionPolicy {
            local_only: true,
            peer_shareable: false,
            allow_external_ai: true,
            internet_public: false,
        };

        policy.normalize();

        assert_eq!(
            policy,
            CollectionPolicy {
                local_only: false,
                peer_shareable: false,
                allow_external_ai: true,
                internet_public: false,
            }
        );
    }

    #[test]
    fn request_limits_are_enforced() {
        let mut request = SearchRequest::new("", SearchPurpose::LocalAssistant, DEFAULT_TOP_K);
        assert!(matches!(
            request.validate(),
            Err(SearchContractError::EmptyQuery)
        ));
        request.query = "ok".into();
        request.top_k = MAX_TOP_K + 1;
        assert!(matches!(
            request.validate(),
            Err(SearchContractError::InvalidTopK(_))
        ));
    }

    #[test]
    fn new_search_requests_use_v2_protocol() {
        let request = SearchRequest::new("pagos", SearchPurpose::LocalAssistant, DEFAULT_TOP_K);

        assert_eq!(request.protocol_version, "/airwiki/search/2.0.0");
        assert!(request.validate().is_ok());
    }

    #[test]
    fn legacy_v1_search_requests_are_rejected() {
        let mut request = SearchRequest::new("pagos", SearchPurpose::LocalAssistant, DEFAULT_TOP_K);
        request.protocol_version = "/airwiki/search/1.0.0".to_owned();

        assert!(matches!(
            request.validate(),
            Err(SearchContractError::UnsupportedProtocol(protocol))
                if protocol == "/airwiki/search/1.0.0"
        ));
    }

    #[test]
    fn draft_is_bounded_and_deduplicated() {
        let mut draft = EnrichmentDraft {
            concept_type: ConceptType::Document,
            title: " Test ".into(),
            description: " D ".into(),
            language: " es ".into(),
            tags: vec!["Pago".into(), "pago".into(), "".into()],
            entities: Vec::new(),
            links: Vec::new(),
            summary: " S ".into(),
            classification_confidence: 3.0,
            classification_explanation: String::new(),
        };
        draft.sanitize();
        assert_eq!(draft.tags, vec!["pago"]);
        assert_eq!(draft.classification_confidence, 1.0);
    }
}
