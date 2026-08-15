#[cfg(target_os = "windows")]
use std::path::Component;
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::paths::AppPaths;

const WORKFLOW_GUIDE_VERSION: &str = "1";
const WORKFLOW_RECEIPT_SCHEMA: u16 = 2;
const IMPORT_LINE: &str = "@AirWiki.md";
const MAX_SKILL_FILE_BYTES: usize = 64 * 1024;
const MAX_INSTRUCTION_BYTES: usize = 256 * 1024;
const MAX_RECEIPT_BYTES: usize = 16 * 1024;
#[cfg(target_os = "windows")]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowGuideKind {
    NativeSkill,
    McpInstructions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowGuideStatus {
    Available,
    Installed,
    UpdateAvailable,
    BuiltIn,
    Conflict,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkflowGuideView {
    pub kind: WorkflowGuideKind,
    pub status: WorkflowGuideStatus,
    pub version: Option<String>,
    pub restart_required: bool,
}

impl WorkflowGuideView {
    pub(crate) fn built_in() -> Self {
        Self {
            kind: WorkflowGuideKind::McpInstructions,
            status: WorkflowGuideStatus::BuiltIn,
            version: Some(WORKFLOW_GUIDE_VERSION.to_owned()),
            restart_required: true,
        }
    }

    pub(crate) fn unsupported() -> Self {
        unsupported_view()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowClient {
    Codex,
    ClaudeCode,
    GeminiCli,
}

impl WorkflowClient {
    const fn receipt_name(self) -> &'static str {
        match self {
            Self::Codex => "chatgpt-desktop.json",
            Self::ClaudeCode => "claude-code.json",
            Self::GeminiCli => "gemini-cli.json",
        }
    }

    const fn receipt_client(self) -> &'static str {
        match self {
            Self::Codex => "chatgpt-desktop",
            Self::ClaudeCode => "claude-code",
            Self::GeminiCli => "gemini-cli",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkflowGuideManager {
    paths: AppPaths,
    home: PathBuf,
    current_exe: PathBuf,
    is_macos: bool,
    discover_host_configuration: bool,
}

#[derive(Debug)]
pub(crate) struct WorkflowChange {
    client: WorkflowClient,
    before: WorkflowSnapshot,
    after: WorkflowSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowSnapshot {
    skill: Option<SkillSnapshot>,
    awareness: Option<Vec<u8>>,
    instructions: Option<Vec<u8>>,
    receipt: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillSnapshot {
    skill_md: Vec<u8>,
    openai_yaml: Vec<u8>,
}

#[derive(Debug)]
struct WorkflowPaths {
    skill_dir: PathBuf,
    awareness: PathBuf,
    instructions: PathBuf,
    receipt: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowReceipt {
    schema_version: u16,
    client: String,
    version: String,
    skill_files: BTreeMap<String, String>,
    awareness_sha256: String,
    instructions_existed: bool,
    instructions_ended_with_newline: bool,
    instructions_after_sha256: String,
}

impl WorkflowGuideManager {
    pub(crate) fn new(
        paths: AppPaths,
        home: PathBuf,
        current_exe: PathBuf,
        is_macos: bool,
        discover_host_configuration: bool,
    ) -> Self {
        Self {
            paths,
            home,
            current_exe,
            is_macos,
            discover_host_configuration,
        }
    }

    pub(crate) async fn inspect(&self, client: WorkflowClient) -> WorkflowGuideView {
        let manager = self.clone();
        match tokio::task::spawn_blocking(move || manager.inspect_sync(client)).await {
            Ok(Ok(view)) => view,
            Ok(Err(_)) | Err(_) => WorkflowGuideView {
                kind: WorkflowGuideKind::NativeSkill,
                status: WorkflowGuideStatus::Conflict,
                version: None,
                restart_required: true,
            },
        }
    }

    pub(crate) async fn install(&self, client: WorkflowClient) -> Result<Option<WorkflowChange>> {
        let manager = self.clone();
        tokio::task::spawn_blocking(move || manager.install_sync(client))
            .await
            .context("la tarea de instalación de memoria asistida no terminó")?
    }

    pub(crate) async fn remove(&self, client: WorkflowClient) -> Result<Option<WorkflowChange>> {
        let manager = self.clone();
        tokio::task::spawn_blocking(move || manager.remove_sync(client))
            .await
            .context("la tarea de retiro de memoria asistida no terminó")?
    }

    pub(crate) async fn rollback(&self, change: WorkflowChange) -> Result<()> {
        let manager = self.clone();
        tokio::task::spawn_blocking(move || manager.rollback_sync(change))
            .await
            .context("la recuperación de memoria asistida no terminó")?
    }

    fn inspect_sync(&self, client: WorkflowClient) -> Result<WorkflowGuideView> {
        let Some(bundled) = self.bundled_skill()? else {
            return Ok(unsupported_view());
        };
        let paths = match self.client_paths(client) {
            Ok(paths) => paths,
            Err(_) => return Ok(unsupported_view()),
        };
        let snapshot = match read_snapshot(&paths) {
            Ok(snapshot) => snapshot,
            Err(_) => return Ok(conflict_view()),
        };
        Ok(view_for_snapshot(client, &snapshot, &bundled))
    }

    fn install_sync(&self, client: WorkflowClient) -> Result<Option<WorkflowChange>> {
        let bundled = self
            .bundled_skill()?
            .context("el paquete no contiene la guía de memoria asistida")?;
        let paths = self.client_paths(client)?;
        let before = read_snapshot(&paths)?;
        match view_for_snapshot(client, &before, &bundled).status {
            WorkflowGuideStatus::Installed => return Ok(None),
            WorkflowGuideStatus::Available | WorkflowGuideStatus::UpdateAvailable => {}
            WorkflowGuideStatus::Conflict => {
                bail!("la guía instalada fue modificada y no se reemplazará")
            }
            WorkflowGuideStatus::Unsupported | WorkflowGuideStatus::BuiltIn => {
                bail!("este cliente no admite una skill administrada")
            }
        }

        let awareness = bundled.awareness.clone();
        let instructions = append_import(before.instructions.as_deref())?;
        let receipt = encode_receipt(client, &bundled.skill, &awareness, &before, &instructions)?;
        let after = WorkflowSnapshot {
            skill: Some(bundled.skill),
            awareness: Some(awareness),
            instructions: Some(instructions),
            receipt: Some(receipt),
        };
        apply_snapshot(&paths, &before, &after)?;
        Ok(Some(WorkflowChange {
            client,
            before,
            after,
        }))
    }

    fn remove_sync(&self, client: WorkflowClient) -> Result<Option<WorkflowChange>> {
        let bundled = self
            .bundled_skill()?
            .context("el paquete no contiene la guía de memoria asistida")?;
        let paths = self.client_paths(client)?;
        let before = read_snapshot(&paths)?;
        match view_for_snapshot(client, &before, &bundled).status {
            WorkflowGuideStatus::Available => return Ok(None),
            WorkflowGuideStatus::Installed | WorkflowGuideStatus::UpdateAvailable => {}
            WorkflowGuideStatus::Conflict => {
                bail!("la guía fue modificada; AirWiki no retirará archivos del usuario")
            }
            WorkflowGuideStatus::Unsupported | WorkflowGuideStatus::BuiltIn => {
                return Ok(None);
            }
        }
        let receipt = parse_receipt(
            before
                .receipt
                .as_deref()
                .context("no existe el recibo de la guía")?,
        )?;
        let after = WorkflowSnapshot {
            skill: None,
            awareness: None,
            instructions: restore_instructions(
                before
                    .instructions
                    .as_deref()
                    .context("no existe el archivo global de instrucciones")?,
                &receipt,
            )?,
            receipt: None,
        };
        apply_snapshot(&paths, &before, &after)?;
        Ok(Some(WorkflowChange {
            client,
            before,
            after,
        }))
    }

    fn rollback_sync(&self, change: WorkflowChange) -> Result<()> {
        let paths = self.client_paths(change.client)?;
        let current = read_snapshot(&paths)?;
        ensure!(
            current == change.after,
            "la guía cambió durante la recuperación y no se sobrescribirá"
        );
        apply_snapshot(&paths, &change.after, &change.before)
    }

    fn client_paths(&self, client: WorkflowClient) -> Result<WorkflowPaths> {
        ensure!(
            self.home.is_absolute(),
            "el directorio personal no es absoluto"
        );
        let (skill_dir, root, instructions_name) = match client {
            WorkflowClient::Codex => {
                let root = self.configuration_root("CODEX_HOME", self.home.join(".codex"))?;
                (self.home.join(".agents/skills/airwiki"), root, "AGENTS.md")
            }
            WorkflowClient::ClaudeCode => {
                let root =
                    self.configuration_root("CLAUDE_CONFIG_DIR", self.home.join(".claude"))?;
                (root.join("skills/airwiki"), root, "CLAUDE.md")
            }
            WorkflowClient::GeminiCli => {
                let default_home = self.home.clone();
                let gemini_home = self.configuration_root("GEMINI_CLI_HOME", default_home)?;
                let root = gemini_home.join(".gemini");
                (root.join("skills/airwiki"), root, "GEMINI.md")
            }
        };
        ensure!(
            root.is_absolute(),
            "la raíz de configuración no es absoluta"
        );
        Ok(WorkflowPaths {
            skill_dir,
            awareness: root.join("AirWiki.md"),
            instructions: root.join(instructions_name),
            receipt: self
                .paths
                .data
                .join("integrations/workflow-receipts")
                .join(client.receipt_name()),
        })
    }

    fn configuration_root(&self, variable: &str, fallback: PathBuf) -> Result<PathBuf> {
        if !self.discover_host_configuration {
            return Ok(fallback);
        }
        let Some(value) = std::env::var_os(variable) else {
            return Ok(fallback);
        };
        let path = PathBuf::from(value);
        ensure!(
            path.is_absolute(),
            "la raíz personalizada de configuración no es absoluta"
        );
        Ok(path)
    }

    fn bundled_skill(&self) -> Result<Option<BundledSkill>> {
        let Some(executable_dir) = self.current_exe.parent() else {
            return Ok(None);
        };
        let mut candidates = vec![executable_dir.join("integrations/workflow")];
        if self.is_macos {
            candidates.insert(0, executable_dir.join("../Resources/integrations/workflow"));
        }
        #[cfg(debug_assertions)]
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("resources/integrations/workflow"),
        );
        for root in candidates {
            if !root.exists() {
                continue;
            }
            ensure_path_has_no_links(&root)?;
            let skill_md =
                read_regular_bounded(&root.join("airwiki/SKILL.md"), MAX_SKILL_FILE_BYTES)?;
            let openai_yaml = read_regular_bounded(
                &root.join("airwiki/agents/openai.yaml"),
                MAX_SKILL_FILE_BYTES,
            )?;
            let awareness = read_regular_bounded(&root.join("AirWiki.md"), MAX_INSTRUCTION_BYTES)?;
            ensure_utf8(&skill_md)?;
            ensure_utf8(&openai_yaml)?;
            ensure_utf8(&awareness)?;
            return Ok(Some(BundledSkill {
                skill: SkillSnapshot {
                    skill_md,
                    openai_yaml,
                },
                awareness,
            }));
        }
        Ok(None)
    }
}

#[derive(Debug)]
struct BundledSkill {
    skill: SkillSnapshot,
    awareness: Vec<u8>,
}

fn unsupported_view() -> WorkflowGuideView {
    WorkflowGuideView {
        kind: WorkflowGuideKind::NativeSkill,
        status: WorkflowGuideStatus::Unsupported,
        version: None,
        restart_required: true,
    }
}

fn conflict_view() -> WorkflowGuideView {
    WorkflowGuideView {
        kind: WorkflowGuideKind::NativeSkill,
        status: WorkflowGuideStatus::Conflict,
        version: None,
        restart_required: true,
    }
}

fn view_for_snapshot(
    client: WorkflowClient,
    snapshot: &WorkflowSnapshot,
    bundled: &BundledSkill,
) -> WorkflowGuideView {
    let available = snapshot.skill.is_none()
        && snapshot.awareness.is_none()
        && snapshot.receipt.is_none()
        && snapshot
            .instructions
            .as_deref()
            .is_none_or(|bytes| import_count(bytes).unwrap_or(usize::MAX) == 0);
    if available {
        return WorkflowGuideView {
            kind: WorkflowGuideKind::NativeSkill,
            status: WorkflowGuideStatus::Available,
            version: Some(WORKFLOW_GUIDE_VERSION.to_owned()),
            restart_required: true,
        };
    }
    let Some(receipt_bytes) = snapshot.receipt.as_deref() else {
        return conflict_view();
    };
    let Ok(receipt) = parse_receipt(receipt_bytes) else {
        return conflict_view();
    };
    if receipt.client != client.receipt_client()
        || !snapshot_matches_receipt(snapshot, &receipt)
        || snapshot
            .instructions
            .as_deref()
            .and_then(|bytes| import_count(bytes).ok())
            != Some(1)
    {
        return conflict_view();
    }
    let bundled_hashes = skill_hashes(&bundled.skill);
    let current = receipt.version == WORKFLOW_GUIDE_VERSION
        && receipt.skill_files == bundled_hashes
        && receipt.awareness_sha256 == digest_hex(&bundled.awareness);
    WorkflowGuideView {
        kind: WorkflowGuideKind::NativeSkill,
        status: if current {
            WorkflowGuideStatus::Installed
        } else {
            WorkflowGuideStatus::UpdateAvailable
        },
        version: Some(receipt.version),
        restart_required: true,
    }
}

fn snapshot_matches_receipt(snapshot: &WorkflowSnapshot, receipt: &WorkflowReceipt) -> bool {
    snapshot.skill.as_ref().is_some_and(|skill| {
        skill_hashes(skill) == receipt.skill_files
            && snapshot
                .awareness
                .as_deref()
                .is_some_and(|bytes| digest_hex(bytes) == receipt.awareness_sha256)
    })
}

fn encode_receipt(
    client: WorkflowClient,
    skill: &SkillSnapshot,
    awareness: &[u8],
    before: &WorkflowSnapshot,
    instructions_after: &[u8],
) -> Result<Vec<u8>> {
    let prior_origin = before
        .receipt
        .as_deref()
        .and_then(|bytes| parse_receipt(bytes).ok())
        .map(|receipt| {
            (
                receipt.instructions_existed,
                receipt.instructions_ended_with_newline,
            )
        });
    let (instructions_existed, instructions_ended_with_newline) =
        prior_origin.unwrap_or_else(|| {
            let instructions = before.instructions.as_deref();
            (
                instructions.is_some(),
                instructions.is_some_and(|bytes| bytes.ends_with(b"\n")),
            )
        });
    let receipt = WorkflowReceipt {
        schema_version: WORKFLOW_RECEIPT_SCHEMA,
        client: client.receipt_client().to_owned(),
        version: WORKFLOW_GUIDE_VERSION.to_owned(),
        skill_files: skill_hashes(skill),
        awareness_sha256: digest_hex(awareness),
        instructions_existed,
        instructions_ended_with_newline,
        instructions_after_sha256: digest_hex(instructions_after),
    };
    let bytes = serde_json::to_vec_pretty(&receipt).context("no se pudo serializar el recibo")?;
    ensure!(
        bytes.len() <= MAX_RECEIPT_BYTES,
        "el recibo excede el límite permitido"
    );
    Ok(bytes)
}

fn parse_receipt(bytes: &[u8]) -> Result<WorkflowReceipt> {
    ensure!(
        bytes.len() <= MAX_RECEIPT_BYTES,
        "el recibo excede el límite permitido"
    );
    let receipt: WorkflowReceipt =
        serde_json::from_slice(bytes).context("el recibo no contiene JSON válido")?;
    ensure!(
        receipt.schema_version == WORKFLOW_RECEIPT_SCHEMA,
        "el recibo usa un esquema desconocido"
    );
    Ok(receipt)
}

fn skill_hashes(skill: &SkillSnapshot) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("SKILL.md".to_owned(), digest_hex(&skill.skill_md)),
        (
            "agents/openai.yaml".to_owned(),
            digest_hex(&skill.openai_yaml),
        ),
    ])
}

fn digest_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn read_snapshot(paths: &WorkflowPaths) -> Result<WorkflowSnapshot> {
    Ok(WorkflowSnapshot {
        skill: read_skill_dir(&paths.skill_dir)?,
        awareness: read_optional_text(&paths.awareness, MAX_INSTRUCTION_BYTES)?,
        instructions: read_optional_text(&paths.instructions, MAX_INSTRUCTION_BYTES)?,
        receipt: read_optional_text(&paths.receipt, MAX_RECEIPT_BYTES)?,
    })
}

fn read_skill_dir(path: &Path) -> Result<Option<SkillSnapshot>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("no se pudo inspeccionar la skill instalada"),
    };
    ensure!(
        metadata.is_dir() && !metadata_is_link_or_reparse(&metadata),
        "la skill instalada no es un directorio regular"
    );
    ensure_path_has_no_links(path)?;
    ensure_exact_directory_entries(path, &["SKILL.md", "agents"])?;
    ensure_exact_directory_entries(&path.join("agents"), &["openai.yaml"])?;
    let skill_md = read_regular_bounded(&path.join("SKILL.md"), MAX_SKILL_FILE_BYTES)?;
    let openai_yaml = read_regular_bounded(&path.join("agents/openai.yaml"), MAX_SKILL_FILE_BYTES)?;
    ensure_utf8(&skill_md)?;
    ensure_utf8(&openai_yaml)?;
    Ok(Some(SkillSnapshot {
        skill_md,
        openai_yaml,
    }))
}

fn ensure_exact_directory_entries(path: &Path, expected: &[&str]) -> Result<()> {
    let mut actual = fs::read_dir(path)
        .context("no se pudo leer el directorio de la skill")?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name())
                .context("no se pudo inspeccionar una entrada de la skill")
        })
        .collect::<Result<Vec<_>>>()?;
    actual.sort();
    let mut expected = expected.iter().map(OsStr::new).collect::<Vec<_>>();
    expected.sort();
    ensure!(
        actual == expected,
        "la skill contiene archivos no administrados"
    );
    Ok(())
}

