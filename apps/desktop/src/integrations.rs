#[cfg(target_os = "windows")]
use std::path::Prefix;
use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    time::timeout,
};
use uuid::Uuid;

use crate::{
    paths::AppPaths,
    workflow_guides::{WorkflowChange, WorkflowClient, WorkflowGuideManager, WorkflowGuideView},
};

const INTEGRATION_NAME: &str = "airwiki";
const BRIDGE_BASENAME: &str = "airwiki-mcp-bridge";
const CLAUDE_MCPB_NAME: &str = "airwiki-claude.mcpb";
const SEARCH_TOOL: &str = "search_airwiki";
const MCP_PROTOCOL_VERSION: &str = "2026-07-28";
const APPLICATION_TOOLS: [&str; 7] = [
    "list_airwiki_memories",
    "create_airwiki_memory",
    "get_airwiki_memory",
    "write_airwiki_memory",
    "deprecate_airwiki_memory",
    "request_airwiki_computation",
    "get_airwiki_computation_run",
];
const MANAGED_TOOLS: [&str; 8] = [
    SEARCH_TOOL,
    APPLICATION_TOOLS[0],
    APPLICATION_TOOLS[1],
    APPLICATION_TOOLS[2],
    APPLICATION_TOOLS[3],
    APPLICATION_TOOLS[4],
    APPLICATION_TOOLS[5],
    APPLICATION_TOOLS[6],
];
const PROCESS_TIMEOUT: Duration = Duration::from_secs(10);
const VERIFY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROCESS_OUTPUT: usize = 64 * 1024;
const MAX_BRIDGE_VERIFY_OUTPUT: usize = airwiki_mcp::MAX_MANAGED_BRIDGE_VERIFICATION_BYTES;
const MAX_BRIDGE_BYTES: u64 = 64 * 1024 * 1024;
#[cfg(target_os = "windows")]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChatClientKind {
    ChatGptDesktop,
    ClaudeDesktop,
    ClaudeCode,
    GeminiCli,
    GenericMcp,
}

impl ChatClientKind {
    pub(crate) const ALL: [Self; 5] = [
        Self::ChatGptDesktop,
        Self::ClaudeDesktop,
        Self::ClaudeCode,
        Self::GeminiCli,
        Self::GenericMcp,
    ];

