use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use rand::RngCore;
use tokio::{
    process::{Child, Command},
    sync::Mutex,
};
use tracing::{info, warn};

#[cfg(target_os = "windows")]
mod child_process_guard {
    use std::{
        ffi::c_void,
        mem::size_of,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };

    use anyhow::{Context, Result};
    use tokio::process::Child;
    use windows::{
        Win32::{
            Foundation::HANDLE,
            System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
        },
        core::PCWSTR,
    };

    #[derive(Debug)]
    pub(super) struct ChildProcessGuard {
        job_handle: OwnedHandle,
    }

    impl ChildProcessGuard {
        pub(super) fn attach(child: &Child) -> Result<Self> {
            // SAFETY: The unnamed job receives no security descriptor or borrowed name.
            let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
                .context("failed to create the llama-server Windows job")?;
            // SAFETY: `CreateJobObjectW` returned a new owned handle. It is moved
            // exactly once into `OwnedHandle`, which closes it on every exit path.
            let guard = Self {
                job_handle: unsafe { OwnedHandle::from_raw_handle(job.0) },
            };

            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `limits` has the exact structure and byte length required for
            // `JobObjectExtendedLimitInformation` and remains alive for this call.
            unsafe {
                SetInformationJobObject(
                    HANDLE(guard.job_handle.as_raw_handle()),
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast::<c_void>(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            }
            .context("failed to configure the llama-server Windows job")?;

            let process = HANDLE(
                child
                    .raw_handle()
                    .context("llama-server exited before Windows job assignment")?,
            );
            // SAFETY: Tokio owns a live process handle for `child`; assigning it does
            // not transfer that handle. The job handle remains owned by `guard`.
            unsafe { AssignProcessToJobObject(HANDLE(guard.job_handle.as_raw_handle()), process) }
                .context("failed to assign llama-server to its Windows job")?;
            Ok(guard)
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod child_process_guard {
    use anyhow::Result;
    use tokio::process::Child;

    #[derive(Debug)]
    pub(super) struct ChildProcessGuard;

    impl ChildProcessGuard {
        pub(super) fn attach(_child: &Child) -> Result<Self> {
            Ok(Self)
        }
    }
}

use child_process_guard::ChildProcessGuard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerReasoningMode {
    Off,
    On,
}

impl ServerReasoningMode {
    const fn cli_value(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub server_binary: PathBuf,
    pub model_path: PathBuf,
    pub model_id: String,
    pub mmproj_path: Option<PathBuf>,
    pub context_tokens: u32,
    pub idle_timeout: Duration,
    pub startup_timeout: Duration,
    pub threads: usize,
    /// The MVP sidecar only performs bounded structured extraction. Reasoning
    /// must remain off or Gemma can spend the complete output budget in
    /// `reasoning_content` and return an empty JSON content field.
    pub reasoning_mode: ServerReasoningMode,
}

impl SupervisorConfig {
    pub fn bundled(server_binary: PathBuf, model_path: PathBuf) -> Self {
        Self {
            server_binary,
            model_path,
            model_id: "qwen3-1.7b-q8".to_owned(),
            mmproj_path: None,
            context_tokens: 4_096,
            idle_timeout: Duration::from_secs(5 * 60),
            startup_timeout: Duration::from_secs(120),
            threads: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(2)
                .max(1),
            reasoning_mode: ServerReasoningMode::Off,
        }
    }
}

#[derive(Clone)]
pub struct LlamaEndpoint {
    pub base_url: String,
    token: Arc<str>,
}

impl LlamaEndpoint {
    pub fn bearer_token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for LlamaEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlamaEndpoint")
            .field("base_url", &self.base_url)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

struct RunningServer {
    child: Child,
    _process_guard: ChildProcessGuard,
    endpoint: LlamaEndpoint,
    last_used: Instant,
}

struct SpawnedServer {
    child: Child,
    process_guard: ChildProcessGuard,
    endpoint: LlamaEndpoint,
}

struct Inner {
    config: SupervisorConfig,
    state: Mutex<Option<RunningServer>>,
    http: reqwest::Client,
}

#[derive(Clone)]
pub struct LlamaSupervisor {
    inner: Arc<Inner>,
}

impl LlamaSupervisor {
    pub fn new(config: SupervisorConfig) -> Self {
        let this = Self {
            inner: Arc::new(Inner {
                config,
                state: Mutex::new(None),
                http: reqwest::Client::new(),
            }),
        };
        this.spawn_idle_reaper();
        this
    }

    pub async fn ensure_running(&self) -> Result<LlamaEndpoint> {
        let mut state = self.inner.state.lock().await;
        if let Some(server) = state.as_mut() {
            if server.child.try_wait()?.is_none() {
                server.last_used = Instant::now();
                return Ok(server.endpoint.clone());
            }
            *state = None;
        }

        let spawned = spawn_server(&self.inner.config).await?;
        wait_until_healthy(
            &self.inner.http,
            &spawned,
            self.inner.config.startup_timeout,
        )
        .await?;
        let SpawnedServer {
            child,
            process_guard,
            endpoint,
        } = spawned;
        info!(url = endpoint.base_url, "llama-server is ready on loopback");
        *state = Some(RunningServer {
            child,
            _process_guard: process_guard,
            endpoint: endpoint.clone(),
            last_used: Instant::now(),
        });
        Ok(endpoint)
    }

    pub async fn mark_activity(&self) {
        if let Some(server) = self.inner.state.lock().await.as_mut() {
            server.last_used = Instant::now();
        }
    }

    pub async fn stop(&self) -> Result<()> {
        let mut state = self.inner.state.lock().await;
        if let Some(mut server) = state.take() {
            if server
                .child
                .try_wait()
                .context("failed to inspect llama-server before shutdown")?
                .is_none()
            {
                server
                    .child
                    .kill()
                    .await
                    .context("failed to terminate llama-server")?;
            }
            server
                .child
                .wait()
                .await
                .context("failed to reap llama-server")?;
            info!("llama-server stopped");
        }
        Ok(())
    }

    fn spawn_idle_reaper(&self) {
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(15));
            loop {
                timer.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                let mut state = inner.state.lock().await;
                let should_stop = state
                    .as_ref()
                    .is_some_and(|server| server.last_used.elapsed() >= inner.config.idle_timeout);
                if should_stop && let Some(mut server) = state.take() {
                    warn!("stopping idle llama-server");
                    server.child.kill().await.ok();
                    server.child.wait().await.ok();
                }
            }
        });
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.try_lock()
            && let Some(server) = state.as_mut()
        {
            let _ = server.child.start_kill();
        }
    }
}

async fn spawn_server(config: &SupervisorConfig) -> Result<SpawnedServer> {
    if !config.server_binary.is_file() {
        bail!(
            "llama-server not found at {}",
            config.server_binary.display()
        );
    }
    if !config.model_path.is_file() {
        bail!(
            "{} model not found at {}",
            config.model_id,
            config.model_path.display()
        );
    }
    if let Some(mmproj) = &config.mmproj_path
        && !mmproj.is_file()
    {
        bail!(
            "{} multimodal projector not found at {}",
            config.model_id,
            mmproj.display()
        );
    }
    let port = reserve_loopback_port()?;
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token: Arc<str> = hex::encode(bytes).into();
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);

    let mut command = server_command(config, address, token.as_ref());
    let mut child = command.spawn().with_context(|| {
        format!(
            "failed to start bundled llama-server {}",
            config.server_binary.display()
        )
    })?;
    let process_guard = match ChildProcessGuard::attach(&child) {
        Ok(guard) => guard,
        Err(error) => {
            child.start_kill().ok();
            child.wait().await.ok();
            return Err(error);
        }
    };
    Ok(SpawnedServer {
        endpoint: LlamaEndpoint {
            base_url: format!("http://{address}"),
            token,
        },
        child,
        process_guard,
    })
}

fn server_command(config: &SupervisorConfig, address: SocketAddr, token: &str) -> Command {
    let mut command = Command::new(&config.server_binary);
    command
        .arg("--model")
        .arg(&config.model_path)
        .arg("--host")
        .arg(address.ip().to_string())
        .arg("--port")
        .arg(address.port().to_string())
        .arg("--api-key")
        .arg(token)
        .arg("--ctx-size")
        .arg(config.context_tokens.to_string())
        .arg("--parallel")
        .arg("1")
        .arg("--threads")
        .arg(config.threads.to_string())
        .arg("--reasoning")
        .arg(config.reasoning_mode.cli_value())
        .arg("--metrics")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if config.reasoning_mode == ServerReasoningMode::Off {
        // b9946 also exposes an explicit budget. Pairing both switches avoids a
        // future chat-template default silently consuming structured-output
        // tokens even if its automatic reasoning detection changes.
        command.arg("--reasoning-budget").arg("0");
    }
    if let Some(mmproj) = &config.mmproj_path {
        command.arg("--mmproj").arg(mmproj);
    }
    if cfg!(target_os = "macos") {
        command.arg("--n-gpu-layers").arg("99");
    } else {
        command.arg("--n-gpu-layers").arg("0");
    }
    command
}

fn reserve_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

async fn wait_until_healthy(
    client: &reqwest::Client,
    server: &SpawnedServer,
    timeout: Duration,
) -> Result<()> {
    let endpoint = &server.endpoint;
    let started = Instant::now();
    while started.elapsed() < timeout {
        let response = client
            .get(format!("{}/health", endpoint.base_url))
            .bearer_auth(endpoint.bearer_token())
            .send()
            .await;
        if response.is_ok_and(|response| response.status().is_success()) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!("llama-server did not become healthy within {timeout:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_sidecar_disables_reasoning_explicitly() {
        let config =
            SupervisorConfig::bundled(PathBuf::from("llama-server"), PathBuf::from("model.gguf"));
        let command = server_command(
            &config,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 43_123),
            "redacted-test-token",
        );
        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let reasoning = args
            .windows(2)
            .find(|pair| pair[0] == "--reasoning")
            .expect("reasoning flag must be present");
        assert_eq!(reasoning[1], "off");
        let budget = args
            .windows(2)
            .find(|pair| pair[0] == "--reasoning-budget")
            .expect("reasoning budget must be present when reasoning is off");
        assert_eq!(budget[1], "0");
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn windows_process_guard_terminates_child_when_dropped() {
        let mut command = Command::new("ping.exe");
        command
            .arg("-t")
            .arg("127.0.0.1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("test child should start");
        let guard = ChildProcessGuard::attach(&child).expect("test child should join the job");
        assert!(
            child
                .try_wait()
                .expect("test child status should remain observable")
                .is_none(),
            "test child should still be running before the job closes"
        );

        drop(guard);

        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("job close should terminate the test child promptly")
            .expect("test child status should remain observable");
    }
}