fn read_optional_text(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata_is_link_or_reparse(&metadata),
                "el archivo administrado no es regular"
            );
            ensure_path_has_no_links(path)?;
            let bytes = read_regular_bounded(path, limit)?;
            ensure_utf8(&bytes)?;
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("no se pudo inspeccionar el archivo administrado"),
    }
}

fn read_regular_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("no se pudo inspeccionar {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata_is_link_or_reparse(&metadata),
        "el recurso no es un archivo regular"
    );
    let length = usize::try_from(metadata.len()).context("el recurso es demasiado grande")?;
    ensure!(length <= limit, "el recurso excede el límite permitido");
    let bytes = fs::read(path).with_context(|| format!("no se pudo leer {}", path.display()))?;
    ensure!(
        bytes.len() == length,
        "el recurso cambió durante la lectura"
    );
    Ok(bytes)
}

fn ensure_utf8(bytes: &[u8]) -> Result<()> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    std::str::from_utf8(bytes)
        .context("el archivo de instrucciones no es UTF-8")
        .map(|_| ())
}

fn append_import(existing: Option<&[u8]>) -> Result<Vec<u8>> {
    let existing = existing.unwrap_or_default();
    ensure_utf8(existing)?;
    ensure!(
        import_count(existing)? == 0 || import_count(existing)? == 1,
        "la importación AirWiki está duplicada"
    );
    if import_count(existing)? == 1 {
        return Ok(existing.to_vec());
    }
    let newline = if existing.windows(2).any(|pair| pair == b"\r\n") {
        b"\r\n".as_slice()
    } else {
        b"\n".as_slice()
    };
    let mut result = existing.to_vec();
    if !result.is_empty() && result != [0xEF, 0xBB, 0xBF] && !result.ends_with(b"\n") {
        result.extend_from_slice(newline);
    }
    result.extend_from_slice(IMPORT_LINE.as_bytes());
    result.extend_from_slice(newline);
    ensure!(
        result.len() <= MAX_INSTRUCTION_BYTES,
        "el archivo global de instrucciones excede el límite permitido"
    );
    Ok(result)
}