    const fn bridge_id(self) -> &'static str {
        match self {
            Self::ChatGptDesktop => "chatgpt-desktop",
            Self::ClaudeDesktop => "claude-desktop",
            Self::ClaudeCode => "claude-code",
            Self::GeminiCli => "gemini-cli",
            Self::GenericMcp => "generic-mcp",
        }
    }

    const fn display_name(self) -> &'static str {
        match self {
            Self::ChatGptDesktop => "ChatGPT/Codex",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::ClaudeCode => "Claude Code",
            Self::GeminiCli => "Gemini CLI",
            Self::GenericMcp => "Generic MCP",
        }
    }

    const fn producer(self) -> &'static str {
        match self {
            Self::ChatGptDesktop => "codex/managed",
            Self::ClaudeDesktop => "claude/managed",
            Self::ClaudeCode => "claude-code/managed",
            Self::GeminiCli => "gemini/managed",
            Self::GenericMcp => "generic-mcp/1",
        }
    }

    const fn workflow_client(self) -> Option<WorkflowClient> {
        match self {
            Self::ChatGptDesktop => Some(WorkflowClient::Codex),
            Self::ClaudeCode => Some(WorkflowClient::ClaudeCode),
            Self::GeminiCli => Some(WorkflowClient::GeminiCli),
            Self::ClaudeDesktop | Self::GenericMcp => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IntegrationStatus {
    NotInstalled,
    Available,
    AwaitingClientApproval,
    Configured,
    UpdateAvailable,
    Conflict,
    Unsupported,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntegrationView {
    pub client: ChatClientKind,
    pub status: IntegrationStatus,
    pub detected_version: Option<String>,
    pub detail: String,
    pub planned_path: Option<PathBuf>,
    pub activity_recent: bool,
    pub restart_required: bool,
    pub workflow_guide: WorkflowGuideView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatIntegrationsSnapshot {
    pub integrations: Vec<IntegrationView>,
    pub external_ai_collection_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntegrationAction {
    Refresh,
    Connect(ChatClientKind),
    Disconnect(ChatClientKind),
    ConfirmClaudeInstalled,
    OpenClaudeSettings,
    InstallWorkflowGuide(ChatClientKind),
    RemoveWorkflowGuide(ChatClientKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapabilityProvision {
    Existing,
    Created,
}

#[derive(Debug, Clone)]
struct CommandSpec {
    executable: PathBuf,
    args: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    stdin: Option<Vec<u8>>,
    timeout: Duration,
    stdout_limit: usize,
}

impl CommandSpec {
    fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            args: Vec::new(),
            environment: Vec::new(),
            stdin: None,
            timeout: PROCESS_TIMEOUT,
            stdout_limit: MAX_PROCESS_OUTPUT,
        }
    }

    fn args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    fn environment(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.environment.push((key.into(), value.into()));
        self
    }

    fn stdin(mut self, bytes: Vec<u8>) -> Self {
        self.stdin = Some(bytes);
        self
    }

    fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn stdout_limit(mut self, limit: usize) -> Self {
        self.stdout_limit = limit;
        self
    }
}

#[derive(Debug, Clone)]
struct CommandOutput {
    success: bool,
    stdout: Vec<u8>,
    _stderr: Vec<u8>,
}

impl CommandOutput {
    fn stdout_text(&self) -> Result<&str> {
        std::str::from_utf8(&self.stdout).context("la salida del proceso no es UTF-8")
    }

    fn stderr_text(&self) -> Result<&str> {
        std::str::from_utf8(&self._stderr).context("la salida de error del proceso no es UTF-8")
    }
}

#[async_trait]
trait CommandRunner: Send + Sync {
    async fn run(&self, spec: CommandSpec) -> Result<CommandOutput>;
}

#[derive(Debug, Default)]
struct SystemCommandRunner;

#[async_trait]
impl CommandRunner for SystemCommandRunner {
    async fn run(&self, spec: CommandSpec) -> Result<CommandOutput> {
        let mut command = Command::new(&spec.executable);
        command
            .args(&spec.args)
            .envs(spec.environment)
            .stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().context("no se pudo iniciar el proceso")?;
        if let Some(input) = spec.stdin {
            let mut stdin = child
                .stdin
                .take()
                .context("el proceso no expuso su entrada estándar")?;
            stdin
                .write_all(&input)
                .await
                .context("no se pudo escribir al proceso")?;
            stdin
                .shutdown()
                .await
                .context("no se pudo cerrar la entrada")?;
        }
        let stdout = child
            .stdout
            .take()
            .context("el proceso no expuso su salida estándar")?;
        let stderr = child
            .stderr
            .take()
            .context("el proceso no expuso su salida de error")?;

        let process = async {
            let (status, stdout, stderr) = tokio::try_join!(
                async { child.wait().await.context("no se pudo esperar al proceso") },
                read_bounded(stdout, spec.stdout_limit),
                read_bounded(stderr, MAX_PROCESS_OUTPUT),
            )?;
            Ok::<_, anyhow::Error>(CommandOutput {
                success: status.success(),
                stdout,
                _stderr: stderr,
            })
        };
        match timeout(spec.timeout, process).await {
            Ok(result) => result,
            Err(_) => {
                let _ = child.kill().await;
                bail!("el proceso excedió el tiempo permitido")
            }
        }
    }
}

async fn read_bounded(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .context("no se pudo leer la salida del proceso")?;
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > limit {
            bail!("la salida del proceso excedió el límite permitido");
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

#[async_trait]
trait PathOpener: Send + Sync {
    async fn open(&self, path: &Path) -> Result<()>;
}

#[derive(Debug, Default)]
struct SystemPathOpener;

#[async_trait]
impl PathOpener for SystemPathOpener {
    async fn open(&self, path: &Path) -> Result<()> {
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("/usr/bin/open");
            command.arg(path);
            command
        };
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = Command::new("explorer.exe");
            command.arg(path);
            command
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        bail!("esta plataforma no admite la apertura administrada");

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let mut child = command.spawn().context("no se pudo abrir la aplicación")?;
            let status = timeout(PROCESS_TIMEOUT, child.wait())
                .await
                .context("abrir la aplicación excedió el tiempo permitido")??;
            if !status.success() {
                bail!("el sistema no pudo abrir la aplicación")
            }
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "all platform variants are constructed when the same source is built on supported targets"
)]
enum HostPlatform {
    MacOs,
    Windows,
    Unsupported,
}

#[derive(Debug, Clone)]
struct IntegrationEnvironment {
    platform: HostPlatform,
    home: PathBuf,
    path_entries: Vec<PathBuf>,
    discover_host_clients: bool,
    current_exe: PathBuf,
}

impl IntegrationEnvironment {
    fn discover(paths: &AppPaths) -> Result<Self> {
        #[cfg(target_os = "macos")]
        let platform = HostPlatform::MacOs;
        #[cfg(target_os = "windows")]
        let platform = HostPlatform::Windows;
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let platform = HostPlatform::Unsupported;

        #[cfg(feature = "e2e")]
        let (home, path_entries, discover_host_clients) =
            (paths.data.join("integration-home"), Vec::new(), false);
        #[cfg(not(feature = "e2e"))]
        let (home, path_entries, discover_host_clients) = {
            let _ = paths;
            let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from)
                .context("no se encontró el directorio personal")?;
            let path_entries = std::env::var_os("PATH")
                .map(|value| std::env::split_paths(&value).collect())
                .unwrap_or_default();
            (home, path_entries, true)
        };
        Ok(Self {
            platform,
            home,
            path_entries,
            discover_host_clients,
            current_exe: std::env::current_exe()
                .context("no se pudo localizar el ejecutable actual")?,
        })
    }
}

#[derive(Clone)]
pub(crate) struct ChatIntegrationManager {
    paths: AppPaths,
    environment: IntegrationEnvironment,
    runner: Arc<dyn CommandRunner>,
    opener: Arc<dyn PathOpener>,
    database: airwiki_core::Database,
    workflow_guides: WorkflowGuideManager,
}

impl ChatIntegrationManager {
    pub(crate) fn new(paths: AppPaths, database: airwiki_core::Database) -> Result<Self> {
        let environment = IntegrationEnvironment::discover(&paths)?;
        let workflow_guides = WorkflowGuideManager::new(
            paths.clone(),
            environment.home.clone(),
            environment.current_exe.clone(),
            environment.platform == HostPlatform::MacOs,
            environment.discover_host_clients,
        );
        Ok(Self {
            paths,
            environment,
            runner: Arc::new(SystemCommandRunner),
            opener: Arc::new(SystemPathOpener),
            database,
            workflow_guides,
        })
    }

    pub(crate) async fn execute(&self, action: IntegrationAction) -> Result<Vec<IntegrationView>> {
        match action {
            IntegrationAction::Refresh => {}
            IntegrationAction::Connect(client) => self.connect(client).await?,
            IntegrationAction::Disconnect(client) => self.disconnect(client).await?,
            IntegrationAction::ConfirmClaudeInstalled => {}
            IntegrationAction::OpenClaudeSettings => self.open_claude_settings().await?,
            IntegrationAction::InstallWorkflowGuide(client) => {
                let workflow_client = client
                    .workflow_client()
                    .context("este cliente no admite una skill nativa")?;
                self.workflow_guides.install(workflow_client).await?;
            }
            IntegrationAction::RemoveWorkflowGuide(client) => {
                let workflow_client = client
                    .workflow_client()
                    .context("este cliente no admite una skill nativa")?;
                self.workflow_guides.remove(workflow_client).await?;
            }
        }
        let mut views = self.inspect_all().await?;
        if matches!(
            action,
            IntegrationAction::Connect(ChatClientKind::ClaudeDesktop)
        ) && let Some(claude) = views
            .iter_mut()
            .find(|view| view.client == ChatClientKind::ClaudeDesktop)
        {
            claude.status = IntegrationStatus::AwaitingClientApproval;
            claude.detail =
                "Completa la aprobación en Claude; luego actualiza el estado en AirWiki."
                    .to_owned();
        }
        Ok(views)
    }

    async fn inspect_all(&self) -> Result<Vec<IntegrationView>> {
        let inspections = ChatClientKind::ALL.map(|client| async move {
            let result = self.inspect(client).await;
            (client, result)
        });
        Ok(futures::future::join_all(inspections)
            .await
            .into_iter()
            .map(|(client, result)| match result {
                Ok(view) => view,
                Err(error) => view(
                    client,
                    IntegrationStatus::Error,
                    format!("No se pudo comprobar esta integración: {error:#}"),
                    None,
                    Some(self.managed_bridge_path()),
                ),
            })
            .collect())
    }

    async fn inspect(&self, client: ChatClientKind) -> Result<IntegrationView> {
        let mut view = match client {
            ChatClientKind::ChatGptDesktop => self.inspect_chatgpt().await,
            ChatClientKind::ClaudeDesktop => self.inspect_claude().await,
            ChatClientKind::ClaudeCode => self.inspect_claude_code().await,
            ChatClientKind::GeminiCli => self.inspect_gemini().await,
            ChatClientKind::GenericMcp => self.inspect_generic_mcp().await,
        }?;
        view.workflow_guide = if let Some(workflow_client) = client.workflow_client() {
            self.workflow_guides.inspect(workflow_client).await
        } else {
            WorkflowGuideView::built_in()
        };
        Ok(view)
    }

    async fn connect(&self, client: ChatClientKind) -> Result<()> {
        let provision = self.ensure_application_capability(client).await?;
        let workflow_change = if let Some(workflow_client) = client.workflow_client() {
            match self.workflow_guides.install(workflow_client).await {
                Ok(change) => change,
                Err(error) if provision == CapabilityProvision::Created => {
                    let rollback = self.revoke_application_capability(client).await;
                    return Err(with_rollback_context(error, rollback));
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        let result = match client {
            ChatClientKind::ChatGptDesktop => self.connect_chatgpt().await,
            ChatClientKind::ClaudeDesktop => self.open_claude_bundle().await,
            ChatClientKind::ClaudeCode => self.connect_claude_code().await,
            ChatClientKind::GeminiCli => self.connect_gemini().await,
            ChatClientKind::GenericMcp => self.connect_generic_mcp().await,
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                let workflow_rollback = self.rollback_workflow_change(workflow_change).await;
                let error = with_rollback_context(
                    error.context("no se pudo conectar la integración"),
                    workflow_rollback,
                );
                if provision == CapabilityProvision::Created {
                    let capability_rollback = self.revoke_application_capability(client).await;
                    Err(with_rollback_context(error, capability_rollback))
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn disconnect(&self, client: ChatClientKind) -> Result<()> {
        self.revoke_application_capability(client).await?;
        let workflow_change = if let Some(workflow_client) = client.workflow_client() {
            self.workflow_guides.remove(workflow_client).await?
        } else {
            None
        };
        let result = match client {
            ChatClientKind::ChatGptDesktop => self.disconnect_chatgpt().await,
            ChatClientKind::ClaudeDesktop => self.open_claude_settings().await,
            ChatClientKind::ClaudeCode => self.disconnect_claude_code().await,
            ChatClientKind::GeminiCli => self.disconnect_gemini().await,
            ChatClientKind::GenericMcp => Ok(()),
        };
        if let Err(error) = result {
            let rollback = self.rollback_workflow_change(workflow_change).await;
            return Err(with_rollback_context(error, rollback));
        }
        Ok(())
    }

    async fn rollback_workflow_change(&self, change: Option<WorkflowChange>) -> Result<()> {
        if let Some(change) = change {
            self.workflow_guides.rollback(change).await
        } else {
            Ok(())
        }
    }

    fn capability_path(&self, client: ChatClientKind) -> PathBuf {
        self.paths
            .data
            .join("integrations")
            .join("capabilities")
            .join(format!("{}.cap", client.bridge_id()))
    }

    async fn ensure_application_capability(
        &self,
        client: ChatClientKind,
    ) -> Result<CapabilityProvision> {
        let path = self.capability_path(client);
        let app_id = application_id(client);
        if regular_file(&path)? {
            let parent = path
                .parent()
                .context("la capacidad no tiene un directorio padre")?;
            if path_contains_link_or_reparse_point(parent).await? {
                bail!("el directorio de capacidades contiene un enlace no permitido")
            }
            let metadata = fs::symlink_metadata(&path)
                .await
                .context("no se pudo inspeccionar la capacidad privada")?;
            if metadata.len() > 256 {
                bail!("la capacidad privada de la integración excede el tamaño permitido")
            }
            set_private_permissions(&path).await?;
            let secret = fs::read_to_string(&path)
                .await
                .context("no se pudo leer la capacidad privada de la integración")?;
            match self
                .database
                .authenticate_application_capability(secret.trim())?
            {
                Some(capability) if capability.app_id == app_id => {
                    return Ok(CapabilityProvision::Existing);
                }
                Some(_) => bail!("la capacidad privada pertenece a otra integración"),
                None if self
                    .database
                    .application_capability_any_by_app_id(app_id)?
                    .is_some_and(|capability| capability.revoked_at.is_some()) =>
                {
                    fs::remove_file(&path)
                        .await
                        .context("no se pudo reemplazar la capacidad revocada")?;
                }
                None => {
                    bail!("la capacidad privada de la integración no coincide con su registro")
                }
            }
        }
        let mut random = [0_u8; 48];
        getrandom::fill(&mut random).context("no se pudo generar una capacidad segura")?;
        let secret = hex::encode(random);
        let prefix = secret
            .get(..16)
            .context("la capacidad generada no contiene un prefijo válido")?;
        let secret_hash = hex::encode(Sha256::digest(secret.as_bytes()));
        if self
            .database
            .application_capability_any_by_app_id(app_id)?
            .is_some()
        {
            self.database.rotate_application_capability(
                app_id,
                client.producer(),
                prefix,
                &secret_hash,
            )?;
        } else {
            self.database.create_application_capability(
                app_id,
                client.display_name(),
                client.bridge_id(),
                client.producer(),
                prefix,
                &secret_hash,
            )?;
        }
        if let Err(error) = write_capability_atomically(&path, secret.as_bytes()).await {
            let rollback = self
                .database
                .set_application_capability_revoked(app_id, true);
            return Err(with_rollback_context(error, rollback));
        }
        Ok(CapabilityProvision::Created)
    }

    async fn revoke_application_capability(&self, client: ChatClientKind) -> Result<()> {
        let app_id = application_id(client);
        if self
            .database
            .application_capability_by_app_id(app_id)?
            .is_some()
        {
            self.database
                .set_application_capability_revoked(app_id, true)?;
        }
        match fs::remove_file(self.capability_path(client)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("no se pudo retirar la capacidad de la integración"),
        }
    }

    fn managed_bridge_path(&self) -> PathBuf {
        self.paths
            .data
            .join("integrations")
            .join("bridge")
            .join(env!("CARGO_PKG_VERSION"))
            .join(bridge_filename())
    }

    fn managed_bridge_root(&self) -> PathBuf {
        self.paths.data.join("integrations").join("bridge")
    }

    fn bundled_bridge(&self) -> Option<PathBuf> {
        let executable_dir = self.environment.current_exe.parent()?;
        let mut candidates = vec![
            executable_dir
                .join("integrations")
                .join("bridge")
                .join(bridge_filename()),
        ];
        if self.environment.platform == HostPlatform::MacOs {
            candidates.insert(
                0,
                executable_dir
                    .join("../Resources/integrations/bridge")
                    .join(bridge_filename()),
            );
        }
        #[cfg(debug_assertions)]
        {
            let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            candidates.extend([
                executable_dir.join(bridge_filename()),
                workspace.join("target/debug").join(bridge_filename()),
                workspace.join("target/release").join(bridge_filename()),
            ]);
        }
        candidates
            .into_iter()
            .find(|candidate| regular_file(candidate).unwrap_or(false))
    }

    fn bundled_claude_mcpb(&self) -> Option<PathBuf> {
        let executable_dir = self.environment.current_exe.parent()?;
        let mut candidates = vec![executable_dir.join("integrations").join(CLAUDE_MCPB_NAME)];
        if self.environment.platform == HostPlatform::MacOs {
            candidates.insert(
                0,
                executable_dir
                    .join("../Resources/integrations")
                    .join(CLAUDE_MCPB_NAME),
            );
        }
        #[cfg(debug_assertions)]
        {
            let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            if let Some(triple) = match self.environment.platform {
                HostPlatform::MacOs => Some("aarch64-apple-darwin"),
                HostPlatform::Windows => Some("x86_64-pc-windows-msvc"),
                HostPlatform::Unsupported => None,
            } {
                candidates.push(
                    workspace
                        .join("target/mcpb")
                        .join(triple)
                        .join(CLAUDE_MCPB_NAME),
                );
            }
            candidates.push(
                workspace
                    .join("resources/integrations")
                    .join(CLAUDE_MCPB_NAME),
            );
        }
        candidates
            .into_iter()
            .find(|candidate| regular_file(candidate).unwrap_or(false))
    }

    async fn materialize_bridge(&self) -> Result<PathBuf> {
        let source = self
            .bundled_bridge()
            .context("el paquete no contiene el puente MCP para esta plataforma")?;
        let destination = self.managed_bridge_path();
        ensure_regular_path(&source)?;
        let source_bytes = read_file_bounded(&source).await?;
        if path_contains_link_or_reparse_point(&self.paths.data).await? {
            bail!(
                "el directorio de datos contiene un enlace simbólico o punto de reanálisis no permitido"
            )
        }
        if destination.exists() {
            ensure_regular_path(&destination)?;
            if read_file_bounded(&destination).await? != source_bytes {
                bail!("el puente instalado no coincide con esta versión de la aplicación");
            }
            return Ok(destination);
        }
        let parent = destination
            .parent()
            .context("la ruta del puente no tiene directorio padre")?;
        if path_contains_link_or_reparse_point(parent).await? {
            bail!(
                "la ruta administrada contiene un enlace simbólico o punto de reanálisis no permitido"
            )
        }
        fs::create_dir_all(parent)
            .await
            .context("no se pudo preparar el directorio de integraciones")?;
        if path_contains_link_or_reparse_point(parent).await? {
            bail!(
                "la ruta administrada contiene un enlace simbólico o punto de reanálisis no permitido"
            )
        }
        let temporary = parent.join(format!(".bridge-{}.tmp", Uuid::new_v4()));
        let copy_result =
            write_bridge_atomically(&temporary, &destination, parent, &source_bytes).await;
        if copy_result.is_err() {
            let _ = fs::remove_file(&temporary).await;
        }
        copy_result?;
        ensure_regular_path(&destination)?;
        if read_file_bounded(&destination).await? != source_bytes {
            let _ = fs::remove_file(&destination).await;
            bail!("el puente MCP cambió durante su instalación")
        }
        Ok(destination)
    }

    async fn verify_bridge(&self, bridge: &Path, client: ChatClientKind) -> Result<()> {
        let request_meta = serde_json::json!({
            "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientInfo": {
                "name": "airwiki-desktop",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "io.modelcontextprotocol/clientCapabilities": {},
        });
        let input = [
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "server/discover",
                "params": { "_meta": request_meta.clone() },
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": { "_meta": request_meta },
            }),
        ]
        .into_iter()
        .map(|request| request.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        let output = self
            .runner
            .run(
                CommandSpec::new(bridge.to_path_buf())
                    .args(["--client", client.bridge_id()])
                    .stdin(input.into_bytes())
                    .timeout(VERIFY_TIMEOUT)
                    .stdout_limit(MAX_BRIDGE_VERIFY_OUTPUT),
            )
            .await
            .context("el puente MCP no superó su verificación local")?;
        if !output.success {
            bail!("el puente MCP rechazó server/discover o tools/list")
        }
        verify_tools_list(output.stdout_text()?)
    }

    async fn inspect_chatgpt(&self) -> Result<IntegrationView> {
        let Some(codex) = self.find_codex() else {
            return Ok(view(
                ChatClientKind::ChatGptDesktop,
                IntegrationStatus::NotInstalled,
                "Instala o actualiza ChatGPT Desktop para habilitar su CLI local.",
                None,
                Some(self.managed_bridge_path()),
            ));
        };
        // Codex startup can include its own plugin discovery. Probe support and
        // version concurrently so a slow CLI cannot serialize two independent
        // process timeouts and block every integration action.
        let (supported, detected_version) =
            tokio::join!(self.codex_supported(&codex), self.program_version(&codex));
        if !supported? {
            return Ok(view(
                ChatClientKind::ChatGptDesktop,
                IntegrationStatus::Unsupported,
                "La versión detectada no admite administración MCP local.",
                detected_version,
                Some(self.managed_bridge_path()),
            ));
        }
        let configured = self.codex_configuration(&codex).await?;
        let (status, detail) = self
            .classify_configuration_securely(configured.as_ref(), ChatClientKind::ChatGptDesktop)
            .await?;
        Ok(view(
            ChatClientKind::ChatGptDesktop,
            status,
            detail,
            detected_version,
            Some(self.managed_bridge_path()),
        ))
    }

    async fn connect_chatgpt(&self) -> Result<()> {
        let codex = self
            .find_codex()
            .context("no se encontró una versión compatible de ChatGPT/Codex")?;
        if !self.codex_supported(&codex).await? {
            bail!("actualiza ChatGPT Desktop antes de conectar AirWiki")
        }
        let current = self.codex_configuration(&codex).await?;
        self.ensure_replaceable(current.as_ref(), ChatClientKind::ChatGptDesktop)
            .await?;
        let bridge = self.materialize_bridge().await?;
        self.verify_bridge(&bridge, ChatClientKind::ChatGptDesktop)
            .await?;
        if current.as_ref().is_some_and(|configuration| {
            configuration.is_exact(&bridge, ChatClientKind::ChatGptDesktop)
        }) {
            return Ok(());
        }
        if current.is_some() {
            self.codex_remove(&codex).await?;
        }
        if let Err(error) = self.codex_add(&codex, &bridge).await {
            let rollback = self.rollback_codex(&codex, &bridge, current.as_ref()).await;
            return Err(with_rollback_context(error, rollback));
        }
        let verified = self
            .codex_configuration(&codex)
            .await
            .and_then(|configured| {
                if configured.as_ref().is_some_and(|configuration| {
                    configuration.is_exact(&bridge, ChatClientKind::ChatGptDesktop)
                }) {
                    Ok(())
                } else {
                    bail!("ChatGPT no confirmó la configuración instalada")
                }
            });
        if let Err(error) = verified {
            let rollback = self.rollback_codex(&codex, &bridge, current.as_ref()).await;
            return Err(with_rollback_context(error, rollback));
        }
        Ok(())
    }

    async fn disconnect_chatgpt(&self) -> Result<()> {
        let codex = self.find_codex().context("no se encontró ChatGPT/Codex")?;
        let current = self.codex_configuration(&codex).await?;
        let Some(current) = current else {
            return Ok(());
        };
        if !self
            .configuration_is_securely_managed(&current, ChatClientKind::ChatGptDesktop)
            .await?
        {
            bail!("la entrada airwiki no pertenece a esta aplicación")
        }
        self.codex_remove(&codex).await
    }

    fn find_codex(&self) -> Option<PathBuf> {
        if !self.environment.discover_host_clients {
            return None;
        }
        let mut candidates = program_candidates("codex", &self.environment.path_entries);
        if self.environment.platform == HostPlatform::MacOs {
            candidates.extend([
                PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"),
                self.environment
                    .home
                    .join("Applications/ChatGPT.app/Contents/Resources/codex"),
            ]);
        }
        candidates.into_iter().find(|path| path.is_file())
    }

    async fn codex_supported(&self, codex: &Path) -> Result<bool> {
        let output = self
            .runner
            .run(CommandSpec::new(codex.to_path_buf()).args(["mcp", "get", "--help"]))
            .await?;
        Ok(output.success && output.stdout_text()?.contains("--json"))
    }

    async fn codex_configuration(&self, codex: &Path) -> Result<Option<ManagedConfiguration>> {
        let output = self
            .runner
            .run(CommandSpec::new(codex.to_path_buf()).args([
                "mcp",
                "get",
                INTEGRATION_NAME,
                "--json",
            ]))
            .await?;
        if !output.success {
            if codex_reports_missing(output.stderr_text()?) {
                return Ok(None);
            }
            bail!("ChatGPT no pudo leer la integración MCP existente")
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .context("ChatGPT devolvió una configuración MCP inválida")?;
        Ok(Some(parse_codex_configuration(&value)))
    }

    async fn codex_add(&self, codex: &Path, bridge: &Path) -> Result<()> {
        self.codex_add_configuration(
            codex,
            &ManagedConfiguration::new(bridge.to_path_buf(), ChatClientKind::ChatGptDesktop),
        )
        .await
    }

    async fn codex_add_configuration(
        &self,
        codex: &Path,
        configuration: &ManagedConfiguration,
    ) -> Result<()> {
        let mut args = vec![
            OsString::from("mcp"),
            OsString::from("add"),
            OsString::from(INTEGRATION_NAME),
            OsString::from("--"),
            configuration.command.as_os_str().to_owned(),
        ];
        args.extend(configuration.args.iter().cloned().map(OsString::from));
        let output = self
            .runner
            .run(CommandSpec::new(codex.to_path_buf()).args(args))
            .await?;
        if !output.success {
            bail!("ChatGPT no pudo guardar la integración")
        }
        Ok(())
    }

    async fn codex_remove(&self, codex: &Path) -> Result<()> {
        let output = self
            .runner
            .run(CommandSpec::new(codex.to_path_buf()).args(["mcp", "remove", INTEGRATION_NAME]))
            .await?;
        if !output.success {
            bail!("ChatGPT no pudo quitar la integración")
        }
        Ok(())
    }

    async fn rollback_codex(
        &self,
        codex: &Path,
        attempted_bridge: &Path,
        previous: Option<&ManagedConfiguration>,
    ) -> Result<()> {
        match self.codex_configuration(codex).await? {
            Some(configuration)
                if configuration.is_exact(attempted_bridge, ChatClientKind::ChatGptDesktop) =>
            {
                self.codex_remove(codex).await?;
            }
            Some(_) => bail!("la configuración de ChatGPT cambió durante la recuperación"),
            None => {}
        }
        if let Some(previous) = previous {
            self.codex_add_configuration(codex, previous).await?;
        }
        Ok(())
    }

    async fn inspect_claude_code(&self) -> Result<IntegrationView> {
        let Some(claude) = self.find_claude_code() else {
            return Ok(view(
                ChatClientKind::ClaudeCode,
                IntegrationStatus::NotInstalled,
                "Instala Claude Code para habilitar esta integración.",
                None,
                Some(self.managed_bridge_path()),
            ));
        };
        let (supported, detected_version) = tokio::join!(
            self.claude_code_supported(&claude),
            self.program_version(&claude)
        );
        if !supported? {
            return Ok(view(
                ChatClientKind::ClaudeCode,
                IntegrationStatus::Unsupported,
                "La versión detectada no admite MCP stdio de alcance de usuario.",
                detected_version,
                Some(self.managed_bridge_path()),
            ));
        }
        let configured = self.claude_code_configuration(&claude).await?;
        let (status, detail) = self
            .classify_configuration_securely(configured.as_ref(), ChatClientKind::ClaudeCode)
            .await?;
        Ok(view(
            ChatClientKind::ClaudeCode,
            status,
            detail,
            detected_version,
            Some(self.managed_bridge_path()),
        ))
    }

    async fn connect_claude_code(&self) -> Result<()> {
        let claude = self
            .find_claude_code()
            .context("no se encontró Claude Code")?;
        if !self.claude_code_supported(&claude).await? {
            bail!("actualiza Claude Code antes de conectar AirWiki")
        }
        let current = self.claude_code_configuration(&claude).await?;
        self.ensure_replaceable(current.as_ref(), ChatClientKind::ClaudeCode)
            .await?;
        let bridge = self.materialize_bridge().await?;
        self.verify_bridge(&bridge, ChatClientKind::ClaudeCode)
            .await?;
        if current.as_ref().is_some_and(|configuration| {
            configuration.is_exact(&bridge, ChatClientKind::ClaudeCode)
        }) {
            return Ok(());
        }
        if current.is_some() {
            self.claude_code_remove(&claude).await?;
        }
        if let Err(error) = self.claude_code_add(&claude, &bridge).await {
            let rollback = self
                .rollback_claude_code(&claude, &bridge, current.as_ref())
                .await;
            return Err(with_rollback_context(error, rollback));
        }
        let verified = self
            .claude_code_configuration(&claude)
            .await
            .and_then(|configured| {
                if configured.as_ref().is_some_and(|configuration| {
                    configuration.is_exact(&bridge, ChatClientKind::ClaudeCode)
                }) {
                    Ok(())
                } else {
                    bail!("Claude Code no confirmó la configuración instalada")
                }
            });
        if let Err(error) = verified {
            let rollback = self
                .rollback_claude_code(&claude, &bridge, current.as_ref())
                .await;
            return Err(with_rollback_context(error, rollback));
        }
        Ok(())
    }

    async fn disconnect_claude_code(&self) -> Result<()> {
        let claude = self
            .find_claude_code()
            .context("no se encontró Claude Code")?;
        let Some(current) = self.claude_code_configuration(&claude).await? else {
            return Ok(());
        };
        if !self
            .configuration_is_securely_managed(&current, ChatClientKind::ClaudeCode)
            .await?
        {
            bail!("la entrada airwiki no pertenece a esta aplicación")
        }
        self.claude_code_remove(&claude).await
    }

    fn find_claude_code(&self) -> Option<PathBuf> {
        if !self.environment.discover_host_clients {
            return None;
        }
        find_program("claude", &self.environment.path_entries)
    }

    async fn claude_code_supported(&self, claude: &Path) -> Result<bool> {
        let output = self
            .runner
            .run(CommandSpec::new(claude.to_path_buf()).args(["mcp", "add", "--help"]))
            .await?;
        let help = output.stdout_text()?;
        Ok(output.success && help.contains("--scope") && help.contains("--transport"))
    }

    async fn claude_code_configuration(
        &self,
        claude: &Path,
    ) -> Result<Option<ManagedConfiguration>> {
        let output = self
            .runner
            .run(home_environment(
                CommandSpec::new(claude.to_path_buf()).args(["mcp", "get", INTEGRATION_NAME]),
                &self.environment.home,
            ))
            .await?;
        if !output.success {
            if claude_code_reports_missing(output.stdout_text()?, output.stderr_text()?) {
                return Ok(None);
            }
            bail!("Claude Code no pudo leer la integración MCP existente")
        }
        Ok(Some(parse_claude_code_configuration(output.stdout_text()?)))
    }

    async fn claude_code_add(&self, claude: &Path, bridge: &Path) -> Result<()> {
        self.claude_code_add_configuration(
            claude,
            &ManagedConfiguration::new(bridge.to_path_buf(), ChatClientKind::ClaudeCode),
        )
        .await
    }

    async fn claude_code_add_configuration(
        &self,
        claude: &Path,
        configuration: &ManagedConfiguration,
    ) -> Result<()> {
        let mut args = vec![
            OsString::from("mcp"),
            OsString::from("add"),
            OsString::from("--scope"),
            OsString::from("user"),
            OsString::from("--transport"),
            OsString::from("stdio"),
            OsString::from(INTEGRATION_NAME),
            OsString::from("--"),
            configuration.command.as_os_str().to_owned(),
        ];
        args.extend(configuration.args.iter().cloned().map(OsString::from));
        let output = self
            .runner
            .run(home_environment(
                CommandSpec::new(claude.to_path_buf()).args(args),
                &self.environment.home,
            ))
            .await?;
        if !output.success {
            bail!("Claude Code no pudo guardar la integración")
        }
        Ok(())
    }

    async fn claude_code_remove(&self, claude: &Path) -> Result<()> {
        let output = self
            .runner
            .run(home_environment(
                CommandSpec::new(claude.to_path_buf()).args([
                    "mcp",
                    "remove",
                    "--scope",
                    "user",
                    INTEGRATION_NAME,
                ]),
                &self.environment.home,
            ))
            .await?;
        if !output.success {
            bail!("Claude Code no pudo quitar la integración")
        }
        Ok(())
    }

    async fn rollback_claude_code(
        &self,
        claude: &Path,
        attempted_bridge: &Path,
        previous: Option<&ManagedConfiguration>,
    ) -> Result<()> {
        match self.claude_code_configuration(claude).await? {
            Some(configuration)
                if configuration.is_exact(attempted_bridge, ChatClientKind::ClaudeCode) =>
            {
                self.claude_code_remove(claude).await?;
            }
            Some(_) => bail!("la configuración de Claude Code cambió durante la recuperación"),
            None => {}
        }
        if let Some(previous) = previous {
            self.claude_code_add_configuration(claude, previous).await?;
        }
        Ok(())
    }

    async fn inspect_gemini(&self) -> Result<IntegrationView> {
        let Some(gemini) = find_program("gemini", &self.environment.path_entries) else {
            return Ok(view(
                ChatClientKind::GeminiCli,
                IntegrationStatus::NotInstalled,
                "Instala Gemini CLI para habilitar esta integración.",
                None,
                Some(self.managed_bridge_path()),
            ));
        };
        if !self.gemini_supported(&gemini).await? {
            return Ok(view(
                ChatClientKind::GeminiCli,
                IntegrationStatus::Unsupported,
                "La versión detectada no admite MCP stdio de alcance de usuario.",
                self.program_version(&gemini).await,
                Some(self.managed_bridge_path()),
            ));
        }
        let configured = self.gemini_configuration(&self.environment.home).await?;
        let (status, detail) = self
            .classify_configuration_securely(configured.as_ref(), ChatClientKind::GeminiCli)
            .await?;
        Ok(view(
            ChatClientKind::GeminiCli,
            status,
            detail,
            self.program_version(&gemini).await,
            Some(self.managed_bridge_path()),
        ))
    }

    async fn inspect_generic_mcp(&self) -> Result<IntegrationView> {
        let planned_path = self.managed_bridge_path();
        if self.bundled_bridge().is_none() {
            return Ok(view(
                ChatClientKind::GenericMcp,
                IntegrationStatus::NotInstalled,
                "El paquete no contiene el puente MCP para esta plataforma.",
                None,
                Some(planned_path),
            ));
        }
        if !self.generic_capability_is_active().await? {
            return Ok(view(
                ChatClientKind::GenericMcp,
                IntegrationStatus::Available,
                "Activa el arnés genérico para obtener su configuración MCP local.",
                None,
                Some(planned_path),
            ));
        }
        let configuration =
            ManagedConfiguration::new(planned_path.clone(), ChatClientKind::GenericMcp);
        if !self
            .configuration_is_securely_managed(&configuration, ChatClientKind::GenericMcp)
            .await?
        {
            return Ok(view(
                ChatClientKind::GenericMcp,
                IntegrationStatus::UpdateAvailable,
                "El puente MCP administrado debe instalarse o repararse antes de usarlo.",
                None,
                Some(planned_path),
            ));
        }
        Ok(view(
            ChatClientKind::GenericMcp,
            IntegrationStatus::Configured,
            "Copia esta configuración en cualquier cliente MCP stdio compatible.",
            None,
            Some(planned_path),
        ))
    }

    async fn generic_capability_is_active(&self) -> Result<bool> {
        let path = self.capability_path(ChatClientKind::GenericMcp);
        if !regular_file(&path)? {
            return Ok(false);
        }
        let parent = path
            .parent()
            .context("la capacidad no tiene un directorio padre")?;
        if path_contains_link_or_reparse_point(parent).await? {
            return Ok(false);
        }
        let metadata = fs::symlink_metadata(&path)
            .await
            .context("no se pudo inspeccionar la capacidad privada")?;
        if metadata.len() > 256 || !private_permissions_are_valid(&metadata) {
            return Ok(false);
        }
        let secret = fs::read_to_string(path)
            .await
            .context("no se pudo leer la capacidad privada de la integración")?;
        Ok(self
            .database
            .authenticate_application_capability(secret.trim())?
            .is_some_and(|capability| {
                capability.app_id == application_id(ChatClientKind::GenericMcp)
            }))
    }

    async fn connect_generic_mcp(&self) -> Result<()> {
        let bridge = self.materialize_bridge().await?;
        self.verify_bridge(&bridge, ChatClientKind::GenericMcp)
            .await
    }

    async fn connect_gemini(&self) -> Result<()> {
        let gemini = find_program("gemini", &self.environment.path_entries)
            .context("no se encontró Gemini CLI")?;
        if !self.gemini_supported(&gemini).await? {
            bail!("actualiza Gemini CLI antes de conectar AirWiki")
        }
        let syntax = self.probe_gemini(&gemini).await?;
        let current = self.gemini_configuration(&self.environment.home).await?;
        self.ensure_replaceable(current.as_ref(), ChatClientKind::GeminiCli)
            .await?;
        let bridge = self.materialize_bridge().await?;
        self.verify_bridge(&bridge, ChatClientKind::GeminiCli)
            .await?;
        if current
            .as_ref()
            .is_some_and(|configuration| configuration.is_exact(&bridge, ChatClientKind::GeminiCli))
        {
            return Ok(());
        }
        if current.is_some() {
            self.gemini_remove(&gemini, &self.environment.home).await?;
        }
        if let Err(error) = self
            .gemini_add(&gemini, &bridge, &self.environment.home, syntax)
            .await
        {
            let rollback = self
                .rollback_gemini(&gemini, &bridge, current.as_ref(), syntax)
                .await;
            return Err(with_rollback_context(error, rollback));
        }
        let verified = self
            .gemini_configuration(&self.environment.home)
            .await
            .and_then(|configured| {
                if configured.as_ref().is_some_and(|configuration| {
                    configuration.is_exact(&bridge, ChatClientKind::GeminiCli)
                }) {
                    Ok(())
                } else {
                    bail!("Gemini CLI no confirmó la configuración instalada")
                }
            });
        if let Err(error) = verified {
            let rollback = self
                .rollback_gemini(&gemini, &bridge, current.as_ref(), syntax)
                .await;
            return Err(with_rollback_context(error, rollback));
        }
        Ok(())
    }

    async fn disconnect_gemini(&self) -> Result<()> {
        let gemini = find_program("gemini", &self.environment.path_entries)
            .context("no se encontró Gemini CLI")?;
        let current = self.gemini_configuration(&self.environment.home).await?;
        let Some(current) = current else {
            return Ok(());
        };
        if !self
            .configuration_is_securely_managed(&current, ChatClientKind::GeminiCli)
            .await?
        {
            bail!("la entrada airwiki no pertenece a esta aplicación")
        }
        self.gemini_remove(&gemini, &self.environment.home).await
    }

    async fn gemini_supported(&self, gemini: &Path) -> Result<bool> {
        let output = self
            .runner
            .run(CommandSpec::new(gemini.to_path_buf()).args(["mcp", "add", "--help"]))
            .await?;
        let help = output.stdout_text()?;
        Ok(output.success
            && help.contains("--scope")
            && help.contains("--transport")
            && help.contains("--include-tools"))
    }

    async fn probe_gemini(&self, gemini: &Path) -> Result<GeminiAddSyntax> {
        let probe_home =
            std::env::temp_dir().join(format!("airwiki-gemini-probe-{}", Uuid::new_v4()));
        fs::create_dir_all(&probe_home)
            .await
            .context("no se pudo preparar la prueba aislada de Gemini")?;
        let probe_bridge = probe_home.join(bridge_filename());
        for syntax in [
            GeminiAddSyntax::OptionsFirst,
            GeminiAddSyntax::PositionalsFirst,
        ] {
            let _ = fs::remove_dir_all(probe_home.join(".gemini")).await;
            if self
                .gemini_add(gemini, &probe_bridge, &probe_home, syntax)
                .await
                .is_ok()
                && self
                    .gemini_configuration(&probe_home)
                    .await?
                    .is_some_and(|configuration| {
                        configuration.is_exact(&probe_bridge, ChatClientKind::GeminiCli)
                    })
            {
                let _ = fs::remove_dir_all(&probe_home).await;
                return Ok(syntax);
            }
        }
        let _ = fs::remove_dir_all(&probe_home).await;
        bail!("Gemini CLI no superó la prueba aislada de configuración MCP")
    }

    async fn gemini_configuration(&self, home: &Path) -> Result<Option<ManagedConfiguration>> {
        let settings = home.join(".gemini").join("settings.json");
        let bytes = match fs::read(&settings).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("no se pudo leer la configuración de Gemini"),
        };
        if bytes.len() > MAX_PROCESS_OUTPUT {
            bail!("la configuración de Gemini excede el límite permitido")
        }
        let value: Value = serde_json::from_slice(&bytes)
            .context("la configuración de Gemini no contiene JSON válido")?;
        let Some(server) = value
            .get("mcpServers")
            .and_then(|servers| servers.get(INTEGRATION_NAME))
        else {
            return Ok(None);
        };
        Ok(Some(parse_gemini_configuration(server)))
    }

    async fn gemini_add(
        &self,
        gemini: &Path,
        bridge: &Path,
        home: &Path,
        syntax: GeminiAddSyntax,
    ) -> Result<()> {
        self.gemini_add_configuration(
            gemini,
            &ManagedConfiguration::new(bridge.to_path_buf(), ChatClientKind::GeminiCli),
            home,
            syntax,
        )
        .await
    }

    async fn gemini_add_configuration(
        &self,
        gemini: &Path,
        configuration: &ManagedConfiguration,
        home: &Path,
        syntax: GeminiAddSyntax,
    ) -> Result<()> {
        let args = gemini_add_args(configuration, syntax);
        let output = self
            .runner
            .run(home_environment(
                CommandSpec::new(gemini.to_path_buf()).args(args),
                home,
            ))
            .await?;
        if !output.success {
            bail!("Gemini CLI no pudo guardar la integración")
        }
        Ok(())
    }

    async fn gemini_remove(&self, gemini: &Path, home: &Path) -> Result<()> {
        let output = self
            .runner
            .run(home_environment(
                CommandSpec::new(gemini.to_path_buf()).args([
                    "mcp",
                    "remove",
                    "--scope",
                    "user",
                    INTEGRATION_NAME,
                ]),
                home,
            ))
            .await?;
        if !output.success {
            bail!("Gemini CLI no pudo quitar la integración")
        }
        Ok(())
    }

    async fn rollback_gemini(
        &self,
        gemini: &Path,
        attempted_bridge: &Path,
        previous: Option<&ManagedConfiguration>,
        syntax: GeminiAddSyntax,
    ) -> Result<()> {
        match self.gemini_configuration(&self.environment.home).await? {
            Some(configuration)
                if configuration.is_exact(attempted_bridge, ChatClientKind::GeminiCli) =>
            {
                self.gemini_remove(gemini, &self.environment.home).await?;
            }
            Some(_) => bail!("la configuración de Gemini cambió durante la recuperación"),
            None => {}
        }
        if let Some(previous) = previous {
            self.gemini_add_configuration(gemini, previous, &self.environment.home, syntax)
                .await?;
        }
        Ok(())
    }

    async fn inspect_claude(&self) -> Result<IntegrationView> {
        let Some(application) = self.find_claude() else {
            return Ok(view(
                ChatClientKind::ClaudeDesktop,
                IntegrationStatus::NotInstalled,
                "Instala Claude Desktop para abrir el paquete MCPB.",
                None,
                self.bundled_claude_mcpb(),
            ));
        };
        let Some(bundle) = self.bundled_claude_mcpb() else {
            return Ok(view(
                ChatClientKind::ClaudeDesktop,
                IntegrationStatus::Error,
                "La instalación de AirWiki no contiene el paquete MCPB para esta plataforma.",
                None,
                None,
            ));
        };
        let mut result = view(
            ChatClientKind::ClaudeDesktop,
            IntegrationStatus::Available,
            "Claude mostrará su confirmación oficial antes de instalar la extensión local.",
            None,
            Some(bundle),
        );
        result.restart_required = false;
        if !application.exists() {
            result.status = IntegrationStatus::NotInstalled;
        }
        Ok(result)
    }

    async fn open_claude_bundle(&self) -> Result<()> {
        self.find_claude()
            .context("no se encontró Claude Desktop")?;
        let bundle = self
            .bundled_claude_mcpb()
            .context("el paquete MCPB de Claude no está incluido")?;
        self.opener.open(&bundle).await
    }

    async fn open_claude_settings(&self) -> Result<()> {
        let application = self
            .find_claude()
            .context("no se encontró Claude Desktop")?;
        self.opener.open(&application).await
    }

    fn find_claude(&self) -> Option<PathBuf> {
        if !self.environment.discover_host_clients {
            return None;
        }
        let candidates = match self.environment.platform {
            HostPlatform::MacOs => vec![
                PathBuf::from("/Applications/Claude.app"),
                self.environment.home.join("Applications/Claude.app"),
            ],
            HostPlatform::Windows => {
                let mut paths = Vec::new();
                if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
                    let base = PathBuf::from(local_app_data);
                    paths.push(base.join("Programs/Claude/Claude.exe"));
                    paths.push(base.join("AnthropicClaude/Claude.exe"));
                }
                paths
            }
            HostPlatform::Unsupported => Vec::new(),
        };
        candidates.into_iter().find(|path| path.exists())
    }

    async fn program_version(&self, executable: &Path) -> Option<String> {
        let output = self
            .runner
            .run(CommandSpec::new(executable.to_path_buf()).args(["--version"]))
            .await
            .ok()?;
        if !output.success {
            return None;
        }
        output
            .stdout_text()
            .ok()
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_owned)
    }

    async fn ensure_replaceable(
        &self,
        configuration: Option<&ManagedConfiguration>,
        client: ChatClientKind,
    ) -> Result<()> {
        if let Some(configuration) = configuration
            && !self
                .configuration_is_securely_managed(configuration, client)
                .await?
        {
            bail!("ya existe una entrada airwiki que no pertenece a esta aplicación")
        }
        Ok(())
    }

    async fn classify_configuration_securely(
        &self,
        configuration: Option<&ManagedConfiguration>,
        client: ChatClientKind,
    ) -> Result<(IntegrationStatus, &'static str)> {
        if let Some(configuration) = configuration
            && configuration.is_managed(&self.managed_bridge_root(), client)
            && !self
                .configuration_is_securely_managed(configuration, client)
                .await?
        {
            return Ok((
                IntegrationStatus::Conflict,
                "La ruta administrada no superó la validación de integridad; no se modificará.",
            ));
        }
        Ok(classify_configuration(
            configuration,
            &self.managed_bridge_path(),
            &self.managed_bridge_root(),
            client,
        ))
    }

    async fn configuration_is_securely_managed(
        &self,
        configuration: &ManagedConfiguration,
        client: ChatClientKind,
    ) -> Result<bool> {
        let root = self.managed_bridge_root();
        if !configuration.is_managed(&root, client)
            || path_contains_link_or_reparse_point(&root).await?
            || path_contains_link_or_reparse_point(&configuration.command).await?
        {
            return Ok(false);
        }
        let canonical_root = match fs::canonicalize(&root).await {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("no se pudo validar la raíz administrada"),
        };
        let canonical_command = match fs::canonicalize(&configuration.command).await {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("no se pudo validar el puente configurado"),
        };
        if !path_is_beneath(&canonical_command, &canonical_root)
            || !executable_regular_file(&canonical_command)?
        {
            return Ok(false);
        }
        if paths_equal(&configuration.command, &self.managed_bridge_path()) {
            let Some(bundled) = self.bundled_bridge() else {
                return Ok(false);
            };
            return files_equal_bounded(&bundled, &canonical_command).await;
        }
        Ok(true)
    }
}

fn application_id(client: ChatClientKind) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, client.bridge_id().as_bytes())
}

fn view(
    client: ChatClientKind,
    status: IntegrationStatus,
    detail: impl Into<String>,
    detected_version: Option<String>,
    planned_path: Option<PathBuf>,
) -> IntegrationView {
    IntegrationView {
        client,
        status,
        detected_version,
        detail: detail.into(),
        planned_path,
        activity_recent: false,
        restart_required: client.workflow_client().is_some(),
        workflow_guide: if client.workflow_client().is_some() {
            WorkflowGuideView::unsupported()
        } else {
            WorkflowGuideView::built_in()
        },
    }
}

fn with_rollback_context(operation: anyhow::Error, rollback: Result<()>) -> anyhow::Error {
    match rollback {
        Ok(()) => operation.context("se restauró la configuración anterior"),
        Err(rollback_error) => {
            anyhow::anyhow!("{operation:#}; además falló la recuperación: {rollback_error:#}")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedConfiguration {
    command: PathBuf,
    args: Vec<String>,
    parse_conflict: bool,
}

fn codex_reports_missing(stderr: &str) -> bool {
    let normalized = stderr.trim();
    normalized.contains("No MCP server named 'airwiki' found")
        || normalized.contains("No MCP server named \"airwiki\" found")
}

fn claude_code_reports_missing(stdout: &str, stderr: &str) -> bool {
    [stdout, stderr].iter().any(|output| {
        output.contains("No MCP server named \"airwiki\"")
            || output.contains("No MCP server named 'airwiki'")
    })
}

fn parse_claude_code_configuration(output: &str) -> ManagedConfiguration {
    let mut scope_is_user = false;
    let mut type_is_stdio = false;
    let mut command = None;
    let mut args = None;
    let mut environment_present = false;
    let mut environment_block = false;
    for raw_line in output.lines() {
        let line = raw_line.trim();
        if environment_block {
            if line.is_empty() {
                environment_block = false;
                continue;
            }
            if raw_line.starts_with(' ') || raw_line.starts_with('\t') {
                environment_present = true;
                environment_block = false;
                continue;
            }
            environment_block = false;
        }
        if let Some(value) = line.strip_prefix("Scope:") {
            scope_is_user = value.trim_start().starts_with("User config");
        } else if let Some(value) = line.strip_prefix("Type:") {
            type_is_stdio = value.trim() == "stdio";
        } else if let Some(value) = line.strip_prefix("Command:") {
            command = Some(PathBuf::from(value.trim()));
        } else if let Some(value) = line.strip_prefix("Args:") {
            args = Some(
                value
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            );
        } else if let Some(value) = line.strip_prefix("Environment:") {
            environment_present = !value.trim().is_empty();
            environment_block = !environment_present;
        }
    }
    if !scope_is_user || !type_is_stdio || environment_present {
        return ManagedConfiguration::conflict();
    }
    let Some(command) = command else {
        return ManagedConfiguration::conflict();
    };
    ManagedConfiguration {
        command,
        args: args.unwrap_or_default(),
        parse_conflict: false,
    }
}

fn parse_codex_configuration(value: &Value) -> ManagedConfiguration {
    const TOP_LEVEL_KEYS: &[&str] = &[
        "name",
        "enabled",
        "disabled_reason",
        "transport",
        "enabled_tools",
        "disabled_tools",
        "startup_timeout_sec",
        "tool_timeout_sec",
    ];
    const TRANSPORT_KEYS: &[&str] = &["type", "command", "args", "env", "env_vars", "cwd"];
    if !object_has_exact_keys(value, TOP_LEVEL_KEYS)
        || value.get("name").and_then(Value::as_str) != Some(INTEGRATION_NAME)
        || value.get("enabled").and_then(Value::as_bool) != Some(true)
        || !value_is_null(value, "disabled_reason")
        || !value_is_null(value, "enabled_tools")
        || !value_is_null(value, "disabled_tools")
        || !value_is_null(value, "startup_timeout_sec")
        || !value_is_null(value, "tool_timeout_sec")
    {
        return ManagedConfiguration::conflict();
    }
    let Some(transport) = value.get("transport") else {
        return ManagedConfiguration::conflict();
    };
    if !object_has_exact_keys(transport, TRANSPORT_KEYS)
        || transport.get("type").and_then(Value::as_str) != Some("stdio")
        || !value_is_null(transport, "env")
        || transport
            .get("env_vars")
            .and_then(Value::as_array)
            .is_none_or(|values| !values.is_empty())
        || !value_is_null(transport, "cwd")
    {
        return ManagedConfiguration::conflict();
    }
    ManagedConfiguration::from_json(transport).unwrap_or_else(|_| ManagedConfiguration::conflict())
}

fn object_has_exact_keys(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
    })
}

fn value_is_null(value: &Value, key: &str) -> bool {
    value.get(key) == Some(&Value::Null)
}

fn parse_gemini_configuration(value: &Value) -> ManagedConfiguration {
    if !object_has_exact_keys(value, &["command", "args", "includeTools"])
        || value
            .get("includeTools")
            .and_then(Value::as_array)
            .is_none_or(|tools| {
                let actual = tools.iter().filter_map(Value::as_str).collect::<Vec<_>>();
                actual.as_slice() != MANAGED_TOOLS
            })
    {
        return ManagedConfiguration::conflict();
    }
    ManagedConfiguration::from_json(value).unwrap_or_else(|_| ManagedConfiguration::conflict())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeminiAddSyntax {
    OptionsFirst,
    PositionalsFirst,
}

fn gemini_add_args(configuration: &ManagedConfiguration, syntax: GeminiAddSyntax) -> Vec<OsString> {
    let mut options = vec![
        OsString::from("--scope"),
        OsString::from("user"),
        OsString::from("--transport"),
        OsString::from("stdio"),
        OsString::from("--include-tools"),
        OsString::from(MANAGED_TOOLS.join(",")),
    ];
    let mut args = vec![OsString::from("mcp"), OsString::from("add")];
    match syntax {
        GeminiAddSyntax::OptionsFirst => {
            args.append(&mut options);
            args.push(OsString::from(INTEGRATION_NAME));
            args.push(configuration.command.as_os_str().to_owned());
            args.push(OsString::from("--"));
            args.extend(configuration.args.iter().cloned().map(OsString::from));
        }
        GeminiAddSyntax::PositionalsFirst => {
            args.push(OsString::from(INTEGRATION_NAME));
            args.push(configuration.command.as_os_str().to_owned());
            args.extend(configuration.args.iter().cloned().map(OsString::from));
            args.append(&mut options);
        }
    }
    args
}

impl ManagedConfiguration {
    fn new(command: PathBuf, client: ChatClientKind) -> Self {
        Self {
            command,
            args: vec!["--client".to_owned(), client.bridge_id().to_owned()],
            parse_conflict: false,
        }
    }

    fn conflict() -> Self {
        Self {
            command: PathBuf::new(),
            args: Vec::new(),
            parse_conflict: true,
        }
    }

    fn from_json(value: &Value) -> Result<Self> {
        let command = value
            .get("command")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .context("la configuración MCP no contiene un comando")?;
        let args = value
            .get("args")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_owned)
                            .context("la configuración MCP contiene argumentos inválidos")
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            command,
            args,
            parse_conflict: false,
        })
    }

    fn is_exact(&self, bridge: &Path, client: ChatClientKind) -> bool {
        !self.parse_conflict
            && paths_equal(&self.command, bridge)
            && self.args == ["--client", client.bridge_id()]
    }

    fn is_managed(&self, managed_root: &Path, client: ChatClientKind) -> bool {
        !self.parse_conflict
            && path_is_beneath(&self.command, managed_root)
            && self
                .command
                .file_name()
                .is_some_and(|name| name == OsStr::new(bridge_filename()))
            && self.args == ["--client", client.bridge_id()]
    }
}

fn classify_configuration(
    configured: Option<&ManagedConfiguration>,
    expected_bridge: &Path,
    managed_root: &Path,
    client: ChatClientKind,
) -> (IntegrationStatus, &'static str) {
    match configured {
        None => (
            IntegrationStatus::Available,
            "Cliente detectado; listo para conectar con confirmación.",
        ),
        Some(configuration) if configuration.is_exact(expected_bridge, client) => (
            IntegrationStatus::Configured,
            "Configuración administrada instalada.",
        ),
        Some(configuration) if configuration.is_managed(managed_root, client) => (
            IntegrationStatus::UpdateAvailable,
            "La configuración usa una versión anterior del puente.",
        ),
        Some(_) => (
            IntegrationStatus::Conflict,
            "Ya existe una entrada airwiki distinta; no se modificará automáticamente.",
        ),
    }
}

fn verify_tools_list(stdout: &str) -> Result<()> {
    let mut found_discovery = false;
    let mut found_tools = false;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match message.get("id").and_then(Value::as_u64) {
            Some(1) => {
                let result = message
                    .get("result")
                    .context("server/discover no devolvió un resultado")?;
                let supports_current = result
                    .get("supportedVersions")
                    .and_then(Value::as_array)
                    .is_some_and(|versions| {
                        versions
                            .iter()
                            .any(|version| version.as_str() == Some(MCP_PROTOCOL_VERSION))
                    });
                found_discovery = supports_current
                    && result.get("resultType").and_then(Value::as_str) == Some("complete")
                    && result.get("ttlMs").and_then(Value::as_u64) == Some(0)
                    && result.get("cacheScope").and_then(Value::as_str) == Some("private");
            }
            Some(2) => {
                let result = message
                    .get("result")
                    .context("tools/list no devolvió un resultado")?;
                ensure!(
                    result.get("resultType").and_then(Value::as_str) == Some("complete"),
                    "tools/list no devolvió un resultado MCP completo"
                );
                ensure!(
                    result.get("ttlMs").and_then(Value::as_u64) == Some(0)
                        && result.get("cacheScope").and_then(Value::as_str) == Some("private"),
                    "tools/list no protegió el conjunto de herramientas por contexto"
                );
                let tools = result
                    .get("tools")
                    .and_then(Value::as_array)
                    .context("tools/list no devolvió herramientas")?;
                for tool in tools {
                    let name = tool
                        .get("name")
                        .and_then(Value::as_str)
                        .context("tools/list devolvió una herramienta sin nombre")?;
                    ensure!(
                        tool.get("title")
                            .and_then(Value::as_str)
                            .is_some_and(|value| !value.is_empty())
                            && tool
                                .get("description")
                                .and_then(Value::as_str)
                                .is_some_and(|value| !value.is_empty()),
                        "la herramienta {name} no tiene metadata comprensible"
                    );
                    ensure!(
                        tool.pointer("/inputSchema/type").and_then(Value::as_str) == Some("object")
                            && tool.get("outputSchema").is_some_and(Value::is_object),
                        "la herramienta {name} no expuso schemas tipados"
                    );
                    let (read_only, destructive, idempotent) = expected_tool_hints(name)
                        .context("tools/list devolvió una herramienta no administrada")?;
                    ensure!(
                        tool.pointer("/annotations/readOnlyHint")
                            .and_then(Value::as_bool)
                            == Some(read_only)
                            && tool
                                .pointer("/annotations/destructiveHint")
                                .and_then(Value::as_bool)
                                == Some(destructive)
                            && tool
                                .pointer("/annotations/idempotentHint")
                                .and_then(Value::as_bool)
                                == Some(idempotent)
                            && tool
                                .pointer("/annotations/openWorldHint")
                                .and_then(Value::as_bool)
                                == Some(false),
                        "la herramienta {name} no expuso anotaciones de seguridad correctas"
                    );
                }
                let mut actual = tools
                    .iter()
                    .filter_map(|tool| tool.get("name"))
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>();
                let mut expected = MANAGED_TOOLS;
                actual.sort_unstable();
                expected.sort_unstable();
                found_tools = actual.as_slice() == expected;
            }
            _ => {}
        }
    }
    if !found_discovery || !found_tools {
        bail!("el puente no expuso exactamente las herramientas administradas esperadas")
    }
    Ok(())
}

fn expected_tool_hints(name: &str) -> Option<(bool, bool, bool)> {
    match name {
        SEARCH_TOOL
        | "list_airwiki_memories"
        | "get_airwiki_memory"
        | "get_airwiki_computation_run" => Some((true, false, true)),
        "create_airwiki_memory" | "request_airwiki_computation" => Some((false, false, false)),
        "write_airwiki_memory" | "deprecate_airwiki_memory" => Some((false, true, false)),
        _ => None,
    }
}

fn bridge_filename() -> &'static str {
    if cfg!(windows) {
        "airwiki-mcp-bridge.exe"
    } else {
        BRIDGE_BASENAME
    }
}

fn program_candidates(name: &str, path_entries: &[PathBuf]) -> Vec<PathBuf> {
    path_entries
        .iter()
        .flat_map(|directory| {
            let plain = directory.join(name);
            if cfg!(windows) {
                vec![plain.clone(), plain.with_extension("exe")]
            } else {
                vec![plain]
            }
        })
        .collect()
}

fn find_program(name: &str, path_entries: &[PathBuf]) -> Option<PathBuf> {
    program_candidates(name, path_entries)
        .into_iter()
        .find(|path| path.is_file())
}

fn home_environment(spec: CommandSpec, home: &Path) -> CommandSpec {
    spec.environment("HOME", home.as_os_str())
        .environment("USERPROFILE", home.as_os_str())
}

fn regular_file(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            Ok(metadata.file_type().is_file() && !metadata_is_link_or_reparse_point(&metadata))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("no se pudo inspeccionar un recurso de integración"),
    }
}

fn ensure_regular_path(path: &Path) -> Result<()> {
    if !regular_file(path)? {
        bail!("el recurso de integración no es un archivo regular")
    }
    Ok(())
}

fn executable_regular_file(path: &Path) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("no se pudo inspeccionar el puente MCP"),
    };
    if !metadata.file_type().is_file() || metadata_is_link_or_reparse_point(&metadata) {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        Ok(metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        Ok(true)
    }
}

#[cfg(unix)]
async fn set_executable_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .await
        .context("no se pudieron aplicar permisos al puente")
}

#[cfg(not(unix))]
async fn set_executable_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

async fn files_equal_bounded(left: &Path, right: &Path) -> Result<bool> {
    let (left_bytes, right_bytes) =
        tokio::try_join!(read_file_bounded(left), read_file_bounded(right))?;
    Ok(left_bytes == right_bytes)
}

async fn read_file_bounded(path: &Path) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path)
        .await
        .with_context(|| format!("no se pudo abrir el recurso {}", path.display()))?;
    let before = file
        .metadata()
        .await
        .context("no se pudo inspeccionar el recurso de integración")?;
    if !before.is_file() || before.len() > MAX_BRIDGE_BYTES {
        bail!("el puente MCP no es regular o excede el tamaño máximo permitido")
    }
    let capacity = usize::try_from(before.len()).context("el puente MCP es demasiado grande")?;
    let mut bytes = Vec::with_capacity(capacity);
    {
        let mut bounded = (&mut file).take(MAX_BRIDGE_BYTES.saturating_add(1));
        bounded
            .read_to_end(&mut bytes)
            .await
            .context("no se pudo leer el puente MCP")?;
    }
    if bytes.len() as u64 > MAX_BRIDGE_BYTES {
        bail!("el puente MCP excede el tamaño máximo permitido")
    }
    let after = file
        .metadata()
        .await
        .context("no se pudo volver a comprobar el puente MCP")?;
    if before.len() != after.len() || after.len() != bytes.len() as u64 {
        bail!("el puente MCP cambió durante su comprobación")
    }
    Ok(bytes)
}

