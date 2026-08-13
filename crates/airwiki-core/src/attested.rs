use std::path::{Component as PathComponent, Path};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use airwiki_types::AttestedComputationContract;
use anyhow::Context;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

wasmtime::component::bindgen!({
    path: "wit/airwiki-attested.wit",
    world: "executor-component",
});

mod attester_bindings {
    wasmtime::component::bindgen!({
        path: "wit/airwiki-attested.wit",
        world: "attester-component",
    });
}

const MAX_COMPONENT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const EXECUTION_FUEL: u64 = 10_000_000;

#[derive(Debug, Clone)]
pub struct AirWikiWasmRequest<'a> {
    pub bundle_root: &'a Path,
    pub concept_path: &'a str,
    pub contract: &'a AttestedComputationContract,
    pub parameters: &'a Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestedVerdict {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AirWikiWasmOutcome {
    pub receipt: Value,
    pub receipt_sha256: String,
    pub verdict: AttestedVerdict,
}

#[derive(Debug, Error)]
pub enum AttestedComputationError {
    #[error("the attested computation contract is not executable")]
    InvalidContract,
    #[error("attested computation parameters are invalid")]
    InvalidParameters,
    #[error("an attested component is unavailable or does not match its declaration")]
    InvalidComponent,
    #[error("the attested component requests capabilities AirWiki does not provide")]
    ImportsForbidden,
    #[error("the attested computation exceeded a resource limit")]
    ResourceLimit,
    #[error("the attested computation failed")]
    ExecutionFailed,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AirWikiWasmRuntime;

struct StoreState {
    limits: StoreLimits,
}

impl AirWikiWasmRuntime {
    pub fn execute(
        &self,
        request: AirWikiWasmRequest<'_>,
    ) -> Result<AirWikiWasmOutcome, AttestedComputationError> {
        validate_contract(request.contract)?;
        validate_parameters(request.contract, request.parameters)?;
        let parameters = serde_json::to_string(request.parameters)
            .map_err(|_| AttestedComputationError::InvalidParameters)?;
        if parameters.len() > MAX_INPUT_BYTES {
            return Err(AttestedComputationError::ResourceLimit);
        }
        let contract_json = serde_json::to_string(request.contract)
            .map_err(|_| AttestedComputationError::InvalidContract)?;
        if contract_json.len().saturating_add(parameters.len()) > MAX_INPUT_BYTES {
            return Err(AttestedComputationError::ResourceLimit);
        }

        let executor_path = checked_artifact_path(
            request.bundle_root,
            request.concept_path,
            &request.contract.executor.resource,
        )?;
        let attester_path = checked_artifact_path(
            request.bundle_root,
            request.concept_path,
            &request.contract.attester.resource,
        )?;
        let executor_bytes = read_component(
            &executor_path,
            request.bundle_root,
            &request.contract.executor.sha256,
        )?;
        let attester_bytes = read_component(
            &attester_path,
            request.bundle_root,
            &request.contract.attester.sha256,
        )?;
        let engine = runtime_engine()?;
        let executor = Component::new(&engine, &executor_bytes)
            .map_err(|_| AttestedComputationError::InvalidComponent)?;
        let attester = Component::new(&engine, &attester_bytes)
            .map_err(|_| AttestedComputationError::InvalidComponent)?;
        ensure_no_imports(&engine, &executor)?;
        ensure_no_imports(&engine, &attester)?;

        // Epoch deadlines apply to stores that already exist. Start the bounded
        // execution window only after both components have compiled and passed
        // the import check so the single epoch tick cannot be consumed early.
        let _deadline = ExecutionDeadline::start(&engine);

        let mut executor_store = limited_store(&engine)?;
        let executor_linker = Linker::new(&engine);
        let executor_instance =
            ExecutorComponent::instantiate(&mut executor_store, &executor, &executor_linker)
                .map_err(classify_wasmtime_error)?;
        let receipt_json = executor_instance
            .airwiki_attested_executor()
            .call_execute(&mut executor_store, &parameters)
            .map_err(classify_wasmtime_error)?;
        if receipt_json.len() > MAX_OUTPUT_BYTES {
            return Err(AttestedComputationError::ResourceLimit);
        }
        let receipt: Value = serde_json::from_str(&receipt_json)
            .map_err(|_| AttestedComputationError::ExecutionFailed)?;
        validate_receipt_fields(request.contract, &receipt)?;

        let mut attester_store = limited_store(&engine)?;
        let attester_linker = Linker::new(&engine);
        let attester_instance = attester_bindings::AttesterComponent::instantiate(
            &mut attester_store,
            &attester,
            &attester_linker,
        )
        .map_err(classify_wasmtime_error)?;
        let verdict = attester_instance
            .airwiki_attested_attester()
            .call_attest(
                &mut attester_store,
                &contract_json,
                &parameters,
                &receipt_json,
            )
            .map_err(classify_wasmtime_error)?;

        Ok(AirWikiWasmOutcome {
            receipt,
            receipt_sha256: hex::encode(Sha256::digest(receipt_json.as_bytes())),
            verdict: match verdict {
                attester_bindings::exports::airwiki::attested::attester::Verdict::Accepted => {
                    AttestedVerdict::Accepted
                }
                attester_bindings::exports::airwiki::attested::attester::Verdict::Rejected => {
                    AttestedVerdict::Rejected
                }
            },
        })
    }
}

struct ExecutionDeadline {
    stop: Option<SyncSender<()>>,
    task: Option<JoinHandle<()>>,
}

impl ExecutionDeadline {
    fn start(engine: &Engine) -> Self {
        let (stop, receiver) = sync_channel(1);
        let engine = engine.clone();
        let task = std::thread::spawn(move || {
            if receiver.recv_timeout(Duration::from_secs(2)).is_err() {
                engine.increment_epoch();
            }
        });
        Self {
            stop: Some(stop),
            task: Some(task),
        }
    }
}

impl Drop for ExecutionDeadline {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

fn runtime_engine() -> Result<Engine, AttestedComputationError> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    Engine::new(&config).map_err(|_| AttestedComputationError::ExecutionFailed)
}

fn limited_store(engine: &Engine) -> Result<Store<StoreState>, AttestedComputationError> {
    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_MEMORY_BYTES)
        .memories(1)
        .tables(1)
        .instances(4)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(engine, StoreState { limits });
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(EXECUTION_FUEL)
        .map_err(|_| AttestedComputationError::ExecutionFailed)?;
    store.set_epoch_deadline(1);
    Ok(store)
}

