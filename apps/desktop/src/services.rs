//! Background-service composition for the native desktop application.
//!
//! This module owns no egui state. It is created and called exclusively by the
//! Tokio worker thread, while the UI communicates with that worker through
//! channels. MCP starts independently; the optional LAN runtime is reconciled
//! after platform readiness is known. Both use fail-closed proxies until
//! [`DesktopServices::enable_models`] installs the real search engines.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use airwiki_core::{
    AppPaths as CoreAppPaths, AuditEvent, CollectionRecord, Database, E5Tokenizer,
    EmbeddingProvider, EvidenceDecision, EvidenceRelevanceProvider, FastEmbedE5Small,
    FastEmbedMmarcoReranker, FolderWatcher, GenerationProvider, GenerationRuntimeConfig,
    GuidedRepairPreview, GuidedRepairResult, HybridSearchEngine, IngestOutcome, IngestPipeline,
    KnowledgeBundleState, KnowledgeBundleView, KnowledgePageId, KnowledgePageView,
    LlamaServerProvider, OkfBundleInspector, OkfPublicationMaterializer, PinnedE5Snapshot,
    PinnedMmarcoRerankerSnapshot, RelevanceInput, ReviewEdits, Tokenizer, WikiRepairExecutor,
    WikiRepairPlanner,
};
use airwiki_inference::{
    GenerationSettings, InstallOutcome, LlamaSupervisor, ModelSelection, SupervisorConfig,
    ThinkingControl,
};
use airwiki_mcp::{McpClientActivitySnapshot, McpServerConfig, McpServerHandle, start_mcp_server};
use airwiki_network::{
    AccessControl, AuthorizedSearchBackend, AuthorizedSearchResult, FederatedCoordinator,
    KeyringSecretStore, MAX_MDNS_ADDRESSES_PER_PEER, MAX_VOLATILE_LAN_PEERS, ManualLanAddress,
    Multiaddr, NetworkConfig, NetworkEvent, NetworkHandle, NetworkWarningKind, NodeIdentity,
    PairingFailureReason, PeerAccess, PeerId, SecretStore, spawn_network,
};
use airwiki_types::{
    CollectionPolicy, DocumentStatus, EnrichmentDraft, FederatedSearch, SearchAuthorization,
    SearchContractError, SearchPurpose, SearchRequest, SearchResponse,
};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::Utc;
use tokio::{
    sync::{Mutex as AsyncMutex, broadcast, mpsc},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    manual_lan_route,
    paths::AppPaths,
    worker::{CollectionView, PeerActivityState, PeerTrustState, PeerView, ReviewItemView},
};

const KEYRING_SERVICE: &str = "io.github.airwiki.AirWiki";
const KEYRING_ACCOUNT: &str = "device-identity";
const MODEL_SMOKE_TEST_DOCUMENT: &str = "Documento sintético de diagnóstico local. Título: Prueba de AirWiki. Contenido: verificar que el enriquecimiento estructurado funciona sin utilizar información de la empresa.";
const RELEVANCE_SMOKE_TEST_QUESTION: &str = "¿Cómo se recupera el servicio Atlas?";
const RELEVANCE_SMOKE_TEST_PASSAGES: [&str; 2] = [
    "Procedimiento de recuperación de Atlas: restaurar el snapshot y validar el servicio.",
    "El presupuesto del evento comunitario se revisará el próximo trimestre.",
];
const KNOWLEDGE_READ_ATTEMPTS: usize = 3;
const KNOWLEDGE_READ_RETRY_DELAY: Duration = Duration::from_millis(100);
const KNOWLEDGE_PAGE_MAX_BYTES: usize = 1024 * 1024;
const LAN_LISTENER_START_GRACE: Duration = Duration::from_secs(10);
const STARTUP_COLLECTION_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Exact runtime assets already verified by [`airwiki_inference::AssetManager`].
#[derive(Debug, Clone)]
pub struct ModelRuntimePaths {
    pub llama_server: PathBuf,
    pub model: PathBuf,
    pub e5_snapshot: PathBuf,
    pub relevance_snapshot: PathBuf,
    pub selection: ModelSelection,
    pub generation_settings: GenerationSettings,
}

impl ModelRuntimePaths {
    pub fn from_install(outcome: &InstallOutcome) -> Self {
        Self {
            llama_server: outcome.llama_server_path.clone(),
            model: outcome.model_path.clone(),
            e5_snapshot: outcome.embedding_snapshot_path.clone(),
            relevance_snapshot: outcome.relevance_snapshot_path.clone(),
            selection: outcome.selection.clone(),
            generation_settings: outcome.generation_settings,
        }
    }

    fn validate(&self) -> Result<()> {
        for (label, path, must_be_file) in [
            ("llama-server", &self.llama_server, true),
            (self.selection.manifest.display_name, &self.model, true),
            ("multilingual-e5-small", &self.e5_snapshot, false),
            (
                "mmarco-mMiniLMv2-L12-H384-v1",
                &self.relevance_snapshot,
                false,
            ),
        ] {
            let present = if must_be_file {
                path.is_file()
            } else {
                path.is_dir()
            };
            if !present {
                bail!("{label} no está instalado en {}", path.display());
            }
        }
        Ok(())
    }
}

/// Authorization proxy retained by the libp2p runtime from process startup.
/// It cannot reveal evidence until a real local engine is installed.
pub struct DynamicAuthorizedSearchBackend {
    database: Database,
    access: AccessControl,
    engine: RwLock<Option<Arc<HybridSearchEngine>>>,
}

impl DynamicAuthorizedSearchBackend {
    fn new(database: Database, access: AccessControl) -> Self {
        Self {
            database,
            access,
            engine: RwLock::new(None),
        }
    }

    fn install(&self, engine: Arc<HybridSearchEngine>) -> Result<()> {
        *write_lock(&self.engine, "authorized search proxy")? = Some(engine);
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        *write_lock(&self.engine, "authorized search proxy")? = None;
        Ok(())
    }

    /// Intersects the runtime authorization snapshot with the durable trust
    /// store. Any malformed/unknown/untrusted/blocked caller is denied; missing
    /// or stale grants simply produce no authorized collections.
    fn durable_authorized_collections(
        &self,
        authorization: &SearchAuthorization,
        purpose: SearchPurpose,
    ) -> std::result::Result<Vec<Uuid>, SearchContractError> {
        let caller = authorization.caller_node_id.trim();
        let peer_id = PeerId::from_str(caller).map_err(|_| SearchContractError::Unauthorized)?;
        if peer_id.to_string() != caller {
            return Err(SearchContractError::Unauthorized);
        }
        let live_access = self.access.state(&peer_id);
        if !live_access.trusted || live_access.blocked {
            return Err(SearchContractError::Unauthorized);
        }
        let peer = self
            .database
            .peer(caller)
            .map_err(|error| SearchContractError::Backend(error.to_string()))?
            .ok_or(SearchContractError::Unauthorized)?;
        if !peer.trusted || peer.blocked {
            return Err(SearchContractError::Unauthorized);
        }

        let runtime_grants = authorization
            .allowed_collections
            .iter()
            .copied()
            .filter(|collection| live_access.grants.contains(collection))
            .collect::<HashSet<_>>();
        let durable_grants = self
            .database
            .granted_collections_for_search(caller, purpose)
            .map_err(|error| SearchContractError::Backend(error.to_string()))?;
        let mut allowed = Vec::new();
        for collection_id in durable_grants {
            if !runtime_grants.contains(&collection_id) {
                continue;
            }
            allowed.push(collection_id);
        }
        allowed.sort_unstable();
        allowed.dedup();
        Ok(allowed)
    }

    async fn finalize_authorized_response(
        &self,
        response: SearchResponse,
        authorization: SearchAuthorization,
        purpose: SearchPurpose,
    ) -> std::result::Result<AuthorizedSearchResult, SearchContractError> {
        let database = self.database.clone();
        let access = self.access.clone();
        tokio::task::spawn_blocking(move || {
            finalize_authorized_response_blocking(
                &database,
                &access,
                response,
                authorization,
                purpose,
            )
        })
        .await
        .map_err(|_| {
            SearchContractError::Unavailable("final authorization worker stopped".into())
        })?
    }
}

#[async_trait]
impl AuthorizedSearchBackend for DynamicAuthorizedSearchBackend {
    async fn search_authorized(
        &self,
        request: SearchRequest,
        authorization: SearchAuthorization,
    ) -> std::result::Result<AuthorizedSearchResult, SearchContractError> {
        request.validate()?;
        if request.purpose != authorization.purpose {
            return Err(SearchContractError::Unauthorized);
        }
        let engine = read_lock(&self.engine, "authorized search proxy")
            .map_err(|error| SearchContractError::Unavailable(error.to_string()))?
            .clone()
            .ok_or_else(|| {
                SearchContractError::Unavailable("los modelos locales aún no están listos".into())
            })?;
        // The AccessControl value is only a runtime snapshot. SQLite is the
        // durable authority for device trust, grants and collection policy.
        let allowed_collections =
            self.durable_authorized_collections(&authorization, request.purpose)?;
        if allowed_collections.is_empty() {
            return self
                .finalize_authorized_response(
                    SearchResponse::empty(request.request_id),
                    authorization,
                    request.purpose,
                )
                .await;
        }
        let request_id = request.request_id;
        let purpose = request.purpose;
        let mut response = engine
            .search_collections(request, &allowed_collections)
            .await
            .map_err(|error| SearchContractError::Backend(error.to_string()))?;

        response.request_id = request_id;
        self.finalize_authorized_response(response, authorization, purpose)
            .await
    }
}

fn finalize_authorized_response_blocking(
    database: &Database,
    access: &AccessControl,
    mut response: SearchResponse,
    authorization: SearchAuthorization,
    purpose: SearchPurpose,
) -> std::result::Result<AuthorizedSearchResult, SearchContractError> {
    let lease = authorization.acquire_disclosure_lease();
    let live_access = access.state(
        &PeerId::from_str(&authorization.caller_node_id)
            .map_err(|_| SearchContractError::Unauthorized)?,
    );
    if !live_access.trusted || live_access.blocked {
        response.hits.clear();
    } else {
        let mut current_hits = Vec::with_capacity(response.hits.len());
        for hit in response.hits {
            if live_access.grants.contains(&hit.collection_id)
                && database
                    .peer_hit_is_current_under_disclosure(
                        &lease,
                        &hit,
                        &authorization.caller_node_id,
                        purpose,
                    )
                    .map_err(|error| SearchContractError::Backend(error.to_string()))?
            {
                current_hits.push(hit);
            }
        }
        response.hits = current_hits;
    }
    for (index, hit) in response.hits.iter_mut().enumerate() {
        hit.rank = u32::try_from(index + 1).unwrap_or(u32::MAX);
    }
    Ok(AuthorizedSearchResult::new(response, lease))
}

/// Stable proxy passed to the MCP server before model initialization.
pub struct DynamicFederatedSearch {
    database: Database,
    access: AccessControl,
    local_node_id: String,
    backend: RwLock<Option<Arc<dyn FederatedSearch>>>,
}