async fn write_bridge_atomically(
    temporary: &Path,
    destination: &Path,
    parent: &Path,
    bytes: &[u8],
) -> Result<()> {
    if bytes.len() as u64 > MAX_BRIDGE_BYTES {
        bail!("el puente MCP excede el tamaño máximo permitido")
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)
        .await
        .context("no se pudo crear el puente MCP temporal")?;
    set_executable_permissions(temporary).await?;
    file.write_all(bytes)
        .await
        .context("no se pudo copiar el puente MCP")?;
    file.flush()
        .await
        .context("no se pudo vaciar el puente MCP temporal")?;
    file.sync_all()
        .await
        .context("no se pudo sincronizar el puente MCP temporal")?;
    drop(file);
    if read_file_bounded(temporary).await? != bytes {
        bail!("la copia temporal del puente MCP no coincide con el recurso")
    }
    fs::rename(temporary, destination)
        .await
        .context("no se pudo activar atómicamente el puente MCP")?;
    sync_directory(parent).await?;
    Ok(())
}

async fn write_capability_atomically(destination: &Path, bytes: &[u8]) -> Result<()> {
    let parent = destination
        .parent()
        .context("la capacidad no tiene un directorio padre")?;
    fs::create_dir_all(parent)
        .await
        .context("no se pudo crear el directorio privado de capacidades")?;
    if path_contains_link_or_reparse_point(parent).await? {
        bail!("el directorio de capacidades contiene un enlace no permitido")
    }
    let temporary = parent.join(format!(".capability-{}.tmp", Uuid::new_v4()));
    let result = async {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await
            .context("no se pudo crear la capacidad temporal")?;
        set_private_permissions(&temporary).await?;
        file.write_all(bytes)
            .await
            .context("no se pudo escribir la capacidad privada")?;
        file.sync_all()
            .await
            .context("no se pudo sincronizar la capacidad privada")?;
        drop(file);
        fs::rename(&temporary, destination)
            .await
            .context("no se pudo activar la capacidad privada")?;
        sync_directory(parent).await
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&temporary).await;
    }
    result
}

