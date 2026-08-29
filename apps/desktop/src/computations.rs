use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use airwiki_core::{
    AirWikiWasmOutcome, AirWikiWasmRequest, AirWikiWasmRuntime, ComputationRunRecord,
    ComputationRunState, Database,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub(crate) struct PendingComputation {
    pub run_id: Uuid,
    pub wiki_id: Uuid,
    pub wiki_name: String,
    pub logical_path: String,
    pub application_name: String,
    pub parameters: Vec<ComputationParameterSummary>,
    pub expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub(crate) struct ComputationParameterSummary {
    pub name: String,
    pub parameter_type: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CompletedComputation {
    pub run_id: Uuid,
    pub wiki_name: String,
    pub logical_path: String,
    pub application_name: String,
    pub verdict: String,
    pub expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct PendingRun {
    app_id: Uuid,
    parameters: Value,
}

#[derive(Debug)]
struct EphemeralOutcome {
    outcome: AirWikiWasmOutcome,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub(crate) struct ComputationCoordinator {
    database: Database,
    pending: Arc<Mutex<HashMap<Uuid, PendingRun>>>,
    outcomes: Arc<Mutex<HashMap<Uuid, EphemeralOutcome>>>,
    updates: watch::Sender<u64>,
}

impl ComputationCoordinator {
    pub(crate) fn new(database: Database) -> Result<Self> {
        database.close_orphaned_computation_runs()?;
        let (updates, _) = watch::channel(0);
        Ok(Self {
            database,
            pending: Arc::new(Mutex::new(HashMap::new())),
            outcomes: Arc::new(Mutex::new(HashMap::new())),
            updates,
        })
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.updates.subscribe()
    }

    pub(crate) fn request(
        &self,
        app_id: Uuid,
        wiki_id: Uuid,
        logical_path: &str,
        parameters: Value,
    ) -> Result<PendingComputation> {
        let wiki = self
            .database
            .collection(wiki_id)?
            .context("computation wiki is unavailable")?;
        if !self.application_can_compute(app_id, &wiki)? {
            bail!("application is not authorized for this computation");
        }
        if !wiki.okf_compatibility.permits_external_disclosure() {
            bail!("computation wiki compatibility is restricted");
        }
        let concept = self
            .database
            .list_okf_concept_projection(wiki_id)?
            .into_iter()
            .find(|concept| concept.logical_path == logical_path)
            .context("attested computation is unavailable")?;
        if !concept_is_current(&concept) {
            bail!("attested computation is not current");
        }
        let contract = concept
            .attested_computation
            .context("attested computation is not executable")?;
        let capability = self
            .database
            .application_capability_by_app_id(app_id)?
            .context("application capability is unavailable")?;
        validate_parameters(&contract, &parameters)?;
        let now = Utc::now();
        self.prune_expired(now)?;
        let expires_at = now + Duration::minutes(10);
        let run_id = Uuid::new_v4();
        let parameter_schema = Value::Array(
            contract
                .parameters
                .iter()
                .map(|parameter| {
                    serde_json::json!({
                        "name": parameter.name,
                        "type": parameter.parameter_type,
                        "required": parameter.required,
                    })
                })
                .collect(),
        );
        let parameter_summaries = contract
            .parameters
            .iter()
            .filter(|parameter| {
                parameters
                    .as_object()
                    .is_some_and(|values| values.contains_key(&parameter.name))
            })
            .map(|parameter| ComputationParameterSummary {
                name: parameter.name.clone(),
                parameter_type: parameter.parameter_type.clone(),
            })
            .collect();
        let contract_fingerprint = hex::encode(Sha256::digest(
            serde_json::to_vec(&contract).context("could not fingerprint computation contract")?,
        ));
        self.database
            .create_computation_run(&ComputationRunRecord {
                id: run_id,
                collection_id: wiki_id,
                logical_path: logical_path.to_owned(),
                actor_kind: "application".to_owned(),
                actor_id: Some(app_id.to_string()),
                state: ComputationRunState::AwaitingConfirmation,
                contract_fingerprint,
                executor_sha256: contract.executor.sha256,
                attester_sha256: contract.attester.sha256,
                parameter_schema,
                receipt_sha256: None,
                verdict: None,
                requested_at: now,
                confirmed_at: None,
                completed_at: None,
                expires_at,
            })?;
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?
            .insert(run_id, PendingRun { app_id, parameters });
        let pending = PendingComputation {
            run_id,
            wiki_id,
            wiki_name: wiki.name,
            logical_path: logical_path.to_owned(),
            application_name: capability.display_name,
            parameters: parameter_summaries,
            expires_at,
        };
        self.notify();
        Ok(pending)
    }

    pub(crate) fn status_for_application(&self, app_id: Uuid, run_id: Uuid) -> Result<Value> {
        self.status_for_application_at(app_id, run_id, Utc::now())
    }

    fn status_for_application_at(
        &self,
        app_id: Uuid,
        run_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Value> {
        let mut run = self
            .database
            .computation_run(run_id)?
            .context("computation run is unavailable")?;
        let app_id = app_id.to_string();
        if run.actor_id.as_deref() != Some(app_id.as_str()) {
            bail!("application is not authorized for this computation run");
        }
        if run.state == ComputationRunState::AwaitingConfirmation
            && run.expires_at <= now
            && self
                .database
                .expire_computation_run_if_awaiting(run_id, now)?
        {
            self.pending
                .lock()
                .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?
                .remove(&run_id);
            run = self
                .database
                .computation_run(run_id)?
                .context("expired computation run disappeared")?;
            self.notify();
        }
        let receipt = self
            .outcomes
            .lock()
            .map_err(|_| anyhow::anyhow!("computation outcomes are unavailable"))?
            .get(&run.id)
            .filter(|outcome| outcome.expires_at > now)
            .map(|outcome| outcome.outcome.receipt.clone());
        Ok(sanitized_run(&run, receipt))
    }

    pub(crate) fn pending(&self) -> Result<Vec<PendingComputation>> {
        self.prune_expired(Utc::now())?;
        let pending = self
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?;
        pending
            .iter()
            .map(|(id, pending)| {
                let run = self
                    .database
                    .computation_run(*id)?
                    .context("pending computation run disappeared")?;
                let app = self
                    .database
                    .application_capability_any_by_app_id(pending.app_id)?
                    .context("pending computation application disappeared")?;
                Ok(PendingComputation {
                    run_id: *id,
                    wiki_id: run.collection_id,
                    wiki_name: self
                        .database
                        .collection(run.collection_id)?
                        .context("pending computation wiki disappeared")?
                        .name,
                    logical_path: run.logical_path,
                    application_name: app.display_name,
                    parameters: parameter_summaries(&run.parameter_schema),
                    expires_at: run.expires_at,
                })
            })
            .collect()
    }

    pub(crate) fn completed(&self) -> Result<Vec<CompletedComputation>> {
        let now = Utc::now();
        self.prune_expired(now)?;
        let outcomes = self
            .outcomes
            .lock()
            .map_err(|_| anyhow::anyhow!("computation outcomes are unavailable"))?;
        outcomes
            .iter()
            .map(|(id, outcome)| {
                let run = self
                    .database
                    .computation_run(*id)?
                    .context("completed computation run disappeared")?;
                let app_id = run
                    .actor_id
                    .as_deref()
                    .and_then(|value| Uuid::parse_str(value).ok())
                    .context("completed computation actor is invalid")?;
                let application = self
                    .database
                    .application_capability_any_by_app_id(app_id)?
                    .context("completed computation application disappeared")?;
                let wiki = self
                    .database
                    .collection(run.collection_id)?
                    .context("completed computation wiki disappeared")?;
                Ok(CompletedComputation {
                    run_id: *id,
                    wiki_name: wiki.name,
                    logical_path: run.logical_path,
                    application_name: application.display_name,
                    verdict: match outcome.outcome.verdict {
                        airwiki_core::AttestedVerdict::Accepted => "accepted",
                        airwiki_core::AttestedVerdict::Rejected => "rejected",
                    }
                    .to_owned(),
                    expires_at: outcome.expires_at,
                })
            })
            .collect()
    }

    pub(crate) fn with_claimed_accepted_receipt<T>(
        &self,
        run_id: Uuid,
        save: impl FnOnce(&Value) -> Result<T>,
    ) -> Result<T> {
        let now = Utc::now();
        let mut outcomes = self
            .outcomes
            .lock()
            .map_err(|_| anyhow::anyhow!("computation outcomes are unavailable"))?;
        let outcome = outcomes
            .get(&run_id)
            .filter(|outcome| outcome.expires_at > now)
            .context("computation result is unavailable")?;
        if outcome.outcome.verdict != airwiki_core::AttestedVerdict::Accepted {
            bail!("only an accepted computation result can be saved");
        }
        let claimed = outcomes
            .remove(&run_id)
            .context("computation result is unavailable")?;
        drop(outcomes);
        match save(&claimed.outcome.receipt) {
            Ok(saved) => {
                self.notify();
                Ok(saved)
            }
            Err(error) => {
                if claimed.expires_at > Utc::now() {
                    self.outcomes
                        .lock()
                        .map_err(|_| anyhow::anyhow!("computation outcomes are unavailable"))?
                        .insert(run_id, claimed);
                }
                self.notify();
                Err(error)
            }
        }
    }

    pub(crate) fn reject(&self, run_id: Uuid) -> Result<()> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?
            .contains_key(&run_id);
        if !pending {
            bail!("pending computation run is unavailable");
        }
        self.database.set_computation_run_state(
            run_id,
            ComputationRunState::Rejected,
            None,
            None,
        )?;
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?
            .remove(&run_id);
        self.notify();
        Ok(())
    }

    pub(crate) fn execute_confirmed(&self, run_id: Uuid) -> Result<AirWikiWasmOutcome> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?
            .get(&run_id)
            .cloned()
            .context("pending computation run is unavailable")?;
        let run = self
            .database
            .computation_run(run_id)?
            .context("computation run is unavailable")?;
        if run.expires_at <= Utc::now() {
            self.database.set_computation_run_state(
                run_id,
                ComputationRunState::Expired,
                None,
                None,
            )?;
            self.pending
                .lock()
                .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?
                .remove(&run_id);
            bail!("computation run expired");
        }
        if self
            .database
            .application_capability_by_app_id(pending.app_id)?
            .is_none()
        {
            return self.reject_before_execution(run_id, "application capability was revoked");
        }
        let wiki = self
            .database
            .collection(run.collection_id)?
            .context("computation wiki is unavailable")?;
        if !self.application_can_compute(pending.app_id, &wiki)? {
            return self.reject_before_execution(
                run_id,
                "application authorization changed before confirmation",
            );
        }
        if !wiki.okf_compatibility.permits_external_disclosure() {
            return self.reject_before_execution(
                run_id,
                "computation wiki compatibility changed before confirmation",
            );
        }
        if !self
            .database
            .pending_managed_bundle_mutations_for_collection(run.collection_id)?
            .is_empty()
        {
            return self.reject_before_execution(
                run_id,
                "computation wiki requires recovery before execution",
            );
        }
        let concept = self
            .database
            .list_okf_concept_projection(run.collection_id)?
            .into_iter()
            .find(|concept| concept.logical_path == run.logical_path)
            .context("attested computation is unavailable")?;
        if !concept_is_current(&concept) {
            return self.reject_before_execution(
                run_id,
                "attested computation changed state before confirmation",
            );
        }
        let current_contract_fingerprint = hex::encode(Sha256::digest(
            serde_json::to_vec(&concept.attested_computation)
                .context("could not fingerprint computation contract")?,
        ));
        if current_contract_fingerprint != run.contract_fingerprint {
            return self.reject_before_execution(
                run_id,
                "attested computation changed before confirmation",
            );
        }
        let contract = concept
            .attested_computation
            .context("attested computation is not executable")?;
        validate_parameters(&contract, &pending.parameters)?;
        // This conditional transition is the execution claim. Revalidation
        // deliberately leaves the queue entry in place, while SQLite permits
        // only one caller to move awaiting_confirmation -> running.
        self.database.set_computation_run_state(
            run_id,
            ComputationRunState::Running,
            None,
            None,
        )?;
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?
            .remove(&run_id);
        let result = AirWikiWasmRuntime.execute(AirWikiWasmRequest {
            bundle_root: &wiki.wiki_folder,
            concept_path: &run.logical_path,
            contract: &contract,
            parameters: &pending.parameters,
        });
        match result {
            Ok(outcome) => {
                let verdict = match outcome.verdict {
                    airwiki_core::AttestedVerdict::Accepted => "accepted",
                    airwiki_core::AttestedVerdict::Rejected => "rejected",
                };
                let result_expires_at = Utc::now() + Duration::minutes(10);
                self.database.complete_computation_run(
                    run_id,
                    &outcome.receipt_sha256,
                    verdict,
                    result_expires_at,
                )?;
                self.outcomes
                    .lock()
                    .map_err(|_| anyhow::anyhow!("computation outcomes are unavailable"))?
                    .insert(
                        run_id,
                        EphemeralOutcome {
                            outcome: outcome.clone(),
                            expires_at: result_expires_at,
                        },
                    );
                self.notify();
                Ok(outcome)
            }
            Err(error) => {
                self.database.set_computation_run_state(
                    run_id,
                    ComputationRunState::Failed,
                    None,
                    None,
                )?;
                self.notify();
                Err(error.into())
            }
        }
    }

    fn application_can_compute(
        &self,
        app_id: Uuid,
        wiki: &airwiki_core::CollectionRecord,
    ) -> Result<bool> {
        match wiki.origin {
            airwiki_core::WikiOrigin::AiMemory => Ok(self
                .database
                .application_wiki_role(app_id, wiki.id)?
                .is_some()),
            airwiki_core::WikiOrigin::Folder | airwiki_core::WikiOrigin::ImportedOkf => {
                Ok(wiki.policy.allow_external_ai)
            }
        }
    }

    fn prune_expired(&self, now: DateTime<Utc>) -> Result<()> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?;
        let mut removed = Vec::new();
        for id in pending.keys().copied().collect::<Vec<_>>() {
            match self.database.computation_run(id)? {
                None => removed.push(id),
                Some(run) if run.expires_at <= now => {
                    if run.state == ComputationRunState::AwaitingConfirmation {
                        self.database.expire_computation_run_if_awaiting(id, now)?;
                    }
                    removed.push(id);
                }
                Some(_) => {}
            }
        }
        for id in &removed {
            pending.remove(id);
        }
        drop(pending);
        let mut outcomes = self
            .outcomes
            .lock()
            .map_err(|_| anyhow::anyhow!("computation outcomes are unavailable"))?;
        let outcome_count = outcomes.len();
        outcomes.retain(|_, outcome| outcome.expires_at > now);
        let changed = !removed.is_empty() || outcomes.len() != outcome_count;
        drop(outcomes);
        if changed {
            self.notify();
        }
        Ok(())
    }

    fn reject_before_execution<T>(&self, run_id: Uuid, message: &str) -> Result<T> {
        self.database.set_computation_run_state(
            run_id,
            ComputationRunState::Rejected,
            None,
            None,
        )?;
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("computation queue is unavailable"))?
            .remove(&run_id);
        self.notify();
        bail!("{message}")
    }

    fn notify(&self) {
        self.updates.send_modify(|sequence| {
            *sequence = sequence.wrapping_add(1);
        });
    }
}

fn concept_is_current(concept: &airwiki_core::OkfConceptProjectionRecord) -> bool {
    concept.lifecycle_status == "stable"
        && !matches!(
            concept.assurance.freshness,
            airwiki_types::FreshnessState::Stale | airwiki_types::FreshnessState::Invalid
        )
}

fn parameter_summaries(schema: &Value) -> Vec<ComputationParameterSummary> {
    schema
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|parameter| {
            Some(ComputationParameterSummary {
                name: parameter.get("name")?.as_str()?.to_owned(),
                parameter_type: parameter.get("type")?.as_str()?.to_owned(),
            })
        })
        .collect()
}

fn validate_parameters(
    contract: &airwiki_types::AttestedComputationContract,
    parameters: &Value,
) -> Result<()> {
    let values = parameters
        .as_object()
        .context("computation parameters must be an object")?;
    if values.keys().any(|name| {
        !contract
            .parameters
            .iter()
            .any(|parameter| parameter.name == *name)
    }) || contract
        .parameters
        .iter()
        .any(|parameter| parameter.required && !values.contains_key(&parameter.name))
        || contract.parameters.iter().any(|parameter| {
            values
                .get(&parameter.name)
                .is_some_and(|value| !parameter_matches_type(value, &parameter.parameter_type))
        })
    {
        bail!("computation parameters do not match the declared contract");
    }
    Ok(())
}

fn parameter_matches_type(value: &Value, parameter_type: &str) -> bool {
    match parameter_type {
        "string" => value.is_string(),
        "boolean" | "bool" => value.is_boolean(),
        "integer" | "int" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" | "float" => value.is_number(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn sanitized_run(run: &ComputationRunRecord, receipt: Option<Value>) -> Value {
    let mut value = serde_json::json!({
        "runId": run.id,
        "wikiId": run.collection_id,
        "logicalPath": run.logical_path,
        "state": match run.state {
            ComputationRunState::AwaitingConfirmation => "awaiting_confirmation",
            ComputationRunState::Running => "running",
            ComputationRunState::Completed => "completed",
            ComputationRunState::Rejected => "rejected",
            ComputationRunState::Failed => "failed",
            ComputationRunState::Expired => "expired",
        },
        "verdict": run.verdict,
        "requestedAt": run.requested_at,
        "expiresAt": run.expires_at,
    });
    if let Some(receipt) = receipt
        && let Some(object) = value.as_object_mut()
    {
        object.insert("receipt".to_owned(), receipt);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use airwiki_core::{
        IndexingMode, InitialApplicationAccess, NewCollection, OkfImportValidator, WikiOrigin,
    };
    use airwiki_types::CollectionPolicy;

    fn coordinator_fixture() -> anyhow::Result<(
        tempfile::TempDir,
        Database,
        ComputationCoordinator,
        Uuid,
        Uuid,
    )> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        let wiki_root = temp.path().join("wiki");
        std::fs::create_dir_all(&source)?;
        std::fs::create_dir_all(wiki_root.join("computations"))?;
        std::fs::write(
            wiki_root.join("computations/contract.md"),
            r#"---
type: Attested Computation
runtime: airwiki-wasm
parameters:
  - { name: year, type: integer, required: true }
executor:
  resource: executor.wasm
  sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
attester:
  resource: attester.wasm
  sha256: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
---
"#,
        )?;
        let database = Database::in_memory()?;
        let wiki_id = Uuid::new_v4();
        database.create_collection_with_id_and_origin(NewCollection {
            id: wiki_id,
            name: "Computations".to_owned(),
            source_folder: source,
            wiki_folder: wiki_root.clone(),
            policy: CollectionPolicy {
                local_only: false,
                peer_shareable: false,
                allow_external_ai: true,
                internet_public: false,
            },
            origin: WikiOrigin::Folder,
            indexing_mode: IndexingMode::Manual,
            initial_application_access: InitialApplicationAccess::None,
        })?;
        let imported = OkfImportValidator::validate_directory(&wiki_root)?;
        database.replace_okf_concept_projection(wiki_id, &imported.concepts)?;
        database.update_collection_okf_metadata(
            wiki_id,
            Some("0.2"),
            &airwiki_types::OkfCompatibility::DeclaredV02,
            0,
        )?;
        let app_id = Uuid::new_v4();
        database.create_application_capability(
            app_id,
            "Generic MCP",
            "generic_mcp",
            "generic-mcp/1",
            "1234567890123456",
            &"c".repeat(64),
        )?;
        let coordinator = ComputationCoordinator::new(database.clone())?;
        Ok((temp, database, coordinator, app_id, wiki_id))
    }

    #[test]
    fn execution_rechecks_external_ai_policy_after_confirmation_request() {
        let (_temp, database, coordinator, app_id, wiki_id) = coordinator_fixture().unwrap();
        let pending = coordinator
            .request(
                app_id,
                wiki_id,
                "computations/contract.md",
                serde_json::json!({"year": 2026}),
            )
            .unwrap();
        database
            .update_collection_policy(wiki_id, CollectionPolicy::local_only())
            .unwrap();

        let error = coordinator.execute_confirmed(pending.run_id).unwrap_err();

        assert!(error.to_string().contains("authorization changed"));
        assert_eq!(
            database
                .computation_run(pending.run_id)
                .unwrap()
                .unwrap()
                .state,
            ComputationRunState::Rejected
        );
        assert!(coordinator.pending().unwrap().is_empty());
    }

    #[test]
    fn request_rejects_missing_or_mistyped_parameters() {
        let (_temp, _database, coordinator, app_id, wiki_id) = coordinator_fixture().unwrap();

        assert!(
            coordinator
                .request(
                    app_id,
                    wiki_id,
                    "computations/contract.md",
                    serde_json::json!({}),
                )
                .is_err()
        );
        assert!(
            coordinator
                .request(
                    app_id,
                    wiki_id,
                    "computations/contract.md",
                    serde_json::json!({"year": "2026"}),
                )
                .is_err()
        );
    }

    #[test]
    fn execution_rechecks_capability_revocation() -> anyhow::Result<()> {
        let (_temp, database, coordinator, app_id, wiki_id) = coordinator_fixture()?;
        let pending = coordinator.request(
            app_id,
            wiki_id,
            "computations/contract.md",
            serde_json::json!({"year": 2026}),
        )?;
        database.set_application_capability_revoked(app_id, true)?;

        let error = coordinator
            .execute_confirmed(pending.run_id)
            .expect_err("a revoked capability must fail closed");

        assert!(error.to_string().contains("revoked"));
        assert_eq!(
            database
                .computation_run(pending.run_id)?
                .context("computation run should remain auditable")?
                .state,
            ComputationRunState::Rejected
        );
        Ok(())
    }

    #[test]
    fn execution_rechecks_the_contract_fingerprint() -> anyhow::Result<()> {
        let (temp, database, coordinator, app_id, wiki_id) = coordinator_fixture()?;
        let pending = coordinator.request(
            app_id,
            wiki_id,
            "computations/contract.md",
            serde_json::json!({"year": 2026}),
        )?;
        let wiki_root = temp.path().join("wiki");
        std::fs::write(
            wiki_root.join("computations/contract.md"),
            r#"---
type: Attested Computation
runtime: airwiki-wasm
parameters:
  - { name: year, type: integer, required: true }
executor:
  resource: executor.wasm
  sha256: cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
attester:
  resource: attester.wasm
  sha256: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
---
"#,
        )?;
        let imported = OkfImportValidator::validate_directory(&wiki_root)?;
        database.replace_okf_concept_projection(wiki_id, &imported.concepts)?;

        let error = coordinator
            .execute_confirmed(pending.run_id)
            .expect_err("a changed contract must fail closed");

        assert!(error.to_string().contains("changed"));
        assert_eq!(
            database
                .computation_run(pending.run_id)?
                .context("computation run should remain auditable")?
                .state,
            ComputationRunState::Rejected
        );
        Ok(())
    }

    #[test]
    fn revalidation_failure_keeps_the_pending_run_recoverable() -> anyhow::Result<()> {
        let (_temp, database, coordinator, app_id, wiki_id) = coordinator_fixture()?;
        let pending = coordinator.request(
            app_id,
            wiki_id,
            "computations/contract.md",
            serde_json::json!({"year": 2026}),
        )?;
        database.replace_okf_concept_projection(wiki_id, &[])?;

        assert!(coordinator.execute_confirmed(pending.run_id).is_err());
        assert_eq!(
            database
                .computation_run(pending.run_id)?
                .context("computation run should remain auditable")?
                .state,
            ComputationRunState::AwaitingConfirmation
        );
        assert_eq!(coordinator.pending()?.len(), 1);

        coordinator.reject(pending.run_id)?;
        assert_eq!(
            database
                .computation_run(pending.run_id)?
                .context("rejected computation run should remain auditable")?
                .state,
            ComputationRunState::Rejected
        );
        Ok(())
    }

    #[test]
    fn computation_status_is_isolated_by_application() -> anyhow::Result<()> {
        let (_temp, database, coordinator, app_id, wiki_id) = coordinator_fixture()?;
        let pending = coordinator.request(
            app_id,
            wiki_id,
            "computations/contract.md",
            serde_json::json!({"year": 2026}),
        )?;
        let other_app_id = Uuid::new_v4();
        database.create_application_capability(
            other_app_id,
            "Other MCP",
            "generic_mcp",
            "other-mcp/1",
            "6543210987654321",
            &"d".repeat(64),
        )?;

        assert!(
            coordinator
                .status_for_application(other_app_id, pending.run_id)
                .is_err()
        );
        assert!(
            coordinator
                .status_for_application(app_id, pending.run_id)
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn computation_requests_are_bounded_per_application() -> anyhow::Result<()> {
        let (_temp, _database, coordinator, app_id, wiki_id) = coordinator_fixture()?;
        for _ in 0..airwiki_types::MAX_PENDING_COMPUTATIONS_PER_APPLICATION {
            coordinator.request(
                app_id,
                wiki_id,
                "computations/contract.md",
                serde_json::json!({"year": 2026}),
            )?;
        }

        assert!(
            coordinator
                .request(
                    app_id,
                    wiki_id,
                    "computations/contract.md",
                    serde_json::json!({"year": 2026}),
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn computation_request_rate_limit_counts_terminal_runs() -> anyhow::Result<()> {
        let (_temp, _database, coordinator, app_id, wiki_id) = coordinator_fixture()?;
        for _ in 0..airwiki_types::MAX_COMPUTATION_REQUESTS_PER_MINUTE {
            let pending = coordinator.request(
                app_id,
                wiki_id,
                "computations/contract.md",
                serde_json::json!({"year": 2026}),
            )?;
            coordinator.reject(pending.run_id)?;
        }

        assert!(
            coordinator
                .request(
                    app_id,
                    wiki_id,
                    "computations/contract.md",
                    serde_json::json!({"year": 2026}),
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn status_query_transitions_an_expired_request() -> anyhow::Result<()> {
        let (_temp, database, coordinator, app_id, wiki_id) = coordinator_fixture()?;
        let pending = coordinator.request(
            app_id,
            wiki_id,
            "computations/contract.md",
            serde_json::json!({"year": 2026}),
        )?;

        let status =
            coordinator.status_for_application_at(app_id, pending.run_id, pending.expires_at)?;

        assert_eq!(status.get("state").and_then(Value::as_str), Some("expired"));
        assert_eq!(
            database
                .computation_run(pending.run_id)?
                .context("expired run should remain auditable")?
                .state,
            ComputationRunState::Expired
        );
        assert!(coordinator.pending()?.is_empty());
        Ok(())
    }

    #[test]
    fn accepted_receipt_claim_is_retry_safe() -> anyhow::Result<()> {
        let (_temp, _database, coordinator, _app_id, _wiki_id) = coordinator_fixture()?;
        let run_id = Uuid::new_v4();
        coordinator
            .outcomes
            .lock()
            .map_err(|_| anyhow::anyhow!("test outcome lock is unavailable"))?
            .insert(
                run_id,
                EphemeralOutcome {
                    outcome: AirWikiWasmOutcome {
                        receipt: serde_json::json!({"value": 42}),
                        receipt_sha256: "a".repeat(64),
                        verdict: airwiki_core::AttestedVerdict::Accepted,
                    },
                    expires_at: Utc::now() + Duration::minutes(10),
                },
            );

        let failed: Result<()> =
            coordinator.with_claimed_accepted_receipt(run_id, |_| bail!("synthetic save failure"));
        assert!(failed.is_err());
        assert!(
            coordinator
                .with_claimed_accepted_receipt(run_id, |_| Ok(()))
                .is_ok()
        );
        assert!(
            coordinator
                .with_claimed_accepted_receipt(run_id, |_| Ok(()))
                .is_err()
        );
        Ok(())
    }
}
