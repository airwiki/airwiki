//! Fail-closed planning and execution for managed OKF repairs.
//!
//! The automatic executor only regenerates `index.md`, which is a derived
//! directory over an already coherent published snapshot. Concept pages and
//! `log.md` are never repaired automatically. The guided executor can withdraw
//! published concepts back to human review or remove confirmed unmanaged
//! orphans, but it refuses to invent knowledge or append-only history.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::knowledge::{
    HealthIssue, HealthSeverity, KnowledgeBundleState, KnowledgeBundleView, KnowledgePageId,
    OkfBundleInspector,
};
use crate::okf::{OkfPublisher, atomic_write};
use crate::storage::{AuditEvent, CollectionRecord, Database};

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const SNAPSHOT_RETENTION: usize = 5;
const MAX_SNAPSHOT_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_GUIDED_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
const DERIVED_INDEX_CODES: &[&str] = &[
    "broken_index_link",
    "index_missing_concept",
    "invalid_index_structure",
    "missing_index",
    "stale_index_metadata",
];

/// Stable identity of one repair plan and its on-disk recovery snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepairPlanId(Uuid);

impl RepairPlanId {
    /// Creates a fresh opaque identity for a repair plan.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Returns the UUID representation used by audit and snapshot manifests.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for RepairPlanId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RepairPlanId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Maximum authority required by a proposed repair action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepairRisk {
    /// A deterministic artifact can be rebuilt without changing knowledge.
    Derived,
    /// A concept or other knowledge-bearing page requires human judgment.
    Content,
    /// Append-only publication history requires human judgment.
    History,
}

impl fmt::Display for RepairRisk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Derived => "derived",
            Self::Content => "content",
            Self::History => "history",
        })
    }
}

/// A deterministic action or a human-review requirement discovered by planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairAction {
    /// Rebuild `index.md` from the current SQLite-published concepts.
    RegenerateIndex { reason_codes: Vec<String> },
    /// Review knowledge-bearing pages without mutating them automatically.
    ReviewContent { issue_codes: Vec<String> },
    /// Review append-only history without mutating it automatically.
    ReviewHistory { issue_codes: Vec<String> },
}

/// Human-selected authority used by a guided repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepairAuthority {
    /// Withdraw a reviewed revision and return it to the normal review queue.
    HumanReview,
    /// Remove an unmanaged orphan that has no published SQLite record.
    PublishedDatabase,
}

impl fmt::Display for RepairAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::HumanReview => "human_review",
            Self::PublishedDatabase => "published_database",
        })
    }
}

/// File-level effect shown before a guided repair can be confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidedRepairChange {
    WithdrawConcept,
    RemoveOrphan,
    RegenerateIndex,
    AppendDeprecationHistory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidedRepairFilePreview {
    pub page: KnowledgePageId,
    pub change: GuidedRepairChange,
    pub before_fingerprint: Option<String>,
}

/// Immutable confirmation payload tied to the inspected bundle fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidedRepairPreview {
    pub plan_id: RepairPlanId,
    pub collection_id: Uuid,
    pub expected_bundle_fingerprint: String,
    pub authorities: Vec<RepairAuthority>,
    pub files: Vec<GuidedRepairFilePreview>,
    pub concepts_returned_to_review: Vec<Uuid>,
    pub orphan_concepts_removed: Vec<Uuid>,
    pub impact_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidedRepairResult {
    pub plan_id: RepairPlanId,
    pub collection_id: Uuid,
    pub concepts_returned_to_review: Vec<Uuid>,
    pub orphan_concepts_removed: Vec<Uuid>,
    pub snapshot_manifest_sha256: String,
    pub bundle_fingerprint: String,
    pub completed_at: DateTime<Utc>,
}

impl RepairAction {
    /// Returns the authority required to carry out this action.
    pub fn risk(&self) -> RepairRisk {
        match self {
            Self::RegenerateIndex { .. } => RepairRisk::Derived,
            Self::ReviewContent { .. } => RepairRisk::Content,
            Self::ReviewHistory { .. } => RepairRisk::History,
        }
    }
}

/// Human-readable impact and affected pages for one proposed action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairPreview {
    pub action: RepairAction,
    pub affected_pages: Vec<KnowledgePageId>,
    /// Stable code translated by the presentation layer.
    pub explanation_code: String,
    pub requires_confirmation: bool,
}

/// Immutable plan tied to one inspected bundle fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairPlan {
    pub id: RepairPlanId,
    pub collection_id: Uuid,
    pub expected_bundle_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub previews: Vec<RepairPreview>,
}

impl RepairPlan {
    /// Returns true when the plan has no repair or review action.
    pub fn is_empty(&self) -> bool {
        self.previews.is_empty()
    }

    /// Returns the highest authority required by this plan.
    pub fn maximum_risk(&self) -> Option<RepairRisk> {
        self.previews
            .iter()
            .map(|preview| preview.action.risk())
            .max()
    }
}

/// Outcome of an automatic derived-artifact execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairResult {
    pub plan_id: RepairPlanId,
    pub collection_id: Uuid,
    pub applied_actions: Vec<RepairAction>,
    pub pending_confirmation: Vec<RepairAction>,
    pub snapshot_manifest_sha256: String,
    pub bundle_fingerprint: String,
    pub completed_at: DateTime<Utc>,
}