#[cfg(unix)]
async fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .await
        .context("no se pudieron restringir los permisos de la capacidad")
}

#[cfg(unix)]
fn private_permissions_are_valid(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn private_permissions_are_valid(_metadata: &std::fs::Metadata) -> bool {
    true
}

#[cfg(not(unix))]
async fn set_private_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
async fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .await
        .context("no se pudo abrir el directorio de integraciones")?
        .sync_all()
        .await
        .context("no se pudo sincronizar el directorio de integraciones")
}

#[cfg(not(unix))]
async fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

async fn path_contains_link_or_reparse_point(path: &Path) -> Result<bool> {
    if !path.is_absolute() {
        return Ok(true);
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        // Disk prefixes such as `C:` and `\\?\C:` are not independently
        // inspectable filesystem objects. UNC and other namespaces are not
        // skipped: they must be inspected successfully or fail closed.
        #[cfg(target_os = "windows")]
        if windows_component_is_incomplete_disk_prefix(component) {
            continue;
        }
        if !current.is_absolute() {
            continue;
        }
        match fs::symlink_metadata(&current).await {
            Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(error).context("no se pudo validar la ruta administrada");
            }
        }
    }
    Ok(false)
}

#[cfg(target_os = "windows")]
fn windows_component_is_incomplete_disk_prefix(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::Prefix(prefix)
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    )
}

