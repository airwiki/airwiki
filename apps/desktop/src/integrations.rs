#[cfg(target_os = "windows")]
use std::path::Prefix;
use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::Value;
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    time::timeout,
};
use uuid::Uuid;

use crate::paths::AppPaths;

const INTEGRATION_NAME: &str = "airwiki";
const BRIDGE_BASENAME: &str = "airwiki-mcp-bridge";
const CLAUDE_MCPB_NAME: &str = "airwiki-claude.mcpb";
const SEARCH_TOOL: &str = "search_airwiki";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(10);
const VERIFY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROCESS_OUTPUT: usize = 64 * 1024;
const MAX_BRIDGE_BYTES: u64 = 64 * 1024 * 1024;
#[cfg(target_os = "windows")]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChatClientKind {
    ChatGptDesktop,
    ClaudeDesktop,
    GeminiCli,
}

impl ChatClientKind {
    pub(crate) const ALL: [Self; 3] = [Self::ChatGptDesktop, Self::ClaudeDesktop, Self::GeminiCli];

    pub(crate) const fn display_name(self) -> &'static str {
        match self {
            Self::ChatGptDesktop => "ChatGPT Desktop / Work",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::GeminiCli => "Gemini CLI",
        }
    }

    const fn bridge_id(self) -> &'static str {
        match self {
            Self::ChatGptDesktop => "chatgpt-desktop",
            Self::ClaudeDesktop => "claude-desktop",
            Self::GeminiCli => "gemini-cli",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IntegrationStatus {
    NotInstalled,
    Available,
    Configuring,
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
}

#[derive(Debug, Clone)]
struct CommandSpec {
    executable: PathBuf,
    args: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    stdin: Option<Vec<u8>>,
    timeout: Duration,
}

impl CommandSpec {
    fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            args: Vec::new(),
            environment: Vec::new(),
            stdin: None,
            timeout: PROCESS_TIMEOUT,
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
                read_bounded(stdout),
                read_bounded(stderr),
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

async fn read_bounded(mut reader: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
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
        if output.len().saturating_add(read) > MAX_PROCESS_OUTPUT {
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
    current_exe: PathBuf,
}

impl IntegrationEnvironment {
    fn discover() -> Result<Self> {
        #[cfg(target_os = "macos")]
        let platform = HostPlatform::MacOs;
        #[cfg(target_os = "windows")]
        let platform = HostPlatform::Windows;
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let platform = HostPlatform::Unsupported;

        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .context("no se encontró el directorio personal")?;
        let path_entries = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default();
        Ok(Self {
            platform,
            home,
            path_entries,
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
}

impl ChatIntegrationManager {
    pub(crate) fn new(paths: AppPaths) -> Result<Self> {
        Ok(Self {
            paths,
            environment: IntegrationEnvironment::discover()?,
            runner: Arc::new(SystemCommandRunner),
            opener: Arc::new(SystemPathOpener),
        })
    }

    pub(crate) async fn execute(&self, action: IntegrationAction) -> Result<Vec<IntegrationView>> {
        match action {
            IntegrationAction::Refresh => {}
            IntegrationAction::Connect(client) => self.connect(client).await?,
            IntegrationAction::Disconnect(client) => self.disconnect(client).await?,
            IntegrationAction::ConfirmClaudeInstalled => {}
            IntegrationAction::OpenClaudeSettings => self.open_claude_settings().await?,
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
        let mut views = Vec::with_capacity(ChatClientKind::ALL.len());
        for client in ChatClientKind::ALL {
            match self.inspect(client).await {
                Ok(view) => views.push(view),
                Err(error) => views.push(view(
                    client,
                    IntegrationStatus::Error,
                    format!("No se pudo comprobar esta integración: {error:#}"),
                    None,
                    Some(self.managed_bridge_path()),
                )),
            }
        }
        Ok(views)
    }

    async fn inspect(&self, client: ChatClientKind) -> Result<IntegrationView> {
        match client {
            ChatClientKind::ChatGptDesktop => self.inspect_chatgpt().await,
            ChatClientKind::ClaudeDesktop => self.inspect_claude().await,
            ChatClientKind::GeminiCli => self.inspect_gemini().await,
        }
    }

    async fn connect(&self, client: ChatClientKind) -> Result<()> {
        match client {
            ChatClientKind::ChatGptDesktop => self.connect_chatgpt().await,
            ChatClientKind::ClaudeDesktop => self.open_claude_bundle().await,
            ChatClientKind::GeminiCli => self.connect_gemini().await,
        }
    }

    async fn disconnect(&self, client: ChatClientKind) -> Result<()> {
        match client {
            ChatClientKind::ChatGptDesktop => self.disconnect_chatgpt().await,
            ChatClientKind::ClaudeDesktop => self.open_claude_settings().await,
            ChatClientKind::GeminiCli => self.disconnect_gemini().await,
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
        let input = format!(
            concat!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",",
                "\"params\":{{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{{}},",
                "\"clientInfo\":{{\"name\":\"airwiki-desktop\",\"version\":\"{}\"}}}}}}\n",
                "{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\",\"params\":{{}}}}\n",
                "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{{}}}}\n"
            ),
            env!("CARGO_PKG_VERSION")
        );
        let output = self
            .runner
            .run(
                CommandSpec::new(bridge.to_path_buf())
                    .args(["--client", client.bridge_id()])
                    .stdin(input.into_bytes())
                    .timeout(VERIFY_TIMEOUT),
            )
            .await
            .context("el puente MCP no superó su verificación local")?;
        if !output.success {
            bail!("el puente MCP rechazó initialize/tools/list")
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
        if !self.codex_supported(&codex).await? {
            return Ok(view(
                ChatClientKind::ChatGptDesktop,
                IntegrationStatus::Unsupported,
                "La versión detectada no admite administración MCP local.",
                self.program_version(&codex).await,
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
            self.program_version(&codex).await,
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
        restart_required: matches!(client, ChatClientKind::ChatGptDesktop),
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
                tools.len() != 1 || tools.first().and_then(Value::as_str) != Some(SEARCH_TOOL)
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
    let options = [
        OsString::from("--scope"),
        OsString::from("user"),
        OsString::from("--transport"),
        OsString::from("stdio"),
        OsString::from("--include-tools"),
        OsString::from(SEARCH_TOOL),
    ];
    let mut args = vec![OsString::from("mcp"), OsString::from("add")];
    match syntax {
        GeminiAddSyntax::OptionsFirst => {
            args.extend(options);
            args.push(OsString::from(INTEGRATION_NAME));
            args.push(configuration.command.as_os_str().to_owned());
            args.push(OsString::from("--"));
            args.extend(configuration.args.iter().cloned().map(OsString::from));
        }
        GeminiAddSyntax::PositionalsFirst => {
            args.push(OsString::from(INTEGRATION_NAME));
            args.push(configuration.command.as_os_str().to_owned());
            args.extend(configuration.args.iter().cloned().map(OsString::from));
            args.extend(options);
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
    let mut found_initialize = false;
    let mut found_tools = false;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match message.get("id").and_then(Value::as_u64) {
            Some(1) if message.get("result").is_some() => found_initialize = true,
            Some(2) => {
                let tools = message
                    .get("result")
                    .and_then(|result| result.get("tools"))
                    .and_then(Value::as_array)
                    .context("tools/list no devolvió herramientas")?;
                found_tools = tools.len() == 1
                    && tools
                        .first()
                        .and_then(|tool| tool.get("name"))
                        .and_then(Value::as_str)
                        == Some(SEARCH_TOOL);
            }
            _ => {}
        }
    }
    if !found_initialize || !found_tools {
        bail!("el puente no expuso exactamente la herramienta de búsqueda esperada")
    }
    Ok(())
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
        ChatIntegrationManager {
            paths: AppPaths {
                data: root.join("data"),
                database: root.join("data/airwiki.sqlite3"),
                vaults: root.join("data/vaults"),
                logs: root.join("data/logs"),
                config: root.join("config/config.json"),
            },
            environment: IntegrationEnvironment {
                platform: test_platform(),
                home: root,
                path_entries: Vec::new(),
                current_exe,
            },
            runner: Arc::new(RecordingRunner::default()),
            opener: Arc::new(RecordingOpener::default()),
        }
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
    fn gemini_configuration_rejects_any_extra_field() {
        let exact = serde_json::json!({
            "command": "/data/bridge",
            "args": ["--client", "gemini-cli"],
            "includeTools": [SEARCH_TOOL]
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
    fn tools_list_verification_accepts_only_the_read_only_search_tool() {
        let output = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"search_airwiki\"}]}}\n"
        );

        assert!(verify_tools_list(output).is_ok());
        assert!(
            verify_tools_list("{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}").is_err()
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
        let manager = ChatIntegrationManager {
            paths: AppPaths {
                data: temp.path().join("data"),
                database: temp.path().join("database"),
                vaults: temp.path().join("vaults"),
                logs: temp.path().join("logs"),
                config: temp.path().join("config"),
            },
            environment: IntegrationEnvironment {
                platform: HostPlatform::MacOs,
                home: temp.path().to_path_buf(),
                path_entries: Vec::new(),
                current_exe: executable,
            },
            runner,
            opener: opener.clone(),
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
                SEARCH_TOOL,
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