/// Typed failures exposed by repair planning and execution.
#[derive(Debug, Error)]
pub enum WikiRepairError {
    #[error("the OKF bundle is being updated")]
    BundleUpdating,
    #[error("the repair plan no longer matches the current bundle")]
    StalePlan,
    #[error("the collection no longer exists")]
    CollectionMissing,
    #[error("the plan only contains actions that require explicit human confirmation: {risks:?}")]
    ConfirmationRequired { risks: Vec<RepairRisk> },
    #[error("the published snapshot is not coherent enough for automatic repair: {codes:?}")]
    IncoherentPublishedSnapshot { codes: Vec<String> },
    #[error("the managed bundle or index has an unsafe filesystem layout")]
    UnsafeBundleLayout,
    #[error("the index is too large to snapshot safely ({actual} bytes, maximum {maximum})")]
    SnapshotTooLarge { actual: u64, maximum: u64 },
    #[error("could not inspect the repair state")]
    Inspection(#[source] anyhow::Error),
    #[error("could not access the published database snapshot")]
    Storage(#[source] anyhow::Error),
    #[error("could not create or retain the repair snapshot")]
    Snapshot(#[source] anyhow::Error),
    #[error("could not regenerate the derived OKF index")]
    Regeneration(#[source] anyhow::Error),
    #[error("history repair requires a separate human recovery decision")]
    HistoryRepairRequiresHumanRecovery,
    #[error("the guided repair scope cannot be resolved safely")]
    UnresolvedGuidedScope,
    #[error("the guided repair did not produce a coherent published bundle: {codes:?}")]
    GuidedPostValidation { codes: Vec<String> },
    #[error("the regenerated bundle failed validation: {codes:?}")]
    PostValidation { codes: Vec<String> },
    #[error("repair failed ({cause}) and restoring the previous index also failed ({rollback})")]
    RollbackFailed { cause: String, rollback: String },
}

/// Creates immutable repair plans from a read-only bundle inspection.
#[derive(Debug, Clone, Copy, Default)]
pub struct WikiRepairPlanner;

impl WikiRepairPlanner {
    /// Plans deterministic and guided work without reading or writing files.
    pub fn plan(bundle: &KnowledgeBundleView) -> Result<RepairPlan, WikiRepairError> {
        if bundle.state == KnowledgeBundleState::Updating {
            return Err(WikiRepairError::BundleUpdating);
        }

        let mut derived_codes = BTreeSet::new();
        let mut content_codes = BTreeSet::new();
        let mut history_codes = BTreeSet::new();
        let mut content_pages = BTreeSet::new();
        let mut history_pages = BTreeSet::new();

        for issue in &bundle.health.issues {
            if issue.severity == HealthSeverity::Info {
                continue;
            }
            match issue_risk(issue) {
                RepairRisk::Derived => {
                    derived_codes.insert(issue.code.clone());
                }
                RepairRisk::Content => {
                    content_codes.insert(issue.code.clone());
                    if let Some(page) = issue.page {
                        content_pages.insert(page);
                    }
                }
                RepairRisk::History => {
                    history_codes.insert(issue.code.clone());
                    if let Some(page) = issue.page {
                        history_pages.insert(page);
                    }
                }
            }
        }

        let mut previews = Vec::new();
        if !derived_codes.is_empty() {
            previews.push(RepairPreview {
                action: RepairAction::RegenerateIndex {
                    reason_codes: derived_codes.into_iter().collect(),
                },
                affected_pages: vec![KnowledgePageId::Index],
                explanation_code: "repair_derived_index".to_owned(),
                requires_confirmation: false,
            });
        }
        if !content_codes.is_empty() {
            previews.push(RepairPreview {
                action: RepairAction::ReviewContent {
                    issue_codes: content_codes.into_iter().collect(),
                },
                affected_pages: content_pages.into_iter().collect(),
                explanation_code: "repair_content_review_required".to_owned(),
                requires_confirmation: true,
            });
        }
        if !history_codes.is_empty() {
            previews.push(RepairPreview {
                action: RepairAction::ReviewHistory {
                    issue_codes: history_codes.into_iter().collect(),
                },
                affected_pages: history_pages.into_iter().collect(),
                explanation_code: "repair_history_review_required".to_owned(),
                requires_confirmation: true,
            });
        }

        Ok(RepairPlan {
            id: RepairPlanId::new(),
            collection_id: bundle.collection_id,
            expected_bundle_fingerprint: bundle.fingerprint.clone(),
            created_at: Utc::now(),
            previews,
        })
    }
}

/// Executes only deterministic derived-artifact actions under the publication lock.
#[derive(Debug, Clone)]
pub struct WikiRepairExecutor {
    database: Database,
    inspector: OkfBundleInspector,
}

impl WikiRepairExecutor {
    pub fn new(database: Database) -> Self {
        Self {
            inspector: OkfBundleInspector::new(database.clone()),
            database,
        }
    }

    /// Applies only [`RepairRisk::Derived`] actions and leaves all other actions pending.
    pub fn execute_automatic(&self, plan: &RepairPlan) -> Result<RepairResult, WikiRepairError> {
        self.execute_automatic_with_observer(plan, |_| Ok(()))
    }

    /// Builds a file-level preview without changing publication state or files.
    pub fn prepare_guided(
        &self,
        plan: &RepairPlan,
    ) -> Result<GuidedRepairPreview, WikiRepairError> {
        if plan.previews.iter().any(|preview| {
            preview.requires_confirmation && preview.action.risk() == RepairRisk::History
        }) {
            return Err(WikiRepairError::HistoryRepairRequiresHumanRecovery);
        }
        let bundle = self
            .inspector
            .inspect_bundle(plan.collection_id)
            .map_err(WikiRepairError::Inspection)?;
        if bundle.state == KnowledgeBundleState::Updating {
            return Err(WikiRepairError::BundleUpdating);
        }
        if bundle.fingerprint != plan.expected_bundle_fingerprint {
            return Err(WikiRepairError::StalePlan);
        }
        let collection = self
            .database
            .collection(plan.collection_id)
            .map_err(WikiRepairError::Storage)?
            .ok_or(WikiRepairError::CollectionMissing)?;
        let mut affected = BTreeSet::new();
        let mut scan_orphans = false;
        for preview in plan.previews.iter().filter(|preview| {
            preview.requires_confirmation && preview.action.risk() == RepairRisk::Content
        }) {
            let RepairAction::ReviewContent { issue_codes } = &preview.action else {
                return Err(WikiRepairError::UnresolvedGuidedScope);
            };
            scan_orphans |= issue_codes.iter().any(|code| code == "unexpected_concept");
            if preview.affected_pages.is_empty()
                && issue_codes.iter().any(|code| code != "unexpected_concept")
            {
                return Err(WikiRepairError::UnresolvedGuidedScope);
            }
            affected.extend(preview.affected_pages.iter().copied());
        }
        if scan_orphans {
            affected.extend(
                guided_orphan_concepts(&collection, &self.database)?
                    .into_iter()
                    .map(KnowledgePageId::Concept),
            );
        }
        if affected.is_empty() {
            return Err(WikiRepairError::UnresolvedGuidedScope);
        }

        let mut returned_to_review = Vec::new();
        let mut removed_orphans = Vec::new();
        let mut files = Vec::new();
        for page in affected {
            let KnowledgePageId::Concept(concept_id) = page else {
                return Err(WikiRepairError::UnresolvedGuidedScope);
            };
            let before_fingerprint = managed_page_fingerprint(&collection, page)?;
            let concept = self
                .database
                .concept(concept_id)
                .map_err(WikiRepairError::Storage)?;
            if concept
                .is_some_and(|concept| concept.status == airwiki_types::DocumentStatus::Published)
            {
                returned_to_review.push(concept_id);
                files.push(GuidedRepairFilePreview {
                    page,
                    change: GuidedRepairChange::WithdrawConcept,
                    before_fingerprint,
                });
            } else {
                removed_orphans.push(concept_id);
                files.push(GuidedRepairFilePreview {
                    page,
                    change: GuidedRepairChange::RemoveOrphan,
                    before_fingerprint,
                });
            }
        }
        returned_to_review.sort_unstable();
        removed_orphans.sort_unstable();
        if !returned_to_review.is_empty() {
            files.push(GuidedRepairFilePreview {
                page: KnowledgePageId::Log,
                change: GuidedRepairChange::AppendDeprecationHistory,
                before_fingerprint: managed_page_fingerprint(&collection, KnowledgePageId::Log)?,
            });
        }
        files.push(GuidedRepairFilePreview {
            page: KnowledgePageId::Index,
            change: GuidedRepairChange::RegenerateIndex,
            before_fingerprint: managed_page_fingerprint(&collection, KnowledgePageId::Index)?,
        });
        let mut authorities = Vec::new();
        if !returned_to_review.is_empty() {
            authorities.push(RepairAuthority::HumanReview);
        }
        if !removed_orphans.is_empty() {
            authorities.push(RepairAuthority::PublishedDatabase);
        }
        Ok(GuidedRepairPreview {
            plan_id: plan.id,
            collection_id: plan.collection_id,
            expected_bundle_fingerprint: plan.expected_bundle_fingerprint.clone(),
            authorities,
            files,
            concepts_returned_to_review: returned_to_review,
            orphan_concepts_removed: removed_orphans,
            impact_code: "guided_repair_withdraws_until_review".to_owned(),
        })
    }

    /// Applies a previously previewed repair. Reviewed concepts are withdrawn
    /// before any file changes and can only return through the normal human
    /// approval workflow.
    pub fn execute_guided(
        &self,
        preview: &GuidedRepairPreview,
    ) -> Result<GuidedRepairResult, WikiRepairError> {
        let _publication_guard = self
            .database
            .publication_guard()
            .map_err(WikiRepairError::Storage)?;
        let before = self
            .inspector
            .inspect_bundle(preview.collection_id)
            .map_err(WikiRepairError::Inspection)?;
        if before.state == KnowledgeBundleState::Updating {
            return Err(WikiRepairError::BundleUpdating);
        }
        if before.fingerprint != preview.expected_bundle_fingerprint {
            return Err(WikiRepairError::StalePlan);
        }
        let plan = WikiRepairPlanner::plan(&before)?;
        let current_preview = self.prepare_guided(&plan)?;
        if !guided_preview_equivalent(preview, &current_preview) {
            return Err(WikiRepairError::StalePlan);
        }
        let collection = self
            .database
            .collection(preview.collection_id)
            .map_err(WikiRepairError::Storage)?
            .ok_or(WikiRepairError::CollectionMissing)?;
        let pages = preview
            .files
            .iter()
            .map(|file| file.page)
            .collect::<BTreeSet<_>>();
        let snapshot = create_guided_snapshot(&collection, preview, &pages)?;
        let publisher = OkfPublisher::new(&collection.wiki_folder);

        let mutation = (|| -> Result<KnowledgeBundleView, WikiRepairError> {
            for concept_id in &preview.concepts_returned_to_review {
                validate_guided_page_fingerprint(
                    &collection,
                    preview,
                    KnowledgePageId::Concept(*concept_id),
                )?;
                let concept = self
                    .database
                    .concept(*concept_id)
                    .map_err(WikiRepairError::Storage)?
                    .ok_or(WikiRepairError::StalePlan)?;
                let source = self
                    .database
                    .source_document(concept.source_document_id)
                    .map_err(WikiRepairError::Storage)?
                    .ok_or(WikiRepairError::StalePlan)?;
                let changed = self
                    .database
                    .return_to_review_if_current(
                        *concept_id,
                        &source.source_sha256,
                        source.revision,
                        "guided_wiki_repair",
                    )
                    .map_err(WikiRepairError::Storage)?;
                if !changed {
                    return Err(WikiRepairError::StalePlan);
                }
                let remaining = self
                    .database
                    .list_published_concepts(preview.collection_id)
                    .map_err(WikiRepairError::Storage)?;
                publisher
                    .remove(*concept_id, &source.source_sha256, &remaining)
                    .map_err(WikiRepairError::Regeneration)?;
            }
            for concept_id in &preview.orphan_concepts_removed {
                validate_guided_page_fingerprint(
                    &collection,
                    preview,
                    KnowledgePageId::Concept(*concept_id),
                )?;
                remove_guided_orphan(&collection, *concept_id)?;
            }
            if !preview.orphan_concepts_removed.is_empty() {
                let remaining = self
                    .database
                    .list_published_concepts(preview.collection_id)
                    .map_err(WikiRepairError::Storage)?;
                publisher
                    .regenerate_index(&remaining)
                    .map_err(WikiRepairError::Regeneration)?;
            }
            let after = self
                .inspector
                .inspect_bundle(preview.collection_id)
                .map_err(WikiRepairError::Inspection)?;
            let codes = guided_post_validation_codes(&after);
            if !codes.is_empty() {
                return Err(WikiRepairError::GuidedPostValidation { codes });
            }
            Ok(after)
        })();
        let after = match mutation {
            Ok(after) => after,
            Err(error) => {
                return Err(restore_guided_or_combine(&collection, &snapshot, error));
            }
        };
        let audit = AuditEvent {
            id: Uuid::new_v4(),
            actor: "user".to_owned(),
            action: "guided_okf_repair_confirmed".to_owned(),
            target_type: "collection".to_owned(),
            target_id: Some(preview.collection_id.to_string()),
            details: serde_json::json!({
                "plan_id": preview.plan_id.to_string(),
                "authority": preview.authorities.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "returned_to_review_count": preview.concepts_returned_to_review.len(),
                "removed_orphan_count": preview.orphan_concepts_removed.len(),
                "snapshot_manifest_sha256": &snapshot.manifest_sha256,
            }),
            created_at: Utc::now(),
        };
        if let Err(error) = self.database.record_audit(&audit) {
            return Err(restore_guided_or_combine(
                &collection,
                &snapshot,
                WikiRepairError::Storage(error),
            ));
        }
        if retain_guided_snapshots(&collection, SNAPSHOT_RETENTION).is_err() {
            tracing::warn!(
                error_kind = "guided_snapshot_retention",
                %preview.collection_id,
                "guided repair completed but old snapshot retention needs a later retry"
            );
        }
        Ok(GuidedRepairResult {
            plan_id: preview.plan_id,
            collection_id: preview.collection_id,
            concepts_returned_to_review: preview.concepts_returned_to_review.clone(),
            orphan_concepts_removed: preview.orphan_concepts_removed.clone(),
            snapshot_manifest_sha256: snapshot.manifest_sha256,
            bundle_fingerprint: after.fingerprint,
            completed_at: Utc::now(),
        })
    }

    fn execute_automatic_with_observer(
        &self,
        plan: &RepairPlan,
        mut observer: impl FnMut(RepairExecutionStep) -> Result<(), WikiRepairError>,
    ) -> Result<RepairResult, WikiRepairError> {
        let automatic = plan
            .previews
            .iter()
            .find(|preview| preview.action.risk() == RepairRisk::Derived)
            .map(|preview| preview.action.clone());
        let pending_confirmation = plan
            .previews
            .iter()
            .filter(|preview| preview.requires_confirmation)
            .map(|preview| preview.action.clone())
            .collect::<Vec<_>>();
        let Some(automatic) = automatic else {
            let risks = pending_confirmation
                .iter()
                .map(RepairAction::risk)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            return Err(WikiRepairError::ConfirmationRequired { risks });
        };

        let _publication_guard = self
            .database
            .publication_guard()
            .map_err(WikiRepairError::Storage)?;
        let before = self
            .inspector
            .inspect_bundle(plan.collection_id)
            .map_err(WikiRepairError::Inspection)?;
        if before.state == KnowledgeBundleState::Updating {
            return Err(WikiRepairError::BundleUpdating);
        }
        if before.fingerprint != plan.expected_bundle_fingerprint {
            return Err(WikiRepairError::StalePlan);
        }
        let planned_codes = match &automatic {
            RepairAction::RegenerateIndex { reason_codes } => {
                reason_codes.iter().cloned().collect::<BTreeSet<_>>()
            }
            RepairAction::ReviewContent { .. } | RepairAction::ReviewHistory { .. } => {
                return Err(WikiRepairError::StalePlan);
            }
        };
        let current_codes = before
            .health
            .issues
            .iter()
            .filter(|issue| issue.severity != HealthSeverity::Info)
            .filter(|issue| issue_risk(issue) == RepairRisk::Derived)
            .map(|issue| issue.code.clone())
            .collect::<BTreeSet<_>>();
        if current_codes.is_empty() || current_codes != planned_codes {
            return Err(WikiRepairError::StalePlan);
        }

        let blocking_codes = blocking_health_codes(&before);
        if !blocking_codes.is_empty() {
            return Err(WikiRepairError::IncoherentPublishedSnapshot {
                codes: blocking_codes,
            });
        }

        let collection = self
            .database
            .collection(plan.collection_id)
            .map_err(WikiRepairError::Storage)?
            .ok_or(WikiRepairError::CollectionMissing)?;
        let published = self
            .database
            .list_published_concepts(plan.collection_id)
            .map_err(WikiRepairError::Storage)?;
        if published.is_empty() || before.concepts.len() != published.len() {
            return Err(WikiRepairError::IncoherentPublishedSnapshot {
                codes: vec!["published_snapshot_incomplete".to_owned()],
            });
        }

        let publisher = OkfPublisher::new(&collection.wiki_folder);
        let rendered =
            OkfPublisher::render_index(&published).map_err(WikiRepairError::Regeneration)?;
        let rendered_sha256 = sha256(rendered.as_bytes());
        let index_path = collection.wiki_folder.join("index.md");
        let original = capture_managed_index(&collection, before.index_fingerprint.as_deref())?;
        let snapshot = create_snapshot(&collection, plan, original.as_deref())?;
        retain_snapshots(&collection, SNAPSHOT_RETENTION)?;

        if let Err(error) = publisher.regenerate_index(&published) {
            return Err(WikiRepairError::Regeneration(error));
        }
        if let Err(error) = observer(RepairExecutionStep::IndexWritten) {
            return Err(rollback_or_combine(
                &index_path,
                original.as_deref(),
                &rendered_sha256,
                error,
            ));
        }

        let after = match self.inspector.inspect_bundle(plan.collection_id) {
            Ok(after) => after,
            Err(error) => {
                return Err(rollback_or_combine(
                    &index_path,
                    original.as_deref(),
                    &rendered_sha256,
                    WikiRepairError::Inspection(error),
                ));
            }
        };
        let validation_codes = post_validation_codes(&after);
        if !validation_codes.is_empty() {
            return Err(rollback_or_combine(
                &index_path,
                original.as_deref(),
                &rendered_sha256,
                WikiRepairError::PostValidation {
                    codes: validation_codes,
                },
            ));
        }

        let audit = AuditEvent {
            id: Uuid::new_v4(),
            actor: "system".to_owned(),
            action: "regenerated_derived_okf_index".to_owned(),
            target_type: "collection".to_owned(),
            target_id: Some(plan.collection_id.to_string()),
            details: serde_json::json!({
                "plan_id": plan.id.to_string(),
                "before_bundle_fingerprint": &before.fingerprint,
                "after_bundle_fingerprint": &after.fingerprint,
                "snapshot_manifest_sha256": &snapshot.manifest_sha256,
            }),
            created_at: Utc::now(),
        };
        if let Err(error) = self.database.record_audit(&audit) {
            return Err(rollback_or_combine(
                &index_path,
                original.as_deref(),
                &rendered_sha256,
                WikiRepairError::Storage(error),
            ));
        }

        Ok(RepairResult {
            plan_id: plan.id,
            collection_id: plan.collection_id,
            applied_actions: vec![automatic],
            pending_confirmation,
            snapshot_manifest_sha256: snapshot.manifest_sha256,
            bundle_fingerprint: after.fingerprint,
            completed_at: Utc::now(),
        })
    }
}

fn guided_preview_equivalent(
    expected: &GuidedRepairPreview,
    current: &GuidedRepairPreview,
) -> bool {
    expected.collection_id == current.collection_id
        && expected.expected_bundle_fingerprint == current.expected_bundle_fingerprint
        && expected.authorities == current.authorities
        && expected.files == current.files
        && expected.concepts_returned_to_review == current.concepts_returned_to_review
        && expected.orphan_concepts_removed == current.orphan_concepts_removed
}

fn guided_orphan_concepts(
    collection: &CollectionRecord,
    database: &Database,
) -> Result<Vec<Uuid>, WikiRepairError> {
    validate_bundle_root(&collection.wiki_folder)?;
    let expected = database
        .list_published_concepts(collection.id)
        .map_err(WikiRepairError::Storage)?
        .into_iter()
        .map(|concept| concept.id)
        .collect::<BTreeSet<_>>();
    let directory = collection.wiki_folder.join("concepts");
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(WikiRepairError::Snapshot(error.into())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WikiRepairError::UnsafeBundleLayout);
    }
    let mut orphans = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| WikiRepairError::Snapshot(error.into()))? {
        let entry = entry.map_err(|error| WikiRepairError::Snapshot(error.into()))?;
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Ok(id) = Uuid::parse_str(stem) else {
            continue;
        };
        if !expected.contains(&id) {
            orphans.push(id);
        }
    }
    orphans.sort_unstable();
    Ok(orphans)
}

fn managed_page_path(collection: &CollectionRecord, page: KnowledgePageId) -> PathBuf {
    match page {
        KnowledgePageId::Index => collection.wiki_folder.join("index.md"),
        KnowledgePageId::Log => collection.wiki_folder.join("log.md"),
        KnowledgePageId::Concept(id) => collection
            .wiki_folder
            .join("concepts")
            .join(format!("{id}.md")),
    }
}

fn validate_guided_page_parent(
    collection: &CollectionRecord,
    page: KnowledgePageId,
) -> Result<(), WikiRepairError> {
    validate_bundle_root(&collection.wiki_folder)?;
    if matches!(page, KnowledgePageId::Concept(_)) {
        let concepts = collection.wiki_folder.join("concepts");
        match fs::symlink_metadata(&concepts) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(WikiRepairError::UnsafeBundleLayout);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(WikiRepairError::Snapshot(error.into())),
        }
    }
    Ok(())
}

fn managed_page_fingerprint(
    collection: &CollectionRecord,
    page: KnowledgePageId,
) -> Result<Option<String>, WikiRepairError> {
    validate_guided_page_parent(collection, page)?;
    let path = managed_page_path(collection, page);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(WikiRepairError::UnsafeBundleLayout)
        }
        Ok(metadata) if metadata.len() > MAX_GUIDED_SNAPSHOT_BYTES => {
            Err(WikiRepairError::SnapshotTooLarge {
                actual: metadata.len(),
                maximum: MAX_GUIDED_SNAPSHOT_BYTES,
            })
        }
        Ok(_) => fs::read(path)
            .map(|bytes| Some(sha256(&bytes)))
            .map_err(|error| WikiRepairError::Snapshot(error.into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(WikiRepairError::Snapshot(error.into())),
    }
}

fn validate_guided_page_fingerprint(
    collection: &CollectionRecord,
    preview: &GuidedRepairPreview,
    page: KnowledgePageId,
) -> Result<(), WikiRepairError> {
    let expected = preview
        .files
        .iter()
        .find(|file| file.page == page)
        .ok_or(WikiRepairError::StalePlan)?
        .before_fingerprint
        .as_ref();
    let actual = managed_page_fingerprint(collection, page)?;
    if actual.as_ref() != expected {
        return Err(WikiRepairError::StalePlan);
    }
    Ok(())
}

fn remove_guided_orphan(
    collection: &CollectionRecord,
    concept_id: Uuid,
) -> Result<(), WikiRepairError> {
    let page = KnowledgePageId::Concept(concept_id);
    validate_guided_page_parent(collection, page)?;
    let path = managed_page_path(collection, page);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(WikiRepairError::UnsafeBundleLayout)
        }
        Ok(_) => fs::remove_file(path).map_err(|error| WikiRepairError::Regeneration(error.into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WikiRepairError::Regeneration(error.into())),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct GuidedSnapshotManifest {
    schema_version: u32,
    plan_id: Uuid,
    collection_id: Uuid,
    created_at: DateTime<Utc>,
    bundle_fingerprint: String,
    files: Vec<GuidedSnapshotFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GuidedSnapshotFile {
    relative_path: String,
    present: bool,
    byte_size: u64,
    sha256: Option<String>,
    content_hex: Option<String>,
}

#[derive(Debug)]
struct CreatedGuidedSnapshot {
    path: PathBuf,
    manifest_sha256: String,
    plan_id: Uuid,
    collection_id: Uuid,
    bundle_fingerprint: String,
}

fn guided_snapshots_root(wiki_root: &Path) -> PathBuf {
    wiki_root.join(".airwiki").join("guided-repair-snapshots")
}

fn create_guided_snapshot(
    collection: &CollectionRecord,
    preview: &GuidedRepairPreview,
    pages: &BTreeSet<KnowledgePageId>,
) -> Result<CreatedGuidedSnapshot, WikiRepairError> {
    validate_bundle_root(&collection.wiki_folder)?;
    ensure_managed_directory(&collection.wiki_folder.join(".airwiki"))?;
    let root = guided_snapshots_root(&collection.wiki_folder);
    ensure_managed_directory(&root)?;
    let path = root.join(format!("{}.json", preview.plan_id));
    if fs::symlink_metadata(&path).is_ok() {
        return Err(WikiRepairError::StalePlan);
    }
    let mut total = 0_u64;
    let mut files = Vec::new();
    for page in pages {
        validate_guided_page_parent(collection, *page)?;
        let page_path = managed_page_path(collection, *page);
        let bytes = match fs::symlink_metadata(&page_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(WikiRepairError::UnsafeBundleLayout);
            }
            Ok(metadata) => {
                total = total.saturating_add(metadata.len());
                if total > MAX_GUIDED_SNAPSHOT_BYTES {
                    return Err(WikiRepairError::SnapshotTooLarge {
                        actual: total,
                        maximum: MAX_GUIDED_SNAPSHOT_BYTES,
                    });
                }
                Some(
                    fs::read(&page_path)
                        .map_err(|error| WikiRepairError::Snapshot(error.into()))?,
                )
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(WikiRepairError::Snapshot(error.into())),
        };
        let actual_fingerprint = bytes.as_deref().map(sha256);
        let expected_fingerprint = preview
            .files
            .iter()
            .find(|file| file.page == *page)
            .ok_or(WikiRepairError::StalePlan)?
            .before_fingerprint
            .as_ref();
        if actual_fingerprint.as_ref() != expected_fingerprint {
            return Err(WikiRepairError::StalePlan);
        }
        files.push(GuidedSnapshotFile {
            relative_path: page.relative_path(),
            present: bytes.is_some(),
            byte_size: bytes
                .as_ref()
                .map_or(0, |bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
            sha256: bytes.as_deref().map(sha256),
            content_hex: bytes.as_deref().map(hex::encode),
        });
    }
    let manifest = GuidedSnapshotManifest {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        plan_id: preview.plan_id.as_uuid(),
        collection_id: preview.collection_id,
        created_at: Utc::now(),
        bundle_fingerprint: preview.expected_bundle_fingerprint.clone(),
        files,
    };
    let bytes =
        serde_json::to_vec(&manifest).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    let manifest_sha256 = sha256(&bytes);
    atomic_write(&path, &bytes).map_err(WikiRepairError::Snapshot)?;
    let verified = verify_guided_snapshot(&path)?;
    if verified.plan_id != preview.plan_id.as_uuid()
        || verified.collection_id != preview.collection_id
        || verified.bundle_fingerprint != preview.expected_bundle_fingerprint
    {
        return Err(WikiRepairError::Snapshot(anyhow::anyhow!(
            "guided repair snapshot identity does not match its preview"
        )));
    }
    Ok(CreatedGuidedSnapshot {
        path,
        manifest_sha256,
        plan_id: preview.plan_id.as_uuid(),
        collection_id: preview.collection_id,
        bundle_fingerprint: preview.expected_bundle_fingerprint.clone(),
    })
}

fn verify_guided_snapshot(path: &Path) -> Result<GuidedSnapshotManifest, WikiRepairError> {
    let bytes = read_guided_snapshot_bytes(path)?;
    verify_guided_snapshot_bytes(&bytes)
}

fn read_guided_snapshot_bytes(path: &Path) -> Result<Vec<u8>, WikiRepairError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len()
            > MAX_GUIDED_SNAPSHOT_BYTES
                .saturating_mul(2)
                .saturating_add(1024 * 1024)
    {
        return Err(WikiRepairError::UnsafeBundleLayout);
    }
    fs::read(path).map_err(|error| WikiRepairError::Snapshot(error.into()))
}

fn verify_guided_snapshot_bytes(bytes: &[u8]) -> Result<GuidedSnapshotManifest, WikiRepairError> {
    let manifest: GuidedSnapshotManifest =
        serde_json::from_slice(bytes).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    if manifest.schema_version != SNAPSHOT_SCHEMA_VERSION || manifest.files.is_empty() {
        return Err(WikiRepairError::Snapshot(anyhow::anyhow!(
            "guided repair snapshot has an unsupported shape"
        )));
    }
    let mut paths = BTreeSet::new();
    let mut total = 0_u64;
    for file in &manifest.files {
        if !paths.insert(file.relative_path.clone())
            || !guided_relative_path_is_safe(&file.relative_path)
        {
            return Err(WikiRepairError::UnsafeBundleLayout);
        }
        match (&file.content_hex, &file.sha256, file.present) {
            (Some(content), Some(expected), true) => {
                let decoded = hex::decode(content)
                    .map_err(|error| WikiRepairError::Snapshot(error.into()))?;
                let byte_size = u64::try_from(decoded.len()).unwrap_or(u64::MAX);
                total = total.saturating_add(byte_size);
                if byte_size != file.byte_size
                    || sha256(&decoded) != *expected
                    || total > MAX_GUIDED_SNAPSHOT_BYTES
                {
                    return Err(WikiRepairError::Snapshot(anyhow::anyhow!(
                        "guided repair snapshot content does not match its manifest"
                    )));
                }
            }
            (None, None, false) if file.byte_size == 0 => {}
            _ => {
                return Err(WikiRepairError::Snapshot(anyhow::anyhow!(
                    "guided repair snapshot file metadata is inconsistent"
                )));
            }
        }
    }
    Ok(manifest)
}

fn guided_relative_path_is_safe(relative: &str) -> bool {
    relative == "index.md"
        || relative == "log.md"
        || relative
            .strip_prefix("concepts/")
            .and_then(|name| name.strip_suffix(".md"))
            .is_some_and(|id| Uuid::parse_str(id).is_ok())
}

fn restore_guided_snapshot(
    collection: &CollectionRecord,
    snapshot: &CreatedGuidedSnapshot,
) -> Result<(), WikiRepairError> {
    let sealed_bytes = read_guided_snapshot_bytes(&snapshot.path)?;
    if sha256(&sealed_bytes) != snapshot.manifest_sha256 {
        return Err(WikiRepairError::UnsafeBundleLayout);
    }
    let manifest = verify_guided_snapshot_bytes(&sealed_bytes)?;
    if manifest.plan_id != snapshot.plan_id
        || manifest.collection_id != snapshot.collection_id
        || manifest.collection_id != collection.id
        || manifest.bundle_fingerprint != snapshot.bundle_fingerprint
    {
        return Err(WikiRepairError::UnsafeBundleLayout);
    }
    for file in manifest.files {
        let page = guided_page_id(&file.relative_path)?;
        validate_guided_page_parent(collection, page)?;
        let path = managed_page_path(collection, page);
        if file.present {
            if matches!(page, KnowledgePageId::Concept(_)) {
                ensure_managed_directory(&collection.wiki_folder.join("concepts"))?;
            }
            let content = file
                .content_hex
                .as_deref()
                .ok_or(WikiRepairError::UnsafeBundleLayout)?;
            let bytes =
                hex::decode(content).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
            atomic_write(&path, &bytes).map_err(WikiRepairError::Snapshot)?;
        } else {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(WikiRepairError::UnsafeBundleLayout);
                }
                Ok(_) => fs::remove_file(path)
                    .map_err(|error| WikiRepairError::Snapshot(error.into()))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(WikiRepairError::Snapshot(error.into())),
            }
        }
    }
    Ok(())
}

fn guided_page_id(relative: &str) -> Result<KnowledgePageId, WikiRepairError> {
    match relative {
        "index.md" => Ok(KnowledgePageId::Index),
        "log.md" => Ok(KnowledgePageId::Log),
        _ => relative
            .strip_prefix("concepts/")
            .and_then(|name| name.strip_suffix(".md"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .map(KnowledgePageId::Concept)
            .ok_or(WikiRepairError::UnsafeBundleLayout),
    }
}

fn restore_guided_or_combine(
    collection: &CollectionRecord,
    snapshot: &CreatedGuidedSnapshot,
    cause: WikiRepairError,
) -> WikiRepairError {
    match restore_guided_snapshot(collection, snapshot) {
        Ok(()) => cause,
        Err(rollback) => WikiRepairError::RollbackFailed {
            cause: cause.to_string(),
            rollback: rollback.to_string(),
        },
    }
}

fn retain_guided_snapshots(
    collection: &CollectionRecord,
    retention: usize,
) -> Result<(), WikiRepairError> {
    let root = guided_snapshots_root(&collection.wiki_folder);
    let mut verified = Vec::new();
    for entry in fs::read_dir(&root).map_err(|error| WikiRepairError::Snapshot(error.into()))? {
        let entry = entry.map_err(|error| WikiRepairError::Snapshot(error.into()))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| WikiRepairError::Snapshot(error.into()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Ok(plan_id) = Uuid::parse_str(stem) else {
            continue;
        };
        let Ok(manifest) = verify_guided_snapshot(&path) else {
            continue;
        };
        if manifest.plan_id == plan_id && manifest.collection_id == collection.id {
            verified.push((manifest.created_at, plan_id, path));
        }
    }
    verified.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    for (_, _, path) in verified.into_iter().skip(retention) {
        fs::remove_file(path).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum RepairExecutionStep {
    IndexWritten,
}

fn issue_risk(issue: &HealthIssue) -> RepairRisk {
    if issue.page == Some(KnowledgePageId::Index)
        && DERIVED_INDEX_CODES.contains(&issue.code.as_str())
    {
        RepairRisk::Derived
    } else if issue.page == Some(KnowledgePageId::Log)
        || issue.code.starts_with("log_")
        || issue.code.starts_with("historical_")
        || issue.code.starts_with("stale_log_")
        || issue.code == "missing_log"
    {
        RepairRisk::History
    } else {
        RepairRisk::Content
    }
}

fn blocking_health_codes(bundle: &KnowledgeBundleView) -> Vec<String> {
    bundle
        .health
        .issues
        .iter()
        .filter(|issue| issue.severity == HealthSeverity::Error)
        .filter(|issue| issue_risk(issue) != RepairRisk::Derived)
        .map(|issue| issue.code.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn post_validation_codes(bundle: &KnowledgeBundleView) -> Vec<String> {
    let mut codes = bundle
        .health
        .issues
        .iter()
        .filter(|issue| issue.severity == HealthSeverity::Error)
        .map(|issue| issue.code.clone())
        .collect::<BTreeSet<_>>();
    if bundle.state != KnowledgeBundleState::Ready {
        codes.insert("bundle_not_ready".to_owned());
    }
    codes.into_iter().collect()
}

fn guided_post_validation_codes(bundle: &KnowledgeBundleView) -> Vec<String> {
    let mut codes = bundle
        .health
        .issues
        .iter()
        .filter(|issue| issue.severity == HealthSeverity::Error)
        .map(|issue| issue.code.clone())
        .collect::<BTreeSet<_>>();
    if bundle.state == KnowledgeBundleState::Updating {
        codes.insert("bundle_updating".to_owned());
    }
    codes.into_iter().collect()
}

fn capture_managed_index(
    collection: &CollectionRecord,
    expected_fingerprint: Option<&str>,
) -> Result<Option<Vec<u8>>, WikiRepairError> {
    validate_bundle_root(&collection.wiki_folder)?;
    let index_path = collection.wiki_folder.join("index.md");
    let metadata = match fs::symlink_metadata(&index_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if expected_fingerprint.is_some() {
                return Err(WikiRepairError::StalePlan);
            }
            return Ok(None);
        }
        Err(error) => return Err(WikiRepairError::Snapshot(error.into())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WikiRepairError::UnsafeBundleLayout);
    }
    if metadata.len() > MAX_SNAPSHOT_INDEX_BYTES {
        return Err(WikiRepairError::SnapshotTooLarge {
            actual: metadata.len(),
            maximum: MAX_SNAPSHOT_INDEX_BYTES,
        });
    }
    let bytes = fs::read(&index_path).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    if expected_fingerprint.is_some_and(|expected| expected != sha256(&bytes)) {
        return Err(WikiRepairError::StalePlan);
    }
    Ok(Some(bytes))
}

fn validate_bundle_root(root: &Path) -> Result<(), WikiRepairError> {
    let metadata =
        fs::symlink_metadata(root).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WikiRepairError::UnsafeBundleLayout);
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotManifest {
    schema_version: u32,
    plan_id: Uuid,
    collection_id: Uuid,
    created_at: DateTime<Utc>,
    bundle_fingerprint: String,
    files: Vec<SnapshotFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotFile {
    relative_path: String,
    present: bool,
    byte_size: u64,
    sha256: Option<String>,
}

struct CreatedSnapshot {
    manifest_sha256: String,
}

fn create_snapshot(
    collection: &CollectionRecord,
    plan: &RepairPlan,
    original_index: Option<&[u8]>,
) -> Result<CreatedSnapshot, WikiRepairError> {
    let snapshots_root = snapshots_root(&collection.wiki_folder);
    ensure_managed_directory(&collection.wiki_folder.join(".airwiki"))?;
    ensure_managed_directory(&snapshots_root)?;
    let snapshot_dir = snapshots_root.join(plan.id.to_string());
    fs::create_dir(&snapshot_dir).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    let mut cleanup = SnapshotCreationGuard::new(&snapshot_dir);

    if let Some(bytes) = original_index {
        atomic_write(&snapshot_dir.join("index.md"), bytes).map_err(WikiRepairError::Snapshot)?;
    }
    let manifest = SnapshotManifest {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        plan_id: plan.id.as_uuid(),
        collection_id: plan.collection_id,
        created_at: Utc::now(),
        bundle_fingerprint: plan.expected_bundle_fingerprint.clone(),
        files: vec![SnapshotFile {
            relative_path: "index.md".to_owned(),
            present: original_index.is_some(),
            byte_size: original_index
                .map(|bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX))
                .unwrap_or(0),
            sha256: original_index.map(sha256),
        }],
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    let manifest_sha256 = sha256(&manifest_bytes);
    atomic_write(&snapshot_dir.join("manifest.json"), &manifest_bytes)
        .map_err(WikiRepairError::Snapshot)?;
    let verified = verify_snapshot(&snapshot_dir)?;
    if verified.plan_id != plan.id.as_uuid()
        || verified.collection_id != plan.collection_id
        || verified.bundle_fingerprint != plan.expected_bundle_fingerprint
    {
        return Err(WikiRepairError::Snapshot(anyhow::anyhow!(
            "repair snapshot identity does not match its plan"
        )));
    }
    cleanup.disarm();
    Ok(CreatedSnapshot { manifest_sha256 })
}

fn snapshots_root(wiki_root: &Path) -> PathBuf {
    wiki_root.join(".airwiki").join("repair-snapshots")
}

fn ensure_managed_directory(path: &Path) -> Result<(), WikiRepairError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(WikiRepairError::UnsafeBundleLayout)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| WikiRepairError::Snapshot(error.into()))
        }
        Err(error) => Err(WikiRepairError::Snapshot(error.into())),
    }
}

struct SnapshotCreationGuard<'a> {
    directory: &'a Path,
    armed: bool,
}

impl<'a> SnapshotCreationGuard<'a> {
    fn new(directory: &'a Path) -> Self {
        Self {
            directory,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SnapshotCreationGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(self.directory.join("manifest.json"));
            let _ = fs::remove_file(self.directory.join("index.md"));
            let _ = fs::remove_dir(self.directory);
        }
    }
}

fn retain_snapshots(
    collection: &CollectionRecord,
    retention: usize,
) -> Result<(), WikiRepairError> {
    let root = snapshots_root(&collection.wiki_folder);
    let mut managed = Vec::new();
    for entry in fs::read_dir(&root).map_err(|error| WikiRepairError::Snapshot(error.into()))? {
        let entry = entry.map_err(|error| WikiRepairError::Snapshot(error.into()))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| WikiRepairError::Snapshot(error.into()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        let Ok(plan_id) = Uuid::parse_str(&name) else {
            continue;
        };
        let Ok(manifest) = verify_snapshot(&entry.path()) else {
            continue;
        };
        if manifest.schema_version == SNAPSHOT_SCHEMA_VERSION
            && manifest.plan_id == plan_id
            && manifest.collection_id == collection.id
        {
            managed.push((manifest.created_at, plan_id, entry.path()));
        }
    }
    managed.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    for (_, _, directory) in managed.into_iter().skip(retention) {
        remove_managed_snapshot(&directory)?;
    }
    Ok(())
}

fn verify_snapshot(directory: &Path) -> Result<SnapshotManifest, WikiRepairError> {
    let directory_metadata =
        fs::symlink_metadata(directory).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err(WikiRepairError::UnsafeBundleLayout);
    }
    for entry in fs::read_dir(directory).map_err(|error| WikiRepairError::Snapshot(error.into()))? {
        let entry = entry.map_err(|error| WikiRepairError::Snapshot(error.into()))?;
        if !matches!(
            entry.file_name().to_str(),
            Some("index.md" | "manifest.json")
        ) {
            return Err(WikiRepairError::UnsafeBundleLayout);
        }
    }

    let manifest_path = directory.join("manifest.json");
    let manifest_metadata = fs::symlink_metadata(&manifest_path)
        .map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    if manifest_metadata.file_type().is_symlink()
        || !manifest_metadata.is_file()
        || manifest_metadata.len() > 64 * 1024
    {
        return Err(WikiRepairError::UnsafeBundleLayout);
    }
    let manifest_bytes =
        fs::read(&manifest_path).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    let manifest: SnapshotManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    if manifest.schema_version != SNAPSHOT_SCHEMA_VERSION
        || manifest.files.len() != 1
        || manifest.files[0].relative_path != "index.md"
    {
        return Err(WikiRepairError::Snapshot(anyhow::anyhow!(
            "repair snapshot manifest has an unsupported shape"
        )));
    }

    let index = &manifest.files[0];
    let index_path = directory.join("index.md");
    match fs::symlink_metadata(&index_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(WikiRepairError::UnsafeBundleLayout);
        }
        Ok(metadata) if !index.present || metadata.len() != index.byte_size => {
            return Err(WikiRepairError::Snapshot(anyhow::anyhow!(
                "repair snapshot index metadata does not match its manifest"
            )));
        }
        Ok(_) => {
            let bytes =
                fs::read(&index_path).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
            let actual_sha256 = sha256(&bytes);
            if index.sha256.as_deref() != Some(actual_sha256.as_str()) {
                return Err(WikiRepairError::Snapshot(anyhow::anyhow!(
                    "repair snapshot index hash does not match its manifest"
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !index.present => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(WikiRepairError::Snapshot(anyhow::anyhow!(
                "repair snapshot index is missing"
            )));
        }
        Err(error) => return Err(WikiRepairError::Snapshot(error.into())),
    }
    Ok(manifest)
}

fn remove_managed_snapshot(directory: &Path) -> Result<(), WikiRepairError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| WikiRepairError::Snapshot(error.into()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in &entries {
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| WikiRepairError::Snapshot(error.into()))?;
        let recognized = matches!(
            entry.file_name().to_str(),
            Some("index.md" | "manifest.json")
        );
        if metadata.file_type().is_symlink() || !metadata.is_file() || !recognized {
            return Ok(());
        }
    }
    for entry in entries {
        fs::remove_file(entry.path()).map_err(|error| WikiRepairError::Snapshot(error.into()))?;
    }
    fs::remove_dir(directory).map_err(|error| WikiRepairError::Snapshot(error.into()))
}

fn rollback_or_combine(
    index_path: &Path,
    original: Option<&[u8]>,
    rendered_sha256: &str,
    cause: WikiRepairError,
) -> WikiRepairError {
    match rollback_index(index_path, original, rendered_sha256) {
        Ok(()) => cause,
        Err(rollback) => WikiRepairError::RollbackFailed {
            cause: cause.to_string(),
            rollback: rollback.to_string(),
        },
    }
}

fn rollback_index(
    index_path: &Path,
    original: Option<&[u8]>,
    rendered_sha256: &str,
) -> Result<(), WikiRepairError> {
    let current =
        fs::read(index_path).map_err(|error| WikiRepairError::Regeneration(error.into()))?;
    if sha256(&current) != rendered_sha256 {
        return Err(WikiRepairError::StalePlan);
    }
    match original {
        Some(bytes) => atomic_write(index_path, bytes).map_err(WikiRepairError::Regeneration),
        None => {
            fs::remove_file(index_path).map_err(|error| WikiRepairError::Regeneration(error.into()))
        }
    }
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use airwiki_types::{CollectionPolicy, ConceptType, EnrichmentDraft};
    use tempfile::TempDir;

    use super::*;
    use crate::{EMBEDDING_DIMENSIONS, StoredChunk};

    struct Fixture {
        _temp: TempDir,
        database: Database,
        collection: CollectionRecord,
        concept_id: Uuid,
    }

    impl Fixture {
        fn published() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let source_root = temp.path().join("source");
            fs::create_dir_all(&source_root).unwrap();
            let source_path = source_root.join("guide.md");
            let source_text = "# Guide\n\nSynthetic evidence.";
            fs::write(&source_path, source_text).unwrap();
            let source_hash = sha256(source_text.as_bytes());
            let database = Database::in_memory().unwrap();
            let collection = database
                .create_collection(
                    "Synthetic",
                    &source_root,
                    temp.path().join("wiki"),
                    CollectionPolicy::local_only(),
                )
                .unwrap();
            let source_id = database
                .register_source(
                    collection.id,
                    &source_path,
                    &source_hash,
                    "markdown",
                    u64::try_from(source_text.len()).unwrap(),
                )
                .unwrap()
                .id();
            database
                .mark_extracted(source_id, 1, u64::try_from(source_text.len()).unwrap())
                .unwrap();
            let draft = EnrichmentDraft {
                concept_type: ConceptType::Document,
                title: "Synthetic guide".to_owned(),
                description: "A synthetic guide used by repair tests.".to_owned(),
                language: "en".to_owned(),
                tags: vec!["synthetic".to_owned()],
                entities: Vec::new(),
                links: Vec::new(),
                summary: "Synthetic evidence.".to_owned(),
                classification_confidence: 1.0,
                classification_explanation: "Fixture".to_owned(),
            };
            let concept = database
                .save_enrichment(source_id, draft.clone(), "test-peer", "fake-model")
                .unwrap();
            database
                .replace_chunks(
                    concept.id,
                    &[StoredChunk {
                        id: Uuid::new_v4(),
                        concept_id: concept.id,
                        source_document_id: source_id,
                        collection_id: collection.id,
                        ordinal: 0,
                        heading_or_page: "Guide".to_owned(),
                        text: source_text.to_owned(),
                        text_sha256: source_hash.clone(),
                        embedding: vec![0.0; EMBEDDING_DIMENSIONS],
                        source_revision: 1,
                    }],
                )
                .unwrap();
            let published = database.approve_concept(concept.id, draft).unwrap();
            let source = database.source_document(source_id).unwrap().unwrap();
            OkfPublisher::new(&collection.wiki_folder)
                .publish(
                    &published,
                    &source,
                    std::slice::from_ref(&published),
                    "published",
                )
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2_100));
            Self {
                _temp: temp,
                database,
                collection,
                concept_id: published.id,
            }
        }

        fn inspector(&self) -> OkfBundleInspector {
            OkfBundleInspector::new(self.database.clone())
        }

        fn planner(&self) -> RepairPlan {
            let view = self.inspector().inspect_bundle(self.collection.id).unwrap();
            WikiRepairPlanner::plan(&view).unwrap()
        }
    }

    #[test]
    fn planner_marks_missing_index_as_derived() {
        let fixture = Fixture::published();
        fs::remove_file(fixture.collection.wiki_folder.join("index.md")).unwrap();

        let plan = fixture.planner();

        assert_eq!(plan.maximum_risk(), Some(RepairRisk::Derived));
    }

    #[test]
    fn planner_requires_content_review_for_concept_drift() {
        let fixture = Fixture::published();
        let concept_path = fixture
            .collection
            .wiki_folder
            .join("concepts")
            .join(format!("{}.md", fixture.concept_id));
        let content = fs::read_to_string(&concept_path).unwrap();
        fs::write(
            &concept_path,
            content.replace(
                "A synthetic guide used by repair tests.",
                "A manually changed description.",
            ),
        )
        .unwrap();

        let plan = fixture.planner();

        assert_eq!(plan.maximum_risk(), Some(RepairRisk::Content));
    }

    #[test]
    fn guided_preview_is_read_only_and_explains_withdrawal() {
        let fixture = Fixture::published();
        let concept_path = fixture
            .collection
            .wiki_folder
            .join("concepts")
            .join(format!("{}.md", fixture.concept_id));
        let content = fs::read_to_string(&concept_path).unwrap();
        fs::write(
            &concept_path,
            content.replace("Synthetic guide", "Changed outside AirWiki"),
        )
        .unwrap();
        let before = fs::read(&concept_path).unwrap();
        let plan = fixture.planner();

        let preview = WikiRepairExecutor::new(fixture.database.clone())
            .prepare_guided(&plan)
            .unwrap();

        assert_eq!(
            preview.concepts_returned_to_review,
            vec![fixture.concept_id]
        );
        assert_eq!(fs::read(concept_path).unwrap(), before);
    }

    #[test]
    fn confirmed_guided_repair_withdraws_before_removing_inconsistent_page() {
        let fixture = Fixture::published();
        let concept_path = fixture
            .collection
            .wiki_folder
            .join("concepts")
            .join(format!("{}.md", fixture.concept_id));
        let content = fs::read_to_string(&concept_path).unwrap();
        fs::write(
            &concept_path,
            content.replace("Synthetic guide", "Changed outside AirWiki"),
        )
        .unwrap();
        let plan = fixture.planner();
        let executor = WikiRepairExecutor::new(fixture.database.clone());
        let preview = executor.prepare_guided(&plan).unwrap();

        let result = executor.execute_guided(&preview).unwrap();

        assert_eq!(
            fixture
                .database
                .concept(fixture.concept_id)
                .unwrap()
                .unwrap()
                .status,
            airwiki_types::DocumentStatus::NeedsReview
        );
        assert!(!concept_path.exists());
        assert_eq!(result.concepts_returned_to_review, vec![fixture.concept_id]);
    }

    #[test]
    fn guided_repair_removes_only_confirmed_orphan_and_retains_snapshot() {
        let fixture = Fixture::published();
        let orphan_id = Uuid::new_v4();
        let orphan_path = fixture
            .collection
            .wiki_folder
            .join("concepts")
            .join(format!("{orphan_id}.md"));
        fs::write(&orphan_path, "# unmanaged orphan\n").unwrap();
        let plan = fixture.planner();
        let executor = WikiRepairExecutor::new(fixture.database.clone());
        let preview = executor.prepare_guided(&plan).unwrap();

        let result = executor.execute_guided(&preview).unwrap();

        assert_eq!(result.orphan_concepts_removed, vec![orphan_id]);
        assert!(!orphan_path.exists());
        assert!(
            guided_snapshots_root(&fixture.collection.wiki_folder)
                .join(format!("{}.json", preview.plan_id))
                .is_file()
        );
    }

    #[test]
    fn guided_history_repair_remains_blocked_without_a_separate_authority() {
        let fixture = Fixture::published();
        fs::write(
            fixture.collection.wiki_folder.join("log.md"),
            "invalid history\n",
        )
        .unwrap();
        let plan = fixture.planner();

        let error = WikiRepairExecutor::new(fixture.database.clone())
            .prepare_guided(&plan)
            .unwrap_err();

        assert!(matches!(
            error,
            WikiRepairError::HistoryRepairRequiresHumanRecovery
        ));
    }

    #[test]
    fn guided_snapshot_restores_every_affected_file_after_interruption() {
        let fixture = Fixture::published();
        let concept_path = fixture
            .collection
            .wiki_folder
            .join("concepts")
            .join(format!("{}.md", fixture.concept_id));
        let content = fs::read_to_string(&concept_path).unwrap();
        fs::write(
            &concept_path,
            content.replace("Synthetic guide", "Changed outside AirWiki"),
        )
        .unwrap();
        let plan = fixture.planner();
        let preview = WikiRepairExecutor::new(fixture.database.clone())
            .prepare_guided(&plan)
            .unwrap();
        let pages = preview
            .files
            .iter()
            .map(|file| file.page)
            .collect::<BTreeSet<_>>();
        let snapshot = create_guided_snapshot(&fixture.collection, &preview, &pages).unwrap();
        let before = pages
            .iter()
            .map(|page| {
                (
                    *page,
                    fs::read(managed_page_path(&fixture.collection, *page)).ok(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for page in &pages {
            fs::write(
                managed_page_path(&fixture.collection, *page),
                b"interrupted",
            )
            .unwrap();
        }

        restore_guided_snapshot(&fixture.collection, &snapshot).unwrap();

        let after = pages
            .iter()
            .map(|page| {
                (
                    *page,
                    fs::read(managed_page_path(&fixture.collection, *page)).ok(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(after, before);
    }

    #[test]
    fn guided_snapshot_rejects_a_page_changed_after_preview() {
        let fixture = Fixture::published();
        let concept_path = fixture
            .collection
            .wiki_folder
            .join("concepts")
            .join(format!("{}.md", fixture.concept_id));
        let content = fs::read_to_string(&concept_path).unwrap();
        fs::write(
            &concept_path,
            content.replace("Synthetic guide", "First external change"),
        )
        .unwrap();
        let plan = fixture.planner();
        let preview = WikiRepairExecutor::new(fixture.database.clone())
            .prepare_guided(&plan)
            .unwrap();
        fs::write(&concept_path, "second external change").unwrap();
        let pages = preview
            .files
            .iter()
            .map(|file| file.page)
            .collect::<BTreeSet<_>>();

        let error = create_guided_snapshot(&fixture.collection, &preview, &pages).unwrap_err();

        assert!(matches!(error, WikiRepairError::StalePlan));
    }

    #[test]
    fn guided_restore_rejects_a_self_consistent_but_unsealed_manifest() {
        let fixture = Fixture::published();
        let concept_path = fixture
            .collection
            .wiki_folder
            .join("concepts")
            .join(format!("{}.md", fixture.concept_id));
        let content = fs::read_to_string(&concept_path).unwrap();
        fs::write(
            &concept_path,
            content.replace("Synthetic guide", "Changed outside AirWiki"),
        )
        .unwrap();
        let plan = fixture.planner();
        let preview = WikiRepairExecutor::new(fixture.database.clone())
            .prepare_guided(&plan)
            .unwrap();
        let pages = preview
            .files
            .iter()
            .map(|file| file.page)
            .collect::<BTreeSet<_>>();
        let snapshot = create_guided_snapshot(&fixture.collection, &preview, &pages).unwrap();
        let mut manifest = verify_guided_snapshot(&snapshot.path).unwrap();
        let file = manifest.files.first_mut().unwrap();
        file.present = true;
        file.byte_size = 8;
        file.sha256 = Some(sha256(b"tampered"));
        file.content_hex = Some(hex::encode(b"tampered"));
        fs::write(&snapshot.path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        let error = restore_guided_snapshot(&fixture.collection, &snapshot).unwrap_err();

        assert!(matches!(error, WikiRepairError::UnsafeBundleLayout));
    }

    #[test]
    fn guided_orphan_scan_ignores_uuid_named_non_markdown_files() {
        let fixture = Fixture::published();
        fs::write(
            fixture
                .collection
                .wiki_folder
                .join("concepts")
                .join(format!("{}.txt", Uuid::new_v4())),
            "not an OKF page",
        )
        .unwrap();

        assert!(
            guided_orphan_concepts(&fixture.collection, &fixture.database)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn guided_snapshot_rejects_traversal_before_restoring() {
        let fixture = Fixture::published();
        let path = guided_snapshots_root(&fixture.collection.wiki_folder).join("malicious.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let manifest = GuidedSnapshotManifest {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            plan_id: Uuid::new_v4(),
            collection_id: fixture.collection.id,
            created_at: Utc::now(),
            bundle_fingerprint: "fixture".to_owned(),
            files: vec![GuidedSnapshotFile {
                relative_path: "../source.md".to_owned(),
                present: true,
                byte_size: 1,
                sha256: Some(sha256(b"x")),
                content_hex: Some(hex::encode(b"x")),
            }],
        };
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        let error = verify_guided_snapshot(&path).unwrap_err();

        assert!(matches!(error, WikiRepairError::UnsafeBundleLayout));
    }

    #[test]
    fn automatic_execution_regenerates_index_and_creates_hashed_manifest() {
        let fixture = Fixture::published();
        fs::remove_file(fixture.collection.wiki_folder.join("index.md")).unwrap();
        let plan = fixture.planner();

        let result = WikiRepairExecutor::new(fixture.database.clone())
            .execute_automatic(&plan)
            .unwrap();

        let snapshot_dir =
            snapshots_root(&fixture.collection.wiki_folder).join(plan.id.to_string());
        let manifest = fs::read(snapshot_dir.join("manifest.json")).unwrap();
        assert_eq!(result.snapshot_manifest_sha256, sha256(&manifest));
    }

    #[test]
    fn automatic_execution_leaves_content_plan_untouched() {
        let fixture = Fixture::published();
        let concept_path = fixture
            .collection
            .wiki_folder
            .join("concepts")
            .join(format!("{}.md", fixture.concept_id));
        let content = fs::read_to_string(&concept_path).unwrap();
        fs::write(
            &concept_path,
            content.replace(
                "A synthetic guide used by repair tests.",
                "A manually changed description.",
            ),
        )
        .unwrap();
        let before = fs::read(&concept_path).unwrap();
        let plan = fixture.planner();

        let error = WikiRepairExecutor::new(fixture.database.clone())
            .execute_automatic(&plan)
            .unwrap_err();

        assert!(
            matches!(error, WikiRepairError::ConfirmationRequired { .. })
                && fs::read(concept_path).unwrap() == before
        );
    }

    #[test]
    fn automatic_execution_rejects_stale_plan_without_writing() {
        let fixture = Fixture::published();
        let index_path = fixture.collection.wiki_folder.join("index.md");
        fs::write(&index_path, "# Broken\n").unwrap();
        let plan = fixture.planner();
        fs::write(&index_path, "# Changed again\n").unwrap();
        let before = fs::read(&index_path).unwrap();

        let error = WikiRepairExecutor::new(fixture.database.clone())
            .execute_automatic(&plan)
            .unwrap_err();

        assert!(
            matches!(error, WikiRepairError::StalePlan) && fs::read(index_path).unwrap() == before
        );
    }

    #[test]
    fn automatic_execution_rolls_back_index_when_post_write_step_fails() {
        let fixture = Fixture::published();
        let index_path = fixture.collection.wiki_folder.join("index.md");
        fs::write(&index_path, "# Broken\n").unwrap();
        let original = fs::read(&index_path).unwrap();
        let plan = fixture.planner();
        let executor = WikiRepairExecutor::new(fixture.database.clone());

        let error = executor
            .execute_automatic_with_observer(&plan, |_| {
                Err(WikiRepairError::Regeneration(anyhow::anyhow!(
                    "test interruption"
                )))
            })
            .unwrap_err();

        assert!(
            matches!(error, WikiRepairError::Regeneration(_))
                && fs::read(index_path).unwrap() == original
        );
    }

    #[test]
    fn automatic_execution_retains_only_five_verified_snapshots() {
        let fixture = Fixture::published();
        let index_path = fixture.collection.wiki_folder.join("index.md");
        let executor = WikiRepairExecutor::new(fixture.database.clone());
        for iteration in 0..6 {
            fs::write(&index_path, format!("# Incomplete {iteration}\n")).unwrap();
            let plan = fixture.planner();
            executor.execute_automatic(&plan).unwrap();
        }

        let count = fs::read_dir(snapshots_root(&fixture.collection.wiki_folder))
            .unwrap()
            .count();

        assert_eq!(count, SNAPSHOT_RETENTION);
    }
}