fn metadata_is_link_or_reparse_point(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;

        // Some name-surrogate reparse points are not reported consistently by
        // `FileType::is_symlink`, but can still redirect an apparently managed
        // path outside the per-user integration root.
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

fn path_is_beneath(path: &Path, root: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        let Some(path) = path_components_for_comparison(path, true) else {
            return false;
        };
        let Some(root) = path_components_for_comparison(root, true) else {
            return false;
        };
        path.starts_with(&root)
    }
    #[cfg(not(target_os = "windows"))]
    {
        normalize_lexically(path).is_some_and(|path| {
            normalize_lexically(root).is_some_and(|root| path.starts_with(root))
        })
    }
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        path_components_for_comparison(left, true) == path_components_for_comparison(right, true)
    }
    #[cfg(not(target_os = "windows"))]
    {
        normalize_lexically(left) == normalize_lexically(right)
    }
}

#[cfg(any(target_os = "windows", test))]
fn path_components_for_comparison(path: &Path, fold_ascii_case: bool) -> Option<Vec<String>> {
    normalize_lexically(path).map(|normalized| {
        normalized
            .components()
            .map(|component| {
                let value = component.as_os_str().to_string_lossy().into_owned();
                if fold_ascii_case {
                    value.to_ascii_lowercase()
                } else {
                    value
                }
            })
            .collect()
    })
}