fn ensure_no_imports(
    engine: &Engine,
    component: &Component,
) -> Result<(), AttestedComputationError> {
    if component.component_type().imports(engine).len() == 0 {
        Ok(())
    } else {
        Err(AttestedComputationError::ImportsForbidden)
    }
}

fn read_component(
    path: &Path,
    root: &Path,
    expected: &str,
) -> Result<Vec<u8>, AttestedComputationError> {
    reject_symlink_ancestors(path, root)?;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| AttestedComputationError::InvalidComponent)?;
    if !metadata.is_file()
        || metadata_is_link_or_reparse(&metadata)
        || metadata.len() > MAX_COMPONENT_BYTES
    {
        return Err(AttestedComputationError::InvalidComponent);
    }
    let bytes = std::fs::read(path).map_err(|_| AttestedComputationError::InvalidComponent)?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual == expected {
        Ok(bytes)
    } else {
        Err(AttestedComputationError::InvalidComponent)
    }
}

fn reject_symlink_ancestors(path: &Path, root: &Path) -> Result<(), AttestedComputationError> {
    let mut current = path.parent();
    while let Some(directory) = current {
        let metadata = std::fs::symlink_metadata(directory)
            .map_err(|_| AttestedComputationError::InvalidComponent)?;
        if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
            return Err(AttestedComputationError::InvalidComponent);
        }
        if directory == root {
            return Ok(());
        }
        current = directory.parent();
    }
    Err(AttestedComputationError::InvalidComponent)
}