fn restore_instructions(existing: &[u8], receipt: &WorkflowReceipt) -> Result<Option<Vec<u8>>> {
    if digest_hex(existing) == receipt.instructions_after_sha256
        && let Ok(restored) = restore_unchanged_instructions(existing, receipt)
    {
        return Ok(restored);
    }
    remove_import_conservatively(existing, receipt.instructions_existed)
}

fn restore_unchanged_instructions(
    existing: &[u8],
    receipt: &WorkflowReceipt,
) -> Result<Option<Vec<u8>>> {
    ensure_utf8(existing)?;
    ensure!(
        import_count(existing)? == 1,
        "la importación AirWiki no es única"
    );
    let newline = if existing.ends_with(b"\r\n") {
        b"\r\n".as_slice()
    } else {
        b"\n".as_slice()
    };
    let mut suffix = Vec::with_capacity(IMPORT_LINE.len() + newline.len());
    suffix.extend_from_slice(IMPORT_LINE.as_bytes());
    suffix.extend_from_slice(newline);
    ensure!(
        existing.ends_with(&suffix),
        "la importación AirWiki ya no está en la posición administrada"
    );
    let mut restored = existing[..existing.len() - suffix.len()].to_vec();
    if !receipt.instructions_ended_with_newline {
        ensure!(
            restored.ends_with(newline),
            "el separador administrado ya no coincide"
        );
        restored.truncate(restored.len() - newline.len());
    }
    if receipt.instructions_existed {
        Ok(Some(restored))
    } else {
        ensure!(
            restored.is_empty(),
            "las instrucciones originales no eran vacías"
        );
        Ok(None)
    }
}