fn normalize_lexically(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use std::{io::Write, sync::Mutex};

    use tempfile::TempDir;

    use super::*;

    fn exact_codex_configuration(command: &str) -> Value {
        serde_json::json!({
            "name": INTEGRATION_NAME,
            "enabled": true,
            "disabled_reason": null,
            "transport": {
                "type": "stdio",
                "command": command,
                "args": ["--client", "chatgpt-desktop"],
                "env": null,
                "env_vars": [],
                "cwd": null
            },
            "enabled_tools": null,
            "disabled_tools": null,
            "startup_timeout_sec": null,
            "tool_timeout_sec": null
        })
    }

    #[derive(Default)]
    struct RecordingRunner {
        specs: Mutex<Vec<CommandSpec>>,
        outputs: Mutex<Vec<CommandOutput>>,
    }

    #[async_trait]
    impl CommandRunner for RecordingRunner {
        async fn run(&self, spec: CommandSpec) -> Result<CommandOutput> {
            self.specs.lock().unwrap().push(spec);
            let mut outputs = self.outputs.lock().unwrap();
            if outputs.is_empty() {
                bail!("missing fake output")
            }
            Ok(outputs.remove(0))
        }
    }

    #[derive(Default)]
    struct RecordingOpener {
        paths: Mutex<Vec<PathBuf>>,
    }

    #[async_trait]
    impl PathOpener for RecordingOpener {
        async fn open(&self, path: &Path) -> Result<()> {
            self.paths.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }
    }

    fn test_platform() -> HostPlatform {
        if cfg!(target_os = "windows") {
            HostPlatform::Windows
        } else {
            HostPlatform::MacOs
        }
    }

    fn test_manager(temp: &TempDir, current_exe: PathBuf) -> ChatIntegrationManager {
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let paths = AppPaths {
            data: root.join("data"),
            database: root.join("data/airwiki.sqlite3"),
            logs: root.join("data/logs"),
            config: root.join("config/config.json"),
        };
        ChatIntegrationManager {
            paths: paths.clone(),
            environment: IntegrationEnvironment {
                platform: test_platform(),
                home: root.clone(),
                path_entries: Vec::new(),
                discover_host_clients: false,
                current_exe: current_exe.clone(),
            },
            runner: Arc::new(RecordingRunner::default()),
            opener: Arc::new(RecordingOpener::default()),
            database: airwiki_core::Database::in_memory().unwrap(),
            workflow_guides: WorkflowGuideManager::new(
                paths,
                root,
                current_exe,
                test_platform() == HostPlatform::MacOs,
                false,
            ),
        }
    }

    #[cfg(feature = "e2e")]
    #[test]
    fn e2e_integration_discovery_does_not_inspect_host_clients() {
        let temp = TempDir::new().unwrap();
        let paths = AppPaths {
            data: temp.path().join("data"),
            database: temp.path().join("data/airwiki.sqlite3"),
            logs: temp.path().join("data/logs"),
            config: temp.path().join("config/config.json"),
        };

        let environment = IntegrationEnvironment::discover(&paths).unwrap();

        assert_eq!(environment.home, paths.data.join("integration-home"));
        assert!(environment.path_entries.is_empty());
        assert!(!environment.discover_host_clients);
    }

    fn tools_list_output() -> Vec<u8> {
        let tools = MANAGED_TOOLS
            .iter()
            .map(|name| {
                let (read_only, destructive, idempotent) =
                    expected_tool_hints(name).expect("managed tool hints");
                serde_json::json!({
                    "name": name,
                    "title": format!("Fixture {name}"),
                    "description": format!("Synthetic contract for {name}"),
                    "inputSchema": { "type": "object" },
                    "outputSchema": { "type": "object" },
                    "annotations": {
                        "readOnlyHint": read_only,
                        "destructiveHint": destructive,
                        "idempotentHint": idempotent,
                        "openWorldHint": false,
                    },
                })
            })
            .collect::<Vec<_>>();
        format!(
            "{}\n{}\n",
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "resultType": "complete",
                    "supportedVersions": [MCP_PROTOCOL_VERSION],
                    "ttlMs": 0,
                    "cacheScope": "private",
                }
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "resultType": "complete",
                    "tools": tools,
                    "ttlMs": 0,
                    "cacheScope": "private",
                }
            })
        )
        .into_bytes()
    }

    fn runner_helper_spec(mode: &str) -> CommandSpec {
        CommandSpec::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "integrations::tests::system_command_runner_helper",
                "--nocapture",
            ])
            .environment("AIRWIKI_RUNNER_TEST", mode)
    }

    #[test]
    fn system_command_runner_helper() {
        match std::env::var("AIRWIKI_RUNNER_TEST").as_deref() {
            Ok("oversized") => {
                let bytes = vec![b'x'; MAX_PROCESS_OUTPUT + 1];
                std::io::stdout().write_all(&bytes).unwrap();
                std::io::stdout().flush().unwrap();
                std::process::exit(0);
            }
            Ok("bridge-oversized") => {
                let bytes = vec![b'x'; MAX_BRIDGE_VERIFY_OUTPUT + 1];
                std::io::stdout().write_all(&bytes).unwrap();
                std::io::stdout().flush().unwrap();
                std::process::exit(0);
            }
            Ok("timeout") => {
                std::thread::sleep(Duration::from_secs(5));
                std::process::exit(0);
            }
            Ok("failure") => std::process::exit(23),
            _ => {}
        }
    }

    #[tokio::test]
    async fn system_command_runner_rejects_excessive_output() {
        let error = SystemCommandRunner
            .run(runner_helper_spec("oversized"))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("excedió el límite"));
    }

    #[tokio::test]
    async fn system_command_runner_accepts_the_bounded_bridge_verification_output() {
        let output = SystemCommandRunner
            .run(runner_helper_spec("oversized").stdout_limit(MAX_BRIDGE_VERIFY_OUTPUT))
            .await
            .unwrap();

        assert!(output.stdout.len() > MAX_PROCESS_OUTPUT);
        assert!(output.stdout.len() <= MAX_BRIDGE_VERIFY_OUTPUT);
    }

    #[tokio::test]
    async fn system_command_runner_rejects_output_above_the_bridge_verification_limit() {
        let error = SystemCommandRunner
            .run(runner_helper_spec("bridge-oversized").stdout_limit(MAX_BRIDGE_VERIFY_OUTPUT))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("excedió el límite"));
    }

    #[tokio::test]
    async fn system_command_runner_enforces_timeout() {
        let error = SystemCommandRunner
            .run(runner_helper_spec("timeout").timeout(Duration::from_millis(25)))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("excedió el tiempo"));
    }

    #[tokio::test]
    async fn system_command_runner_reports_nonzero_status() {
        let output = SystemCommandRunner
            .run(runner_helper_spec("failure"))
            .await
            .unwrap();

        assert!(!output.success);
        assert!(output.stdout.len() <= MAX_PROCESS_OUTPUT);
        assert!(output._stderr.len() <= MAX_PROCESS_OUTPUT);
    }

    #[test]
    fn managed_configuration_requires_exact_client_and_managed_path() {
        let directory = TempDir::new().expect("temporary directory");
        let root = directory.path().join("data/integrations/bridge");
        let bridge = root.join("0.1.0").join(bridge_filename());
        let configuration = ManagedConfiguration::new(bridge, ChatClientKind::ChatGptDesktop);

        assert!(configuration.is_managed(&root, ChatClientKind::ChatGptDesktop));
        assert!(!configuration.is_managed(&root, ChatClientKind::GeminiCli));
    }

    #[test]
    fn managed_configuration_rejects_lexical_traversal() {
        let directory = TempDir::new().expect("temporary directory");
        let root = directory.path().join("data/integrations/bridge");
        let configuration = ManagedConfiguration::new(
            root.join("0.1.0/../../../foreign/airwiki-mcp-bridge"),
            ChatClientKind::ChatGptDesktop,
        );

        assert!(!configuration.is_managed(&root, ChatClientKind::ChatGptDesktop));
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn managed_path_rejects_windows_directory_junction() {
        use std::os::windows::fs::MetadataExt;

        let directory = TempDir::new().expect("temporary directory");
        let target = directory.path().join("junction-target");
        let junction = directory.path().join("managed-junction");
        std::fs::create_dir(&target).expect("junction target");
        let status = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("create directory junction");
        assert!(
            status.success(),
            "Windows could not create the junction fixture"
        );

        let metadata = std::fs::symlink_metadata(&junction).expect("junction metadata");
        assert!(
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0,
            "the fixture must carry the Windows reparse-point attribute"
        );
        assert!(metadata_is_link_or_reparse_point(&metadata));
        assert!(
            path_contains_link_or_reparse_point(&junction.join("0.1.0").join(bridge_filename()))
                .await
                .expect("inspect managed path")
        );

        std::fs::remove_dir(&junction).expect("remove directory junction");
        assert!(
            target.is_dir(),
            "removing the junction must not remove its target"
        );
    }

    #[test]
    fn codex_configuration_rejects_extra_or_environment_fields() {
        let mut extra = exact_codex_configuration("/data/bridge");
        extra
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_owned(), Value::Bool(true));
        let mut environment = exact_codex_configuration("/data/bridge");
        environment["transport"]["env"] = serde_json::json!({"TOKEN": "value"});

        assert!(parse_codex_configuration(&extra).parse_conflict);
        assert!(parse_codex_configuration(&environment).parse_conflict);
    }

    #[test]
    fn codex_missing_detection_does_not_hide_other_process_failures() {
        assert!(codex_reports_missing(
            "Error: No MCP server named 'airwiki' found."
        ));
        assert!(!codex_reports_missing("permission denied"));
    }

    #[test]
    fn claude_code_configuration_accepts_exact_user_stdio_output() {
        let configuration = parse_claude_code_configuration(
            "airwiki:\n  Scope: User config (available in all your projects)\n  \
             Status: Connected\n  Type: stdio\n  Command: /data/airwiki-mcp-bridge\n  \
             Args: --client claude-code\n  Environment:\n\nTo remove this server, run: \
             claude mcp remove airwiki -s user\n",
        );

        assert!(!configuration.parse_conflict);
        assert_eq!(configuration.command, Path::new("/data/airwiki-mcp-bridge"));
        assert_eq!(configuration.args, ["--client", "claude-code"]);
    }

    #[test]
    fn claude_code_configuration_rejects_other_scopes_and_environment() {
        let local = parse_claude_code_configuration(
            "Scope: Local config\nType: stdio\nCommand: /data/bridge\n\
             Args: --client claude-code\nEnvironment:\n",
        );
        let environment = parse_claude_code_configuration(
            "Scope: User config\nType: stdio\nCommand: /data/bridge\n\
             Args: --client claude-code\nEnvironment:\n  TOKEN=value\n",
        );

        assert!(local.parse_conflict);
        assert!(environment.parse_conflict);
    }

    #[test]
    fn claude_code_missing_detection_does_not_hide_other_process_failures() {
        assert!(claude_code_reports_missing(
            "No MCP server named \"airwiki\".",
            ""
        ));
        assert!(!claude_code_reports_missing("", "permission denied"));
    }

    #[tokio::test]
    async fn claude_code_add_uses_user_scoped_stdio_without_credentials() {
        let temp = TempDir::new().expect("temporary directory");
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").expect("write desktop fixture");
        let runner = Arc::new(RecordingRunner {
            specs: Mutex::new(Vec::new()),
            outputs: Mutex::new(vec![CommandOutput {
                success: true,
                stdout: Vec::new(),
                _stderr: Vec::new(),
            }]),
        });
        let mut manager = test_manager(&temp, executable);
        manager.runner = runner.clone();
        let claude = temp.path().join("claude");
        let bridge = temp.path().join("airwiki-mcp-bridge");

        manager
            .claude_code_add_configuration(
                &claude,
                &ManagedConfiguration::new(bridge.clone(), ChatClientKind::ClaudeCode),
            )
            .await
            .expect("add Claude Code configuration");

        let specs = runner.specs.lock().expect("recorded command lock");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].executable, claude);
        let expected_args = vec![
            "mcp".to_owned(),
            "add".to_owned(),
            "--scope".to_owned(),
            "user".to_owned(),
            "--transport".to_owned(),
            "stdio".to_owned(),
            "airwiki".to_owned(),
            "--".to_owned(),
            bridge.to_string_lossy().into_owned(),
            "--client".to_owned(),
            "claude-code".to_owned(),
        ];
        assert_eq!(
            specs[0]
                .args
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            expected_args
        );
        assert!(specs[0].args.iter().all(|argument| {
            !argument.to_string_lossy().contains("capability")
                && !argument.to_string_lossy().contains("secret")
        }));
    }

    #[test]
    fn gemini_configuration_rejects_any_extra_field() {
        let exact = serde_json::json!({
            "command": "/data/bridge",
            "args": ["--client", "gemini-cli"],
            "includeTools": MANAGED_TOOLS
        });
        let mut altered = exact.clone();
        altered["env"] = serde_json::json!({"TOKEN": "value"});

        assert!(!parse_gemini_configuration(&exact).parse_conflict);
        assert!(parse_gemini_configuration(&altered).parse_conflict);
    }

    #[test]
    fn windows_comparison_components_fold_ascii_case() {
        #[cfg(target_os = "windows")]
        let (mixed_case, lower_case) = (
            Path::new(r"C:\Data\Integrations\Bridge\AirWiki.EXE"),
            Path::new(r"c:\data\integrations\bridge\airwiki.exe"),
        );
        #[cfg(not(target_os = "windows"))]
        let (mixed_case, lower_case) = (
            Path::new("/Data/Integrations/Bridge/AirWiki.EXE"),
            Path::new("/data/integrations/bridge/airwiki.exe"),
        );

        assert_eq!(
            path_components_for_comparison(mixed_case, true),
            path_components_for_comparison(lower_case, true)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_path_validation_skips_only_incomplete_disk_prefixes() {
        assert!(windows_component_is_incomplete_disk_prefix(
            Path::new(r"C:\managed")
                .components()
                .next()
                .expect("disk path must have a prefix")
        ));
        assert!(windows_component_is_incomplete_disk_prefix(
            Path::new(r"\\?\C:\managed")
                .components()
                .next()
                .expect("verbatim disk path must have a prefix")
        ));
        assert!(!windows_component_is_incomplete_disk_prefix(
            Path::new(r"\\server\share\managed")
                .components()
                .next()
                .expect("UNC path must have a prefix")
        ));
        assert!(!windows_component_is_incomplete_disk_prefix(
            Path::new(r"\\?\UNC\server\share\managed")
                .components()
                .next()
                .expect("verbatim UNC path must have a prefix")
        ));
    }

    #[test]
    fn tools_list_verification_accepts_the_exact_managed_tool_set() {
        let exact_tools = MANAGED_TOOLS
            .iter()
            .map(|name| {
                let (read_only, destructive, idempotent) =
                    expected_tool_hints(name).expect("managed tool hints");
                serde_json::json!({
                    "name": name,
                    "title": format!("{name} title"),
                    "description": format!("{name} description"),
                    "inputSchema": { "type": "object", "properties": {} },
                    "outputSchema": { "type": "object", "properties": {} },
                    "annotations": {
                        "readOnlyHint": read_only,
                        "destructiveHint": destructive,
                        "idempotentHint": idempotent,
                        "openWorldHint": false,
                    }
                })
            })
            .collect::<Vec<_>>();
        let output_for = |tools: Vec<Value>| {
            format!(
                "{}\n{}\n",
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "resultType": "complete",
                        "supportedVersions": [MCP_PROTOCOL_VERSION],
                        "ttlMs": 0,
                        "cacheScope": "private",
                    },
                }),
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "result": {
                        "resultType": "complete",
                        "ttlMs": 0,
                        "cacheScope": "private",
                        "tools": tools,
                    },
                })
            )
        };
        let mut reordered_tools = exact_tools.clone();
        reordered_tools.reverse();
        let mut duplicated_tools = exact_tools.clone();
        duplicated_tools.push(serde_json::json!({"name": SEARCH_TOOL}));
        let mut extra_tools = exact_tools.clone();
        extra_tools.push(serde_json::json!({"name": "unexpected_tool"}));

        assert!(verify_tools_list(&output_for(exact_tools)).is_ok());
        assert!(verify_tools_list(&output_for(reordered_tools)).is_ok());
        assert!(verify_tools_list(&output_for(duplicated_tools)).is_err());
        assert!(verify_tools_list(&output_for(extra_tools)).is_err());
        assert!(
            verify_tools_list(
                "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"resultType\":\"complete\",\"tools\":[]}}"
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn claude_bundle_is_opened_through_the_injected_opener() {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("airwiki");
        std::fs::write(&executable, b"desktop").unwrap();
        let resource_dir = temp.path().join("integrations");
        std::fs::create_dir_all(&resource_dir).unwrap();
        let bundle = resource_dir.join(CLAUDE_MCPB_NAME);
        std::fs::write(&bundle, b"mcpb").unwrap();
        let claude = temp.path().join("Claude.app");
        std::fs::create_dir_all(&claude).unwrap();
        let runner = Arc::new(RecordingRunner::default());
        let opener = Arc::new(RecordingOpener::default());
        let paths = AppPaths {
            data: temp.path().join("data"),
            database: temp.path().join("database"),
            logs: temp.path().join("logs"),
            config: temp.path().join("config"),
        };
        let manager = ChatIntegrationManager {
            paths: paths.clone(),
            environment: IntegrationEnvironment {
                platform: HostPlatform::MacOs,
                home: temp.path().to_path_buf(),
                path_entries: Vec::new(),
                discover_host_clients: false,
                current_exe: executable.clone(),
            },
            runner,
            opener: opener.clone(),
            database: airwiki_core::Database::in_memory().unwrap(),
            workflow_guides: WorkflowGuideManager::new(
                paths,
                temp.path().to_path_buf(),
                executable,
                true,
                false,
            ),
        };

        manager.opener.open(&bundle).await.unwrap();

        assert_eq!(opener.paths.lock().unwrap().as_slice(), [bundle]);
    }

    #[tokio::test]
    async fn bounded_bridge_read_rejects_oversized_file() {
        let temp = TempDir::new().unwrap();
        let bridge = temp.path().join(bridge_filename());
        let file = std::fs::File::create(&bridge).unwrap();
        file.set_len(MAX_BRIDGE_BYTES + 1).unwrap();

        let error = read_file_bounded(&bridge).await.unwrap_err();

        assert!(error.to_string().contains("tamaño máximo"));
    }

    #[tokio::test]
    async fn materialized_bridge_is_exact_and_executable() {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").unwrap();
        let bundled = temp
            .path()
            .join("integrations/bridge")
            .join(bridge_filename());
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, b"trusted bridge").unwrap();
        let manager = test_manager(&temp, executable);

        let installed = manager.materialize_bridge().await.unwrap();

        assert_eq!(std::fs::read(&installed).unwrap(), b"trusted bridge");
        assert!(executable_regular_file(&installed).unwrap());
    }

    #[tokio::test]
    async fn generic_mcp_capability_is_scoped_revocable_and_never_an_argument() {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").unwrap();
        let bundled = temp
            .path()
            .join("integrations/bridge")
            .join(bridge_filename());
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, b"trusted bridge").unwrap();
        let runner = Arc::new(RecordingRunner {
            specs: Mutex::new(Vec::new()),
            outputs: Mutex::new(vec![CommandOutput {
                success: true,
                stdout: tools_list_output(),
                _stderr: Vec::new(),
            }]),
        });
        let mut manager = test_manager(&temp, executable);
        manager.runner = runner.clone();

        manager.connect(ChatClientKind::GenericMcp).await.unwrap();

        assert!(manager.generic_capability_is_active().await.unwrap());
        assert_eq!(
            manager.inspect_generic_mcp().await.unwrap().status,
            IntegrationStatus::Configured
        );
        {
            let specs = runner.specs.lock().unwrap();
            assert_eq!(specs.len(), 1);
            assert_eq!(
                specs[0]
                    .args
                    .iter()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                ["--client", "generic-mcp"]
            );
        }

        manager
            .disconnect(ChatClientKind::GenericMcp)
            .await
            .unwrap();

        assert!(!manager.generic_capability_is_active().await.unwrap());
        assert_eq!(
            manager.inspect_generic_mcp().await.unwrap().status,
            IntegrationStatus::Available
        );
    }

    #[tokio::test]
    async fn failed_generic_mcp_connection_revokes_new_capability() {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").unwrap();
        let bundled = temp
            .path()
            .join("integrations/bridge")
            .join(bridge_filename());
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, b"trusted bridge").unwrap();
        let manager = test_manager(&temp, executable);

        assert!(manager.connect(ChatClientKind::GenericMcp).await.is_err());
        assert!(!manager.generic_capability_is_active().await.unwrap());
        assert!(!manager.capability_path(ChatClientKind::GenericMcp).exists());
    }

    #[tokio::test]
    async fn disconnect_revokes_capability_before_a_workflow_conflict() {
        let temp = TempDir::new().expect("temporary directory");
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").expect("desktop fixture");
        let manager = test_manager(&temp, executable);
        manager
            .ensure_application_capability(ChatClientKind::ClaudeCode)
            .await
            .expect("provision Claude Code capability");
        manager
            .workflow_guides
            .install(WorkflowClient::ClaudeCode)
            .await
            .expect("install Claude Code workflow guide");
        std::fs::write(
            temp.path().join(".claude/skills/airwiki/SKILL.md"),
            b"user-modified skill",
        )
        .expect("modify installed workflow guide");

        assert!(
            manager
                .disconnect(ChatClientKind::ClaudeCode)
                .await
                .is_err()
        );
        assert!(!manager.capability_path(ChatClientKind::ClaudeCode).exists());
    }

    #[tokio::test]
    async fn revoked_capability_file_is_replaced_on_reconnect() {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").unwrap();
        let manager = test_manager(&temp, executable);
        manager
            .ensure_application_capability(ChatClientKind::GenericMcp)
            .await
            .unwrap();
        let path = manager.capability_path(ChatClientKind::GenericMcp);
        let previous = std::fs::read_to_string(&path).unwrap();
        manager
            .database
            .set_application_capability_revoked(application_id(ChatClientKind::GenericMcp), true)
            .unwrap();

        let provision = manager
            .ensure_application_capability(ChatClientKind::GenericMcp)
            .await
            .unwrap();
        let replacement = std::fs::read_to_string(path).unwrap();

        assert_eq!(provision, CapabilityProvision::Created);
        assert_ne!(replacement, previous);
        assert!(manager.generic_capability_is_active().await.unwrap());
    }

    #[tokio::test]
    async fn reused_capability_must_belong_to_the_selected_client() {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").unwrap();
        let manager = test_manager(&temp, executable);
        manager
            .ensure_application_capability(ChatClientKind::ChatGptDesktop)
            .await
            .unwrap();
        let chatgpt_secret =
            std::fs::read(manager.capability_path(ChatClientKind::ChatGptDesktop)).unwrap();
        let generic_path = manager.capability_path(ChatClientKind::GenericMcp);
        std::fs::write(&generic_path, chatgpt_secret).unwrap();

        let error = manager
            .ensure_application_capability(ChatClientKind::GenericMcp)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("otra integración"));
        assert!(!manager.generic_capability_is_active().await.unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reused_capability_permissions_are_repaired_before_acceptance() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").unwrap();
        let manager = test_manager(&temp, executable);
        manager
            .ensure_application_capability(ChatClientKind::GenericMcp)
            .await
            .unwrap();
        let path = manager.capability_path(ChatClientKind::GenericMcp);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(!manager.generic_capability_is_active().await.unwrap());

        let provision = manager
            .ensure_application_capability(ChatClientKind::GenericMcp)
            .await
            .unwrap();

        assert_eq!(provision, CapabilityProvision::Existing);
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(manager.generic_capability_is_active().await.unwrap());
    }

    #[tokio::test]
    async fn current_managed_bridge_rejects_same_size_tampering() {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").unwrap();
        let bundled = temp
            .path()
            .join("integrations/bridge")
            .join(bridge_filename());
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, b"trusted").unwrap();
        let manager = test_manager(&temp, executable);
        let installed = manager.managed_bridge_path();
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, b"altered").unwrap();
        set_executable_permissions(&installed).await.unwrap();
        let configuration = ManagedConfiguration::new(installed, ChatClientKind::ChatGptDesktop);

        assert!(
            !manager
                .configuration_is_securely_managed(&configuration, ChatClientKind::ChatGptDesktop,)
                .await
                .unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn materialization_rejects_symlink_in_managed_path() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("airwiki-desktop");
        std::fs::write(&executable, b"desktop").unwrap();
        let bundled = temp
            .path()
            .join("integrations/bridge")
            .join(bridge_filename());
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, b"trusted bridge").unwrap();
        let manager = test_manager(&temp, executable);
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(manager.paths.data.join("integrations")).unwrap();
        symlink(&outside, manager.paths.data.join("integrations/bridge")).unwrap();

        let error = manager.materialize_bridge().await.unwrap_err();

        assert!(error.to_string().contains("enlace simbólico"));
        assert!(!outside.join(env!("CARGO_PKG_VERSION")).exists());
    }

    #[test]
    fn gemini_options_first_arguments_are_exact_and_never_enable_trust() {
        let configuration =
            ManagedConfiguration::new(PathBuf::from("/tmp/bridge"), ChatClientKind::GeminiCli);
        let args = gemini_add_args(&configuration, GeminiAddSyntax::OptionsFirst);
        let strings = args
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            strings,
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "stdio",
                "--include-tools",
                &MANAGED_TOOLS.join(","),
                INTEGRATION_NAME,
                "/tmp/bridge",
                "--",
                "--client",
                "gemini-cli",
            ]
        );
        assert!(!strings.iter().any(|argument| argument == "--trust"));
    }
}