fn checked_artifact_path(
    root: &Path,
    concept_path: &str,
    resource: &str,
) -> Result<std::path::PathBuf, AttestedComputationError> {
    if resource.is_empty() || resource.contains(['\\', ':', '?', '#']) || resource.starts_with("//")
    {
        return Err(AttestedComputationError::InvalidContract);
    }
    let relative = resource.strip_prefix('/').unwrap_or(resource);
    let concept_file = root.join(concept_path);
    let base = if resource.starts_with('/') {
        root
    } else {
        concept_file
            .parent()
            .context("concept path has no parent")
            .map_err(|_| AttestedComputationError::InvalidContract)?
    };
    let mut path = base.to_path_buf();
    for part in Path::new(relative).components() {
        match part {
            PathComponent::Normal(part) => path.push(part),
            PathComponent::CurDir => {}
            PathComponent::ParentDir => {
                if !path.pop() || !path.starts_with(root) {
                    return Err(AttestedComputationError::InvalidContract);
                }
            }
            PathComponent::Prefix(_) | PathComponent::RootDir => {
                return Err(AttestedComputationError::InvalidContract);
            }
        }
    }
    if path.starts_with(root) {
        Ok(path)
    } else {
        Err(AttestedComputationError::InvalidContract)
    }
}

fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

fn validate_contract(
    contract: &AttestedComputationContract,
) -> Result<(), AttestedComputationError> {
    if contract.runtime != "airwiki-wasm"
        || contract.parameters.len() > 64
        || contract.executor.receipt.len() > 64
    {
        return Err(AttestedComputationError::InvalidContract);
    }
    Ok(())
}