fn remove_import_conservatively(
    existing: &[u8],
    instructions_existed: bool,
) -> Result<Option<Vec<u8>>> {
    ensure_utf8(existing)?;
    ensure!(
        import_count(existing)? == 1,
        "la importación AirWiki no es única"
    );
    let bom_len = usize::from(existing.starts_with(&[0xEF, 0xBB, 0xBF])) * 3;
    let mut result = existing[..bom_len].to_vec();
    let text = std::str::from_utf8(&existing[bom_len..])
        .context("el archivo global de instrucciones no es UTF-8")?;
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) != IMPORT_LINE {
            result.extend_from_slice(line.as_bytes());
        }
    }
    if !text.ends_with('\n') {
        let last = text.rsplit('\n').next().unwrap_or_default();
        if last == IMPORT_LINE {
            let suffix = last.as_bytes();
            if result.ends_with(suffix) {
                result.truncate(result.len().saturating_sub(suffix.len()));
            }
        }
    }
    if instructions_existed {
        Ok(Some(result))
    } else {
        Ok((result != [0xEF, 0xBB, 0xBF] && !result.is_empty()).then_some(result))
    }
}

fn import_count(bytes: &[u8]) -> Result<usize> {
    ensure_utf8(bytes)?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).context("las instrucciones no son UTF-8")?;
    Ok(text
        .lines()
        .filter(|line| line.trim_end_matches('\r') == IMPORT_LINE)
        .count())
}