impl DynamicFederatedSearch {
    fn new(database: Database, access: AccessControl, local_node_id: String) -> Self {
        Self {
            database,
            access,
            local_node_id,
            backend: RwLock::new(None),
        }
    }

    fn install(&self, backend: Arc<dyn FederatedSearch>) -> Result<()> {
        *write_lock(&self.backend, "federated search proxy")? = Some(backend);
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        *write_lock(&self.backend, "federated search proxy")? = None;
        Ok(())
    }

    /// SQLite remains the authority for peers contacted by an outbound search.
    /// This closes the small window where a lossy presentation event could leave
    /// runtime trust broader than the state successfully persisted to disk.
    fn reconcile_access(&self) -> Result<()> {
        let stored = self
            .database
            .list_peers()?
            .into_iter()
            .map(|peer| (peer.peer_id.clone(), peer))
            .collect::<HashMap<_, _>>();
        for peer in self.access.trusted_peers() {
            let peer_id = peer.to_string();
            let Some(durable) = stored
                .get(&peer_id)
                .filter(|stored| stored.trusted && !stored.blocked)
            else {
                self.access.block(peer);
                continue;
            };
            let durable_grants = self
                .database
                .granted_collections(&durable.peer_id)?
                .into_iter()
                .collect::<HashSet<_>>();
            for runtime_grant in self.access.state(&peer).grants {
                if !durable_grants.contains(&runtime_grant) {
                    self.access.remove_grant(peer, runtime_grant);
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl FederatedSearch for DynamicFederatedSearch {
    async fn search(
        &self,
        request: SearchRequest,
    ) -> std::result::Result<SearchResponse, SearchContractError> {
        request.validate()?;
        self.reconcile_access()
            .map_err(|error| SearchContractError::Backend(error.to_string()))?;
        let backend = read_lock(&self.backend, "federated search proxy")
            .map_err(|error| SearchContractError::Unavailable(error.to_string()))?
            .clone()
            .ok_or_else(|| {
                SearchContractError::Unavailable("los modelos locales aún no están listos".into())
            })?;
        let purpose = request.purpose;
        let mut response = backend.search(request).await?;
        let before_revalidation = response.hits.len();
        let mut current = Vec::with_capacity(response.hits.len());
        for hit in response.hits {
            let keep = if hit.node_id == self.local_node_id {
                self.database
                    .hit_is_current(&hit, purpose)
                    .map_err(|error| SearchContractError::Backend(error.to_string()))?
            } else {
                PeerId::from_str(&hit.node_id).is_ok_and(|peer| {
                    let access = self.access.state(&peer);
                    access.trusted && !access.blocked
                })
            };
            if keep {
                current.push(hit);
            }
        }
        for (index, hit) in current.iter_mut().enumerate() {
            hit.rank = u32::try_from(index + 1).unwrap_or(u32::MAX);
        }
        if current.len() < before_revalidation {
            response.partial = true;
            response
                .warnings
                .push("results changed during final authorization revalidation".into());
        }
        response.hits = current;
        Ok(response)
    }
}

struct LocalOnlyFederatedSearch {
    local: Arc<dyn FederatedSearch>,
    database: Database,
}

impl LocalOnlyFederatedSearch {
    fn new(local: Arc<dyn FederatedSearch>, database: Database) -> Self {
        Self { local, database }
    }
}

#[async_trait]
impl FederatedSearch for LocalOnlyFederatedSearch {
    async fn search(
        &self,
        request: SearchRequest,
    ) -> std::result::Result<SearchResponse, SearchContractError> {
        let mut response = self.local.search(request).await?;
        let has_trusted_peers = self
            .database
            .list_peers()
            .map_err(|error| SearchContractError::Backend(error.to_string()))?
            .into_iter()
            .any(|peer| peer.trusted && !peer.blocked);
        if has_trusted_peers {
            response.partial = true;
            response.warnings.push("federation_disabled".to_owned());
        }
        Ok(response)
    }
}

/// Generation provider that starts the bundled llama.cpp sidecar on first use
/// and refreshes its five-minute idle deadline after each request.
#[derive(Clone)]
struct SupervisedGenerationProvider {
    supervisor: LlamaSupervisor,
    model_id: String,
    runtime_config: GenerationRuntimeConfig,
}

#[async_trait]
impl GenerationProvider for SupervisedGenerationProvider {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn enrich(&self, document_text: &str) -> Result<EnrichmentDraft> {
        let endpoint = self.supervisor.ensure_running().await?;
        let provider = LlamaServerProvider::with_config(
            &endpoint.base_url,
            endpoint.bearer_token(),
            self.runtime_config.clone(),
        )?;
        // Hierarchical enrichment can span several individually bounded HTTP
        // calls. Keep the sidecar leased for the whole operation so its idle
        // reaper cannot stop it between summary stages.
        let lease_cancel = CancellationToken::new();
        let lease_task_cancel = lease_cancel.clone();
        let lease_supervisor = self.supervisor.clone();
        let lease_task = tokio::spawn(async move {
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = lease_task_cancel.cancelled() => break,
                    _ = heartbeat.tick() => lease_supervisor.mark_activity().await,
                }
            }
        });
        let result = provider.enrich(document_text).await;
        lease_cancel.cancel();
        let _ = lease_task.await;
        self.supervisor.mark_activity().await;
        result
    }
}

#[derive(Clone)]
struct ModelServices {
    supervisor: LlamaSupervisor,
    pipeline: Arc<IngestPipeline>,
    local_search: Arc<HybridSearchEngine>,
}

struct NetworkRuntime {
    handle: NetworkHandle,
    task: JoinHandle<()>,
    event_forwarder: JoinHandle<()>,
    started_at: Instant,
}

/// Keeps the runtime generation attached until the worker consumes the event.
/// An old event can already be queued when LAN is disabled or restarted.
#[derive(Debug, Clone)]
pub(crate) struct SequencedNetworkEvent {
    pub(crate) generation: u64,
    pub(crate) event: NetworkEvent,
}

#[derive(Debug, Default, Clone)]
struct LivePeer {
    addresses: VecDeque<String>,
    connected: bool,
    pairing: bool,
    node_name: Option<String>,
    sas_words: Option<[String; 6]>,
}

/// Side effects useful to the worker after applying a network event.
#[derive(Debug, Default, Clone)]
pub struct NetworkEventEffect {
    pub peers_changed: bool,
    pub notice: Option<String>,
    pub warning: Option<String>,
}

/// Complete background service graph for one workstation.
pub struct DesktopServices {
    core_paths: CoreAppPaths,
    database: Database,
    node_id: String,
    identity: NodeIdentity,
    access: AccessControl,
    authorized_proxy: Arc<DynamicAuthorizedSearchBackend>,
    federated_proxy: Arc<DynamicFederatedSearch>,
    network: Mutex<Option<NetworkRuntime>>,
    search_topology: AsyncMutex<()>,
    network_generation: Arc<AtomicU64>,
    network_events: broadcast::Sender<SequencedNetworkEvent>,
    mcp: Option<McpServerHandle>,
    models: RwLock<Option<ModelServices>>,
    live_peers: RwLock<HashMap<PeerId, LivePeer>>,
    startup_preflight_blocked: RwLock<HashSet<Uuid>>,
}

impl DesktopServices {
    /// Starts storage, persistent Ed25519 identity, LAN and loopback MCP. Models
    /// are intentionally optional at this stage.
    pub async fn start(paths: &AppPaths, lan_enabled: bool) -> Result<Self> {
        let store: Arc<dyn SecretStore> =
            Arc::new(KeyringSecretStore::new(KEYRING_SERVICE, KEYRING_ACCOUNT));
        Self::start_with_secret_store_and_lan(paths, store, lan_enabled).await
    }

    async fn start_with_secret_store_and_lan(
        paths: &AppPaths,
        secret_store: Arc<dyn SecretStore>,
        lan_enabled: bool,
    ) -> Result<Self> {
        let core_paths = CoreAppPaths::at(&paths.data);
        core_paths.ensure()?;
        let database = Database::open(&paths.database)?;
        let recovery_database = database.clone();
        let recovery = tokio::task::spawn_blocking(move || {
            OkfPublicationMaterializer::new(recovery_database).recover_pending()
        })
        .await
        .context("falló el worker de recuperación de publicaciones OKF")??;
        if recovery.pending > 0 {
            tracing::warn!(
                pending = recovery.pending,
                completed = recovery.completed,
                cancelled = recovery.cancelled,
                "some OKF publications remain pending after startup recovery"
            );
        }
        let identity = NodeIdentity::load_or_create(secret_store.as_ref())
            .context("no se pudo cargar la identidad Ed25519 del dispositivo")?;
        let node_id = identity.peer_id().to_string();
        let access = restore_access_control(&database)?;

        let authorized_proxy = Arc::new(DynamicAuthorizedSearchBackend::new(
            database.clone(),
            access.clone(),
        ));
        let (network_events, _) = broadcast::channel(128);
        let network_generation = Arc::new(AtomicU64::new(1));
        let network = if lan_enabled {
            Some(spawn_network_runtime(
                identity.clone(),
                access.clone(),
                authorized_proxy.clone(),
                network_events.clone(),
                Arc::clone(&network_generation),
                1,
            )?)
        } else {
            None
        };

        let federated_proxy = Arc::new(DynamicFederatedSearch::new(
            database.clone(),
            access.clone(),
            node_id.clone(),
        ));
        let mcp = match start_mcp_server(McpServerConfig::default(), federated_proxy.clone()).await
        {
            Ok(handle) => handle,
            Err(error) => {
                if let Some(runtime) = network {
                    let _ = runtime.handle.shutdown().await;
                    runtime.task.abort();
                    runtime.event_forwarder.abort();
                }
                return Err(error).context("no se pudo iniciar el MCP local");
            }
        };

        Ok(Self {
            core_paths,
            database,
            node_id,
            identity,
            access,
            authorized_proxy,
            federated_proxy,
            network: Mutex::new(network),
            search_topology: AsyncMutex::new(()),
            network_generation,
            network_events,
            mcp: Some(mcp),
            models: RwLock::new(None),
            live_peers: RwLock::new(HashMap::new()),
            startup_preflight_blocked: RwLock::new(HashSet::new()),
        })
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn mcp_endpoint(&self) -> &str {
        // `mcp` remains present throughout normal operation. Returning the
        // canonical endpoint keeps this accessor usable during teardown tests.
        "http://127.0.0.1:43123/mcp"
    }

    pub fn advertised_lan_addresses(&self, listener: &Multiaddr) -> Result<Vec<String>> {
        manual_lan_route::advertised_addresses(listener, self.identity.peer_id())
            .context("no se pudo preparar la dirección LAN manual")
    }

    /// Returns the latest informational bridge activity. This value is
    /// diagnostic only and must never participate in authorization decisions.
    pub fn latest_mcp_client_activities(&self) -> McpClientActivitySnapshot {
        let Some(handle) = self.mcp.as_ref() else {
            return McpClientActivitySnapshot::default();
        };
        let receiver = handle.subscribe_client_activities();
        *receiver.borrow()
    }

    pub fn subscribe_network_events(&self) -> broadcast::Receiver<SequencedNetworkEvent> {
        self.network_events.subscribe()
    }

    pub(crate) fn network_event_is_current(&self, generation: u64) -> bool {
        network_generation_is_current(&self.network_generation, generation)
    }

    pub fn models_ready(&self) -> bool {
        read_lock(&self.models, "model services")
            .map(|models| models.is_some())
            .unwrap_or(false)
    }

    pub(crate) fn startup_preflight_blocks_automatic_scan(
        &self,
        collection_id: Uuid,
    ) -> Result<bool> {
        Ok(
            read_lock(&self.startup_preflight_blocked, "startup preflight")?
                .contains(&collection_id),
        )
    }

    pub(crate) fn clear_startup_preflight_block(&self, collection_id: Uuid) -> Result<()> {
        write_lock(&self.startup_preflight_blocked, "startup preflight")?.remove(&collection_id);
        Ok(())
    }

    /// Initializes ONNX/tokenizer state off the async executor's worker threads,
    /// then atomically enables local, LAN and MCP search.
    pub async fn enable_models(&self, paths: ModelRuntimePaths) -> Result<()> {
        paths.validate()?;
        let selection = paths.selection.clone();
        let generation_settings = paths.generation_settings;
        let mut supervisor_config = SupervisorConfig::bundled(paths.llama_server, paths.model);
        supervisor_config.model_id = selection.model_id.to_owned();
        supervisor_config.context_tokens = generation_settings.context_tokens;
        let supervisor = LlamaSupervisor::new(supervisor_config);
        let e5_snapshot_path = paths.e5_snapshot;
        let relevance_snapshot_path = paths.relevance_snapshot;
        let threads = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(2)
            .clamp(1, 4);
        let (embeddings, tokenizer, relevance) = tokio::task::spawn_blocking(move || {
            let snapshot = PinnedE5Snapshot::open(e5_snapshot_path)?;
            let embeddings: Arc<dyn EmbeddingProvider> =
                Arc::new(FastEmbedE5Small::from_snapshot(&snapshot, threads)?);
            let tokenizer: Arc<dyn Tokenizer> = Arc::new(E5Tokenizer::from_snapshot(&snapshot)?);
            let relevance_snapshot = PinnedMmarcoRerankerSnapshot::open(relevance_snapshot_path)?;
            let relevance: Arc<dyn EvidenceRelevanceProvider> = Arc::new(
                FastEmbedMmarcoReranker::from_snapshot(relevance_snapshot, threads)?,
            );
            Ok::<_, anyhow::Error>((embeddings, tokenizer, relevance))
        })
        .await
        .context("falló el worker de inicialización de los modelos de búsqueda")??;

        let smoke_embeddings = embeddings
            .embed(&[format!("query: {MODEL_SMOKE_TEST_DOCUMENT}")])
            .await
            .context("falló el smoke test de multilingual-e5-small")?;
        if smoke_embeddings.len() != 1
            || smoke_embeddings[0].len() != airwiki_core::EMBEDDING_DIMENSIONS
        {
            bail!("el smoke test de embeddings devolvió una forma inesperada");
        }

        let relevance_inputs = RELEVANCE_SMOKE_TEST_PASSAGES
            .into_iter()
            .map(|text| RelevanceInput {
                title: String::new(),
                heading: String::new(),
                text: text.to_owned(),
            })
            .collect::<Vec<_>>();
        let relevance_decisions = relevance
            .classify(RELEVANCE_SMOKE_TEST_QUESTION, &relevance_inputs)
            .await
            .context("falló el smoke test del clasificador de relevancia")?;
        if relevance_decisions != [EvidenceDecision::Relevant, EvidenceDecision::Irrelevant] {
            bail!("el smoke test de relevancia devolvió decisiones inesperadas");
        }

        let generation_provider = SupervisedGenerationProvider {
            supervisor: supervisor.clone(),
            model_id: selection.model_id.to_owned(),
            runtime_config: GenerationRuntimeConfig {
                model_id: selection.model_id.to_owned(),
                temperature: generation_settings.temperature,
                max_input_tokens: generation_settings.max_input_tokens as usize,
                max_output_tokens: generation_settings.max_output_tokens as usize,
                thinking_directive: match generation_settings.thinking_control {
                    ThinkingControl::None => None,
                    ThinkingControl::NoThinkDirective => Some("/no_think".to_owned()),
                },
            },
        };
        if let Err(error) = generation_provider.enrich(MODEL_SMOKE_TEST_DOCUMENT).await {
            let _ = supervisor.stop().await;
            return Err(error).with_context(|| {
                format!(
                    "falló el smoke test generativo de {}/llama.cpp",
                    selection.manifest.display_name
                )
            });
        }
        let generation: Arc<dyn GenerationProvider> = Arc::new(generation_provider);
        let pipeline = Arc::new(
            IngestPipeline::new(
                self.database.clone(),
                generation,
                embeddings.clone(),
                self.node_id.clone(),
            )
            .with_tokenizer(tokenizer),
        );
        let local = Arc::new(HybridSearchEngine::new(
            self.database.clone(),
            embeddings,
            relevance,
            self.node_id.clone(),
        ));
        // Keep LAN and MCP fail-closed until every collection has reconciled
        // hashes and tombstones. A source may have changed while the app was
        // closed; exposing the previous index before this pass would leak a
        // stale published revision during startup.
        let collection_ids = self
            .database
            .list_collections()?
            .into_iter()
            .map(|collection| collection.id)
            .collect::<Vec<_>>();
        let preflight = async {
            let mut blocked = HashSet::new();
            for collection_id in collection_ids {
                if pipeline
                    .preflight_collection_with_discovery_timeout(
                        collection_id,
                        STARTUP_COLLECTION_DISCOVERY_TIMEOUT,
                    )
                    .await
                    .is_err()
                {
                    // One unavailable folder must not disable healthy
                    // collections. Quarantine this one and continue; abort only
                    // if the fail-closed transition itself cannot be committed.
                    blocked.insert(collection_id);
                    let quarantine_pipeline = Arc::clone(&pipeline);
                    tokio::task::spawn_blocking(move || {
                        quarantine_pipeline.quarantine_collection(
                            collection_id,
                            "startup collection preflight did not complete safely",
                        )
                    })
                    .await
                    .context("falló el worker de cuarentena inicial")?
                    .context("no se pudo retirar una colección insegura durante el arranque")?;
                }
            }
            Ok::<_, anyhow::Error>(blocked)
        }
        .await;
        let blocked = match preflight {
            Ok(blocked) => blocked,
            Err(error) => {
                let _ = supervisor.stop().await;
                return Err(error).context("falló la prevalidación inicial de las colecciones");
            }
        };
        if let Err(error) = write_lock(&self.startup_preflight_blocked, "startup preflight")
            .map(|mut current| *current = blocked)
        {
            let _ = supervisor.stop().await;
            return Err(error);
        }

        // Commit the search topology under one short async lock. Model loading
        // and smoke tests deliberately happen above the lock so disabling LAN
        // remains responsive. Re-read the runtime here instead of retaining a
        // stale handle across a concurrent LAN transition.
        let _topology = self.search_topology.lock().await;
        let local_federated: Arc<dyn FederatedSearch> = local.clone();
        let federated: Arc<dyn FederatedSearch> = if let Some(network) = self.network_handle()? {
            Arc::new(FederatedCoordinator::new(local_federated, network))
        } else {
            Arc::new(LocalOnlyFederatedSearch::new(
                local_federated,
                self.database.clone(),
            ))
        };
        // Install both search paths before advertising readiness to the worker.
        self.authorized_proxy.install(local.clone())?;
        self.federated_proxy.install(federated)?;
        let old = write_lock(&self.models, "model services")?.replace(ModelServices {
            supervisor,
            pipeline,
            local_search: local,
        });
        drop(_topology);
        if let Some(old) = old {
            old.supervisor.stop().await?;
        }
        Ok(())
    }

    pub fn add_collection(
        &self,
        name: impl Into<String>,
        source_folder: impl AsRef<Path>,
    ) -> Result<CollectionRecord> {
        let source_folder = source_folder.as_ref();
        if !source_folder.is_dir() {
            bail!("la carpeta fuente no existe: {}", source_folder.display());
        }
        let source_folder = source_folder
            .canonicalize()
            .context("no se pudo resolver la carpeta fuente")?;
        let managed_root = self
            .core_paths
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.core_paths.root.clone());
        if source_folder.starts_with(&managed_root) || managed_root.starts_with(&source_folder) {
            bail!(
                "la carpeta fuente no puede contener ni estar dentro del directorio administrado por AirWiki"
            );
        }
        for existing in self.database.list_collections()? {
            let existing_folder = existing
                .source_folder
                .canonicalize()
                .unwrap_or(existing.source_folder);
            if source_folder.starts_with(&existing_folder)
                || existing_folder.starts_with(&source_folder)
            {
                bail!(
                    "la carpeta fuente se superpone con la colección {}",
                    existing.name
                );
            }
        }
        let bundle_root = self.core_paths.vaults.join(Uuid::new_v4().to_string());
        let collection = self.database.create_collection(
            name,
            &source_folder,
            bundle_root,
            CollectionPolicy::local_only(),
        )?;
        self.audit(
            "collection_created",
            "collection",
            Some(collection.id.to_string()),
            serde_json::json!({"source_folder": "[LOCAL_PATH_REDACTED]"}),
        )?;
        Ok(collection)
    }

    pub fn relink_collection(
        &self,
        collection_id: Uuid,
        source_folder: impl AsRef<Path>,
    ) -> Result<()> {
        let source_folder = source_folder.as_ref();
        if !source_folder.is_dir() {
            bail!("la carpeta seleccionada no existe");
        }
        let source_folder = source_folder
            .canonicalize()
            .context("no se pudo resolver la carpeta seleccionada")?;
        let managed_root = self
            .core_paths
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.core_paths.root.clone());
        if source_folder.starts_with(&managed_root) || managed_root.starts_with(&source_folder) {
            bail!("la carpeta seleccionada se superpone con los datos administrados");
        }
        for existing in self.database.list_collections()? {
            if existing.id == collection_id {
                continue;
            }
            let existing_folder = existing
                .source_folder
                .canonicalize()
                .unwrap_or(existing.source_folder);
            if source_folder.starts_with(&existing_folder)
                || existing_folder.starts_with(&source_folder)
            {
                bail!("la carpeta seleccionada se superpone con otra colección");
            }
        }
        self.database
            .collection(collection_id)?
            .context("la colección ya no existe")?;
        self.database
            .update_collection_source_folder(collection_id, &source_folder)?;
        self.audit(
            "collection_source_relinked",
            "collection",
            Some(collection_id.to_string()),
            serde_json::json!({"source_folder": "[LOCAL_PATH_REDACTED]"}),
        )
    }

    pub fn update_collection_policy(
        &self,
        collection_id: Uuid,
        policy: CollectionPolicy,
    ) -> Result<()> {
        self.database
            .update_collection_policy(collection_id, policy)?;
        let effective = self
            .database
            .collection(collection_id)?
            .context("la colección desapareció al actualizar su política")?
            .policy;
        if !effective.peer_shareable {
            // Remove stale grants from both durable state and the AccessControl
            // instance shared with the running libp2p swarm. The authorized
            // proxy also revalidates policy on every request as defense in depth.
            for grant in self.database.list_grants(None)? {
                if grant.collection_id == collection_id {
                    self.database
                        .set_grant(&grant.peer_id, collection_id, false)?;
                    if let Ok(peer) = PeerId::from_str(&grant.peer_id) {
                        self.access.remove_grant(peer, collection_id);
                    }
                }
            }
        }
        self.audit(
            "collection_policy_updated",
            "collection",
            Some(collection_id.to_string()),
            serde_json::json!({
                "local_only": effective.local_only,
                "peer_shareable": effective.peer_shareable,
                "allow_external_ai": effective.allow_external_ai,
            }),
        )
    }

    pub async fn scan_collection(&self, collection_id: Uuid) -> Result<Vec<IngestOutcome>> {
        self.pipeline()?.scan_collection(collection_id).await
    }

    /// Reconciles filesystem hashes and immediately withdraws stale published
    /// revisions without waiting for parsing, embeddings or local generation.
    /// The worker runs this on a blocking thread when a watcher event arrives
    /// while another scan is still performing inference.
    pub fn preflight_collection(&self, collection_id: Uuid) -> Result<()> {
        let _ = self.pipeline()?.preflight_collection(collection_id)?;
        Ok(())
    }

    pub fn quarantine_collection(&self, collection_id: Uuid, reason: &str) -> Result<()> {
        self.database
            .collection(collection_id)?
            .with_context(|| format!("la colección {collection_id} no existe"))?;
        let artifacts = self.database.quarantine_collection(collection_id, reason)?;
        OkfPublicationMaterializer::new(self.database.clone())
            .withdraw_published_artifacts(collection_id, &artifacts)?;
        self.audit(
            "collection_quarantined",
            "collection",
            Some(collection_id.to_string()),
            serde_json::json!({"reason": reason}),
        )
    }

    pub fn approve_review(&self, concept_id: Uuid, draft: EnrichmentDraft) -> Result<()> {
        self.pipeline()?
            .approve(concept_id, ReviewEdits { draft })?;
        Ok(())
    }

    /// A rejection is fail-closed: the concept remains in NeedsReview and is
    /// therefore absent from all searchable publication surfaces. Chunks are
    /// deliberately retained so a human can edit and approve it later.
    pub fn reject_review(&self, concept_id: Uuid) -> Result<()> {
        let concept = self
            .database
            .concept(concept_id)?
            .context("el concepto a rechazar no existe")?;
        if concept.status != DocumentStatus::NeedsReview {
            bail!("solo se puede rechazar un concepto pendiente de revisión");
        }
        self.audit(
            "review_rejected",
            "concept",
            Some(concept_id.to_string()),
            serde_json::json!({}),
        )
    }

    pub async fn reanalyze_review(&self, concept_id: Uuid) -> Result<()> {
        self.pipeline()?.reanalyze_review(concept_id).await?;
        Ok(())
    }

    pub async fn search(
        &self,
        question: impl Into<String>,
        top_k: u8,
        purpose: SearchPurpose,
    ) -> std::result::Result<SearchResponse, SearchContractError> {
        self.federated_proxy
            .search(SearchRequest::new(question, purpose, top_k))
            .await
    }

    /// Reports whether both tasks that constitute the optional LAN runtime are
    /// still alive. It performs no network I/O and is used by the worker's
    /// periodic reconciliation before deciding whether a restart is needed.
    pub fn lan_runtime_is_healthy(&self) -> Result<bool> {
        Ok(mutex_lock(&self.network, "LAN runtime")?
            .as_ref()
            .is_some_and(|runtime| {
                !runtime.task.is_finished()
                    && !runtime.event_forwarder.is_finished()
                    && !runtime.handle.listener_is_unavailable()
                    && (runtime.handle.listener_is_ready()
                        || runtime.started_at.elapsed() < LAN_LISTENER_START_GRACE)
            }))
    }

    pub async fn enable_lan(&self) -> Result<()> {
        let _topology = self.search_topology.lock().await;
        if self.network_handle()?.is_some() {
            return Ok(());
        }
        let generation = self
            .network_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        let runtime = spawn_network_runtime(
            self.identity.clone(),
            self.access.clone(),
            self.authorized_proxy.clone(),
            self.network_events.clone(),
            Arc::clone(&self.network_generation),
            generation,
        )?;
        let local = match read_lock(&self.models, "model services") {
            Ok(models) => models.as_ref().map(|models| models.local_search.clone()),
            Err(error) => {
                let _ = runtime.handle.shutdown().await;
                runtime.event_forwarder.abort();
                runtime.task.abort();
                return Err(error);
            }
        };
        let search_install = if let Some(local) = local {
            self.authorized_proxy.install(local.clone()).and_then(|()| {
                let local: Arc<dyn FederatedSearch> = local;
                self.federated_proxy
                    .install(Arc::new(FederatedCoordinator::new(
                        local,
                        runtime.handle.clone(),
                    )))
            })
        } else {
            Ok(())
        };
        if let Err(error) = search_install {
            self.authorized_proxy.clear().ok();
            let _ = runtime.handle.shutdown().await;
            runtime.event_forwarder.abort();
            runtime.task.abort();
            return Err(error);
        }
        let mut pending_runtime = Some(runtime);
        let install_result = mutex_lock(&self.network, "LAN runtime").map(|mut network| {
            if network.is_none() {
                *network = pending_runtime.take();
            }
        });
        if let Err(error) = install_result {
            self.authorized_proxy.clear().ok();
            if let Some(runtime) = pending_runtime {
                let _ = runtime.handle.shutdown().await;
                runtime.event_forwarder.abort();
                runtime.task.abort();
            }
            return Err(error);
        }
        if let Some(runtime) = pending_runtime {
            let _ = runtime.handle.shutdown().await;
            runtime.event_forwarder.abort();
            runtime.task.abort();
            return Ok(());
        }
        Ok(())
    }

    pub async fn disable_lan(&self) -> Result<()> {
        let _topology = self.search_topology.lock().await;
        // Invalidate the forwarder before taking the runtime so an already
        // queued event from the previous generation cannot mutate trust or UI
        // state after LAN was disabled or restarted.
        self.network_generation.fetch_add(1, Ordering::AcqRel);
        let runtime = mutex_lock(&self.network, "LAN runtime")?.take();
        let Some(runtime) = runtime else {
            return Ok(());
        };
        // Withdraw the inbound evidence backend before asking libp2p to close.
        // This makes an in-flight request fail closed during the short graceful
        // shutdown window after consent, profile or firewall readiness changes.
        let proxy_result = self.authorized_proxy.clear().and_then(|()| {
            self.federated_proxy.clear()?;
            let local = read_lock(&self.models, "model services")?
                .as_ref()
                .map(|models| models.local_search.clone());
            if let Some(local) = local {
                let local: Arc<dyn FederatedSearch> = local;
                self.federated_proxy
                    .install(Arc::new(LocalOnlyFederatedSearch::new(
                        local,
                        self.database.clone(),
                    )))?;
            }
            write_lock(&self.live_peers, "live peer view")?.clear();
            Ok(())
        });
        let shutdown = runtime.handle.shutdown().await;
        runtime.event_forwarder.abort();
        runtime.task.abort();
        proxy_result?;
        shutdown.context("no se pudo detener la red LAN")
    }

    pub async fn dial(&self, address: &str) -> Result<()> {
        let address = ManualLanAddress::from_str(address.trim())
            .context("la dirección LAN manual no es válida")?;
        let route_candidate = address.clone();
        tokio::task::spawn_blocking(move || manual_lan_route::validate(&route_candidate))
            .await
            .context("no se pudo comprobar la subred local")??;
        self.required_network()?.dial(address.into()).await?;
        Ok(())
    }

    pub async fn begin_pairing(&self, peer_id: &str) -> Result<()> {
        let peer = parse_peer_id(peer_id)?;
        self.required_network()?.begin_pairing(peer).await?;
        Ok(())
    }

    pub async fn confirm_pairing(&self, peer_id: &str, accepted: bool) -> Result<()> {
        let peer = parse_peer_id(peer_id)?;
        let network = self.required_network()?;
        if accepted {
            network.confirm_pairing(peer).await?;
        } else {
            // Persist the human SAS rejection immediately. The corresponding
            // NetworkEvent repeats this idempotently after the connection closes.
            self.access.block(peer);
            let persistence = persist_blocked_peer(&self.database, peer, None);
            let network = network.reject_pairing(peer).await;
            persistence?;
            network?;
        }
        Ok(())
    }

    pub async fn set_collection_grant(
        &self,
        peer_id: &str,
        collection_id: Uuid,
        granted: bool,
    ) -> Result<()> {
        let peer = parse_peer_id(peer_id)?;
        let collection = self
            .database
            .collection(collection_id)?
            .context("la colección no existe")?;
        if granted && !collection.policy.peer_shareable {
            bail!("la colección no permite compartir evidencia con peers");
        }
        if granted {
            let peer_access = self.access.state(&peer);
            if peer_access.blocked {
                bail!("el PeerId está bloqueado");
            }
            if !peer_access.trusted {
                bail!("el PeerId no está emparejado como dispositivo confiable");
            }
            let durable_peer = self
                .database
                .peer(peer_id)?
                .context("el PeerId no existe en el almacén de confianza")?;
            if !durable_peer.trusted || durable_peer.blocked {
                bail!("el PeerId no está autorizado por el almacén persistente");
            }
        }

        // Apply denial to the shared in-memory ACL before any fallible storage
        // operation. Granting is the reverse: stage it in ACL, persist, and roll
        // it back on any failure.
        if granted {
            self.access.grant(peer, collection_id)?;
            if let Err(error) = self.database.set_grant(peer_id, collection_id, true) {
                self.access.remove_grant(peer, collection_id);
                return Err(error);
            }
            if let Some(network) = self.network_handle()?
                && let Err(error) = network.grant_collection(peer, collection_id).await
            {
                self.access.remove_grant(peer, collection_id);
                self.database.set_grant(peer_id, collection_id, false)?;
                return Err(error.into());
            }
        } else {
            self.access.remove_grant(peer, collection_id);
            let persistence = self.database.set_grant(peer_id, collection_id, false);
            let network: std::result::Result<(), airwiki_network::NetworkError> =
                if let Some(network) = self.network_handle()? {
                    network.remove_grant(peer, collection_id).await
                } else {
                    Ok(())
                };
            persistence?;
            network?;
        }
        self.audit(
            if granted {
                "collection_granted"
            } else {
                "collection_grant_removed"
            },
            "grant",
            Some(format!("{peer_id}:{collection_id}")),
            serde_json::json!({"peer_id": peer_id, "collection_id": collection_id}),
        )
    }

    pub async fn revoke_peer(&self, peer_id: &str) -> Result<()> {
        let peer = parse_peer_id(peer_id)?;
        self.access.revoke_and_block(peer);
        // Always close the connection even when durable storage is unhealthy;
        // the shared ACL already denies inbound searches synchronously.
        let network: std::result::Result<(), airwiki_network::NetworkError> =
            if let Some(network) = self.network_handle()? {
                network.revoke_peer(peer).await
            } else {
                Ok(())
            };
        let persistence = self.database.revoke_peer(peer_id);
        persistence?;
        network?;
        self.audit(
            "peer_revoked",
            "peer",
            Some(peer_id.to_owned()),
            serde_json::json!({"peer_id": peer_id}),
        )
    }

    pub fn collection_views(&self) -> Result<Vec<CollectionView>> {
        self.database
            .list_collections()?
            .into_iter()
            .map(|collection| {
                let stats = self.database.collection_stats(collection.id)?;
                Ok(CollectionView {
                    id: collection.id,
                    name: collection.name,
                    folder: collection.source_folder,
                    document_count: usize::try_from(stats.sources).unwrap_or(usize::MAX),
                    published_count: usize::try_from(stats.published).unwrap_or(usize::MAX),
                    local_only: collection.policy.local_only,
                    peer_shareable: collection.policy.peer_shareable,
                    allow_external_ai: collection.policy.allow_external_ai,
                    maintenance: self.database.collection_maintenance(collection.id)?,
                })
            })
            .collect()
    }

    /// Builds the read-only bundle snapshot used by the local Wiki, graph and
    /// health views. Inspection may briefly observe SQLite and the atomically
    /// published OKF files on opposite sides of a publication boundary, so an
    /// `Updating` result and transient read failures are retried off the UI
    /// thread before being returned to the caller.
    pub fn load_knowledge_bundle(&self, collection_id: Uuid) -> Result<KnowledgeBundleView> {
        let inspector = OkfBundleInspector::new(self.database.clone());
        retry_knowledge_read(
            || inspector.inspect_bundle(collection_id),
            |view| matches!(&view.state, KnowledgeBundleState::Updating),
            KNOWLEDGE_READ_ATTEMPTS,
            KNOWLEDGE_READ_RETRY_DELAY,
        )
    }

    /// Loads at most one MiB from a page selected from a previously inspected
    /// bundle. The expected fingerprint makes a stale UI selection fail closed
    /// rather than displaying a different revision under the old metadata.
    pub fn load_knowledge_page(
        &self,
        collection_id: Uuid,
        page_id: KnowledgePageId,
        expected_fingerprint: &str,
    ) -> Result<KnowledgePageView> {
        let inspector = OkfBundleInspector::new(self.database.clone());
        retry_knowledge_read(
            || {
                inspector.load_page(
                    collection_id,
                    page_id,
                    Some(expected_fingerprint),
                    KNOWLEDGE_PAGE_MAX_BYTES,
                )
            },
            |_| false,
            KNOWLEDGE_READ_ATTEMPTS,
            KNOWLEDGE_READ_RETRY_DELAY,
        )
    }

    /// Regenerates only deterministic derived Wiki artifacts. Content-bearing
    /// concepts and history are left untouched for the guided repair UI.
    pub fn maintain_derived_wiki(&self, collection_id: Uuid) -> Result<bool> {
        let bundle =
            OkfBundleInspector::new(self.database.clone()).inspect_bundle(collection_id)?;
        let plan = WikiRepairPlanner::plan(&bundle)?;
        let has_derived_action = plan.previews.iter().any(|preview| {
            !preview.requires_confirmation
                && preview.action.risk() == airwiki_core::RepairRisk::Derived
        });
        if !has_derived_action {
            return Ok(false);
        }
        WikiRepairExecutor::new(self.database.clone()).execute_automatic(&plan)?;
        Ok(true)
    }

    pub fn prepare_guided_wiki_repair(&self, collection_id: Uuid) -> Result<GuidedRepairPreview> {
        let bundle =
            OkfBundleInspector::new(self.database.clone()).inspect_bundle(collection_id)?;
        let plan = WikiRepairPlanner::plan(&bundle)?;
        WikiRepairExecutor::new(self.database.clone())
            .prepare_guided(&plan)
            .map_err(Into::into)
    }

    pub fn execute_guided_wiki_repair(
        &self,
        preview: &GuidedRepairPreview,
    ) -> Result<GuidedRepairResult> {
        WikiRepairExecutor::new(self.database.clone())
            .execute_guided(preview)
            .map_err(Into::into)
    }

    pub fn wiki_health_rollup(&self) -> Result<(usize, usize, usize)> {
        let inspector = OkfBundleInspector::new(self.database.clone());
        let mut errors = 0_usize;
        let mut warnings = 0_usize;
        let mut updating = 0_usize;
        for collection in self.database.list_collections()? {
            let bundle = inspector.inspect_bundle(collection.id)?;
            errors = errors.saturating_add(bundle.health.error_count);
            warnings = warnings.saturating_add(bundle.health.warning_count);
            updating = updating
                .saturating_add(usize::from(bundle.state == KnowledgeBundleState::Updating));
        }
        Ok((errors, warnings, updating))
    }

    pub fn review_views(&self) -> Result<Vec<ReviewItemView>> {
        self.database
            .list_concepts_for_review()?
            .into_iter()
            .map(|concept| {
                let source = self
                    .database
                    .source_document(concept.source_document_id)?
                    .context("un concepto en revisión perdió su documento fuente")?;
                let collection = self
                    .database
                    .collection(concept.collection_id)?
                    .context("un concepto en revisión perdió su colección")?;
                let source_name = source
                    .source_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Documento")
                    .to_owned();
                Ok(ReviewItemView {
                    concept_id: concept.id,
                    source_name,
                    collection_name: collection.name,
                    draft: concept.draft,
                })
            })
            .collect()
    }

    pub fn peer_views(&self) -> Result<Vec<PeerView>> {
        let persisted = self
            .database
            .list_peers()?
            .into_iter()
            .map(|peer| (peer.peer_id.clone(), peer))
            .collect::<HashMap<_, _>>();
        let grants = self.database.list_grants(None)?;
        let mut grants_by_peer = HashMap::<String, HashSet<Uuid>>::new();
        for grant in grants {
            grants_by_peer
                .entry(grant.peer_id)
                .or_default()
                .insert(grant.collection_id);
        }
        let live = read_lock(&self.live_peers, "live peer view")?.clone();
        let mut peer_ids = persisted.keys().cloned().collect::<HashSet<_>>();
        peer_ids.extend(live.keys().map(ToString::to_string));
        let mut views = Vec::with_capacity(peer_ids.len());
        for peer_id in peer_ids {
            let peer = PeerId::from_str(&peer_id).ok();
            let runtime = peer.as_ref().and_then(|peer| live.get(peer));
            let stored = persisted.get(&peer_id);
            let trusted = stored.is_some_and(|peer| peer.trusted);
            let blocked = stored.is_some_and(|peer| peer.blocked);
            let (trust, activity) = peer_presentation_states(trusted, blocked, runtime);
            views.push(PeerView {
                peer_id: peer_id.clone(),
                device_name: stored
                    .and_then(|peer| peer.display_name.clone())
                    .or_else(|| runtime.and_then(|peer| peer.node_name.clone())),
                address: runtime
                    .and_then(|peer| peer.addresses.iter().next().cloned())
                    .unwrap_or_default(),
                trust,
                activity,
                sas_words: runtime.and_then(|peer| peer.sas_words.clone()),
                granted_collections: grants_by_peer.remove(&peer_id).unwrap_or_default(),
            });
        }
        views.sort_by(|left, right| {
            left.device_name
                .as_deref()
                .unwrap_or(&left.peer_id)
                .to_lowercase()
                .cmp(
                    &right
                        .device_name
                        .as_deref()
                        .unwrap_or(&right.peer_id)
                        .to_lowercase(),
                )
        });
        Ok(views)
    }

    /// Applies one broadcast event to durable and presentation state. The worker
    /// should call this from its Tokio select loop and then refresh PeerView when
    /// `peers_changed` is true.
    pub fn handle_network_event(&self, event: NetworkEvent) -> Result<NetworkEventEffect> {
        let mut effect = NetworkEventEffect::default();
        match event {
            NetworkEvent::DiscoveryStarted => {}
            NetworkEvent::Listening { address } => {
                let _ = address;
                effect.notice = Some("La red local está lista".to_owned());
            }
            NetworkEvent::ListenerUnavailable => {
                effect.warning = Some(
                    "La conexión local se interrumpió; AirWiki intentará recuperarla".to_owned(),
                );
            }
            NetworkEvent::Discovered { peer, address } => {
                let mut live = write_lock(&self.live_peers, "live peer view")?;
                if let Some(state) = live_peer_slot(&mut live, peer, false) {
                    record_live_discovery(state, address.to_string());
                    effect.peers_changed = true;
                }
            }
            NetworkEvent::DiscoveryExpired { peer, address } => {
                let mut live = write_lock(&self.live_peers, "live peer view")?;
                if let Some(state) = live.get_mut(&peer) {
                    let address = address.to_string();
                    state.addresses.retain(|candidate| candidate != &address);
                    if state.addresses.is_empty() && !state.connected && !state.pairing {
                        live.remove(&peer);
                    }
                }
                effect.peers_changed = true;
            }
            NetworkEvent::Connected { peer } => {
                let mut live = write_lock(&self.live_peers, "live peer view")?;
                if let Some(state) = live_peer_slot(&mut live, peer, true) {
                    state.connected = true;
                }
                drop(live);
                touch_known_peer(&self.database, peer)?;
                effect.peers_changed = true;
            }
            NetworkEvent::Disconnected { peer } => {
                if let Some(state) = write_lock(&self.live_peers, "live peer view")?.get_mut(&peer)
                {
                    state.connected = false;
                }
                effect.peers_changed = true;
            }
            NetworkEvent::PairingCode {
                peer,
                remote_node_name,
                sas,
                expires_in_seconds,
            } => {
                let mut live = write_lock(&self.live_peers, "live peer view")?;
                if let Some(state) = live_peer_slot(&mut live, peer, true) {
                    state.pairing = true;
                    state.node_name = Some(remote_node_name);
                    state.sas_words = Some(sas.words);
                    effect.peers_changed = true;
                    effect.notice = Some(format!(
                        "Confirma el código SAS antes de {expires_in_seconds} segundos"
                    ));
                } else {
                    effect.warning = Some(
                        "Hay demasiados dispositivos activos para mostrar otro emparejamiento"
                            .to_owned(),
                    );
                }
            }
            NetworkEvent::PairingTrusted { peer } => {
                let node_name = {
                    let mut live = write_lock(&self.live_peers, "live peer view")?;
                    live_peer_slot(&mut live, peer, true).and_then(|state| {
                        state.pairing = false;
                        state.sas_words = None;
                        state.connected = true;
                        state.node_name.clone()
                    })
                };
                if let Err(error) = persist_trusted_peer(&self.database, peer, node_name) {
                    // The libp2p runtime completed the SAS exchange first. If
                    // durable trust cannot be committed, immediately narrow the
                    // shared ACL so neither inbound evidence nor outbound queries
                    // treat this device as paired.
                    self.access.block(peer);
                    return Err(error).context("no se pudo persistir el emparejamiento");
                }
                self.access.mark_trusted(peer);
                self.audit(
                    "peer_paired",
                    "peer",
                    Some(peer.to_string()),
                    serde_json::json!({"peer_id": peer.to_string()}),
                )?;
                effect.peers_changed = true;
                effect.notice = Some("Nodo emparejado y confiable".into());
            }
            NetworkEvent::PairingExpired { peer } => {
                clear_pairing(&self.live_peers, peer)?;
                effect.peers_changed = true;
                effect.warning = Some("El emparejamiento expiró".into());
            }
            NetworkEvent::PairingCancelled { peer, blocked } => {
                clear_pairing(&self.live_peers, peer)?;
                if blocked {
                    self.access.block(peer);
                    persist_blocked_peer(&self.database, peer, None)?;
                }
                effect.peers_changed = true;
                effect.warning = Some(if blocked {
                    "Código SAS rechazado; el PeerId quedó bloqueado".into()
                } else {
                    "Emparejamiento cancelado".into()
                });
            }
            NetworkEvent::PairingFailed { peer, reason } => {
                clear_pairing(&self.live_peers, peer)?;
                effect.peers_changed = true;
                effect.warning = Some(pairing_failure_message(reason).to_owned());
            }
            NetworkEvent::PeerRevoked { peer } => {
                self.access.revoke_and_block(peer);
                persist_blocked_peer(&self.database, peer, None)?;
                effect.peers_changed = true;
                effect.notice = Some("Peer revocado y conexiones cerradas".into());
            }
            NetworkEvent::InboundSearchCompleted {
                peer,
                request_id,
                hits,
            } => {
                tracing::info!(%peer, %request_id, hits, "served authorized LAN search");
            }
            NetworkEvent::Warning { peer, kind } => {
                tracing::warn!(
                    peer_present = peer.is_some(),
                    warning_kind = ?kind,
                    "LAN warning"
                );
                effect.warning = Some(network_warning_message(kind).to_owned());
            }
        }
        Ok(effect)
    }

    pub async fn shutdown(mut self) -> Result<()> {
        self.authorized_proxy.clear()?;
        self.federated_proxy.clear()?;
        let model = write_lock(&self.models, "model services")?.take();
        if let Some(model) = model {
            model.supervisor.stop().await?;
        }
        if let Some(mcp) = self.mcp.take() {
            mcp.shutdown().await?;
        }
        let network = self
            .network
            .get_mut()
            .map_err(|_| anyhow!("LAN runtime lock está envenenado"))?
            .take();
        if let Some(runtime) = network {
            let _ = runtime.handle.shutdown().await;
            runtime.event_forwarder.abort();
            runtime.task.abort();
        }
        Ok(())
    }

    fn network_handle(&self) -> Result<Option<NetworkHandle>> {
        Ok(mutex_lock(&self.network, "LAN runtime")?
            .as_ref()
            .map(|runtime| runtime.handle.clone()))
    }

    fn required_network(&self) -> Result<NetworkHandle> {
        self.network_handle()?
            .context("la red local está desactivada")
    }

    fn pipeline(&self) -> Result<Arc<IngestPipeline>> {
        read_lock(&self.models, "model services")?
            .as_ref()
            .map(|models| models.pipeline.clone())
            .ok_or_else(|| anyhow!("los modelos locales aún no están listos"))
    }

    fn audit(
        &self,
        action: &str,
        target_type: &str,
        target_id: Option<String>,
        details: serde_json::Value,
    ) -> Result<()> {
        self.database.record_audit(&AuditEvent {
            id: Uuid::new_v4(),
            actor: "desktop".into(),
            action: action.into(),
            target_type: target_type.into(),
            target_id,
            details,
            created_at: Utc::now(),
        })
    }
}

impl Drop for DesktopServices {
    fn drop(&mut self) {
        self.authorized_proxy.clear().ok();
        self.federated_proxy.clear().ok();
        if let Some(mcp) = self.mcp.as_ref() {
            mcp.cancel();
        }
        if let Ok(network) = self.network.get_mut()
            && let Some(runtime) = network.take()
        {
            runtime.event_forwarder.abort();
            runtime.task.abort();
        }
    }
}

#[derive(Debug, Clone)]
pub enum CollectionWatchEvent {
    Changed {
        collection_id: Uuid,
        paths: Vec<PathBuf>,
    },
    Failed {
        collection_id: Uuid,
        error: String,
    },
}

/// Handle for a blocking OS watcher. It is intentionally detached from service
/// state so the worker can add/remove watched collections as commands arrive.
pub struct CollectionWatcherHandle {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl CollectionWatcherHandle {
    pub fn spawn(
        collection_id: Uuid,
        folder: impl AsRef<Path>,
        events: mpsc::UnboundedSender<CollectionWatchEvent>,
    ) -> Result<Self> {
        let watcher = FolderWatcher::two_second(folder)?;
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task = tokio::task::spawn_blocking(move || {
            while !task_cancel.is_cancelled() {
                match watcher.recv_debounced_timeout(Duration::from_millis(250)) {
                    Ok(Some(paths)) => {
                        if events
                            .send(CollectionWatchEvent::Changed {
                                collection_id,
                                paths,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(None) => continue,
                    Err(error) => {
                        let _ = events.send(CollectionWatchEvent::Failed {
                            collection_id,
                            error: error.to_string(),
                        });
                        break;
                    }
                }
            }
        });
        Ok(Self { cancel, task })
    }
}

impl Drop for CollectionWatcherHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

fn restore_access_control(database: &Database) -> Result<AccessControl> {
    let access = AccessControl::with_disclosure_gate(database.disclosure_gate());
    let mut grants = HashMap::<String, HashSet<Uuid>>::new();
    for grant in database.list_grants(None)? {
        grants
            .entry(grant.peer_id)
            .or_default()
            .insert(grant.collection_id);
    }
    for peer in database.list_peers()? {
        match PeerId::from_str(&peer.peer_id) {
            Ok(peer_id) => access.restore(
                peer_id,
                PeerAccess {
                    trusted: peer.trusted,
                    blocked: peer.blocked,
                    grants: grants.remove(&peer.peer_id).unwrap_or_default(),
                },
            ),
            Err(error) => tracing::warn!(
                peer_id = %peer.peer_id,
                %error,
                "ignoring malformed persisted PeerId"
            ),
        }
    }
    Ok(access)
}

fn persist_trusted_peer(
    database: &Database,
    peer: PeerId,
    display_name: Option<String>,
) -> Result<()> {
    let now = Utc::now();
    let existing = database.peer(&peer.to_string())?;
    database.upsert_peer(&airwiki_core::PeerRecord {
        peer_id: peer.to_string(),
        display_name: display_name.or_else(|| existing.as_ref()?.display_name.clone()),
        trusted: true,
        blocked: false,
        paired_at: existing
            .as_ref()
            .and_then(|peer| peer.paired_at)
            .or(Some(now)),
        last_seen_at: Some(now),
    })
}

fn persist_blocked_peer(
    database: &Database,
    peer: PeerId,
    display_name: Option<String>,
) -> Result<()> {
    let peer_id = peer.to_string();
    let existing = database.peer(&peer_id)?;
    database.upsert_peer(&airwiki_core::PeerRecord {
        peer_id: peer_id.clone(),
        display_name: display_name.or_else(|| existing.as_ref()?.display_name.clone()),
        trusted: false,
        blocked: true,
        paired_at: existing.as_ref().and_then(|peer| peer.paired_at),
        last_seen_at: existing.as_ref().and_then(|peer| peer.last_seen_at),
    })?;
    database.revoke_peer(&peer_id)
}

fn touch_known_peer(database: &Database, peer: PeerId) -> Result<()> {
    let peer_id = peer.to_string();
    if let Some(mut stored) = database.peer(&peer_id)? {
        stored.last_seen_at = Some(Utc::now());
        database.upsert_peer(&stored)?;
    }
    Ok(())
}

fn clear_pairing(peers: &RwLock<HashMap<PeerId, LivePeer>>, peer: PeerId) -> Result<()> {
    if let Some(state) = write_lock(peers, "live peer view")?.get_mut(&peer) {
        state.pairing = false;
        state.sas_words = None;
    }
    Ok(())
}

/// Returns a bounded runtime-only presentation slot.
///
/// High-priority connection and pairing events may replace an address-only
/// discovery. Durable trusted peers remain in SQLite and are not constrained by
/// this volatile UI cache.
fn live_peer_slot(
    peers: &mut HashMap<PeerId, LivePeer>,
    peer: PeerId,
    high_priority: bool,
) -> Option<&mut LivePeer> {
    if peers.contains_key(&peer) {
        return peers.get_mut(&peer);
    }
    if peers.len() >= MAX_VOLATILE_LAN_PEERS {
        if !high_priority {
            return None;
        }
        let eviction = peers
            .iter()
            .filter(|(_, state)| !state.connected && !state.pairing)
            .map(|(candidate, _)| *candidate)
            .min_by(|left, right| left.to_bytes().cmp(&right.to_bytes()));
        if let Some(eviction) = eviction {
            peers.remove(&eviction);
        }
    }
    if peers.len() >= MAX_VOLATILE_LAN_PEERS {
        return None;
    }
    Some(peers.entry(peer).or_default())
}

fn record_live_discovery(peer: &mut LivePeer, address: String) {
    peer.addresses.retain(|candidate| candidate != &address);
    peer.addresses.push_front(address);
    peer.addresses.truncate(MAX_MDNS_ADDRESSES_PER_PEER);
}

fn peer_presentation_states(
    trusted: bool,
    blocked: bool,
    runtime: Option<&LivePeer>,
) -> (PeerTrustState, PeerActivityState) {
    let trust = if blocked {
        PeerTrustState::Blocked
    } else if trusted {
        PeerTrustState::Trusted
    } else {
        PeerTrustState::Unpaired
    };
    let activity = if runtime.is_some_and(|peer| peer.pairing) {
        PeerActivityState::Pairing
    } else if runtime.is_some_and(|peer| peer.connected) {
        PeerActivityState::Connected
    } else if runtime.is_some_and(|peer| !peer.addresses.is_empty()) {
        PeerActivityState::Discovered
    } else {
        PeerActivityState::NotObserved
    };
    (trust, activity)
}

fn parse_peer_id(value: &str) -> Result<PeerId> {
    PeerId::from_str(value.trim()).with_context(|| format!("PeerId inválido: {value}"))
}

fn retry_knowledge_read<T, F, P>(
    mut read: F,
    mut is_updating: P,
    attempts: usize,
    delay: Duration,
) -> Result<T>
where
    F: FnMut() -> Result<T>,
    P: FnMut(&T) -> bool,
{
    assert!(attempts > 0, "knowledge read attempts must be positive");
    let mut last_updating = None;
    let mut last_error = None;
    for attempt in 0..attempts {
        match read() {
            Ok(value) if !is_updating(&value) => return Ok(value),
            Ok(value) => last_updating = Some(value),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < attempts && !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
    if let Some(value) = last_updating {
        Ok(value)
    } else {
        Err(last_error.unwrap_or_else(|| anyhow!("knowledge bundle inspection did not complete")))
    }
}

fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "AirWiki node".into())
        .chars()
        .take(120)
        .collect()
}

const fn pairing_failure_message(reason: PairingFailureReason) -> &'static str {
    match reason {
        PairingFailureReason::NoCurrentAddress => {
            "No se encontró una dirección vigente para emparejar; vuelve a buscar equipos"
        }
        PairingFailureReason::DialFailed | PairingFailureReason::ConnectionFailed => {
            "No se pudo conectar para emparejar; comprueba que ambos equipos estén disponibles"
        }
        PairingFailureReason::HandshakeFailed => {
            "No se pudo completar el emparejamiento seguro; vuelve a intentarlo"
        }
    }
}

const fn network_warning_message(kind: NetworkWarningKind) -> &'static str {
    match kind {
        NetworkWarningKind::DialFailed | NetworkWarningKind::ConnectionFailed => {
            "No se pudo conectar con el otro equipo; se volverá a intentar cuando esté disponible"
        }
        NetworkWarningKind::PairingStateInvalid | NetworkWarningKind::PairingProtocolFailed => {
            "El emparejamiento seguro no pudo continuar; vuelve a iniciarlo"
        }
        NetworkWarningKind::AccessChangeRejected => {
            "No se pudo actualizar el acceso del otro equipo"
        }
        NetworkWarningKind::ListenerDegraded => {
            "La conexión local tuvo un problema transitorio; AirWiki sigue supervisándola"
        }
        NetworkWarningKind::InboundSearchFailed => {
            "No se pudo completar una búsqueda solicitada por otro equipo"
        }
    }
}

fn spawn_network_runtime(
    identity: NodeIdentity,
    access: AccessControl,
    backend: Arc<DynamicAuthorizedSearchBackend>,
    presentation_events: broadcast::Sender<SequencedNetworkEvent>,
    active_generation: Arc<AtomicU64>,
    generation: u64,
) -> Result<NetworkRuntime> {
    let config = NetworkConfig {
        node_name: device_name(),
        ..NetworkConfig::default()
    };
    let (handle, events, task) = spawn_network(config, identity, access, backend)
        .context("no se pudo iniciar la red LAN privada")?;
    let event_forwarder = tokio::spawn(forward_network_events(
        events,
        presentation_events,
        active_generation,
        generation,
    ));
    Ok(NetworkRuntime {
        handle,
        task,
        event_forwarder,
        started_at: Instant::now(),
    })
}

async fn forward_network_events(
    mut events: broadcast::Receiver<NetworkEvent>,
    presentation_events: broadcast::Sender<SequencedNetworkEvent>,
    active_generation: Arc<AtomicU64>,
    generation: u64,
) {
    loop {
        match events.recv().await {
            Ok(event) => {
                if network_generation_is_current(&active_generation, generation) {
                    let _ = presentation_events.send(SequencedNetworkEvent { generation, event });
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(
                    skipped,
                    "LAN presentation events were coalesced; requesting a runtime restart"
                );
                if network_generation_is_current(&active_generation, generation) {
                    let _ = presentation_events.send(SequencedNetworkEvent {
                        generation,
                        event: NetworkEvent::ListenerUnavailable,
                    });
                }
                // Continuing would present a state assembled from an unknown
                // subset of events. Ending the forwarder also makes the runtime
                // health check fail closed until the worker restarts discovery.
                break;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

fn network_generation_is_current(active: &AtomicU64, event_generation: u64) -> bool {
    active.load(Ordering::Acquire) == event_generation
}

fn read_lock<'a, T>(lock: &'a RwLock<T>, label: &str) -> Result<std::sync::RwLockReadGuard<'a, T>> {
    lock.read()
        .map_err(|_| anyhow!("{label} lock está envenenado"))
}

fn mutex_lock<'a, T>(lock: &'a Mutex<T>, label: &str) -> Result<std::sync::MutexGuard<'a, T>> {
    lock.lock()
        .map_err(|_| anyhow!("{label} lock está envenenado"))
}

fn write_lock<'a, T>(
    lock: &'a RwLock<T>,
    label: &str,
) -> Result<std::sync::RwLockWriteGuard<'a, T>> {
    lock.write()
        .map_err(|_| anyhow!("{label} lock está envenenado"))
}

#[cfg(test)]
mod tests {
    use airwiki_network::MemorySecretStore;

    use super::*;

    struct EmptyFederatedSearch;

    #[test]
    fn stale_network_generation_is_rejected_after_runtime_restart() {
        let active = AtomicU64::new(4);
        let queued = SequencedNetworkEvent {
            generation: 3,
            event: NetworkEvent::ListenerUnavailable,
        };

        assert!(!network_generation_is_current(&active, queued.generation));
    }

    #[test]
    fn current_network_generation_is_accepted() {
        let active = AtomicU64::new(4);

        assert!(network_generation_is_current(&active, 4));
    }

    #[tokio::test]
    async fn lagged_internal_network_events_request_a_fail_closed_restart() {
        let (source, source_receiver) = broadcast::channel(1);
        let (presentation, mut presentation_receiver) = broadcast::channel(4);
        let generation = 7;
        let active_generation = Arc::new(AtomicU64::new(generation));
        source.send(NetworkEvent::DiscoveryStarted).unwrap();
        source.send(NetworkEvent::DiscoveryStarted).unwrap();

        forward_network_events(source_receiver, presentation, active_generation, generation).await;

        let forwarded = presentation_receiver.recv().await.unwrap();
        assert_eq!(forwarded.generation, generation);
        assert!(matches!(forwarded.event, NetworkEvent::ListenerUnavailable));
    }

    #[test]
    fn trusted_peer_remains_paired_without_an_idle_tcp_connection() {
        let (trust, activity) = peer_presentation_states(true, false, None);

        assert_eq!(trust, PeerTrustState::Trusted);
        assert_eq!(activity, PeerActivityState::NotObserved);
    }

    #[test]
    fn trusted_peer_can_enter_a_visible_repairing_state() {
        let runtime = LivePeer {
            pairing: true,
            sas_words: Some(std::array::from_fn(|index| format!("word-{index}"))),
            ..Default::default()
        };

        let (trust, activity) = peer_presentation_states(true, false, Some(&runtime));

        assert_eq!(trust, PeerTrustState::Trusted);
        assert_eq!(activity, PeerActivityState::Pairing);
        assert!(runtime.sas_words.is_some());
    }

    #[test]
    fn peer_connection_activity_does_not_grant_trust() {
        let runtime = LivePeer {
            connected: true,
            ..Default::default()
        };

        let (trust, activity) = peer_presentation_states(false, false, Some(&runtime));

        assert_eq!(trust, PeerTrustState::Unpaired);
        assert_eq!(activity, PeerActivityState::Connected);
    }

    #[test]
    fn volatile_peer_view_rejects_new_discovery_at_capacity() {
        let mut peers = HashMap::new();
        for _ in 0..MAX_VOLATILE_LAN_PEERS {
            peers.insert(PeerId::random(), LivePeer::default());
        }

        let slot = live_peer_slot(&mut peers, PeerId::random(), false);

        assert!(slot.is_none());
        assert_eq!(peers.len(), MAX_VOLATILE_LAN_PEERS);
    }

    #[test]
    fn connected_peer_replaces_address_only_discovery_at_capacity() {
        let mut peers = HashMap::new();
        for _ in 0..MAX_VOLATILE_LAN_PEERS {
            peers.insert(PeerId::random(), LivePeer::default());
        }
        let connected = PeerId::random();

        let slot = live_peer_slot(&mut peers, connected, true);

        assert!(slot.is_some());
        assert!(peers.contains_key(&connected));
        assert_eq!(peers.len(), MAX_VOLATILE_LAN_PEERS);
    }

    #[test]
    fn connected_peer_does_not_evict_other_active_peers() {
        let mut peers = HashMap::new();
        for _ in 0..MAX_VOLATILE_LAN_PEERS {
            peers.insert(
                PeerId::random(),
                LivePeer {
                    connected: true,
                    ..Default::default()
                },
            );
        }

        let slot = live_peer_slot(&mut peers, PeerId::random(), true);

        assert!(slot.is_none());
        assert_eq!(peers.len(), MAX_VOLATILE_LAN_PEERS);
    }

    #[test]
    fn live_discovery_retains_only_recent_bounded_addresses() {
        let mut peer = LivePeer::default();
        for port in 0..=MAX_MDNS_ADDRESSES_PER_PEER {
            record_live_discovery(&mut peer, format!("/ip4/10.0.0.5/tcp/{}", 40_000 + port));
        }
        let newest = format!("/ip4/10.0.0.5/tcp/{}", 40_000 + MAX_MDNS_ADDRESSES_PER_PEER);

        assert_eq!(peer.addresses.len(), MAX_MDNS_ADDRESSES_PER_PEER);
        assert_eq!(
            peer.addresses.front().map(String::as_str),
            Some(newest.as_str())
        );
    }

    #[test]
    fn pairing_failures_are_sanitized_and_actionable() {
        for reason in [
            PairingFailureReason::NoCurrentAddress,
            PairingFailureReason::DialFailed,
            PairingFailureReason::ConnectionFailed,
            PairingFailureReason::HandshakeFailed,
        ] {
            let message = pairing_failure_message(reason);
            assert!(message.contains("empareja"));
            assert!(!message.contains('/'));
            assert!(!message.contains("PeerId"));
        }
    }

    #[test]
    fn network_warning_messages_are_stable_and_do_not_expose_transport_details() {
        for kind in [
            NetworkWarningKind::DialFailed,
            NetworkWarningKind::PairingStateInvalid,
            NetworkWarningKind::AccessChangeRejected,
            NetworkWarningKind::ConnectionFailed,
            NetworkWarningKind::ListenerDegraded,
            NetworkWarningKind::InboundSearchFailed,
            NetworkWarningKind::PairingProtocolFailed,
        ] {
            let message = network_warning_message(kind);
            assert!(!message.contains("/ip4/"));
            assert!(!message.contains("PeerId"));
            assert!(!message.contains("libp2p"));
            assert!(!message.contains("error:"));
        }
    }

    #[async_trait]
    impl FederatedSearch for EmptyFederatedSearch {
        async fn search(
            &self,
            request: SearchRequest,
        ) -> std::result::Result<SearchResponse, SearchContractError> {
            Ok(SearchResponse::empty(request.request_id))
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum TestKnowledgeState {
        Updating(usize),
        Ready,
    }

    #[test]
    fn knowledge_read_retries_updating_until_ready() {
        let mut calls = 0_usize;
        let result = retry_knowledge_read(
            || {
                calls += 1;
                Ok(if calls < 3 {
                    TestKnowledgeState::Updating(calls)
                } else {
                    TestKnowledgeState::Ready
                })
            },
            |state| matches!(state, TestKnowledgeState::Updating(_)),
            3,
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(result, TestKnowledgeState::Ready);
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn disabled_federation_reports_a_gap_only_when_trusted_peers_exist() {
        let database = Database::in_memory().unwrap();
        let search =
            LocalOnlyFederatedSearch::new(Arc::new(EmptyFederatedSearch), database.clone());
        let first = search
            .search(SearchRequest::new(
                "atlas",
                SearchPurpose::LocalAssistant,
                3,
            ))
            .await
            .unwrap();
        assert!(!first.partial);
        assert!(first.warnings.is_empty());

        let identity = NodeIdentity::load_or_create(&MemorySecretStore::default()).unwrap();
        database
            .upsert_peer(&airwiki_core::PeerRecord {
                peer_id: identity.peer_id().to_string(),
                display_name: Some("trusted peer".to_owned()),
                trusted: true,
                blocked: false,
                paired_at: Some(Utc::now()),
                last_seen_at: None,
            })
            .unwrap();
        let second = search
            .search(SearchRequest::new(
                "atlas",
                SearchPurpose::LocalAssistant,
                3,
            ))
            .await
            .unwrap();
        assert!(second.partial);
        assert_eq!(second.warnings, ["federation_disabled"]);
        assert!(second.offline_nodes.is_empty());
    }

    #[test]
    fn knowledge_read_returns_last_updating_snapshot_after_bounded_retries() {
        let mut calls = 0_usize;
        let result = retry_knowledge_read(
            || {
                calls += 1;
                Ok(TestKnowledgeState::Updating(calls))
            },
            |state| matches!(state, TestKnowledgeState::Updating(_)),
            3,
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(result, TestKnowledgeState::Updating(3));
        assert_eq!(calls, 3);
    }

    #[test]
    fn knowledge_read_retries_errors_and_returns_the_last_failure() {
        let mut calls = 0_usize;
        let error = retry_knowledge_read::<(), _, _>(
            || {
                calls += 1;
                Err(anyhow!("attempt {calls}"))
            },
            |_| false,
            3,
            Duration::ZERO,
        )
        .unwrap_err();

        assert_eq!(calls, 3);
        assert!(error.to_string().contains("attempt 3"));
    }

    fn durable_fixture() -> (Database, DynamicAuthorizedSearchBackend, PeerId, Uuid) {
        let database = Database::in_memory().unwrap();
        let identity = NodeIdentity::load_or_create(&MemorySecretStore::default()).unwrap();
        let peer_id = identity.peer_id();
        let now = Utc::now();
        database
            .upsert_peer(&airwiki_core::PeerRecord {
                peer_id: peer_id.to_string(),
                display_name: Some("test peer".into()),
                trusted: true,
                blocked: false,
                paired_at: Some(now),
                last_seen_at: Some(now),
            })
            .unwrap();
        let unique = Uuid::new_v4();
        let collection = database
            .create_collection(
                "shared",
                std::env::temp_dir().join(format!("airwiki-source-{unique}")),
                std::env::temp_dir().join(format!("airwiki-wiki-{unique}")),
                CollectionPolicy::shared_with_peers(),
            )
            .unwrap();
        database
            .set_grant(&peer_id.to_string(), collection.id, true)
            .unwrap();
        let access = AccessControl::with_disclosure_gate(database.disclosure_gate());
        access.mark_trusted(peer_id);
        access.grant(peer_id, collection.id).unwrap();
        let proxy = DynamicAuthorizedSearchBackend::new(database.clone(), access);
        (database, proxy, peer_id, collection.id)
    }

    #[test]
    fn durable_peer_grant_and_policy_are_all_required() {
        let (database, proxy, peer_id, collection_id) = durable_fixture();
        let mut authorization = SearchAuthorization::new(
            peer_id.to_string(),
            vec![collection_id],
            SearchPurpose::LocalAssistant,
            database.disclosure_gate(),
        );
        assert_eq!(
            proxy
                .durable_authorized_collections(&authorization, SearchPurpose::LocalAssistant)
                .unwrap(),
            vec![collection_id]
        );

        // A runtime-only grant never overrides the durable grant table.
        database
            .set_grant(&peer_id.to_string(), collection_id, false)
            .unwrap();
        assert!(
            proxy
                .durable_authorized_collections(&authorization, SearchPurpose::LocalAssistant)
                .unwrap()
                .is_empty()
        );
        database
            .set_grant(&peer_id.to_string(), collection_id, true)
            .unwrap();

        // Conversely, a durable grant absent from the runtime snapshot is not
        // broadened into the current request.
        authorization.allowed_collections.clear();
        assert!(
            proxy
                .durable_authorized_collections(&authorization, SearchPurpose::LocalAssistant)
                .unwrap()
                .is_empty()
        );
        authorization.allowed_collections.push(collection_id);

        // Peer sharing does not imply permission to disclose to external AI.
        assert!(
            proxy
                .durable_authorized_collections(&authorization, SearchPurpose::ExternalAi)
                .unwrap()
                .is_empty()
        );
        database
            .update_collection_policy(
                collection_id,
                CollectionPolicy {
                    local_only: true,
                    peer_shareable: false,
                    allow_external_ai: true,
                },
            )
            .unwrap();
        assert!(
            proxy
                .durable_authorized_collections(&authorization, SearchPurpose::LocalAssistant)
                .unwrap()
                .is_empty()
        );
        assert!(
            proxy
                .durable_authorized_collections(&authorization, SearchPurpose::ExternalAi)
                .unwrap()
                .is_empty()
        );
        database
            .update_collection_policy(
                collection_id,
                CollectionPolicy {
                    local_only: false,
                    peer_shareable: true,
                    allow_external_ai: true,
                },
            )
            .unwrap();
        assert_eq!(
            proxy
                .durable_authorized_collections(&authorization, SearchPurpose::ExternalAi)
                .unwrap(),
            vec![collection_id]
        );

        // A UI revocation narrows the shared ACL before its SQLite transaction
        // completes; the final authorization check must honor it immediately.
        proxy.access.revoke_and_block(peer_id);
        assert!(matches!(
            proxy.durable_authorized_collections(&authorization, SearchPurpose::LocalAssistant),
            Err(SearchContractError::Unauthorized)
        ));
        proxy.access.mark_trusted(peer_id);
        proxy.access.grant(peer_id, collection_id).unwrap();

        database.revoke_peer(&peer_id.to_string()).unwrap();
        assert!(matches!(
            proxy.durable_authorized_collections(&authorization, SearchPurpose::LocalAssistant),
            Err(SearchContractError::Unauthorized)
        ));
    }

    #[test]
    fn final_disclosure_lease_observes_a_policy_change_before_handoff() {
        let (database, proxy, peer_id, collection_id) = durable_fixture();
        let source_path =
            std::env::temp_dir().join(format!("airwiki-disclosure-race-{}.md", Uuid::new_v4()));
        std::fs::write(&source_path, "Authorized evidence").unwrap();
        let source_hash = airwiki_core::sha256_file(&source_path).unwrap();
        let source = database
            .register_source(collection_id, &source_path, &source_hash, "markdown", 32)
            .unwrap();
        database.mark_extracted(source.id(), 0, 32).unwrap();
        let source_record = database.source_document(source.id()).unwrap().unwrap();
        let draft = EnrichmentDraft {
            concept_type: airwiki_types::ConceptType::Document,
            title: "Authorized evidence".into(),
            description: "Synthetic authorization fixture".into(),
            language: "en".into(),
            tags: vec![],
            entities: vec![],
            links: vec![],
            summary: "Synthetic evidence".into(),
            classification_confidence: 1.0,
            classification_explanation: "fixture".into(),
        };
        let concept = database
            .save_enrichment(source.id(), draft.clone(), "test-node", "test-model")
            .unwrap();
        let chunk_id = Uuid::new_v4();
        database
            .replace_chunks(
                concept.id,
                &[airwiki_core::StoredChunk {
                    id: chunk_id,
                    concept_id: concept.id,
                    source_document_id: source.id(),
                    collection_id,
                    ordinal: 0,
                    heading_or_page: "Fixture".into(),
                    text: "Authorized evidence".into(),
                    text_sha256: "chunk-hash".into(),
                    embedding: vec![0.0; airwiki_core::EMBEDDING_DIMENSIONS],
                    source_revision: source_record.revision,
                }],
            )
            .unwrap();
        let published = OkfPublicationMaterializer::new(database.clone())
            .approve(concept.id, draft)
            .unwrap();
        let authorization = proxy
            .access
            .authorize(&peer_id, SearchPurpose::LocalAssistant)
            .unwrap();
        let mut response = SearchResponse::empty(Uuid::new_v4());
        response.hits.push(airwiki_types::SearchHit {
            concept_id: published.id,
            collection_id,
            chunk_id,
            title: published.draft.title,
            snippet: "Authorized evidence".into(),
            heading_or_page: "Fixture".into(),
            logical_resource_uri: published.logical_resource_uri,
            source_revision: source_record.revision,
            source_sha256: source_hash,
            updated_at: published.updated_at,
            rank: 1,
            node_id: "test-node".into(),
        });

        database
            .update_collection_policy(collection_id, CollectionPolicy::local_only())
            .unwrap();
        let result = finalize_authorized_response_blocking(
            &database,
            &proxy.access,
            response,
            authorization,
            SearchPurpose::LocalAssistant,
        )
        .unwrap();

        assert!(result.response().hits.is_empty());
        std::fs::remove_file(source_path).unwrap();
    }

    #[test]
    fn unknown_or_malformed_caller_is_denied() {
        let (_database, proxy, _peer_id, collection_id) = durable_fixture();
        for caller in ["", "not-a-peer-id"] {
            let authorization = SearchAuthorization::new(
                caller.into(),
                vec![collection_id],
                SearchPurpose::LocalAssistant,
                proxy.database.disclosure_gate(),
            );
            assert!(matches!(
                proxy.durable_authorized_collections(&authorization, SearchPurpose::LocalAssistant),
                Err(SearchContractError::Unauthorized)
            ));
        }

        let unknown = NodeIdentity::load_or_create(&MemorySecretStore::default())
            .unwrap()
            .peer_id();
        let authorization = SearchAuthorization::new(
            unknown.to_string(),
            vec![collection_id],
            SearchPurpose::LocalAssistant,
            proxy.database.disclosure_gate(),
        );
        assert!(matches!(
            proxy.durable_authorized_collections(&authorization, SearchPurpose::LocalAssistant),
            Err(SearchContractError::Unauthorized)
        ));
    }
}