fn validate_parameters(
    contract: &AttestedComputationContract,
    parameters: &Value,
) -> Result<(), AttestedComputationError> {
    let values = parameters
        .as_object()
        .ok_or(AttestedComputationError::InvalidParameters)?;
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
        return Err(AttestedComputationError::InvalidParameters);
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

fn validate_receipt_fields(
    contract: &AttestedComputationContract,
    receipt: &Value,
) -> Result<(), AttestedComputationError> {
    let values = receipt
        .as_object()
        .ok_or(AttestedComputationError::ExecutionFailed)?;
    if contract
        .executor
        .receipt
        .iter()
        .all(|field| values.contains_key(field))
    {
        Ok(())
    } else {
        Err(AttestedComputationError::ExecutionFailed)
    }
}

fn classify_wasmtime_error(error: wasmtime::Error) -> AttestedComputationError {
    if error
        .downcast_ref::<wasmtime::Trap>()
        .is_some_and(|trap| matches!(trap, wasmtime::Trap::OutOfFuel | wasmtime::Trap::Interrupt))
    {
        return AttestedComputationError::ResourceLimit;
    }
    let message = format!("{error:#}");
    if message.contains("fuel") || message.contains("memory") || message.contains("epoch") {
        AttestedComputationError::ResourceLimit
    } else {
        AttestedComputationError::ExecutionFailed
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use airwiki_types::{
        AttestedArtifact, AttestedComputationContract, AttestedExecutor, AttestedParameter,
    };
    use serde_json::json;
    use tempfile::TempDir;
    use wasm_encoder::{
        BlockType, CanonicalOption, CodeSection, ComponentBuilder, ComponentExportKind,
        ComponentExportSection, ComponentInstanceSection, ComponentSection, ComponentTypeRef,
        ComponentValType, ConstExpr, DataSection, ExportKind, ExportSection, Function,
        FunctionSection, Instruction, MemorySection, MemoryType, Module, PrimitiveValType,
        TypeSection, ValType,
    };

    use super::{
        AirWikiWasmOutcome, AirWikiWasmRequest, AirWikiWasmRuntime, AttestedComputationError,
        AttestedVerdict, runtime_engine,
    };

    #[derive(Debug, Clone, Copy)]
    enum ExecutorBehavior {
        ReturnReceipt,
        ReturnMalformedReceipt,
        ReturnIncompleteReceipt,
        ReturnOversizedReceipt,
        GrowMemoryBeyondLimit,
        ExhaustFuel,
    }

    #[derive(Debug, Clone, Copy)]
    enum AttesterBehavior {
        Accept,
        Reject,
    }

    #[test]
    fn executes_valid_components_and_returns_an_attested_receipt() -> anyhow::Result<()> {
        let fixture = WasmFixture::new(false, ExecutorBehavior::ReturnReceipt)?;
        let outcome = AirWikiWasmRuntime.execute(fixture.request())?;

        assert_eq!(
            outcome,
            AirWikiWasmOutcome {
                receipt: json!({ "value": 42 }),
                receipt_sha256: sha256(br#"{"value":42}"#),
                verdict: AttestedVerdict::Accepted,
            }
        );
        Ok(())
    }

    #[test]
    fn rejects_a_component_whose_hash_does_not_match() -> anyhow::Result<()> {
        let mut fixture = WasmFixture::new(false, ExecutorBehavior::ReturnReceipt)?;
        fixture.contract.executor.sha256 = "00".repeat(32);

        assert!(matches!(
            AirWikiWasmRuntime.execute(fixture.request()),
            Err(AttestedComputationError::InvalidComponent)
        ));
        Ok(())
    }

    #[test]
    fn rejects_components_with_host_imports() -> anyhow::Result<()> {
        let fixture = WasmFixture::new(true, ExecutorBehavior::ReturnReceipt)?;

        assert!(matches!(
            AirWikiWasmRuntime.execute(fixture.request()),
            Err(AttestedComputationError::ImportsForbidden)
        ));
        Ok(())
    }

    #[test]
    fn bounds_non_terminating_components_with_fuel() -> anyhow::Result<()> {
        let fixture = WasmFixture::new(false, ExecutorBehavior::ExhaustFuel)?;

        let result = AirWikiWasmRuntime.execute(fixture.request());
        assert!(
            matches!(result, Err(AttestedComputationError::ResourceLimit)),
            "unexpected result: {result:?}"
        );
        Ok(())
    }

    #[test]
    fn bounds_component_memory_growth() -> anyhow::Result<()> {
        let fixture = WasmFixture::new(false, ExecutorBehavior::GrowMemoryBeyondLimit)?;

        let result = AirWikiWasmRuntime.execute(fixture.request());
        assert!(
            matches!(result, Err(AttestedComputationError::ResourceLimit)),
            "unexpected result: {result:?}"
        );
        Ok(())
    }

    #[test]
    fn rejects_malformed_and_incomplete_receipts() -> anyhow::Result<()> {
        for behavior in [
            ExecutorBehavior::ReturnMalformedReceipt,
            ExecutorBehavior::ReturnIncompleteReceipt,
        ] {
            let fixture = WasmFixture::new(false, behavior)?;
            assert!(matches!(
                AirWikiWasmRuntime.execute(fixture.request()),
                Err(AttestedComputationError::ExecutionFailed)
            ));
        }
        Ok(())
    }

    #[test]
    fn bounds_receipt_and_parameter_payloads() -> anyhow::Result<()> {
        let output_fixture = WasmFixture::new(false, ExecutorBehavior::ReturnOversizedReceipt)?;
        assert!(matches!(
            AirWikiWasmRuntime.execute(output_fixture.request()),
            Err(AttestedComputationError::ResourceLimit)
        ));

        let mut input_fixture = WasmFixture::new(false, ExecutorBehavior::ReturnReceipt)?;
        input_fixture.parameters = json!({ "query": "x".repeat(super::MAX_INPUT_BYTES) });
        assert!(matches!(
            AirWikiWasmRuntime.execute(input_fixture.request()),
            Err(AttestedComputationError::ResourceLimit)
        ));
        Ok(())
    }

    #[test]
    fn preserves_a_negative_attester_verdict() -> anyhow::Result<()> {
        let fixture = WasmFixture::new_with_attester(
            false,
            ExecutorBehavior::ReturnReceipt,
            AttesterBehavior::Reject,
        )?;
        let outcome = AirWikiWasmRuntime.execute(fixture.request())?;

        assert_eq!(outcome.verdict, AttestedVerdict::Rejected);
        Ok(())
    }

    #[test]
    fn rejects_artifacts_outside_the_bundle() -> anyhow::Result<()> {
        let mut fixture = WasmFixture::new(false, ExecutorBehavior::ReturnReceipt)?;
        fixture.contract.executor.resource = "../../../executor.wasm".to_owned();

        assert!(matches!(
            AirWikiWasmRuntime.execute(fixture.request()),
            Err(AttestedComputationError::InvalidContract)
        ));
        Ok(())
    }

    #[test]
    fn rejects_components_over_the_declared_size_budget() -> anyhow::Result<()> {
        let mut fixture = WasmFixture::new(false, ExecutorBehavior::ReturnReceipt)?;
        let oversized = vec![0_u8; (super::MAX_COMPONENT_BYTES + 1) as usize];
        fs::write(
            fixture
                .bundle_root
                .join("computations/artifacts/executor.wasm"),
            &oversized,
        )?;
        fixture.contract.executor.sha256 = sha256(&oversized);

        assert!(matches!(
            AirWikiWasmRuntime.execute(fixture.request()),
            Err(AttestedComputationError::InvalidComponent)
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_component_symlinks() -> anyhow::Result<()> {
        use std::os::unix::fs::symlink;

        let fixture = WasmFixture::new(false, ExecutorBehavior::ReturnReceipt)?;
        let component = fixture
            .bundle_root
            .join("computations/artifacts/executor.wasm");
        let target = fixture.bundle_root.join("executor-target.wasm");
        fs::rename(&component, &target)?;
        symlink(&target, &component)?;

        assert!(matches!(
            AirWikiWasmRuntime.execute(fixture.request()),
            Err(AttestedComputationError::InvalidComponent)
        ));
        Ok(())
    }

    struct WasmFixture {
        _temp: TempDir,
        bundle_root: std::path::PathBuf,
        contract: AttestedComputationContract,
        parameters: serde_json::Value,
    }

    impl WasmFixture {
        fn new(imported_executor: bool, behavior: ExecutorBehavior) -> anyhow::Result<Self> {
            Self::new_with_attester(imported_executor, behavior, AttesterBehavior::Accept)
        }

        fn new_with_attester(
            imported_executor: bool,
            behavior: ExecutorBehavior,
            attester_behavior: AttesterBehavior,
        ) -> anyhow::Result<Self> {
            let temp = TempDir::new()?;
            let bundle_root = temp.path().join("bundle");
            let artifact_root = bundle_root.join("computations/artifacts");
            fs::create_dir_all(&artifact_root)?;
            let executor = if imported_executor {
                imported_executor_component()
            } else {
                executor_component(behavior)
            };
            let attester = attester_component(attester_behavior);
            let engine = runtime_engine().map_err(|error| anyhow::anyhow!("{error:?}"))?;
            wasmtime::component::Component::new(&engine, &executor)
                .map_err(|error| anyhow::anyhow!("invalid executor fixture: {error:#}"))?;
            wasmtime::component::Component::new(&engine, &attester)
                .map_err(|error| anyhow::anyhow!("invalid attester fixture: {error:#}"))?;
            fs::write(artifact_root.join("executor.wasm"), &executor)?;
            fs::write(artifact_root.join("attester.wasm"), &attester)?;
            fs::write(
                bundle_root.join("computations/example.md"),
                "---\ntype: Attested Computation\n---\n",
            )?;

            Ok(Self {
                _temp: temp,
                bundle_root,
                contract: AttestedComputationContract {
                    runtime: "airwiki-wasm".to_owned(),
                    parameters: vec![AttestedParameter {
                        name: "query".to_owned(),
                        parameter_type: "string".to_owned(),
                        required: true,
                    }],
                    computation: Some("synthetic fixture".to_owned()),
                    executor: AttestedExecutor {
                        resource: "artifacts/executor.wasm".to_owned(),
                        sha256: sha256(&executor),
                        receipt: vec!["value".to_owned()],
                    },
                    attester: AttestedArtifact {
                        resource: "artifacts/attester.wasm".to_owned(),
                        sha256: sha256(&attester),
                    },
                },
                parameters: json!({ "query": "safe" }),
            })
        }

        fn request(&self) -> AirWikiWasmRequest<'_> {
            AirWikiWasmRequest {
                bundle_root: &self.bundle_root,
                concept_path: "computations/example.md",
                contract: &self.contract,
                parameters: &self.parameters,
            }
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};

        hex::encode(Sha256::digest(bytes))
    }

    fn executor_component(behavior: ExecutorBehavior) -> Vec<u8> {
        let oversized_receipt;
        let receipt: &[u8] = match behavior {
            ExecutorBehavior::ReturnReceipt | ExecutorBehavior::ExhaustFuel => br#"{"value":42}"#,
            ExecutorBehavior::GrowMemoryBeyondLimit => br#"{"value":42}"#,
            ExecutorBehavior::ReturnMalformedReceipt => b"not-json",
            ExecutorBehavior::ReturnIncompleteReceipt => br#"{"other":42}"#,
            ExecutorBehavior::ReturnOversizedReceipt => {
                oversized_receipt = vec![b'x'; super::MAX_OUTPUT_BYTES + 1];
                &oversized_receipt
            }
        };
        let module = executor_module(receipt, behavior);
        let mut component = ComponentBuilder::default();
        let module_index = component.core_module(Some("executor"), &module);
        let instance = component.core_instantiate(Some("executor"), module_index, []);
        let memory =
            component.core_alias_export(Some("memory"), instance, "memory", ExportKind::Memory);
        let execute =
            component.core_alias_export(Some("execute"), instance, "execute", ExportKind::Func);
        let realloc = component.core_alias_export(
            Some("cabi_realloc"),
            instance,
            "cabi_realloc",
            ExportKind::Func,
        );
        let (function_type, mut encoder) = component.type_function(Some("execute-type"));
        encoder
            .params([("parameters-json", PrimitiveValType::String)])
            .result(Some(PrimitiveValType::String.into()));
        let lifted = component.lift_func(
            Some("execute"),
            execute,
            function_type,
            [
                CanonicalOption::UTF8,
                CanonicalOption::Memory(memory),
                CanonicalOption::Realloc(realloc),
            ],
        );
        export_interface_instance(
            component.finish(),
            "airwiki:attested/executor@1.0.0",
            "execute",
            lifted,
            None,
        )
    }

    fn imported_executor_component() -> Vec<u8> {
        let mut component = ComponentBuilder::default();
        let (function_type, mut encoder) = component.type_function(Some("execute-type"));
        encoder
            .params([("parameters-json", PrimitiveValType::String)])
            .result(Some(PrimitiveValType::String.into()));
        component.import("host-executor", ComponentTypeRef::Func(function_type));
        component.finish()
    }

    fn attester_component(behavior: AttesterBehavior) -> Vec<u8> {
        let module = attester_module(behavior);
        let mut component = ComponentBuilder::default();
        let module_index = component.core_module(Some("attester"), &module);
        let instance = component.core_instantiate(Some("attester"), module_index, []);
        let memory =
            component.core_alias_export(Some("memory"), instance, "memory", ExportKind::Memory);
        let attest =
            component.core_alias_export(Some("attest"), instance, "attest", ExportKind::Func);
        let realloc = component.core_alias_export(
            Some("cabi_realloc"),
            instance,
            "cabi_realloc",
            ExportKind::Func,
        );
        let (verdict_type, encoder) = component.type_defined(Some("verdict"));
        encoder.enum_type(["accepted", "rejected"]);
        let (function_type, mut encoder) = component.type_function(Some("attest-type"));
        encoder
            .params([
                ("contract-json", PrimitiveValType::String),
                ("parameters-json", PrimitiveValType::String),
                ("receipt-json", PrimitiveValType::String),
            ])
            .result(Some(ComponentValType::Type(verdict_type)));
        let lifted = component.lift_func(
            Some("attest"),
            attest,
            function_type,
            [
                CanonicalOption::UTF8,
                CanonicalOption::Memory(memory),
                CanonicalOption::Realloc(realloc),
            ],
        );
        export_interface_instance(
            component.finish(),
            "airwiki:attested/attester@1.0.0",
            "attest",
            lifted,
            Some(("verdict", verdict_type)),
        )
    }

    fn export_interface_instance(
        mut component: Vec<u8>,
        interface_name: &str,
        function_name: &str,
        function_index: u32,
        exported_type: Option<(&str, u32)>,
    ) -> Vec<u8> {
        let mut instances = ComponentInstanceSection::new();
        if let Some((type_name, type_index)) = exported_type {
            instances.export_items([
                (type_name, ComponentExportKind::Type, type_index),
                (function_name, ComponentExportKind::Func, function_index),
            ]);
        } else {
            instances.export_items([(function_name, ComponentExportKind::Func, function_index)]);
        }
        instances.append_to_component(&mut component);
        let mut exports = ComponentExportSection::new();
        exports.export(interface_name, ComponentExportKind::Instance, 0, None);
        exports.append_to_component(&mut component);
        component
    }

    fn executor_module(receipt: &[u8], behavior: ExecutorBehavior) -> Module {
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types
            .ty()
            .function([ValType::I32, ValType::I32], [ValType::I32]);
        types.ty().function(
            [ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            [ValType::I32],
        );
        module.section(&types);
        let mut functions = FunctionSection::new();
        functions.function(0).function(1);
        module.section(&functions);
        module.section(&single_memory());
        let mut exports = ExportSection::new();
        exports.export("execute", ExportKind::Func, 0);
        exports.export("cabi_realloc", ExportKind::Func, 1);
        exports.export("memory", ExportKind::Memory, 0);
        module.section(&exports);
        let mut code = CodeSection::new();
        let mut execute = Function::new([]);
        match behavior {
            ExecutorBehavior::ReturnReceipt
            | ExecutorBehavior::ReturnMalformedReceipt
            | ExecutorBehavior::ReturnIncompleteReceipt
            | ExecutorBehavior::ReturnOversizedReceipt => {
                execute.instruction(&Instruction::I32Const(0));
            }
            ExecutorBehavior::GrowMemoryBeyondLimit => {
                execute.instruction(&Instruction::I32Const(1_024));
                execute.instruction(&Instruction::MemoryGrow(0));
                execute.instruction(&Instruction::Drop);
                execute.instruction(&Instruction::I32Const(0));
            }
            ExecutorBehavior::ExhaustFuel => {
                execute.instruction(&Instruction::Loop(BlockType::Empty));
                execute.instruction(&Instruction::Br(0));
                execute.instruction(&Instruction::End);
                execute.instruction(&Instruction::I32Const(0));
            }
        }
        execute.instruction(&Instruction::End);
        code.function(&execute);
        code.function(&realloc_function());
        module.section(&code);
        let mut data = DataSection::new();
        let mut descriptor = Vec::with_capacity(8);
        descriptor.extend_from_slice(&16_u32.to_le_bytes());
        descriptor.extend_from_slice(&(receipt.len() as u32).to_le_bytes());
        data.active(0, &ConstExpr::i32_const(0), descriptor);
        data.active(0, &ConstExpr::i32_const(16), receipt.iter().copied());
        module.section(&data);
        module
    }

    fn attester_module(behavior: AttesterBehavior) -> Module {
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types.ty().function(
            [
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
            ],
            [ValType::I32],
        );
        types.ty().function(
            [ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            [ValType::I32],
        );
        module.section(&types);
        let mut functions = FunctionSection::new();
        functions.function(0).function(1);
        module.section(&functions);
        module.section(&single_memory());
        let mut exports = ExportSection::new();
        exports.export("attest", ExportKind::Func, 0);
        exports.export("cabi_realloc", ExportKind::Func, 1);
        exports.export("memory", ExportKind::Memory, 0);
        module.section(&exports);
        let mut code = CodeSection::new();
        let mut attest = Function::new([]);
        attest.instruction(&Instruction::I32Const(match behavior {
            AttesterBehavior::Accept => 0,
            AttesterBehavior::Reject => 1,
        }));
        attest.instruction(&Instruction::End);
        code.function(&attest);
        code.function(&realloc_function());
        module.section(&code);
        module
    }

    fn single_memory() -> MemorySection {
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: Some(1_024),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        memories
    }

    fn realloc_function() -> Function {
        let mut function = Function::new([]);
        function.instruction(&Instruction::I32Const(1_024));
        function.instruction(&Instruction::End);
        function
    }
}