fn apply_snapshot(
    paths: &WorkflowPaths,
    before: &WorkflowSnapshot,
    after: &WorkflowSnapshot,
) -> Result<()> {
    let current = read_snapshot(paths).context("no se pudo revalidar la guía antes de escribir")?;
    ensure!(
        &current == before,
        "la guía cambió después de inspeccionarla y no se sobrescribirá"
    );
    let result = write_snapshot(paths, after);
    if let Err(operation) = result {
        return match write_snapshot(paths, before) {
            Ok(()) => Err(operation.context("se restauró la guía anterior")),
            Err(rollback) => Err(anyhow::anyhow!(
                "{operation:#}; además falló la recuperación: {rollback:#}"
            )),
        };
    }
    Ok(())
}

fn write_snapshot(paths: &WorkflowPaths, snapshot: &WorkflowSnapshot) -> Result<()> {
    replace_skill_dir(&paths.skill_dir, snapshot.skill.as_ref())?;
    replace_optional_file(&paths.awareness, snapshot.awareness.as_deref(), false)?;
    replace_optional_file(&paths.instructions, snapshot.instructions.as_deref(), false)?;
    replace_optional_file(&paths.receipt, snapshot.receipt.as_deref(), true)?;
    Ok(())
}

fn replace_skill_dir(path: &Path, desired: Option<&SkillSnapshot>) -> Result<()> {
    let parent = path
        .parent()
        .context("la skill no tiene un directorio padre")?;
    prepare_managed_parent(parent)?;
    if desired.is_none() {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir() && !metadata_is_link_or_reparse(&metadata),
                    "la skill administrada no es un directorio regular"
                );
                ensure_path_has_no_links(path)?;
                fs::remove_dir_all(path).context("no se pudo retirar la skill administrada")?;
                sync_directory(parent)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("no se pudo inspeccionar la skill"),
        }
        return Ok(());
    }
    let desired = desired.context("la skill deseada no existe")?;
    let had_existing = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_dir() && !metadata_is_link_or_reparse(&metadata),
                "la skill administrada no es un directorio regular"
            );
            ensure_path_has_no_links(path)?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).context("no se pudo inspeccionar la skill"),
    };
    let stage = parent.join(format!(".airwiki-skill-{}.tmp", Uuid::new_v4()));
    fs::create_dir(&stage).context("no se pudo crear el staging de la skill")?;
    let stage_result = (|| {
        fs::create_dir(stage.join("agents")).context("no se pudo preparar metadata de la skill")?;
        write_new_file(&stage.join("SKILL.md"), &desired.skill_md, false)?;
        write_new_file(
            &stage.join("agents/openai.yaml"),
            &desired.openai_yaml,
            false,
        )?;
        sync_directory(&stage.join("agents"))?;
        sync_directory(&stage)?;
        Ok::<_, anyhow::Error>(())
    })();
    if let Err(error) = stage_result {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    let backup = parent.join(format!(".airwiki-skill-{}.bak", Uuid::new_v4()));
    if had_existing && let Err(error) = fs::rename(path, &backup) {
        let _ = fs::remove_dir_all(&stage);
        return Err(error).context("no se pudo apartar la skill anterior");
    }
    if let Err(error) = fs::rename(&stage, path) {
        if had_existing {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_dir_all(&stage);
        return Err(error).context("no se pudo activar la skill administrada");
    }
    sync_directory(parent)?;
    if had_existing {
        fs::remove_dir_all(&backup).context("no se pudo limpiar la skill anterior")?;
    }
    Ok(())
}

fn replace_optional_file(path: &Path, desired: Option<&[u8]>, private: bool) -> Result<()> {
    let parent = path
        .parent()
        .context("el archivo administrado no tiene directorio padre")?;
    prepare_managed_parent(parent)?;
    let Some(bytes) = desired else {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file() && !metadata_is_link_or_reparse(&metadata),
                    "el destino no es un archivo regular"
                );
                fs::remove_file(path).context("no se pudo retirar el archivo administrado")?;
                sync_directory(parent)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("no se pudo inspeccionar el destino"),
        }
        return Ok(());
    };
    let had_existing = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata_is_link_or_reparse(&metadata),
                "el destino no es un archivo regular"
            );
            ensure_path_has_no_links(path)?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).context("no se pudo inspeccionar el destino"),
    };
    let temporary = parent.join(format!(".airwiki-{}.tmp", Uuid::new_v4()));
    write_new_file(&temporary, bytes, private)?;
    let backup = parent.join(format!(".airwiki-{}.bak", Uuid::new_v4()));
    if had_existing && let Err(error) = fs::rename(path, &backup) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("no se pudo apartar el archivo anterior");
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if had_existing {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(error).context("no se pudo activar el archivo administrado");
    }
    sync_directory(parent)?;
    if had_existing {
        fs::remove_file(backup).context("no se pudo limpiar el archivo anterior")?;
    }
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("no se pudo crear el archivo temporal")?;
    if private {
        set_private_permissions(path)?;
    }
    file.write_all(bytes)
        .context("no se pudo escribir el archivo temporal")?;
    file.sync_all()
        .context("no se pudo sincronizar el archivo temporal")?;
    Ok(())
}

