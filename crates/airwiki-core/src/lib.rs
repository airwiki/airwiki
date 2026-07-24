//! Local-first knowledge engine used by every AirWiki workstation.
//!
//! The crate deliberately has no UI or network runtime. Expensive operations are
//! exposed through ordinary synchronous APIs or async provider traits so callers
//! can run them on Tokio worker tasks and keep the desktop thread responsive.

mod chunk_identity;
pub mod config;
pub mod inference;
pub mod ingest;
pub mod knowledge;
pub mod okf;
pub mod pipeline;
pub mod publication;
pub mod repair;
#[cfg(feature = "fastembed-runtime")]
mod reranker;
pub mod search;
pub mod storage;

pub use config::{AppPaths, CollectionPaths};
#[cfg(feature = "fastembed-runtime")]
pub use inference::fastembed_provider::{
    E5SnapshotLoadError, E5Tokenizer, FastEmbedE5Small, PinnedE5Snapshot,
};
pub use inference::{
    DeterministicEmbeddingProvider, DeterministicGenerationProvider, E5_MODEL_REPOSITORY,
    E5_MODEL_REVISION, EmbeddingProvider, GenerationProvider, GenerationRuntimeConfig,
    LlamaServerProvider,
};
pub use ingest::{
    ChunkDraft, Chunker, ExtractedDocument, ExtractedSection, FileCandidate, FolderWatcher,
    IngestLimits, SourceFormat, SourceIssueCode, Tokenizer, WhitespaceTokenizer, discover_files,
    extract_file, sha256_file,
};
pub use knowledge::{
    BundleHealthReport, HealthIssue, HealthRecovery, HealthSeverity, KnowledgeBundleState,
    KnowledgeBundleView, KnowledgeConceptView, KnowledgeLinkDisposition, KnowledgeLinkView,
    KnowledgePageId, KnowledgePageView, MAX_KNOWLEDGE_PAGE_BYTES, OkfBundleInspector,
};
pub use okf::{OkfConcept, OkfPublisher, OkfValidationError};
pub use pipeline::{CollectionPreflight, IngestOutcome, IngestPipeline, ReviewEdits};
pub use publication::{OkfPublicationMaterializer, PublicationRecoveryReport};
pub use repair::{
    GuidedRepairChange, GuidedRepairFilePreview, GuidedRepairPreview, GuidedRepairResult,
    RepairAction, RepairAuthority, RepairPlan, RepairPlanId, RepairPreview, RepairResult,
    RepairRisk, WikiRepairError, WikiRepairExecutor, WikiRepairPlanner,
};
#[cfg(feature = "fastembed-runtime")]
pub use reranker::{
    FastEmbedMmarcoReranker, MMARCO_RERANKER_PROFILE_ID, MMARCO_RERANKER_REPOSITORY,
    MMARCO_RERANKER_REVISION, MmarcoRerankerLoadError, PinnedMmarcoRerankerSnapshot,
};
pub use search::{
    DeterministicEvidenceRelevanceProvider, EvidenceDecision, EvidenceRelevanceError,
    EvidenceRelevanceProvider, HybridSearchEngine, RELEVANCE_CANDIDATE_LIMIT, RelevanceInput,
};
pub use storage::{
    AuditEvent, CollectionMaintenanceCounts, CollectionMaintenanceRecord,
    CollectionMaintenanceResult, CollectionMaintenanceStatus, CollectionRecord, CollectionStats,
    ConceptRecord, Database, FederationIndexRecord, GrantRecord, JobRecord, PeerRecord,
    ReviewEvidenceChunkRecord, ReviewEvidencePageRecord, ReviewReanalysisClaim, ReviewVersionToken,
    SourceDocumentRecord, StoredChunk,
};

/// Embedding dimensionality required by multilingual-e5-small.
pub const EMBEDDING_DIMENSIONS: usize = 384;