fn prepare_managed_parent(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "la ruta administrada no es absoluta");
    if path.exists() {
        ensure_path_has_no_links(path)?;
    } else {
        let existing = nearest_existing_ancestor(path)?;
        ensure_path_has_no_links(&existing)?;
        fs::create_dir_all(path).context("no se pudo crear el directorio administrado")?;
        ensure_path_has_no_links(path)?;
    }
    Ok(())
}

fn nearest_existing_ancestor(path: &Path) -> Result<PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return Ok(current);
        }
        ensure!(current.pop(), "la ruta no tiene un ancestro existente");
    }
}

fn ensure_path_has_no_links(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "la ruta administrada no es absoluta");
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        #[cfg(target_os = "windows")]
        if matches!(component, Component::Prefix(prefix) if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
        {
            continue;
        }
        if !current.is_absolute() {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => ensure!(
                !metadata_is_link_or_reparse(&metadata),
                "la ruta contiene un enlace o punto de reanálisis"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error).context("no se pudo validar la ruta administrada"),
        }
    }
    Ok(())
}

fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .context("no se pudieron restringir los permisos del recibo")
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .context("no se pudo abrir el directorio administrado")?
        .sync_all()
        .context("no se pudo sincronizar el directorio administrado")
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn manager(temp: &TempDir) -> WorkflowGuideManager {
        let root = fs::canonicalize(temp.path()).expect("canonical temp directory");
        let executable = root.join("airwiki");
        fs::write(&executable, b"desktop").expect("desktop fixture");
        let resources = root.join("integrations/workflow");
        fs::create_dir_all(resources.join("airwiki/agents")).expect("resource directories");
        fs::write(
            resources.join("airwiki/SKILL.md"),
            b"---\nname: airwiki\ndescription: fixture\n---\nInstructions\n",
        )
        .expect("skill fixture");
        fs::write(
            resources.join("airwiki/agents/openai.yaml"),
            b"interface:\n  display_name: \"AirWiki\"\n",
        )
        .expect("metadata fixture");
        fs::write(resources.join("AirWiki.md"), b"# AirWiki\n").expect("awareness fixture");
        WorkflowGuideManager::new(
            AppPaths {
                data: root.join("data"),
                database: root.join("data/airwiki.sqlite3"),
                logs: root.join("data/logs"),
                config: root.join("config/config.json"),
            },
            root,
            executable,
            false,
            false,
        )
    }

    #[tokio::test]
    async fn installs_and_removes_codex_guide_without_touching_other_instructions() {
        let temp = TempDir::new().expect("temporary directory");
        let manager = manager(&temp);
        let instructions = temp.path().join(".codex/AGENTS.md");
        fs::create_dir_all(instructions.parent().expect("instruction parent"))
            .expect("instruction directory");
        fs::write(&instructions, b"# Existing\r\n\r\nKeep me.\r\n").expect("existing instructions");

        let change = manager
            .install(WorkflowClient::Codex)
            .await
            .expect("install guide");
        assert!(change.is_some());
        assert_eq!(
            manager.inspect(WorkflowClient::Codex).await.status,
            WorkflowGuideStatus::Installed
        );
        let installed = fs::read(&instructions).expect("installed instructions");
        assert_eq!(installed, b"# Existing\r\n\r\nKeep me.\r\n@AirWiki.md\r\n");

        manager
            .remove(WorkflowClient::Codex)
            .await
            .expect("remove guide");
        assert_eq!(
            fs::read(&instructions).expect("remaining instructions"),
            b"# Existing\r\n\r\nKeep me.\r\n"
        );
        assert_eq!(
            manager.inspect(WorkflowClient::Codex).await.status,
            WorkflowGuideStatus::Available
        );
    }

    #[tokio::test]
    async fn modified_managed_resource_becomes_a_conflict_and_is_preserved() {
        let temp = TempDir::new().expect("temporary directory");
        let manager = manager(&temp);
        manager
            .install(WorkflowClient::ClaudeCode)
            .await
            .expect("install guide");
        let skill = temp.path().join(".claude/skills/airwiki/SKILL.md");
        fs::write(&skill, b"user modification").expect("modify managed skill");

        assert_eq!(
            manager.inspect(WorkflowClient::ClaudeCode).await.status,
            WorkflowGuideStatus::Conflict
        );
        assert!(manager.remove(WorkflowClient::ClaudeCode).await.is_err());
        assert_eq!(
            fs::read(skill).expect("preserved skill"),
            b"user modification"
        );
    }

    #[tokio::test]
    async fn bundled_skill_update_preserves_one_import_and_unrelated_instructions() {
        let temp = TempDir::new().expect("temporary directory");
        let manager = manager(&temp);
        let instructions = temp.path().join(".codex/AGENTS.md");
        fs::create_dir_all(instructions.parent().expect("instruction parent"))
            .expect("instruction directory");
        fs::write(&instructions, b"# Existing\n").expect("existing instructions");
        manager
            .install(WorkflowClient::Codex)
            .await
            .expect("install initial guide");
        fs::write(
            temp.path().join("integrations/workflow/airwiki/SKILL.md"),
            b"---\nname: airwiki\ndescription: updated fixture\n---\nUpdated\n",
        )
        .expect("updated bundled skill");

        assert_eq!(
            manager.inspect(WorkflowClient::Codex).await.status,
            WorkflowGuideStatus::UpdateAvailable
        );
        manager
            .install(WorkflowClient::Codex)
            .await
            .expect("install updated guide");

        let installed = fs::read(&instructions).expect("updated instructions");
        assert_eq!(installed, b"# Existing\n@AirWiki.md\n");
        assert_eq!(import_count(&installed).expect("count import"), 1);
    }

    #[tokio::test]
    async fn non_utf8_and_oversized_global_instructions_fail_closed() {
        let temp = TempDir::new().expect("temporary directory");
        let manager = manager(&temp);
        let instructions = temp.path().join(".codex/AGENTS.md");
        fs::create_dir_all(instructions.parent().expect("instruction parent"))
            .expect("instruction directory");
        fs::write(&instructions, [0xFF]).expect("non-UTF-8 instructions");

        assert_eq!(
            manager.inspect(WorkflowClient::Codex).await.status,
            WorkflowGuideStatus::Conflict
        );
        assert!(manager.install(WorkflowClient::Codex).await.is_err());

        fs::write(&instructions, vec![b'x'; MAX_INSTRUCTION_BYTES + 1])
            .expect("oversized instructions");
        assert_eq!(
            manager.inspect(WorkflowClient::Codex).await.status,
            WorkflowGuideStatus::Conflict
        );
        assert!(manager.install(WorkflowClient::Codex).await.is_err());
    }

    #[tokio::test]
    async fn rollback_restores_the_exact_previous_state() {
        let temp = TempDir::new().expect("temporary directory");
        let manager = manager(&temp);
        let change = manager
            .install(WorkflowClient::GeminiCli)
            .await
            .expect("install guide")
            .expect("guide change");

        manager.rollback(change).await.expect("rollback guide");

        assert_eq!(
            manager.inspect(WorkflowClient::GeminiCli).await.status,
            WorkflowGuideStatus::Available
        );
        assert!(!temp.path().join(".gemini/AirWiki.md").exists());
    }

    #[test]
    fn concurrent_user_edit_is_detected_before_any_managed_write() {
        let temp = TempDir::new().expect("temporary directory");
        let root = fs::canonicalize(temp.path()).expect("canonical temporary directory");
        let paths = WorkflowPaths {
            skill_dir: root.join(".agents/skills/airwiki"),
            awareness: root.join(".codex/AirWiki.md"),
            instructions: root.join(".codex/AGENTS.md"),
            receipt: root.join("data/receipt.json"),
        };
        fs::create_dir_all(paths.instructions.parent().expect("instruction parent"))
            .expect("instruction directory");
        fs::write(&paths.instructions, b"before").expect("prior instructions");
        let before = read_snapshot(&paths).expect("initial snapshot");
        let after = WorkflowSnapshot {
            skill: Some(SkillSnapshot {
                skill_md: b"skill".to_vec(),
                openai_yaml: b"metadata".to_vec(),
            }),
            awareness: Some(b"awareness".to_vec()),
            instructions: Some(b"before\n@AirWiki.md\n".to_vec()),
            receipt: Some(b"receipt".to_vec()),
        };
        fs::write(&paths.instructions, b"concurrent user edit").expect("concurrent edit");

        assert!(apply_snapshot(&paths, &before, &after).is_err());
        assert_eq!(
            fs::read(&paths.instructions).expect("preserved concurrent edit"),
            b"concurrent user edit"
        );
        assert!(!paths.skill_dir.exists());
        assert!(!paths.awareness.exists());
        assert!(!paths.receipt.exists());
    }

    #[test]
    fn invalid_destination_is_rejected_without_user_changes_or_staging_residue() {
        let temp = TempDir::new().expect("temporary directory");
        let root = fs::canonicalize(temp.path()).expect("canonical temporary directory");
        let paths = WorkflowPaths {
            skill_dir: root.join(".agents/skills/airwiki"),
            awareness: root.join(".codex/AirWiki.md"),
            instructions: root.join(".codex/AGENTS.md"),
            receipt: root.join("data/receipt.json"),
        };
        fs::create_dir_all(paths.instructions.parent().expect("instruction parent"))
            .expect("instruction directory");
        fs::write(&paths.instructions, b"before").expect("prior instructions");
        fs::create_dir_all(&paths.receipt).expect("injected invalid receipt destination");
        let before = WorkflowSnapshot {
            skill: None,
            awareness: None,
            instructions: Some(b"before".to_vec()),
            receipt: None,
        };
        let after = WorkflowSnapshot {
            skill: Some(SkillSnapshot {
                skill_md: b"skill".to_vec(),
                openai_yaml: b"metadata".to_vec(),
            }),
            awareness: Some(b"awareness".to_vec()),
            instructions: Some(b"before\n@AirWiki.md\n".to_vec()),
            receipt: Some(b"receipt".to_vec()),
        };

        assert!(apply_snapshot(&paths, &before, &after).is_err());
        assert!(!paths.skill_dir.exists());
        assert!(!paths.awareness.exists());
        assert_eq!(
            fs::read(&paths.instructions).expect("restored instructions"),
            b"before"
        );
        for parent in [
            paths.skill_dir.parent().expect("skill parent"),
            paths.awareness.parent().expect("awareness parent"),
            paths.receipt.parent().expect("receipt parent"),
        ] {
            if !parent.exists() {
                continue;
            }
            let residues = fs::read_dir(parent)
                .expect("managed parent")
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with(".airwiki-"))
                .count();
            assert_eq!(residues, 0);
        }
    }

    #[test]
    fn bom_and_line_endings_are_preserved() {
        let existing = b"\xEF\xBB\xBF# Existing\r\n";
        let installed = append_import(Some(existing)).expect("append import");
        assert_eq!(installed, b"\xEF\xBB\xBF# Existing\r\n@AirWiki.md\r\n");
        let receipt = WorkflowReceipt {
            schema_version: WORKFLOW_RECEIPT_SCHEMA,
            client: "fixture".to_owned(),
            version: WORKFLOW_GUIDE_VERSION.to_owned(),
            skill_files: BTreeMap::new(),
            awareness_sha256: digest_hex(b"awareness"),
            instructions_existed: true,
            instructions_ended_with_newline: true,
            instructions_after_sha256: digest_hex(&installed),
        };
        assert_eq!(
            restore_instructions(&installed, &receipt).expect("remove import"),
            Some(existing.to_vec())
        );
    }

    #[test]
    fn removal_restores_a_file_without_a_trailing_newline_exactly() {
        let existing = b"# Existing";
        let installed = append_import(Some(existing)).expect("append import");
        let receipt = WorkflowReceipt {
            schema_version: WORKFLOW_RECEIPT_SCHEMA,
            client: "fixture".to_owned(),
            version: WORKFLOW_GUIDE_VERSION.to_owned(),
            skill_files: BTreeMap::new(),
            awareness_sha256: digest_hex(b"awareness"),
            instructions_existed: true,
            instructions_ended_with_newline: false,
            instructions_after_sha256: digest_hex(&installed),
        };

        assert_eq!(
            restore_instructions(&installed, &receipt).expect("restore instructions"),
            Some(existing.to_vec())
        );
    }

    #[test]
    fn removal_preserves_unrelated_instructions_added_later() {
        let installed = b"# Existing\n@AirWiki.md\nUser addition\n";
        let receipt = WorkflowReceipt {
            schema_version: WORKFLOW_RECEIPT_SCHEMA,
            client: "fixture".to_owned(),
            version: WORKFLOW_GUIDE_VERSION.to_owned(),
            skill_files: BTreeMap::new(),
            awareness_sha256: digest_hex(b"awareness"),
            instructions_existed: true,
            instructions_ended_with_newline: true,
            instructions_after_sha256: digest_hex(b"different installed bytes"),
        };

        assert_eq!(
            restore_instructions(installed, &receipt).expect("restore instructions"),
            Some(b"# Existing\nUser addition\n".to_vec())
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_skill_destination_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().expect("temporary directory");
        let manager = manager(&temp);
        let foreign = temp.path().join("foreign");
        fs::create_dir(&foreign).expect("foreign directory");
        let skills = temp.path().join(".agents/skills");
        fs::create_dir_all(&skills).expect("skill root");
        symlink(&foreign, skills.join("airwiki")).expect("skill symlink");

        assert_eq!(
            manager.inspect(WorkflowClient::Codex).await.status,
            WorkflowGuideStatus::Conflict
        );
        assert!(manager.install(WorkflowClient::Codex).await.is_err());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn reparse_point_skill_destination_is_rejected() {
        let temp = TempDir::new().expect("temporary directory");
        let manager = manager(&temp);
        let foreign = temp.path().join("foreign");
        let skills = temp.path().join(".agents/skills");
        fs::create_dir_all(&foreign).expect("foreign directory");
        fs::create_dir_all(&skills).expect("skill root");
        let destination = skills.join("airwiki");
        let destination_argument = destination.to_string_lossy().replace('\'', "''");
        let foreign_argument = foreign.to_string_lossy().replace('\'', "''");
        let create_junction = format!(
            "New-Item -ItemType Junction -Path '{destination_argument}' -Target '{foreign_argument}' | Out-Null"
        );
        let status = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &create_junction,
            ])
            .status()
            .expect("create junction fixture");
        assert!(status.success());

        assert_eq!(
            manager.inspect(WorkflowClient::Codex).await.status,
            WorkflowGuideStatus::Conflict
        );
        assert!(manager.install(WorkflowClient::Codex).await.is_err());
        fs::remove_dir(&destination).expect("remove junction fixture");
        assert!(foreign.is_dir());
    }
}
