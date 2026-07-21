//! Tokio-driven LAN runtime. The desktop UI talks to it only through channels.

use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use airwiki_types::{
    DisclosureLease, FederatedSearch, SearchAuthorization, SearchContractError, SearchHit,
    SearchPurpose, SearchRequest, SearchResponse,
};
use async_trait::async_trait;
use futures::StreamExt;
use libp2p::core::{ConnectedPoint, Transport, upgrade};
use libp2p::request_response::{self, OutboundRequestId, ProtocolSupport, ResponseChannel};
use libp2p::swarm::{
    DialError, NetworkBehaviour, SwarmEvent, behaviour::toggle::Toggle, dial_opts::DialOpts,
};
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, tcp, yamux};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tracing::{info, warn};

use crate::access::{AccessControl, AccessError};
use crate::address::{ManualLanAddress, PeerAddressBook};
use crate::codec::{
    BoundedSearchCodec, SearchWireError, SearchWireErrorCode, SearchWireResponse, response_fits,
};
use crate::identity::NodeIdentity;
use crate::pairing::{
    PAIRING_PROTOCOL, PairingCode, PairingCoordinator, PairingNonce, PairingRequest,
    PairingResponse,
};
use crate::rate_limit::PeerRateLimiter;
use crate::{NetworkError, SEARCH_DEADLINE};

const PAIRING_FRAME_LIMIT: u64 = 4 * 1024;
const MDNS_QUERY_INTERVAL: Duration = Duration::from_secs(8);

fn supported_search_protocols() -> [(StreamProtocol, ProtocolSupport); 1] {
    [(
        StreamProtocol::new(airwiki_types::SEARCH_PROTOCOL),
        ProtocolSupport::Full,
    )]
}

#[async_trait]
pub trait AuthorizedSearchBackend: Send + Sync + 'static {
    /// Search only the collections in `authorization.allowed_collections` and enforce
    /// `external_ai` policy at the source node. Implementations must perform their final
    /// durable revalidation while holding the lease returned with the response.
    async fn search_authorized(
        &self,
        request: SearchRequest,
        authorization: SearchAuthorization,
    ) -> Result<AuthorizedSearchResult, SearchContractError>;
}

pub struct AuthorizedSearchResult {
    response: SearchResponse,
    disclosure_lease: DisclosureLease,
}

impl AuthorizedSearchResult {
    pub fn new(response: SearchResponse, disclosure_lease: DisclosureLease) -> Self {
        Self {
            response,
            disclosure_lease,
        }
    }

    pub fn response(&self) -> &SearchResponse {
        &self.response
    }
}

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub node_name: String,
    pub listen_address: Multiaddr,
    pub search_deadline: Duration,
    pub command_capacity: usize,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            node_name: "AirWiki node".to_owned(),
            listen_address: "/ip4/0.0.0.0/tcp/0"
                .parse()
                .expect("static listen multiaddress is valid"),
            search_deadline: SEARCH_DEADLINE,
            command_capacity: 64,
        }
    }
}

#[derive(Debug, Clone)]
pub enum NetworkEvent {
    /// The mDNS behaviour has been constructed and its event loop is running.
    /// This is distinct from the TCP listener becoming ready.
    DiscoveryStarted,
    Listening {
        address: Multiaddr,
    },
    /// The configured TCP listener closed and the node cannot accept new LAN
    /// connections until the desktop restarts the runtime.
    ListenerUnavailable,
    Discovered {
        peer: PeerId,
        address: Multiaddr,
    },
    DiscoveryExpired {
        peer: PeerId,
        address: Multiaddr,
    },
    Connected {
        peer: PeerId,
    },
    Disconnected {
        peer: PeerId,
    },
    PairingCode {
        peer: PeerId,
        remote_node_name: String,
        sas: PairingCode,
        expires_in_seconds: u64,
    },
    PairingTrusted {
        peer: PeerId,
    },
    PairingExpired {
        peer: PeerId,
    },
    PairingCancelled {
        peer: PeerId,
        blocked: bool,
    },
    PairingFailed {
        peer: PeerId,
        reason: PairingFailureReason,
    },
    PeerRevoked {
        peer: PeerId,
    },
    InboundSearchCompleted {
        peer: PeerId,
        request_id: uuid::Uuid,
        hits: usize,
    },
    Warning {
        peer: Option<PeerId>,
        kind: NetworkWarningKind,
    },
}

/// Stable, content-free categories for non-fatal LAN diagnostics. Transport
/// errors can contain addresses and peer identifiers, so they never cross the
/// runtime boundary as strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkWarningKind {
    DialFailed,
    PairingStateInvalid,
    AccessChangeRejected,
    ConnectionFailed,
    ListenerDegraded,
    InboundSearchFailed,
    PairingProtocolFailed,
}

/// Stable, content-free reason for an outgoing pairing attempt to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingFailureReason {
    NoCurrentAddress,
    DialFailed,
    ConnectionFailed,
    HandshakeFailed,
}

#[derive(Debug, PartialEq, Eq)]
enum PairingDialSource {
    Explicit(Vec<Multiaddr>),
    Behaviour,
}

fn select_pairing_dial_source(
    addresses: Vec<Multiaddr>,
    mdns_has_peer: bool,
) -> Option<PairingDialSource> {
    if !addresses.is_empty() {
        Some(PairingDialSource::Explicit(addresses))
    } else if mdns_has_peer {
        Some(PairingDialSource::Behaviour)
    } else {
        None
    }
}

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "BehaviourEvent")]
struct AirWikiBehaviour {
    mdns: Toggle<libp2p::mdns::tokio::Behaviour>,
    search: request_response::Behaviour<BoundedSearchCodec>,
    pairing: request_response::cbor::Behaviour<PairingRequest, PairingResponse>,
}

#[derive(Debug)]
enum BehaviourEvent {
    Mdns(libp2p::mdns::Event),
    Search(request_response::Event<SearchRequest, SearchWireResponse>),
    Pairing(request_response::Event<PairingRequest, PairingResponse>),
}

impl From<libp2p::mdns::Event> for BehaviourEvent {
    fn from(event: libp2p::mdns::Event) -> Self {
        Self::Mdns(event)
    }
}

impl From<request_response::Event<SearchRequest, SearchWireResponse>> for BehaviourEvent {
    fn from(event: request_response::Event<SearchRequest, SearchWireResponse>) -> Self {
        Self::Search(event)
    }
}

impl From<request_response::Event<PairingRequest, PairingResponse>> for BehaviourEvent {
    fn from(event: request_response::Event<PairingRequest, PairingResponse>) -> Self {
        Self::Pairing(event)
    }
}

#[derive(Clone)]
pub struct NetworkHandle {
    local_peer_id: PeerId,
    command_tx: mpsc::Sender<Command>,
    event_tx: broadcast::Sender<NetworkEvent>,
    listener_ready: Arc<AtomicBool>,
    listener_unavailable: Arc<AtomicBool>,
}

impl std::fmt::Debug for NetworkHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkHandle")
            .field("local_peer_id", &self.local_peer_id)
            .finish_non_exhaustive()
    }
}

impl NetworkHandle {
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NetworkEvent> {
        self.event_tx.subscribe()
    }

    /// Reports whether the runtime has at least one current TCP listen address.
    pub fn listener_is_ready(&self) -> bool {
        self.listener_ready.load(Ordering::Acquire)
    }

    /// Reports a terminal listener closure independently from normal startup.
    pub fn listener_is_unavailable(&self) -> bool {
        self.listener_unavailable.load(Ordering::Acquire)
    }

    pub async fn dial(&self, address: Multiaddr) -> Result<(), NetworkError> {
        let address = ManualLanAddress::try_from(address)?.into_multiaddr();
        self.command(Command::Dial { address }).await
    }

    pub async fn begin_pairing(&self, peer: PeerId) -> Result<(), NetworkError> {
        self.command(Command::BeginPairing { peer }).await
    }

    pub async fn confirm_pairing(&self, peer: PeerId) -> Result<(), NetworkError> {
        self.command(Command::ConfirmPairing { peer }).await
    }

    /// Cancel the current pairing attempt without trusting or blocking the peer.
    pub async fn cancel_pairing(&self, peer: PeerId) -> Result<(), NetworkError> {
        self.command(Command::CancelPairing { peer, block: false })
            .await
    }

    /// Reject a pairing because the human-visible SAS differs. This blocks the peer
    /// and closes active connections; an explicit future pairing attempt can unblock it.
    pub async fn reject_pairing(&self, peer: PeerId) -> Result<(), NetworkError> {
        self.command(Command::CancelPairing { peer, block: true })
            .await
    }

    pub async fn grant_collection(
        &self,
        peer: PeerId,
        collection: uuid::Uuid,
    ) -> Result<(), NetworkError> {
        self.command(Command::Grant { peer, collection }).await
    }

    pub async fn remove_grant(
        &self,
        peer: PeerId,
        collection: uuid::Uuid,
    ) -> Result<(), NetworkError> {
        self.command(Command::RemoveGrant { peer, collection })
            .await
    }

    pub async fn revoke_peer(&self, peer: PeerId) -> Result<(), NetworkError> {
        self.command(Command::Revoke { peer }).await
    }

    pub async fn search_peers(
        &self,
        request: SearchRequest,
    ) -> Result<SearchResponse, NetworkError> {
        request.validate()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(Command::SearchPeers {
                request,
                reply: reply_tx,
            })
            .await
            .map_err(|_| NetworkError::RuntimeStopped)?;
        reply_rx.await.map_err(|_| NetworkError::RuntimeStopped)?
    }

    pub async fn shutdown(&self) -> Result<(), NetworkError> {
        let (completed_tx, completed_rx) = oneshot::channel();
        self.command(Command::Shutdown {
            completed: completed_tx,
        })
        .await?;
        completed_rx.await.map_err(|_| NetworkError::RuntimeStopped)
    }

    async fn command(&self, command: Command) -> Result<(), NetworkError> {
        self.command_tx
            .send(command)
            .await
            .map_err(|_| NetworkError::RuntimeStopped)
    }
}

#[async_trait]
impl FederatedSearch for NetworkHandle {
    async fn search(&self, request: SearchRequest) -> Result<SearchResponse, SearchContractError> {
        self.search_peers(request)
            .await
            .map_err(|error| SearchContractError::Unavailable(error.to_string()))
    }
}

enum Command {
    Dial {
        address: Multiaddr,
    },
    BeginPairing {
        peer: PeerId,
    },
    ConfirmPairing {
        peer: PeerId,
    },
    CancelPairing {
        peer: PeerId,
        block: bool,
    },
    Grant {
        peer: PeerId,
        collection: uuid::Uuid,
    },
    RemoveGrant {
        peer: PeerId,
        collection: uuid::Uuid,
    },
    Revoke {
        peer: PeerId,
    },
    SearchPeers {
        request: SearchRequest,
        reply: oneshot::Sender<Result<SearchResponse, NetworkError>>,
    },
    #[cfg(test)]
    DisconnectPeer {
        peer: PeerId,
    },
    #[cfg(test)]
    SearchThenRevokeAndBeginPairing {
        peer: PeerId,
        request: SearchRequest,
        reply: oneshot::Sender<Result<SearchResponse, NetworkError>>,
    },
    #[cfg(test)]
    RecordDiscoveredAddress {
        peer: PeerId,
        address: Multiaddr,
    },
    #[cfg(test)]
    PairingAttemptActive {
        peer: PeerId,
        reply: oneshot::Sender<bool>,
    },
    #[cfg(test)]
    ExpirePairings {
        now: Instant,
    },
    Shutdown {
        completed: oneshot::Sender<()>,
    },
}

struct BackendResult {
    peer: PeerId,
    request_id: uuid::Uuid,
    channel: ResponseChannel<SearchWireResponse>,
    response: SearchWireResponse,
    hit_count: usize,
    disclosure_lease: Option<DisclosureLease>,
}

struct QueryAggregate {
    request: SearchRequest,
    pending: HashMap<OutboundRequestId, PeerId>,
    connecting: HashSet<PeerId>,
    responses: Vec<(PeerId, SearchResponse)>,
    offline: HashSet<PeerId>,
    warnings: Vec<String>,
    deadline: Instant,
    reply: oneshot::Sender<Result<SearchResponse, NetworkError>>,
}

struct PairingHelloRequest {
    peer: PeerId,
    local_nonce: PairingNonce,
}

struct Runtime {
    swarm: Swarm<AirWikiBehaviour>,
    config: NetworkConfig,
    local_peer_id: PeerId,
    access: AccessControl,
    backend: Arc<dyn AuthorizedSearchBackend>,
    limiter: Arc<PeerRateLimiter>,
    pairing: PairingCoordinator,
    pairing_hellos: HashMap<OutboundRequestId, PairingHelloRequest>,
    pairing_confirms: HashMap<OutboundRequestId, PeerId>,
    pairing_dials: HashMap<PeerId, PairingNonce>,
    /// Peers whose durable block is lifted only for the current explicit
    /// re-pairing attempt. Every non-success terminal restores the block.
    pairing_reblock_on_failure: HashSet<PeerId>,
    addresses: PeerAddressBook,
    queries: HashMap<u64, QueryAggregate>,
    query_by_request: HashMap<OutboundRequestId, u64>,
    next_query_id: u64,
    command_rx: mpsc::Receiver<Command>,
    event_tx: broadcast::Sender<NetworkEvent>,
    listener_ready: Arc<AtomicBool>,
    listener_unavailable: Arc<AtomicBool>,
    listen_addresses: HashSet<Multiaddr>,
    backend_result_tx: mpsc::Sender<BackendResult>,
    backend_result_rx: mpsc::Receiver<BackendResult>,
    backend_tasks: JoinSet<()>,
}

pub fn spawn_network(
    config: NetworkConfig,
    identity: NodeIdentity,
    access: AccessControl,
    backend: Arc<dyn AuthorizedSearchBackend>,
) -> Result<
    (
        NetworkHandle,
        broadcast::Receiver<NetworkEvent>,
        JoinHandle<()>,
    ),
    NetworkError,
> {
    let local_peer_id = identity.peer_id();
    let noise_config = libp2p::noise::Config::new(identity.keypair())
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    let transport = tcp::tokio::Transport::new(tcp::Config::default().nodelay(true))
        .upgrade(upgrade::Version::V1)
        .authenticate(noise_config)
        .multiplex(yamux::Config::default())
        .timeout(Duration::from_secs(10))
        .boxed();

    let request_config = request_response::Config::default()
        .with_request_timeout(config.search_deadline)
        .with_max_concurrent_streams(32);
    let search = request_response::Behaviour::with_codec(
        BoundedSearchCodec,
        supported_search_protocols(),
        request_config.clone(),
    );
    let pairing_codec =
        request_response::cbor::codec::Codec::<PairingRequest, PairingResponse>::default()
            .set_request_size_maximum(PAIRING_FRAME_LIMIT)
            .set_response_size_maximum(PAIRING_FRAME_LIMIT);
    let pairing = request_response::cbor::Behaviour::with_codec(
        pairing_codec,
        [(StreamProtocol::new(PAIRING_PROTOCOL), ProtocolSupport::Full)],
        request_config,
    );
    // Unit tests must never advertise random libp2p identities on the real LAN.
    // Production keeps the same mDNS behaviour; Toggle is only the narrow seam
    // that makes the test runtime network-isolated.
    let mdns = if cfg!(test) {
        None
    } else {
        Some(
            libp2p::mdns::tokio::Behaviour::new(mdns_config(), local_peer_id)
                .map_err(|error| NetworkError::Mdns(error.to_string()))?,
        )
    }
    .into();
    let behaviour = AirWikiBehaviour {
        mdns,
        search,
        pairing,
    };
    let mut swarm = Swarm::new(
        transport,
        behaviour,
        local_peer_id,
        libp2p::swarm::Config::with_tokio_executor(),
    );
    swarm
        .listen_on(config.listen_address.clone())
        .map_err(|error| NetworkError::Listen(error.to_string()))?;

    let (command_tx, command_rx) = mpsc::channel(config.command_capacity);
    let (event_tx, _) = broadcast::channel(128);
    let (backend_result_tx, backend_result_rx) = mpsc::channel(32);
    let listener_ready = Arc::new(AtomicBool::new(false));
    let listener_unavailable = Arc::new(AtomicBool::new(false));
    let handle = NetworkHandle {
        local_peer_id,
        command_tx,
        event_tx: event_tx.clone(),
        listener_ready: Arc::clone(&listener_ready),
        listener_unavailable: Arc::clone(&listener_unavailable),
    };
    // Create the first receiver before the runtime task can emit Listening or
    // mDNS discoveries. Desktop startup may await other services before it can
    // poll this receiver, but the bounded broadcast buffer retains those events.
    let initial_events = event_tx.subscribe();
    let runtime = Runtime {
        swarm,
        config,
        local_peer_id,
        access,
        backend,
        limiter: Arc::new(PeerRateLimiter::default()),
        pairing: PairingCoordinator::new(local_peer_id),
        pairing_hellos: HashMap::new(),
        pairing_confirms: HashMap::new(),
        pairing_dials: HashMap::new(),
        pairing_reblock_on_failure: HashSet::new(),
        addresses: PeerAddressBook::default(),
        queries: HashMap::new(),
        query_by_request: HashMap::new(),
        next_query_id: 1,
        command_rx,
        event_tx,
        listener_ready,
        listener_unavailable,
        listen_addresses: HashSet::new(),
        backend_result_tx,
        backend_result_rx,
        backend_tasks: JoinSet::new(),
    };
    let task = tokio::spawn(runtime.run());
    Ok((handle, initial_events, task))
}

fn mdns_config() -> libp2p::mdns::Config {
    libp2p::mdns::Config {
        // Receiving mDNS traffic resets this timer. The bounded retry repairs
        // an initial multicast packet lost while an interface or firewall is
        // becoming ready without waiting for libp2p's five-minute default.
        query_interval: MDNS_QUERY_INTERVAL,
        ..libp2p::mdns::Config::default()
    }
}

impl Runtime {
    async fn run(mut self) {
        if self.swarm.behaviour().mdns.is_enabled() {
            let _ = self.event_tx.send(NetworkEvent::DiscoveryStarted);
        }
        let mut maintenance = tokio::time::interval(Duration::from_millis(100));
        let mut shutdown_completed = None;
        loop {
            tokio::select! {
                command = self.command_rx.recv() => {
                    match command {
                        Some(Command::Shutdown { completed }) => {
                            shutdown_completed = Some(completed);
                            break;
                        }
                        None => break,
                        Some(command) => self.handle_command(command),
                    }
                }
                event = self.swarm.select_next_some() => self.handle_swarm_event(event),
                result = self.backend_result_rx.recv() => {
                    if let Some(result) = result {
                        self.handle_backend_result(result);
                    }
                }
                completion = self.backend_tasks.join_next(), if !self.backend_tasks.is_empty() => {
                    if completion.is_some_and(|result| result.is_err()) {
                        warn!(
                            error_kind = "authorized_search_task_join",
                            "an authorized search task did not join cleanly"
                        );
                    }
                }
                _ = maintenance.tick() => self.maintenance(),
            }
        }
        self.backend_tasks.abort_all();
        while self.backend_tasks.join_next().await.is_some() {}
        self.restore_pairing_blocks_for_shutdown();
        for (_, query) in self.queries.drain() {
            let _ = query.reply.send(Err(NetworkError::RuntimeStopped));
        }
        self.listener_ready.store(false, Ordering::Release);
        self.listener_unavailable.store(true, Ordering::Release);
        info!("LAN runtime stopped");
        if let Some(completed) = shutdown_completed {
            let _ = completed.send(());
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::Dial { address } => {
                let dial = DialOpts::unknown_peer_id()
                    .address(address)
                    .allocate_new_port()
                    .build();
                if self.swarm.dial(dial).is_err() {
                    self.emit_warning(None, NetworkWarningKind::DialFailed);
                }
            }
            Command::BeginPairing { peer } => {
                self.begin_pairing(peer);
            }
            Command::ConfirmPairing { peer } => {
                match self.pairing.confirm_local(&peer, Instant::now()) {
                    Ok(code) => {
                        let request_id = self.swarm.behaviour_mut().pairing.send_request(
                            &peer,
                            PairingRequest::Confirm {
                                session_hash: code.session_hash,
                            },
                        );
                        self.pairing_confirms.insert(request_id, peer);
                        self.complete_pairing_if_ready(peer);
                    }
                    Err(_) => {
                        self.emit_warning(Some(peer), NetworkWarningKind::PairingStateInvalid)
                    }
                }
            }
            Command::CancelPairing { peer, block } => {
                self.pairing.remove(&peer);
                self.pairing_dials.remove(&peer);
                self.pairing_hellos
                    .retain(|_, pending| pending.peer != peer);
                self.pairing_confirms
                    .retain(|_, pending_peer| *pending_peer != peer);
                if block {
                    self.pairing_reblock_on_failure.remove(&peer);
                    self.enforce_peer_block(peer);
                } else {
                    self.restore_pairing_block_if_needed(peer);
                }
                let _ = self.event_tx.send(NetworkEvent::PairingCancelled {
                    peer,
                    blocked: block,
                });
            }
            Command::Grant { peer, collection } => {
                if self.access.grant(peer, collection).is_err() {
                    self.emit_warning(Some(peer), NetworkWarningKind::AccessChangeRejected);
                }
            }
            Command::RemoveGrant { peer, collection } => {
                self.access.remove_grant(peer, collection);
            }
            Command::Revoke { peer } => {
                self.pairing_reblock_on_failure.remove(&peer);
                self.access.revoke_and_block(peer);
                self.addresses.clear_peer(&peer);
                self.pairing.remove(&peer);
                self.pairing_dials.remove(&peer);
                self.pairing_hellos
                    .retain(|_, pending| pending.peer != peer);
                self.pairing_confirms
                    .retain(|_, pending_peer| *pending_peer != peer);
                self.retire_peer_searches(peer);
                let _ = self.swarm.disconnect_peer_id(peer);
                let _ = self.event_tx.send(NetworkEvent::PeerRevoked { peer });
            }
            Command::SearchPeers { request, reply } => {
                self.start_search(request, reply);
            }
            #[cfg(test)]
            Command::DisconnectPeer { peer } => {
                let _ = self.swarm.disconnect_peer_id(peer);
            }
            #[cfg(test)]
            Command::SearchThenRevokeAndBeginPairing {
                peer,
                request,
                reply,
            } => {
                self.handle_command(Command::SearchPeers { request, reply });
                self.handle_command(Command::Revoke { peer });
                self.handle_command(Command::BeginPairing { peer });
            }
            #[cfg(test)]
            Command::RecordDiscoveredAddress { peer, address } => {
                self.addresses
                    .record_mdns(peer, address)
                    .expect("test discovery address must be valid");
            }
            #[cfg(test)]
            Command::PairingAttemptActive { peer, reply } => {
                let _ = reply.send(self.pairing_attempt_active(&peer));
            }
            #[cfg(test)]
            Command::ExpirePairings { now } => self.expire_pairings(now),
            Command::Shutdown { .. } => unreachable!("shutdown is handled in run"),
        }
    }

    fn begin_pairing(&mut self, peer: PeerId) {
        if self.pairing_attempt_active(&peer) {
            self.emit_pairing_code(peer);
            return;
        }

        if self.access.is_blocked(&peer) {
            self.pairing_reblock_on_failure.insert(peer);
            self.access.unblock(peer);
        }
        let local_nonce = self.pairing.begin_outgoing(peer);
        if self.swarm.is_connected(&peer) {
            self.send_pairing_hello(peer, local_nonce);
            return;
        }

        let addresses = self.addresses.dial_addresses(&peer);
        let mdns_has_peer = self
            .swarm
            .behaviour()
            .mdns
            .as_ref()
            .is_some_and(|mdns| mdns.discovered_nodes().any(|candidate| candidate == &peer));
        let Some(source) = select_pairing_dial_source(addresses, mdns_has_peer) else {
            self.fail_pairing(peer, PairingFailureReason::NoCurrentAddress);
            return;
        };
        let dial = match source {
            PairingDialSource::Explicit(addresses) => DialOpts::peer_id(peer)
                .addresses(addresses)
                .allocate_new_port()
                .build(),
            // A revocation deliberately clears our address book. For an
            // explicit re-pair only, a still-current mDNS observation may
            // supply its address through the composed behaviour.
            PairingDialSource::Behaviour => DialOpts::peer_id(peer).allocate_new_port().build(),
        };
        match self.swarm.dial(dial) {
            Ok(()) | Err(DialError::DialPeerConditionFalse(_)) => {
                self.pairing_dials.insert(peer, local_nonce);
            }
            Err(DialError::NoAddresses) => {
                self.fail_pairing(peer, PairingFailureReason::NoCurrentAddress);
            }
            Err(_) => {
                self.fail_pairing(peer, PairingFailureReason::DialFailed);
            }
        }
    }

    fn pairing_attempt_active(&self, peer: &PeerId) -> bool {
        self.pairing_dials.contains_key(peer)
            || self
                .pairing_hellos
                .values()
                .any(|pending| pending.peer == *peer)
            || self.pairing.pending(peer, Instant::now()).is_ok()
    }

    fn send_pairing_hello(&mut self, peer: PeerId, local_nonce: PairingNonce) {
        if self
            .pairing_hellos
            .values()
            .any(|pending| pending.peer == peer)
        {
            return;
        }
        let request = PairingRequest::Hello {
            nonce: local_nonce,
            node_name: clean_node_name(&self.config.node_name),
        };
        let request_id = self
            .swarm
            .behaviour_mut()
            .pairing
            .send_request(&peer, request);
        self.pairing_hellos
            .insert(request_id, PairingHelloRequest { peer, local_nonce });
    }

    fn fail_pairing(&mut self, peer: PeerId, reason: PairingFailureReason) {
        self.pairing.remove(&peer);
        self.pairing_dials.remove(&peer);
        self.pairing_hellos
            .retain(|_, pending| pending.peer != peer);
        self.pairing_confirms
            .retain(|_, pending_peer| *pending_peer != peer);
        self.restore_pairing_block_if_needed(peer);
        let _ = self
            .event_tx
            .send(NetworkEvent::PairingFailed { peer, reason });
    }

    /// Restores a durable revocation after an explicit re-pairing attempt ends
    /// without establishing trust. This is deliberately stronger than merely
    /// flipping the ACL bit: stale addresses, searches and connections must not
    /// survive a failed authorization transition.
    fn restore_pairing_block_if_needed(&mut self, peer: PeerId) {
        if self.pairing_reblock_on_failure.remove(&peer) {
            self.enforce_peer_block(peer);
        }
    }

    fn enforce_peer_block(&mut self, peer: PeerId) {
        self.access.block(peer);
        self.addresses.clear_peer(&peer);
        self.retire_peer_searches(peer);
        let _ = self.swarm.disconnect_peer_id(peer);
    }

    fn restore_pairing_blocks_for_shutdown(&mut self) {
        let peers = self.pairing_reblock_on_failure.drain().collect::<Vec<_>>();
        for peer in peers {
            self.pairing.remove(&peer);
            self.enforce_peer_block(peer);
        }
        self.pairing_dials.clear();
        self.pairing_hellos.clear();
        self.pairing_confirms.clear();
    }

    fn start_search(
        &mut self,
        request: SearchRequest,
        reply: oneshot::Sender<Result<SearchResponse, NetworkError>>,
    ) {
        if let Err(error) = request.validate() {
            let _ = reply.send(Err(error.into()));
            return;
        }
        let query_id = self.next_query_id;
        self.next_query_id = self.next_query_id.wrapping_add(1);
        let mut pending = HashMap::new();
        let mut connecting = HashSet::new();
        let mut offline = HashSet::new();
        let mut warnings = Vec::new();
        for peer in self.access.trusted_peers() {
            if self.swarm.is_connected(&peer) {
                let outbound_id = self
                    .swarm
                    .behaviour_mut()
                    .search
                    .send_request(&peer, request.clone());
                pending.insert(outbound_id, peer);
                self.query_by_request.insert(outbound_id, query_id);
                continue;
            }

            // Search-triggered dials use a fresh local port. The request-response
            // default attempts listener-port reuse, which is not portable across
            // the two supported desktop targets after an idle connection closes.
            let addresses = self.addresses.dial_addresses(&peer);
            if addresses.is_empty() {
                offline.insert(peer);
                warnings.push(peer_unavailable_warning(peer));
                continue;
            }
            let dial = DialOpts::peer_id(peer)
                .addresses(addresses)
                .allocate_new_port()
                .build();
            match self.swarm.dial(dial) {
                Ok(()) | Err(DialError::DialPeerConditionFalse(_)) => {
                    connecting.insert(peer);
                }
                Err(_) => {
                    offline.insert(peer);
                    warnings.push(peer_unavailable_warning(peer));
                }
            }
        }
        let should_finish = pending.is_empty() && connecting.is_empty();
        self.queries.insert(
            query_id,
            QueryAggregate {
                request,
                pending,
                connecting,
                responses: Vec::new(),
                offline,
                warnings,
                deadline: Instant::now() + self.config.search_deadline,
                reply,
            },
        );
        if should_finish {
            self.finish_query(query_id);
        }
    }

    fn handle_swarm_event(&mut self, event: SwarmEvent<BehaviourEvent>) {
        match event {
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(event)) => self.handle_mdns(event),
            SwarmEvent::Behaviour(BehaviourEvent::Search(event)) => self.handle_search_event(event),
            SwarmEvent::Behaviour(BehaviourEvent::Pairing(event)) => {
                self.handle_pairing_event(event);
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                self.listen_addresses.insert(address.clone());
                self.listener_unavailable.store(false, Ordering::Release);
                self.listener_ready.store(true, Ordering::Release);
                let _ = self.event_tx.send(NetworkEvent::Listening { address });
            }
            SwarmEvent::ExpiredListenAddr { address, .. } => {
                self.listen_addresses.remove(&address);
                self.listener_ready
                    .store(!self.listen_addresses.is_empty(), Ordering::Release);
            }
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                if self.access.is_blocked(&peer_id) {
                    self.pairing_reblock_on_failure.remove(&peer_id);
                    self.pairing_dials.remove(&peer_id);
                    self.pairing.remove(&peer_id);
                    let _ = self.swarm.disconnect_peer_id(peer_id);
                    self.retire_peer_searches(peer_id);
                } else {
                    // Retain only an address that completed an outgoing Noise
                    // handshake. Listener send-back addresses can be ephemeral
                    // client ports and are not reliable redial targets.
                    if let ConnectedPoint::Dialer { address, .. } = endpoint {
                        match self
                            .addresses
                            .record_authenticated_outbound(peer_id, address)
                        {
                            Ok(address) => self.swarm.add_peer_address(peer_id, address),
                            Err(_) => warn!(
                                error_kind = "non_lan_authenticated_address",
                                "ignored non-LAN authenticated outbound address"
                            ),
                        }
                    }
                    if let Some(local_nonce) = self.pairing_dials.remove(&peer_id) {
                        self.send_pairing_hello(peer_id, local_nonce);
                    }
                    let _ = self
                        .event_tx
                        .send(NetworkEvent::Connected { peer: peer_id });
                    if self.peer_is_trusted_for_search(&peer_id) {
                        self.resume_waiting_searches(peer_id);
                    } else {
                        self.retire_peer_searches(peer_id);
                    }
                }
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                num_established: 0,
                ..
            } => {
                let _ = self
                    .event_tx
                    .send(NetworkEvent::Disconnected { peer: peer_id });
            }
            SwarmEvent::OutgoingConnectionError { peer_id, .. } => {
                let pairing_failed = peer_id.is_some_and(|peer| {
                    if self.pairing_dials.contains_key(&peer) {
                        self.fail_pairing(peer, PairingFailureReason::ConnectionFailed);
                        true
                    } else {
                        false
                    }
                });
                if let Some(peer) = peer_id {
                    self.fail_waiting_searches(peer);
                }
                if pairing_failed {
                    warn!(peer = ?peer_id, "pairing connection failed");
                } else {
                    self.emit_warning(peer_id, NetworkWarningKind::ConnectionFailed);
                }
            }
            SwarmEvent::ListenerError { .. } => {
                self.emit_warning(None, NetworkWarningKind::ListenerDegraded);
            }
            SwarmEvent::ListenerClosed { addresses, .. } => {
                for address in addresses {
                    self.listen_addresses.remove(&address);
                }
                self.listener_ready.store(false, Ordering::Release);
                self.listener_unavailable.store(true, Ordering::Release);
                let _ = self.event_tx.send(NetworkEvent::ListenerUnavailable);
            }
            _ => {}
        }
    }

    fn handle_mdns(&mut self, event: libp2p::mdns::Event) {
        match event {
            libp2p::mdns::Event::Discovered(entries) => {
                for (peer, address) in entries {
                    if peer == self.local_peer_id {
                        continue;
                    }
                    match self.addresses.record_mdns(peer, address) {
                        Ok(address) => {
                            // Pairing request-response still consumes behaviour
                            // addresses. Search redials use PeerAddressBook
                            // explicitly and never extend from this cache.
                            self.swarm.add_peer_address(peer, address.clone());
                            let _ = self
                                .event_tx
                                .send(NetworkEvent::Discovered { peer, address });
                        }
                        Err(_) => warn!(
                            error_kind = "invalid_mdns_address",
                            "ignored invalid mDNS address"
                        ),
                    }
                }
            }
            libp2p::mdns::Event::Expired(entries) => {
                for (peer, address) in entries {
                    match self.addresses.expire_mdns(peer, address) {
                        Ok(Some(address)) => {
                            let _ = self
                                .event_tx
                                .send(NetworkEvent::DiscoveryExpired { peer, address });
                        }
                        Ok(None) => {}
                        Err(_) => warn!(
                            error_kind = "invalid_expired_mdns_address",
                            "ignored invalid expired mDNS address"
                        ),
                    }
                }
            }
        }
    }

    fn handle_search_event(
        &mut self,
        event: request_response::Event<SearchRequest, SearchWireResponse>,
    ) {
        match event {
            request_response::Event::Message { peer, message, .. } => match message {
                request_response::Message::Request {
                    request, channel, ..
                } => self.handle_inbound_search(peer, request, channel),
                request_response::Message::Response {
                    request_id,
                    response,
                } => self.handle_search_response(peer, request_id, response),
            },
            request_response::Event::OutboundFailure {
                peer,
                request_id,
                error,
                ..
            } => self.fail_outbound_search(peer, request_id, &error),
            request_response::Event::InboundFailure { peer, error, .. } => {
                let _ = error;
                self.emit_warning(Some(peer), NetworkWarningKind::InboundSearchFailed);
            }
            request_response::Event::ResponseSent { .. } => {}
        }
    }

    fn handle_inbound_search(
        &mut self,
        peer: PeerId,
        request: SearchRequest,
        channel: ResponseChannel<SearchWireResponse>,
    ) {
        let error_response =
            |code, message| SearchWireResponse::Error(SearchWireError::new(code, message));
        if self.access.is_blocked(&peer) {
            self.send_search_response(
                channel,
                error_response(SearchWireErrorCode::Unauthorized, "peer is blocked"),
            );
            return;
        }
        let authorization = match self.access.authorize(&peer, request.purpose) {
            Ok(authorization) => authorization,
            Err(error) => {
                self.send_search_response(
                    channel,
                    error_response(SearchWireErrorCode::Unauthorized, &error.to_string()),
                );
                return;
            }
        };
        // Only trusted peers enter the limiter map. Otherwise an unauthenticated
        // LAN host could grow it indefinitely by rotating ephemeral PeerIds.
        if !self.limiter.check(peer) {
            self.send_search_response(
                channel,
                error_response(
                    SearchWireErrorCode::RateLimited,
                    "30 searches per minute exceeded",
                ),
            );
            return;
        }
        if let Err(error) = request.validate() {
            self.send_search_response(
                channel,
                error_response(SearchWireErrorCode::InvalidRequest, &error.to_string()),
            );
            return;
        }

        let backend = Arc::clone(&self.backend);
        let result_tx = self.backend_result_tx.clone();
        let deadline = self.config.search_deadline;
        let local_node = self.local_peer_id.to_string();
        self.backend_tasks.spawn(async move {
            let request_id = request.request_id;
            let allowed: HashSet<_> = authorization.allowed_collections.iter().copied().collect();
            let result = tokio::time::timeout(
                deadline,
                backend.search_authorized(request.clone(), authorization),
            )
            .await;
            let (response, disclosure_lease) = match result {
                Ok(Ok(AuthorizedSearchResult {
                    mut response,
                    disclosure_lease,
                })) => {
                    response.request_id = request_id;
                    sanitize_untrusted_search_diagnostics(&mut response);
                    response
                        .hits
                        .retain(|hit| allowed.contains(&hit.collection_id));
                    response
                        .authorized_candidates
                        .retain(|hit| allowed.contains(&hit.collection_id));
                    if request.purpose != SearchPurpose::ExternalAi {
                        response.authorized_candidates.clear();
                    }
                    response.hits.truncate(usize::from(request.top_k));
                    response
                        .authorized_candidates
                        .truncate(usize::from(request.top_k));
                    for hit in response
                        .hits
                        .iter_mut()
                        .chain(&mut response.authorized_candidates)
                    {
                        hit.sanitize_for_wire();
                        hit.node_id.clone_from(&local_node);
                    }
                    let mut wire = SearchWireResponse::Success(response);
                    shrink_to_wire_limit(&mut wire);
                    (wire, Some(disclosure_lease))
                }
                Ok(Err(SearchContractError::Unauthorized)) => (
                    error_response(
                        SearchWireErrorCode::Unauthorized,
                        "source policy rejected this search",
                    ),
                    None,
                ),
                Ok(Err(error)) => {
                    warn!(
                        error_kind = search_error_kind(&error),
                        "authorized search backend failed"
                    );
                    (
                        error_response(SearchWireErrorCode::Internal, "search backend failed"),
                        None,
                    )
                }
                Err(_) => (
                    error_response(SearchWireErrorCode::Unavailable, "search timed out"),
                    None,
                ),
            };
            let hit_count = match &response {
                SearchWireResponse::Success(response) => response.hits.len(),
                SearchWireResponse::Error(_) => 0,
            };
            let _ = result_tx
                .send(BackendResult {
                    peer,
                    request_id,
                    channel,
                    response,
                    hit_count,
                    disclosure_lease,
                })
                .await;
        });
    }

    fn handle_backend_result(&mut self, result: BackendResult) {
        let BackendResult {
            peer,
            request_id,
            channel,
            response,
            hit_count,
            disclosure_lease,
        } = result;
        let access = self.access.clone();
        let hit_count = deliver_backend_response(
            &access,
            peer,
            response,
            hit_count,
            disclosure_lease,
            |response| self.send_search_response(channel, response),
        );
        let _ = self.event_tx.send(NetworkEvent::InboundSearchCompleted {
            peer,
            request_id,
            hits: hit_count,
        });
    }

    fn send_search_response(
        &mut self,
        channel: ResponseChannel<SearchWireResponse>,
        response: SearchWireResponse,
    ) {
        if let Err(response) = self
            .swarm
            .behaviour_mut()
            .search
            .send_response(channel, response)
        {
            let (response_kind, hit_count) = match response {
                SearchWireResponse::Success(response) => ("success", response.hits.len()),
                SearchWireResponse::Error(_) => ("error", 0),
            };
            warn!(
                response_kind,
                hit_count, "search response channel closed before response"
            );
        }
    }

    fn handle_search_response(
        &mut self,
        peer: PeerId,
        request_id: OutboundRequestId,
        response: SearchWireResponse,
    ) {
        let peer_is_trusted = self.peer_is_trusted_for_search(&peer);
        let Some(query_id) = self.query_by_request.remove(&request_id) else {
            return;
        };
        let Some(query) = self.queries.get_mut(&query_id) else {
            return;
        };
        query.pending.remove(&request_id);
        if !peer_is_trusted {
            query
                .responses
                .retain(|(response_peer, _)| *response_peer != peer);
            query
                .warnings
                .push(peer_authorization_changed_warning(peer));
        } else {
            match response {
                SearchWireResponse::Success(response)
                    if response.request_id == query.request.request_id =>
                {
                    query.responses.push((peer, response));
                }
                SearchWireResponse::Success(_) => {
                    query
                        .warnings
                        .push(format!("peer {peer} returned a mismatched request id"));
                }
                SearchWireResponse::Error(error) => {
                    query
                        .warnings
                        .push(remote_search_error_warning(peer, &error));
                }
            }
        }
        if query.pending.is_empty() && query.connecting.is_empty() {
            self.finish_query(query_id);
        }
    }

    fn fail_outbound_search(
        &mut self,
        peer: PeerId,
        request_id: OutboundRequestId,
        failure: &request_response::OutboundFailure,
    ) {
        let Some(query_id) = self.query_by_request.remove(&request_id) else {
            return;
        };
        let Some(query) = self.queries.get_mut(&query_id) else {
            return;
        };
        query.pending.remove(&request_id);
        record_outbound_search_failure(query, peer, failure);
        if query.pending.is_empty() && query.connecting.is_empty() {
            self.finish_query(query_id);
        }
    }

    fn resume_waiting_searches(&mut self, peer: PeerId) {
        if !self.peer_is_trusted_for_search(&peer) {
            self.retire_peer_searches(peer);
            return;
        }
        let waiting = self
            .queries
            .iter_mut()
            .filter_map(|(query_id, query)| {
                query
                    .connecting
                    .remove(&peer)
                    .then(|| (*query_id, query.request.clone()))
            })
            .collect::<Vec<_>>();

        for (query_id, request) in waiting {
            let outbound_id = self
                .swarm
                .behaviour_mut()
                .search
                .send_request(&peer, request);
            if let Some(query) = self.queries.get_mut(&query_id) {
                query.pending.insert(outbound_id, peer);
                self.query_by_request.insert(outbound_id, query_id);
            }
        }
    }

    fn peer_is_trusted_for_search(&self, peer: &PeerId) -> bool {
        let access = self.access.state(peer);
        access.trusted && !access.blocked
    }

    fn retire_peer_searches(&mut self, peer: PeerId) {
        let mut retired_requests = Vec::new();
        let mut finished = Vec::new();
        for (query_id, query) in &mut self.queries {
            let was_connecting = query.connecting.remove(&peer);
            let peer_requests = query
                .pending
                .iter()
                .filter_map(|(request_id, request_peer)| {
                    (*request_peer == peer).then_some(*request_id)
                })
                .collect::<Vec<_>>();
            for request_id in &peer_requests {
                query.pending.remove(request_id);
            }
            let response_count = query.responses.len();
            query
                .responses
                .retain(|(response_peer, _)| *response_peer != peer);
            let had_response = query.responses.len() != response_count;
            if !was_connecting && peer_requests.is_empty() && !had_response {
                continue;
            }
            retired_requests.extend(peer_requests);
            query
                .warnings
                .push(peer_authorization_changed_warning(peer));
            if query.pending.is_empty() && query.connecting.is_empty() {
                finished.push(*query_id);
            }
        }
        for request_id in retired_requests {
            self.query_by_request.remove(&request_id);
        }
        for query_id in finished {
            self.finish_query(query_id);
        }
    }

    fn fail_waiting_searches(&mut self, peer: PeerId) {
        let mut finished = Vec::new();
        for (query_id, query) in &mut self.queries {
            if !query.connecting.remove(&peer) {
                continue;
            }
            query.offline.insert(peer);
            query.warnings.push(peer_unavailable_warning(peer));
            if query.pending.is_empty() && query.connecting.is_empty() {
                finished.push(*query_id);
            }
        }
        for query_id in finished {
            self.finish_query(query_id);
        }
    }

    fn finish_query(&mut self, query_id: u64) {
        let Some(query) = self.queries.remove(&query_id) else {
            return;
        };
        for request_id in query.pending.keys() {
            self.query_by_request.remove(request_id);
        }
        let response = fuse_peer_rankings(query);
        let _ = response.1.send(Ok(response.0));
    }

    fn handle_pairing_event(
        &mut self,
        event: request_response::Event<PairingRequest, PairingResponse>,
    ) {
        match event {
            request_response::Event::Message { peer, message, .. } => match message {
                request_response::Message::Request {
                    request, channel, ..
                } => self.handle_pairing_request(peer, request, channel),
                request_response::Message::Response {
                    request_id,
                    response,
                } => self.handle_pairing_response(peer, request_id, response),
            },
            request_response::Event::OutboundFailure {
                peer,
                request_id,
                error,
                ..
            } => {
                let hello_failed = self.pairing_hellos.remove(&request_id).is_some();
                self.pairing_confirms.remove(&request_id);
                if hello_failed {
                    self.fail_pairing(peer, PairingFailureReason::HandshakeFailed);
                    warn!(error_kind = "pairing_hello_failed", "pairing Hello failed");
                } else {
                    let _ = error;
                    self.emit_warning(Some(peer), NetworkWarningKind::PairingProtocolFailed);
                }
            }
            request_response::Event::InboundFailure { peer, error, .. } => {
                let _ = error;
                self.emit_warning(Some(peer), NetworkWarningKind::PairingProtocolFailed);
            }
            request_response::Event::ResponseSent { .. } => {}
        }
    }

    fn handle_pairing_request(
        &mut self,
        peer: PeerId,
        request: PairingRequest,
        channel: ResponseChannel<PairingResponse>,
    ) {
        if self.access.is_blocked(&peer) {
            let _ = self.swarm.behaviour_mut().pairing.send_response(
                channel,
                PairingResponse::Rejected {
                    reason: "peer is blocked".to_owned(),
                },
            );
            return;
        }
        match request {
            PairingRequest::Hello { nonce, node_name } => {
                let response =
                    match self
                        .pairing
                        .accept_hello(peer, nonce, node_name, Instant::now())
                    {
                        Ok(local_nonce) => PairingResponse::Hello {
                            nonce: local_nonce,
                            node_name: clean_node_name(&self.config.node_name),
                        },
                        Err(_) => PairingResponse::Rejected {
                            reason: "pairing unavailable".to_owned(),
                        },
                    };
                let accepted = matches!(&response, PairingResponse::Hello { .. });
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .pairing
                    .send_response(channel, response);
                if accepted {
                    self.emit_pairing_code(peer);
                }
            }
            PairingRequest::Confirm { session_hash } => {
                let result = self
                    .pairing
                    .confirm_remote(&peer, session_hash, Instant::now());
                let response = match result {
                    Ok(both_confirmed) => PairingResponse::Confirmed { both_confirmed },
                    Err(error) => PairingResponse::Rejected {
                        reason: error.to_string(),
                    },
                };
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .pairing
                    .send_response(channel, response);
                self.complete_pairing_if_ready(peer);
            }
        }
    }

    fn handle_pairing_response(
        &mut self,
        peer: PeerId,
        request_id: OutboundRequestId,
        response: PairingResponse,
    ) {
        if let Some(hello) = self.pairing_hellos.remove(&request_id) {
            if hello.peer != peer {
                self.fail_pairing(hello.peer, PairingFailureReason::HandshakeFailed);
                return;
            }
            if let PairingResponse::Hello { nonce, node_name } = response {
                let accepted = self.pairing.accept_hello_response(
                    peer,
                    hello.local_nonce,
                    nonce,
                    node_name,
                    Instant::now(),
                );
                if accepted.is_ok() {
                    self.emit_pairing_code(peer);
                } else {
                    self.fail_pairing(peer, PairingFailureReason::HandshakeFailed);
                }
            } else {
                self.fail_pairing(peer, PairingFailureReason::HandshakeFailed);
            }
            return;
        }

        if let Some(expected_peer) = self.pairing_confirms.remove(&request_id) {
            if expected_peer != peer {
                self.fail_pairing(expected_peer, PairingFailureReason::HandshakeFailed);
                return;
            }
            if let PairingResponse::Confirmed { both_confirmed } = response {
                if both_confirmed && let Ok(pending) = self.pairing.pending(&peer, Instant::now()) {
                    let session_hash = pending.code(&self.local_peer_id).session_hash;
                    let _ = self
                        .pairing
                        .confirm_remote(&peer, session_hash, Instant::now());
                }
                self.complete_pairing_if_ready(peer);
            } else {
                self.fail_pairing(peer, PairingFailureReason::HandshakeFailed);
            }
        }
    }

    fn emit_pairing_code(&self, peer: PeerId) {
        if let Ok(pending) = self.pairing.pending(&peer, Instant::now()) {
            let _ = self.event_tx.send(NetworkEvent::PairingCode {
                peer,
                remote_node_name: pending.remote_node_name.clone(),
                sas: pending.code(&self.local_peer_id),
                expires_in_seconds: pending
                    .expires_at
                    .saturating_duration_since(Instant::now())
                    .as_secs(),
            });
        }
    }

    fn complete_pairing_if_ready(&mut self, peer: PeerId) {
        if self.access.is_blocked(&peer) {
            self.pairing_reblock_on_failure.remove(&peer);
            self.pairing.remove(&peer);
            self.pairing_dials.remove(&peer);
            self.pairing_hellos
                .retain(|_, pending| pending.peer != peer);
            self.pairing_confirms
                .retain(|_, pending_peer| *pending_peer != peer);
            return;
        }
        if self.pairing.is_complete(&peer, Instant::now()) {
            self.pairing_reblock_on_failure.remove(&peer);
            self.access.mark_trusted(peer);
            self.pairing.remove(&peer);
            self.pairing_dials.remove(&peer);
            let _ = self.event_tx.send(NetworkEvent::PairingTrusted { peer });
        }
    }

    fn maintenance(&mut self) {
        let now = Instant::now();
        self.expire_pairings(now);
        let expired_queries: Vec<_> = self
            .queries
            .iter()
            .filter_map(|(id, query)| (now >= query.deadline).then_some(*id))
            .collect();
        for query_id in expired_queries {
            if let Some(query) = self.queries.get_mut(&query_id) {
                for (request_id, peer) in query.pending.drain() {
                    self.query_by_request.remove(&request_id);
                    query.offline.insert(peer);
                    query
                        .warnings
                        .push(format!("peer {peer}: 3 second deadline exceeded"));
                }
                for peer in query.connecting.drain() {
                    query.offline.insert(peer);
                    query
                        .warnings
                        .push(format!("peer {peer}: 3 second deadline exceeded"));
                }
            }
            self.finish_query(query_id);
        }
    }

    fn expire_pairings(&mut self, now: Instant) {
        for peer in self.pairing.expire(now) {
            self.pairing_dials.remove(&peer);
            self.pairing_hellos
                .retain(|_, pending| pending.peer != peer);
            self.pairing_confirms
                .retain(|_, pending_peer| *pending_peer != peer);
            self.restore_pairing_block_if_needed(peer);
            let _ = self.event_tx.send(NetworkEvent::PairingExpired { peer });
        }
    }

    fn emit_warning(&self, peer: Option<PeerId>, kind: NetworkWarningKind) {
        warn!(warning_kind = ?kind, peer_present = peer.is_some(), "LAN warning");
        let _ = self.event_tx.send(NetworkEvent::Warning { peer, kind });
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // `AccessControl` survives LAN runtime replacement. If Tokio aborts or
        // the task panics before the acknowledged shutdown path completes, a
        // durable revocation must never remain transiently unblocked.
        for peer in self.pairing_reblock_on_failure.drain() {
            self.access.block(peer);
        }
    }
}

fn clean_node_name(name: &str) -> String {
    name.trim().chars().take(128).collect()
}

/// Preserve only the boolean completeness signal from diagnostics supplied by a
/// search backend or remote peer. Diagnostic strings can contain document text,
/// local paths, or prompt-injection payloads and must not cross this trust boundary.
fn deliver_backend_response(
    access_control: &AccessControl,
    peer: PeerId,
    mut response: SearchWireResponse,
    mut hit_count: usize,
    disclosure_lease: Option<DisclosureLease>,
    send: impl FnOnce(SearchWireResponse),
) -> usize {
    let lease_is_current = disclosure_lease
        .as_ref()
        .is_some_and(|lease| access_control.disclosure_lease_is_current(lease));
    let access = access_control.state(&peer);
    if matches!(&response, SearchWireResponse::Success(_)) && !lease_is_current {
        response = SearchWireResponse::Error(SearchWireError::new(
            SearchWireErrorCode::Unauthorized,
            "search authorization barrier was not retained",
        ));
        hit_count = 0;
    } else if !access.trusted || access.blocked {
        response = SearchWireResponse::Error(SearchWireError::new(
            SearchWireErrorCode::Unauthorized,
            "peer authorization changed while the search was running",
        ));
        hit_count = 0;
    } else if let SearchWireResponse::Success(success) = &mut response {
        success
            .hits
            .retain(|hit| access.grants.contains(&hit.collection_id));
        success
            .authorized_candidates
            .retain(|hit| access.grants.contains(&hit.collection_id));
        for (index, hit) in success.hits.iter_mut().enumerate() {
            hit.rank = u32::try_from(index + 1).unwrap_or(u32::MAX);
        }
        for (index, hit) in success.authorized_candidates.iter_mut().enumerate() {
            hit.rank = u32::try_from(index + 1).unwrap_or(u32::MAX);
        }
        hit_count = success.hits.len();
    }
    send(response);
    drop(disclosure_lease);
    hit_count
}

fn sanitize_untrusted_search_diagnostics(response: &mut SearchResponse) {
    response.partial |= !response.warnings.is_empty() || !response.offline_nodes.is_empty();
    response.warnings.clear();
    response.offline_nodes.clear();
}

fn remote_search_error_warning(peer: PeerId, error: &SearchWireError) -> String {
    let message = match error.code {
        SearchWireErrorCode::InvalidRequest => "remote search rejected the request",
        SearchWireErrorCode::Unauthorized => "remote search was not authorized",
        SearchWireErrorCode::RateLimited => "remote search rate limit reached",
        SearchWireErrorCode::Unavailable => "remote search unavailable",
        SearchWireErrorCode::Internal => "remote search failed",
    };
    format!("peer {peer}: {message}")
}

fn outbound_search_failure_warning(
    peer: PeerId,
    failure: &request_response::OutboundFailure,
) -> String {
    let message = match failure {
        request_response::OutboundFailure::Timeout => "remote search timed out",
        request_response::OutboundFailure::UnsupportedProtocols => {
            "remote peer does not support the search protocol"
        }
        request_response::OutboundFailure::DialFailure
        | request_response::OutboundFailure::ConnectionClosed
        | request_response::OutboundFailure::Io(_) => "remote search unavailable",
    };
    format!("peer {peer}: {message}")
}

fn peer_unavailable_warning(peer: PeerId) -> String {
    format!("peer {peer}: remote search unavailable")
}

fn peer_authorization_changed_warning(peer: PeerId) -> String {
    format!("peer {peer}: remote search was not authorized")
}

fn record_outbound_search_failure(
    query: &mut QueryAggregate,
    peer: PeerId,
    failure: &request_response::OutboundFailure,
) {
    // An incompatible peer answered protocol negotiation and is therefore not
    // offline. Its absence still makes coverage partial through the warning.
    if !matches!(
        failure,
        request_response::OutboundFailure::UnsupportedProtocols
    ) {
        query.offline.insert(peer);
    }
    query
        .warnings
        .push(outbound_search_failure_warning(peer, failure));
}

fn search_error_kind(error: &SearchContractError) -> &'static str {
    match error {
        SearchContractError::EmptyQuery => "empty_query",
        SearchContractError::QueryTooLarge(_) => "query_too_large",
        SearchContractError::InvalidTopK(_) => "invalid_top_k",
        SearchContractError::UnsupportedProtocol(_) => "unsupported_protocol",
        SearchContractError::Unauthorized => "unauthorized",
        SearchContractError::Unavailable(_) => "unavailable",
        SearchContractError::Backend(_) => "backend",
    }
}

fn shrink_to_wire_limit(response: &mut SearchWireResponse) {
    loop {
        if response_fits(response) {
            return;
        }
        let SearchWireResponse::Success(success) = response else {
            return;
        };
        if success.authorized_candidates.pop().is_some() {
            success.partial = true;
            if !success
                .warnings
                .iter()
                .any(|warning| warning == "response truncated to 256 KiB")
            {
                success
                    .warnings
                    .push("response truncated to 256 KiB".to_owned());
            }
            continue;
        }
        if success.hits.pop().is_some() {
            success.partial = true;
            if !success
                .warnings
                .iter()
                .any(|warning| warning == "response truncated to 256 KiB")
            {
                success
                    .warnings
                    .push("response truncated to 256 KiB".to_owned());
            }
            continue;
        }
        success.warnings.clear();
        success.offline_nodes.clear();
        debug_assert!(response_fits(response));
        return;
    }
}

fn fuse_peer_rankings(
    query: QueryAggregate,
) -> (
    SearchResponse,
    oneshot::Sender<Result<SearchResponse, NetworkError>>,
) {
    const RRF_K: f64 = 60.0;
    let mut fused: HashMap<(String, uuid::Uuid), (SearchHit, f64)> = HashMap::new();
    let mut fused_candidates: HashMap<(String, uuid::Uuid), (SearchHit, f64)> = HashMap::new();
    let include_candidates = query.request.purpose == SearchPurpose::ExternalAi;
    let mut warnings = query.warnings;
    for (peer, mut response) in query.responses {
        sanitize_untrusted_search_diagnostics(&mut response);
        if response.partial {
            warnings.push(format!("peer {peer}: remote results may be incomplete"));
        }
        add_peer_rankings(&mut fused, &peer, response.hits, RRF_K);
        if include_candidates {
            add_peer_rankings(
                &mut fused_candidates,
                &peer,
                response.authorized_candidates,
                RRF_K,
            );
        }
    }
    let hits = finish_fused_rankings(fused, query.request.top_k);
    let mut authorized_candidates = rank_fused_rankings(fused_candidates);
    remove_evidence_duplicates(&hits, &mut authorized_candidates);
    truncate_and_renumber(&mut authorized_candidates, query.request.top_k);
    let mut offline_nodes: Vec<_> = query
        .offline
        .into_iter()
        .map(|peer| peer.to_string())
        .collect();
    offline_nodes.sort();
    offline_nodes.dedup();
    warnings.sort();
    warnings.dedup();
    let partial = !offline_nodes.is_empty() || !warnings.is_empty();
    (
        SearchResponse {
            request_id: query.request.request_id,
            hits,
            authorized_candidates,
            offline_nodes,
            warnings,
            partial,
        },
        query.reply,
    )
}

fn remove_evidence_duplicates(hits: &[SearchHit], candidates: &mut Vec<SearchHit>) {
    let evidence = hits
        .iter()
        .map(|hit| (hit.source_sha256.as_str(), hit.chunk_id))
        .collect::<HashSet<_>>();
    candidates.retain(|hit| !evidence.contains(&(hit.source_sha256.as_str(), hit.chunk_id)));
}

fn add_peer_rankings(
    fused: &mut HashMap<(String, uuid::Uuid), (SearchHit, f64)>,
    peer: &PeerId,
    hits: Vec<SearchHit>,
    rrf_k: f64,
) {
    for (position, mut hit) in hits.into_iter().enumerate() {
        // Noise authenticates the sending peer. Response-controlled metadata
        // must never choose the citation identity.
        hit.node_id = peer.to_string();
        let rank = if hit.rank == 0 {
            u32::try_from(position.saturating_add(1)).unwrap_or(u32::MAX)
        } else {
            hit.rank
        };
        let contribution = 1.0 / (rrf_k + f64::from(rank));
        hit.sanitize_for_wire();
        let key = (hit.source_sha256.clone(), hit.chunk_id);
        fused
            .entry(key)
            .and_modify(|(_, score)| *score += contribution)
            .or_insert((hit, contribution));
    }
}

fn finish_fused_rankings(
    fused: HashMap<(String, uuid::Uuid), (SearchHit, f64)>,
    top_k: u8,
) -> Vec<SearchHit> {
    let mut hits = rank_fused_rankings(fused);
    truncate_and_renumber(&mut hits, top_k);
    hits
}

fn rank_fused_rankings(fused: HashMap<(String, uuid::Uuid), (SearchHit, f64)>) -> Vec<SearchHit> {
    let mut ranked: Vec<_> = fused.into_values().collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.title.cmp(&right.0.title))
    });
    ranked.into_iter().map(|(hit, _)| hit).collect()
}

fn truncate_and_renumber(hits: &mut Vec<SearchHit>, top_k: u8) {
    hits.truncate(usize::from(top_k));
    for (index, hit) in hits.iter_mut().enumerate() {
        hit.rank = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
    }
}

impl From<AccessError> for NetworkError {
    fn from(error: AccessError) -> Self {
        Self::Access(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::Utc;
    use libp2p::multiaddr::Protocol;
    use tokio::sync::Notify;

    use super::*;

    #[test]
    fn mdns_retries_a_lost_initial_query_inside_ten_seconds() {
        let config = mdns_config();

        assert!(config.query_interval < Duration::from_secs(10));
        assert!(!config.enable_ipv6);
    }

    struct FixtureBackend {
        collection: uuid::Uuid,
        title: &'static str,
    }

    struct GatedFixtureBackend {
        collection: uuid::Uuid,
        title: &'static str,
        calls: Arc<AtomicUsize>,
        first_call_started: Arc<Notify>,
        release_first_call: Arc<Notify>,
    }

    fn fixture_response(
        request: &SearchRequest,
        authorization: &SearchAuthorization,
        collection: uuid::Uuid,
        title: &str,
    ) -> SearchResponse {
        let mut response = SearchResponse::empty(request.request_id);
        if authorization.allowed_collections.contains(&collection) {
            response.hits.push(SearchHit {
                concept_id: uuid::Uuid::new_v4(),
                collection_id: collection,
                chunk_id: uuid::Uuid::new_v4(),
                title: title.to_owned(),
                snippet: "authorized evidence".to_owned(),
                heading_or_page: "page 1".to_owned(),
                logical_resource_uri: "urn:airwiki:test".to_owned(),
                source_revision: 1,
                source_sha256: "abc123".to_owned(),
                updated_at: Utc::now(),
                rank: 1,
                node_id: "backend-must-not-control-this".to_owned(),
            });
        }
        response
    }

    #[async_trait]
    impl AuthorizedSearchBackend for FixtureBackend {
        async fn search_authorized(
            &self,
            request: SearchRequest,
            authorization: SearchAuthorization,
        ) -> Result<AuthorizedSearchResult, SearchContractError> {
            let response = fixture_response(&request, &authorization, self.collection, self.title);
            let lease = authorization.acquire_disclosure_lease();
            Ok(AuthorizedSearchResult::new(response, lease))
        }
    }

    #[async_trait]
    impl AuthorizedSearchBackend for GatedFixtureBackend {
        async fn search_authorized(
            &self,
            request: SearchRequest,
            authorization: SearchAuthorization,
        ) -> Result<AuthorizedSearchResult, SearchContractError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.first_call_started.notify_one();
                self.release_first_call.notified().await;
            }
            let response = fixture_response(&request, &authorization, self.collection, self.title);
            let lease = authorization.acquire_disclosure_lease();
            Ok(AuthorizedSearchResult::new(response, lease))
        }
    }

    fn available_loopback_address() -> Multiaddr {
        let socket = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        format!("/ip4/127.0.0.1/tcp/{port}").parse().unwrap()
    }

    #[test]
    fn disclosure_lease_is_held_through_the_transport_handoff() {
        use std::sync::mpsc;

        let access = AccessControl::default();
        let peer = PeerId::random();
        let collection = uuid::Uuid::new_v4();
        access.mark_trusted(peer);
        access.grant(peer, collection).unwrap();
        let authorization = access
            .authorize(&peer, airwiki_types::SearchPurpose::LocalAssistant)
            .unwrap();
        let lease = authorization.acquire_disclosure_lease();
        let request = SearchRequest::new(
            "authorized",
            airwiki_types::SearchPurpose::LocalAssistant,
            1,
        );
        let response = SearchWireResponse::Success(fixture_response(
            &request,
            &authorization,
            collection,
            "authorized evidence",
        ));
        let (revocation_started_tx, revocation_started_rx) = mpsc::channel();
        let (revocation_finished_tx, revocation_finished_rx) = mpsc::channel();
        let revoking_access = access.clone();
        let revocation = std::thread::spawn(move || {
            revocation_started_tx.send(()).ok();
            revoking_access.revoke_and_block(peer);
            revocation_finished_tx.send(()).ok();
        });
        revocation_started_rx.recv().unwrap();

        let hit_count =
            deliver_backend_response(&access, peer, response, 1, Some(lease), |response| {
                assert!(matches!(response, SearchWireResponse::Success(_)));
                assert!(revocation_finished_rx.try_recv().is_err());
            });

        assert_eq!(hit_count, 1);
        revocation_finished_rx.recv().unwrap();
        revocation.join().unwrap();
        assert!(access.is_blocked(&peer));
    }

    #[test]
    fn response_with_a_lease_from_another_gate_is_denied() {
        let access = AccessControl::default();
        let peer = PeerId::random();
        let collection = uuid::Uuid::new_v4();
        access.mark_trusted(peer);
        access.grant(peer, collection).unwrap();
        let authorization = access
            .authorize(&peer, airwiki_types::SearchPurpose::LocalAssistant)
            .unwrap();
        let request = SearchRequest::new(
            "authorized",
            airwiki_types::SearchPurpose::LocalAssistant,
            1,
        );
        let response = SearchWireResponse::Success(fixture_response(
            &request,
            &authorization,
            collection,
            "authorized evidence",
        ));
        let unrelated_lease = airwiki_types::DisclosureGate::default().acquire_disclosure();
        let mut sent = None;

        let hit_count = deliver_backend_response(
            &access,
            peer,
            response,
            1,
            Some(unrelated_lease),
            |response| sent = Some(response),
        );

        assert_eq!(hit_count, 0);
        assert!(matches!(sent, Some(SearchWireResponse::Error(_))));
    }

    #[test]
    fn final_transport_barrier_removes_candidate_without_current_grant() {
        let access = AccessControl::default();
        let peer = PeerId::random();
        let granted_collection = uuid::Uuid::new_v4();
        let ungranted_collection = uuid::Uuid::new_v4();
        access.mark_trusted(peer);
        access.grant(peer, granted_collection).unwrap();
        let authorization = access
            .authorize(&peer, airwiki_types::SearchPurpose::ExternalAi)
            .unwrap();
        let request = SearchRequest::new("authorized", airwiki_types::SearchPurpose::ExternalAi, 1);
        let mut response = fixture_response(
            &request,
            &authorization,
            granted_collection,
            "authorized evidence",
        );
        let mut candidate = response.hits[0].clone();
        candidate.collection_id = ungranted_collection;
        response.authorized_candidates.push(candidate);
        let lease = authorization.acquire_disclosure_lease();
        let mut sent = None;

        deliver_backend_response(
            &access,
            peer,
            SearchWireResponse::Success(response),
            1,
            Some(lease),
            |response| sent = Some(response),
        );

        let Some(SearchWireResponse::Success(sent)) = sent else {
            panic!("expected an authorized response");
        };
        assert_eq!(sent.hits.len(), 1);
        assert!(sent.authorized_candidates.is_empty());
    }

    async fn wait_for_connected(events: &mut broadcast::Receiver<NetworkEvent>, expected: PeerId) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let NetworkEvent::Connected { peer } = events.recv().await.unwrap()
                    && peer == expected
                {
                    return;
                }
            }
        })
        .await
        .expect("nodes should establish Noise connection");
    }

    async fn wait_for_disconnected(
        events: &mut broadcast::Receiver<NetworkEvent>,
        expected: PeerId,
    ) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let NetworkEvent::Disconnected { peer } = events.recv().await.unwrap()
                    && peer == expected
                {
                    return;
                }
            }
        })
        .await
        .expect("nodes should observe the closed connection");
    }

    async fn wait_for_pairing_code(
        events: &mut broadcast::Receiver<NetworkEvent>,
        expected: PeerId,
    ) -> PairingCode {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let NetworkEvent::PairingCode { peer, sas, .. } = events.recv().await.unwrap()
                    && peer == expected
                {
                    return sas;
                }
            }
        })
        .await
        .expect("both nodes should receive an SAS")
    }

    async fn wait_for_trusted(events: &mut broadcast::Receiver<NetworkEvent>, expected: PeerId) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let NetworkEvent::PairingTrusted { peer } = events.recv().await.unwrap()
                    && peer == expected
                {
                    return;
                }
            }
        })
        .await
        .expect("both confirmations should establish trust");
    }

    async fn wait_for_cancelled(events: &mut broadcast::Receiver<NetworkEvent>, expected: PeerId) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let NetworkEvent::PairingCancelled {
                    peer,
                    blocked: false,
                } = events.recv().await.unwrap()
                    && peer == expected
                {
                    return;
                }
            }
        })
        .await
        .expect("pairing cancellation should be observable");
    }

    async fn wait_for_pairing_expired(
        events: &mut broadcast::Receiver<NetworkEvent>,
        expected: PeerId,
    ) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let NetworkEvent::PairingExpired { peer } = events.recv().await.unwrap()
                    && peer == expected
                {
                    return;
                }
            }
        })
        .await
        .expect("pairing expiration should be observable");
    }

    async fn wait_for_pairing_failure(
        events: &mut broadcast::Receiver<NetworkEvent>,
        expected: PeerId,
    ) -> PairingFailureReason {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let NetworkEvent::PairingFailed { peer, reason } = events.recv().await.unwrap()
                    && peer == expected
                {
                    return reason;
                }
            }
        })
        .await
        .expect("pairing failure should be observable")
    }

    async fn record_discovered_address(handle: &NetworkHandle, peer: PeerId, address: Multiaddr) {
        handle
            .command(Command::RecordDiscoveredAddress { peer, address })
            .await
            .unwrap();
    }

    async fn pairing_attempt_active(handle: &NetworkHandle, peer: PeerId) -> bool {
        let (reply, result) = oneshot::channel();
        handle
            .command(Command::PairingAttemptActive { peer, reply })
            .await
            .unwrap();
        result.await.unwrap()
    }

    fn hit(node: &str, rank: u32, hash: &str, chunk_id: uuid::Uuid) -> SearchHit {
        SearchHit {
            concept_id: uuid::Uuid::new_v4(),
            collection_id: uuid::Uuid::new_v4(),
            chunk_id,
            title: node.to_owned(),
            snippet: "evidence".to_owned(),
            heading_or_page: "page 1".to_owned(),
            logical_resource_uri: "urn:test".to_owned(),
            source_revision: 1,
            source_sha256: hash.to_owned(),
            updated_at: Utc::now(),
            rank,
            node_id: node.to_owned(),
        }
    }

    #[test]
    fn runtime_registers_only_search_protocol_v2() {
        let protocols = supported_search_protocols();

        assert_eq!(protocols.len(), 1);
        assert_eq!(protocols[0].0.as_ref(), "/airwiki/search/2.0.0");
        assert_ne!(protocols[0].0.as_ref(), "/airwiki/search/1.0.0");
    }

    #[test]
    fn repairing_selects_behaviour_after_revoke_clears_address_book_when_mdns_is_current() {
        let peer = PeerId::random();
        let current_address: Multiaddr = "/ip4/192.168.1.25/tcp/41000".parse().unwrap();
        let mut address_book = PeerAddressBook::default();
        address_book
            .record_mdns(peer, current_address.clone())
            .unwrap();

        assert!(address_book.clear_peer(&peer));
        let cleared_addresses = address_book.dial_addresses(&peer);
        assert!(cleared_addresses.is_empty());
        assert_eq!(
            select_pairing_dial_source(cleared_addresses, true),
            Some(PairingDialSource::Behaviour)
        );
        assert_eq!(select_pairing_dial_source(Vec::new(), false), None);
        assert_eq!(
            select_pairing_dial_source(vec![current_address.clone()], true),
            Some(PairingDialSource::Explicit(vec![current_address]))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runtime_unit_tests_keep_mdns_off_the_lan() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let (handle, mut events, task) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity,
            AccessControl::default(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();

        let mut discovery_started = false;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await.unwrap() {
                    NetworkEvent::DiscoveryStarted => discovery_started = true,
                    NetworkEvent::Listening { .. } => break,
                    _ => {}
                }
            }
        })
        .await
        .expect("TCP listener startup should be observable");

        assert!(!discovery_started);
        assert!(handle.listener_is_ready());
        assert!(!handle.listener_is_unavailable());
        handle.shutdown().await.unwrap();
        task.await.unwrap();
        assert!(!handle.listener_is_ready());
        assert!(handle.listener_is_unavailable());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runtime_rejects_non_lan_manual_dial_before_network_io() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let (handle, _events, task) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity,
            AccessControl::default(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();

        let error = handle
            .dial("/ip4/8.8.8.8/tcp/443".parse().unwrap())
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            NetworkError::Address(crate::LanAddressError::NonLocalIp)
        ));
        handle.shutdown().await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn peer_ranking_fusion_deduplicates_document_and_chunk() {
        let request = SearchRequest::new("query", airwiki_types::SearchPurpose::LocalAssistant, 5);
        let duplicate_chunk = uuid::Uuid::new_v4();
        let (reply, _receiver) = oneshot::channel();
        let mut first = SearchResponse::empty(request.request_id);
        first.hits.push(hit("a", 1, "same", duplicate_chunk));
        let mut second = SearchResponse::empty(request.request_id);
        second.hits.push(hit("b", 2, "same", duplicate_chunk));
        second
            .hits
            .push(hit("b", 1, "different", uuid::Uuid::new_v4()));
        let query = QueryAggregate {
            request,
            pending: HashMap::new(),
            connecting: HashSet::new(),
            responses: vec![(PeerId::random(), first), (PeerId::random(), second)],
            offline: HashSet::new(),
            warnings: Vec::new(),
            deadline: Instant::now(),
            reply,
        };
        let (response, _) = fuse_peer_rankings(query);
        assert_eq!(response.hits.len(), 2);
        assert_eq!(response.hits[0].source_sha256, "same");
    }

    #[tokio::test]
    async fn peer_ranking_discards_candidates_for_local_assistant_requests() {
        let request = SearchRequest::new("query", airwiki_types::SearchPurpose::LocalAssistant, 5);
        let (reply, _receiver) = oneshot::channel();
        let mut remote = SearchResponse::empty(request.request_id);
        remote.authorized_candidates.push(hit(
            "candidate",
            1,
            "candidate-hash",
            uuid::Uuid::new_v4(),
        ));
        let query = QueryAggregate {
            request,
            pending: HashMap::new(),
            connecting: HashSet::new(),
            responses: vec![(PeerId::random(), remote)],
            offline: HashSet::new(),
            warnings: Vec::new(),
            deadline: Instant::now(),
            reply,
        };

        let (response, _) = fuse_peer_rankings(query);

        assert!(response.authorized_candidates.is_empty());
    }

    #[tokio::test]
    async fn peer_evidence_duplicate_does_not_displace_unique_candidate_at_top_k() {
        let request = SearchRequest::new("query", airwiki_types::SearchPurpose::ExternalAi, 1);
        let duplicate = uuid::Uuid::new_v4();
        let (reply, _receiver) = oneshot::channel();
        let mut remote = SearchResponse::empty(request.request_id);
        remote.hits.push(hit("evidence", 1, "same", duplicate));
        remote
            .authorized_candidates
            .push(hit("duplicate", 1, "same", duplicate));
        remote
            .authorized_candidates
            .push(hit("unique", 2, "unique", uuid::Uuid::new_v4()));
        let query = QueryAggregate {
            request,
            pending: HashMap::new(),
            connecting: HashSet::new(),
            responses: vec![(PeerId::random(), remote)],
            offline: HashSet::new(),
            warnings: Vec::new(),
            deadline: Instant::now(),
            reply,
        };

        let (response, _) = fuse_peer_rankings(query);

        assert_eq!(response.authorized_candidates.len(), 1);
        assert_eq!(response.authorized_candidates[0].title, "unique");
        assert_eq!(response.authorized_candidates[0].rank, 1);
    }

    #[tokio::test]
    async fn authenticated_peer_overrides_claimed_hit_node_identity() {
        let request = SearchRequest::new("query", airwiki_types::SearchPurpose::ExternalAi, 1);
        let authenticated_peer = PeerId::random();
        let claimed_peer = PeerId::random();
        let (reply, _receiver) = oneshot::channel();
        let mut remote = SearchResponse::empty(request.request_id);
        remote.hits.push(hit(
            &claimed_peer.to_string(),
            1,
            "source-hash",
            uuid::Uuid::new_v4(),
        ));
        let query = QueryAggregate {
            request,
            pending: HashMap::new(),
            connecting: HashSet::new(),
            responses: vec![(authenticated_peer, remote)],
            offline: HashSet::new(),
            warnings: Vec::new(),
            deadline: Instant::now(),
            reply,
        };

        let (response, _) = fuse_peer_rankings(query);

        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].node_id, authenticated_peer.to_string());
    }

    #[test]
    fn remote_diagnostics_are_replaced_and_only_local_offline_peers_are_reported() {
        let request = SearchRequest::new("query", airwiki_types::SearchPurpose::ExternalAi, 1);
        let responding_peer = PeerId::random();
        let locally_observed_offline_peer = PeerId::random();
        let (reply, _receiver) = oneshot::channel();
        let mut remote = SearchResponse::empty(request.request_id);
        remote
            .warnings
            .push("CANARY-WARNING at /Users/alice/private/source.md".to_owned());
        remote
            .offline_nodes
            .push("C:\\secrets\\CANARY-OFFLINE".to_owned());
        let query = QueryAggregate {
            request,
            pending: HashMap::new(),
            connecting: HashSet::new(),
            responses: vec![(responding_peer, remote)],
            offline: HashSet::from([locally_observed_offline_peer]),
            warnings: Vec::new(),
            deadline: Instant::now(),
            reply,
        };

        let (response, _) = fuse_peer_rankings(query);

        assert!(response.partial);
        assert_eq!(
            response.offline_nodes,
            [locally_observed_offline_peer.to_string()]
        );
        assert_eq!(
            response.warnings,
            [format!(
                "peer {responding_peer}: remote results may be incomplete"
            )]
        );
        let visible = format!(
            "{} {}",
            response.warnings.join(" "),
            response.offline_nodes.join(" ")
        );
        assert!(!visible.contains("CANARY"));
        assert!(!visible.contains("/Users/alice"));
        assert!(!visible.contains("C:\\secrets"));
    }

    #[test]
    fn remote_error_message_is_replaced_with_a_stable_local_warning() {
        let peer = PeerId::random();
        let error = SearchWireError::new(
            SearchWireErrorCode::Internal,
            "CANARY-ERROR at /var/private/airwiki.db",
        );

        let warning = remote_search_error_warning(peer, &error);

        assert_eq!(warning, format!("peer {peer}: remote search failed"));
        assert!(!warning.contains("CANARY-ERROR"));
        assert!(!warning.contains("/var/private"));
    }

    #[test]
    fn outbound_failure_reason_is_replaced_with_a_stable_local_warning() {
        let peer = PeerId::random();
        let failure = request_response::OutboundFailure::Io(std::io::Error::other(
            "CANARY-REASON at C:\\private\\transport.sock",
        ));

        let warning = outbound_search_failure_warning(peer, &failure);

        assert_eq!(warning, format!("peer {peer}: remote search unavailable"));
        assert!(!warning.contains("CANARY-REASON"));
        assert!(!warning.contains("C:\\private"));
    }

    #[test]
    fn incompatible_peer_makes_coverage_partial_without_being_reported_offline() {
        let request = SearchRequest::new("query", airwiki_types::SearchPurpose::ExternalAi, 1);
        let incompatible_peer = PeerId::random();
        let (reply, _receiver) = oneshot::channel();
        let mut query = QueryAggregate {
            request,
            pending: HashMap::new(),
            connecting: HashSet::new(),
            responses: Vec::new(),
            offline: HashSet::new(),
            warnings: Vec::new(),
            deadline: Instant::now(),
            reply,
        };

        record_outbound_search_failure(
            &mut query,
            incompatible_peer,
            &request_response::OutboundFailure::UnsupportedProtocols,
        );
        let (response, _) = fuse_peer_rankings(query);

        assert!(response.partial);
        assert!(response.offline_nodes.is_empty());
        assert_eq!(
            response.warnings,
            [format!(
                "peer {incompatible_peer}: remote peer does not support the search protocol"
            )]
        );
    }

    #[test]
    fn oversized_success_is_reduced_before_wire() {
        let mut response = SearchResponse::empty(uuid::Uuid::new_v4());
        for _ in 0..10 {
            let mut oversized = hit("node", 1, "hash", uuid::Uuid::new_v4());
            oversized.snippet = "x".repeat(100_000);
            response.hits.push(oversized);
        }
        let mut wire = SearchWireResponse::Success(response);
        shrink_to_wire_limit(&mut wire);
        assert!(response_fits(&wire));
    }

    #[test]
    fn wire_reduction_discards_candidates_before_evidence() {
        let mut response = SearchResponse::empty(uuid::Uuid::new_v4());
        response
            .hits
            .push(hit("evidence", 1, "evidence", uuid::Uuid::new_v4()));
        for _ in 0..10 {
            let mut oversized = hit("candidate", 1, "candidate", uuid::Uuid::new_v4());
            oversized.snippet = "x".repeat(100_000);
            response.authorized_candidates.push(oversized);
        }
        let mut wire = SearchWireResponse::Success(response);

        shrink_to_wire_limit(&mut wire);

        let SearchWireResponse::Success(response) = wire else {
            panic!("expected success response");
        };
        assert_eq!(response.hits.len(), 1);
        assert!(response.authorized_candidates.len() < 10);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn discovered_disconnected_peer_is_dialed_before_pairing_hello() {
        let collection = uuid::Uuid::new_v4();
        let identity_a =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let identity_b =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let peer_a = identity_a.peer_id();
        let peer_b = identity_b.peer_id();
        let listen_b = available_loopback_address();
        let (handle_a, mut events_a, task_a) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity_a,
            AccessControl::default(),
            Arc::new(FixtureBackend {
                collection,
                title: "local",
            }),
        )
        .unwrap();
        let (handle_b, mut events_b, task_b) = spawn_network(
            NetworkConfig {
                listen_address: listen_b.clone(),
                ..NetworkConfig::default()
            },
            identity_b,
            AccessControl::default(),
            Arc::new(FixtureBackend {
                collection,
                title: "remote",
            }),
        )
        .unwrap();
        record_discovered_address(&handle_a, peer_b, listen_b).await;

        handle_a.begin_pairing(peer_b).await.unwrap();
        handle_a.begin_pairing(peer_b).await.unwrap();
        let (sas_a, sas_b) = tokio::join!(
            wait_for_pairing_code(&mut events_a, peer_b),
            wait_for_pairing_code(&mut events_b, peer_a)
        );

        assert_eq!(sas_a, sas_b);
        let duplicate_hello = tokio::time::timeout(
            Duration::from_millis(200),
            wait_for_pairing_code(&mut events_b, peer_a),
        )
        .await;
        assert!(duplicate_hello.is_err());
        handle_a.shutdown().await.unwrap();
        handle_b.shutdown().await.unwrap();
        task_a.await.unwrap();
        task_b.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pairing_uses_new_mdns_port_after_peer_restart() {
        let collection = uuid::Uuid::new_v4();
        let identity_a =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let peer_a = identity_a.peer_id();
        let identity_store_b = crate::MemorySecretStore::default();
        let identity_b = NodeIdentity::load_or_create(&identity_store_b).unwrap();
        let peer_b = identity_b.peer_id();
        let listen_b_old = available_loopback_address();
        let (handle_a, mut events_a, task_a) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity_a,
            AccessControl::default(),
            Arc::new(FixtureBackend {
                collection,
                title: "local",
            }),
        )
        .unwrap();
        let (handle_b_old, mut events_b_old, task_b_old) = spawn_network(
            NetworkConfig {
                listen_address: listen_b_old.clone(),
                ..NetworkConfig::default()
            },
            identity_b,
            AccessControl::default(),
            Arc::new(FixtureBackend {
                collection,
                title: "old",
            }),
        )
        .unwrap();
        let listen_b_new = available_loopback_address();
        assert_ne!(listen_b_old, listen_b_new);
        record_discovered_address(&handle_a, peer_b, listen_b_old).await;
        handle_a.begin_pairing(peer_b).await.unwrap();
        tokio::join!(
            wait_for_pairing_code(&mut events_a, peer_b),
            wait_for_pairing_code(&mut events_b_old, peer_a)
        );
        handle_a.cancel_pairing(peer_b).await.unwrap();
        wait_for_cancelled(&mut events_a, peer_b).await;
        handle_b_old.shutdown().await.unwrap();
        task_b_old.await.unwrap();
        wait_for_disconnected(&mut events_a, peer_b).await;

        let identity_b_restarted = NodeIdentity::load_or_create(&identity_store_b).unwrap();
        assert_eq!(identity_b_restarted.peer_id(), peer_b);
        let (handle_b_new, mut events_b_new, task_b_new) = spawn_network(
            NetworkConfig {
                listen_address: listen_b_new.clone(),
                ..NetworkConfig::default()
            },
            identity_b_restarted,
            AccessControl::default(),
            Arc::new(FixtureBackend {
                collection,
                title: "new",
            }),
        )
        .unwrap();
        record_discovered_address(&handle_a, peer_b, listen_b_new).await;

        handle_a.begin_pairing(peer_b).await.unwrap();
        let (sas_a, sas_b) = tokio::join!(
            wait_for_pairing_code(&mut events_a, peer_b),
            wait_for_pairing_code(&mut events_b_new, peer_a)
        );

        assert_eq!(sas_a, sas_b);
        handle_a.shutdown().await.unwrap();
        handle_b_new.shutdown().await.unwrap();
        task_a.await.unwrap();
        task_b_new.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_without_current_address_fails_without_changing_trust() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let unavailable_peer = PeerId::random();
        let access = AccessControl::default();
        let (handle, mut events, task) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity,
            access.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();

        handle.begin_pairing(unavailable_peer).await.unwrap();
        let reason = wait_for_pairing_failure(&mut events, unavailable_peer).await;

        assert_eq!(reason, PairingFailureReason::NoCurrentAddress);
        assert_eq!(
            access.state(&unavailable_peer),
            crate::PeerAccess::default()
        );
        assert!(!pairing_attempt_active(&handle, unavailable_peer).await);
        handle.shutdown().await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_repairing_restores_the_previous_block() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let blocked_peer = PeerId::random();
        let access = AccessControl::default();
        access.block(blocked_peer);
        let (handle, mut events, task) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity,
            access.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();

        handle.begin_pairing(blocked_peer).await.unwrap();
        let reason = wait_for_pairing_failure(&mut events, blocked_peer).await;

        assert_eq!(reason, PairingFailureReason::NoCurrentAddress);
        assert!(access.is_blocked(&blocked_peer));
        assert!(!pairing_attempt_active(&handle, blocked_peer).await);
        handle.shutdown().await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_connection_failure_is_typed_and_clears_the_attempt() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let unavailable_peer = PeerId::random();
        let unavailable_listener =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let unavailable_address = format!(
            "/ip4/127.0.0.1/tcp/{}",
            unavailable_listener.local_addr().unwrap().port()
        )
        .parse()
        .unwrap();
        let (handle, mut events, task) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity,
            AccessControl::default(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();
        drop(unavailable_listener);
        record_discovered_address(&handle, unavailable_peer, unavailable_address).await;

        handle.begin_pairing(unavailable_peer).await.unwrap();
        let reason = wait_for_pairing_failure(&mut events, unavailable_peer).await;

        assert_eq!(reason, PairingFailureReason::ConnectionFailed);
        assert!(!pairing_attempt_active(&handle, unavailable_peer).await);
        handle.shutdown().await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_pairing_clears_a_pending_dial() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let pending_peer = PeerId::random();
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let blackhole = format!(
            "/ip4/127.0.0.1/tcp/{}",
            listener.local_addr().unwrap().port()
        )
        .parse()
        .unwrap();
        let access = AccessControl::default();
        access.block(pending_peer);
        let (handle, mut events, task) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity,
            access.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();
        record_discovered_address(&handle, pending_peer, blackhole).await;
        handle.begin_pairing(pending_peer).await.unwrap();
        assert!(pairing_attempt_active(&handle, pending_peer).await);
        assert!(!access.is_blocked(&pending_peer));

        handle.cancel_pairing(pending_peer).await.unwrap();
        wait_for_cancelled(&mut events, pending_peer).await;

        assert!(access.is_blocked(&pending_peer));
        assert!(!pairing_attempt_active(&handle, pending_peer).await);
        handle.begin_pairing(pending_peer).await.unwrap();
        assert_eq!(
            wait_for_pairing_failure(&mut events, pending_peer).await,
            PairingFailureReason::NoCurrentAddress
        );
        assert!(access.is_blocked(&pending_peer));
        drop(listener);
        handle.shutdown().await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_restores_a_transient_pairing_block_before_acknowledging() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let blocked_peer = PeerId::random();
        let blackhole_listener =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let blackhole = format!(
            "/ip4/127.0.0.1/tcp/{}",
            blackhole_listener.local_addr().unwrap().port()
        )
        .parse()
        .unwrap();
        let access = AccessControl::default();
        access.block(blocked_peer);
        let config = NetworkConfig {
            listen_address: available_loopback_address(),
            ..NetworkConfig::default()
        };
        let (handle, _events, task) = spawn_network(
            config.clone(),
            identity.clone(),
            access.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();
        record_discovered_address(&handle, blocked_peer, blackhole).await;
        handle.begin_pairing(blocked_peer).await.unwrap();
        assert!(pairing_attempt_active(&handle, blocked_peer).await);
        assert!(!access.is_blocked(&blocked_peer));

        handle.shutdown().await.unwrap();

        // The shutdown acknowledgement is the cleanup boundary used by the
        // desktop before it may abort or replace the runtime task.
        assert!(access.is_blocked(&blocked_peer));
        task.await.unwrap();

        let (restarted, _events, restarted_task) = spawn_network(
            config,
            identity,
            access.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();
        assert!(access.is_blocked(&blocked_peer));
        assert!(!pairing_attempt_active(&restarted, blocked_peer).await);

        restarted.shutdown().await.unwrap();
        restarted_task.await.unwrap();
        drop(blackhole_listener);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborting_runtime_restores_a_transient_pairing_block_on_drop() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let blocked_peer = PeerId::random();
        let blackhole_listener =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let blackhole = format!(
            "/ip4/127.0.0.1/tcp/{}",
            blackhole_listener.local_addr().unwrap().port()
        )
        .parse()
        .unwrap();
        let access = AccessControl::default();
        access.block(blocked_peer);
        let (handle, _events, task) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity,
            access.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();
        record_discovered_address(&handle, blocked_peer, blackhole).await;
        handle.begin_pairing(blocked_peer).await.unwrap();
        assert!(pairing_attempt_active(&handle, blocked_peer).await);
        assert!(!access.is_blocked(&blocked_peer));

        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        assert!(access.is_blocked(&blocked_peer));
        drop(blackhole_listener);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn expired_repairing_restores_block_and_closes_connection() {
        let collection = uuid::Uuid::new_v4();
        let identity_a =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let identity_b =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let peer_a = identity_a.peer_id();
        let peer_b = identity_b.peer_id();
        let access_a = AccessControl::default();
        access_a.block(peer_b);
        let listen_b = available_loopback_address();
        let (handle_a, mut events_a, task_a) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity_a,
            access_a.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "a",
            }),
        )
        .unwrap();
        let (handle_b, mut events_b, task_b) = spawn_network(
            NetworkConfig {
                listen_address: listen_b.clone(),
                ..NetworkConfig::default()
            },
            identity_b,
            AccessControl::default(),
            Arc::new(FixtureBackend {
                collection,
                title: "b",
            }),
        )
        .unwrap();
        record_discovered_address(&handle_a, peer_b, listen_b).await;

        handle_a.begin_pairing(peer_b).await.unwrap();
        tokio::join!(
            wait_for_pairing_code(&mut events_a, peer_b),
            wait_for_pairing_code(&mut events_b, peer_a)
        );
        assert!(!access_a.is_blocked(&peer_b));

        handle_a
            .command(Command::ExpirePairings {
                now: Instant::now() + crate::pairing::PAIRING_TTL,
            })
            .await
            .unwrap();
        wait_for_pairing_expired(&mut events_a, peer_b).await;

        assert!(access_a.is_blocked(&peer_b));
        assert!(!pairing_attempt_active(&handle_a, peer_b).await);
        tokio::join!(
            wait_for_disconnected(&mut events_a, peer_b),
            wait_for_disconnected(&mut events_b, peer_a)
        );
        handle_a.begin_pairing(peer_b).await.unwrap();
        assert_eq!(
            wait_for_pairing_failure(&mut events_a, peer_b).await,
            PairingFailureReason::NoCurrentAddress
        );
        assert!(access_a.is_blocked(&peer_b));

        handle_a.shutdown().await.unwrap();
        handle_b.shutdown().await.unwrap();
        task_a.await.unwrap();
        task_b.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn blocked_peer_cannot_complete_an_inbound_pairing_race() {
        let collection = uuid::Uuid::new_v4();
        let identity_a =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let identity_b =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let peer_a = identity_a.peer_id();
        let peer_b = identity_b.peer_id();
        let access_a = AccessControl::default();
        let access_b = AccessControl::default();
        let listen_a = available_loopback_address();
        let (handle_a, mut events_a, task_a) = spawn_network(
            NetworkConfig {
                listen_address: listen_a.clone(),
                ..NetworkConfig::default()
            },
            identity_a,
            access_a.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "a",
            }),
        )
        .unwrap();
        let (handle_b, mut events_b, task_b) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity_b,
            access_b.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "b",
            }),
        )
        .unwrap();
        let mut dial_a = listen_a;
        dial_a.push(Protocol::P2p(peer_a));
        handle_b.dial(dial_a).await.unwrap();
        tokio::join!(
            wait_for_connected(&mut events_a, peer_b),
            wait_for_connected(&mut events_b, peer_a)
        );

        // Simulate a revoke racing with an already authenticated connection and
        // an inbound Hello queued by the remote peer.
        access_a.block(peer_b);
        handle_b.begin_pairing(peer_a).await.unwrap();
        assert_eq!(
            wait_for_pairing_failure(&mut events_b, peer_a).await,
            PairingFailureReason::HandshakeFailed
        );
        assert!(access_a.state(&peer_b).blocked);
        assert!(!access_a.state(&peer_b).trusted);
        assert!(!access_b.state(&peer_a).trusted);
        assert!(!pairing_attempt_active(&handle_a, peer_b).await);
        assert!(!pairing_attempt_active(&handle_b, peer_a).await);

        handle_a.shutdown().await.unwrap();
        handle_b.shutdown().await.unwrap();
        task_a.await.unwrap();
        task_b.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revoking_peer_clears_a_pending_pairing_dial() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let pending_peer = PeerId::random();
        let access = AccessControl::default();
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let blackhole = format!(
            "/ip4/127.0.0.1/tcp/{}",
            listener.local_addr().unwrap().port()
        )
        .parse()
        .unwrap();
        let (handle, mut events, task) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity,
            access.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();
        record_discovered_address(&handle, pending_peer, blackhole).await;
        handle.begin_pairing(pending_peer).await.unwrap();
        assert!(pairing_attempt_active(&handle, pending_peer).await);

        handle.revoke_peer(pending_peer).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let NetworkEvent::PeerRevoked { peer } = events.recv().await.unwrap()
                    && peer == pending_peer
                {
                    return;
                }
            }
        })
        .await
        .expect("revocation should be observable");

        assert!(!pairing_attempt_active(&handle, pending_peer).await);
        assert!(access.state(&pending_peer).blocked);
        drop(listener);
        handle.shutdown().await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_real_nodes_pair_and_search_over_tcp_noise_yamux() {
        let collection = uuid::Uuid::new_v4();
        let identity_a =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let identity_b =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let peer_a = identity_a.peer_id();
        let peer_b = identity_b.peer_id();
        let access_a = AccessControl::default();
        let access_b = AccessControl::default();
        let listen_a = available_loopback_address();
        let listen_b = available_loopback_address();

        let config_a = NetworkConfig {
            node_name: "Mac fixture".to_owned(),
            listen_address: listen_a,
            ..NetworkConfig::default()
        };
        let config_b = NetworkConfig {
            node_name: "Windows fixture".to_owned(),
            listen_address: listen_b.clone(),
            ..NetworkConfig::default()
        };
        let (handle_a, mut events_a, task_a) = spawn_network(
            config_a,
            identity_a,
            access_a.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "local",
            }),
        )
        .unwrap();
        let (handle_b, mut events_b, task_b) = spawn_network(
            config_b,
            identity_b,
            access_b.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "remote Windows evidence",
            }),
        )
        .unwrap();
        let mut listen_b = listen_b;
        listen_b.push(Protocol::P2p(peer_b));

        handle_a.dial(listen_b).await.unwrap();
        tokio::join!(
            wait_for_connected(&mut events_a, peer_b),
            wait_for_connected(&mut events_b, peer_a)
        );

        handle_a.begin_pairing(peer_b).await.unwrap();
        let (sas_a, sas_b) = tokio::join!(
            wait_for_pairing_code(&mut events_a, peer_b),
            wait_for_pairing_code(&mut events_b, peer_a)
        );
        assert_eq!(sas_a, sas_b);

        handle_a.cancel_pairing(peer_b).await.unwrap();
        wait_for_cancelled(&mut events_a, peer_b).await;
        assert!(!access_a.state(&peer_b).trusted);

        handle_a.begin_pairing(peer_b).await.unwrap();
        let (sas_a, sas_b) = tokio::join!(
            wait_for_pairing_code(&mut events_a, peer_b),
            wait_for_pairing_code(&mut events_b, peer_a)
        );
        assert_eq!(sas_a, sas_b);

        handle_a.confirm_pairing(peer_b).await.unwrap();
        handle_b.confirm_pairing(peer_a).await.unwrap();
        tokio::join!(
            wait_for_trusted(&mut events_a, peer_b),
            wait_for_trusted(&mut events_b, peer_a)
        );

        let denied = handle_a
            .search_peers(SearchRequest::new(
                "responsable",
                airwiki_types::SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap();
        assert!(denied.hits.is_empty(), "no grant may return no evidence");
        assert!(denied.partial);

        access_b.grant(peer_a, collection).unwrap();

        let response = handle_a
            .search_peers(SearchRequest::new(
                "responsable",
                airwiki_types::SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap();
        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].title, "remote Windows evidence");
        assert_eq!(response.hits[0].node_id, peer_b.to_string());

        handle_a
            .command(Command::DisconnectPeer { peer: peer_b })
            .await
            .unwrap();
        tokio::join!(
            wait_for_disconnected(&mut events_a, peer_b),
            wait_for_disconnected(&mut events_b, peer_a)
        );

        let (response, ()) = tokio::join!(
            handle_a.search_peers(SearchRequest::new(
                "responsable",
                airwiki_types::SearchPurpose::LocalAssistant,
                5,
            )),
            wait_for_connected(&mut events_a, peer_b),
        );
        let response = response.unwrap();
        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].title, "remote Windows evidence");
        assert_eq!(response.hits[0].node_id, peer_b.to_string());

        handle_a.shutdown().await.unwrap();
        handle_b.shutdown().await.unwrap();
        task_a.await.unwrap();
        task_b.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn trusted_search_redials_same_peer_after_restart_on_a_new_port() {
        let collection = uuid::Uuid::new_v4();
        let identity_a =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let identity_store_b = crate::MemorySecretStore::default();
        let identity_b = NodeIdentity::load_or_create(&identity_store_b).unwrap();
        let peer_a = identity_a.peer_id();
        let peer_b = identity_b.peer_id();
        let access_a = AccessControl::default();
        let access_b = AccessControl::default();
        access_a.mark_trusted(peer_b);
        access_b.mark_trusted(peer_a);
        access_b.grant(peer_a, collection).unwrap();

        let listen_b_old = available_loopback_address();
        let (handle_a, mut events_a, task_a) = spawn_network(
            NetworkConfig {
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity_a,
            access_a,
            Arc::new(FixtureBackend {
                collection,
                title: "local",
            }),
        )
        .unwrap();
        let (handle_b_old, _events_b_old, task_b_old) = spawn_network(
            NetworkConfig {
                listen_address: listen_b_old.clone(),
                ..NetworkConfig::default()
            },
            identity_b,
            access_b.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "old port evidence",
            }),
        )
        .unwrap();
        record_discovered_address(&handle_a, peer_b, listen_b_old.clone()).await;

        let (initial, ()) = tokio::join!(
            handle_a.search_peers(SearchRequest::new(
                "responsable",
                airwiki_types::SearchPurpose::LocalAssistant,
                5,
            )),
            wait_for_connected(&mut events_a, peer_b),
        );
        assert_eq!(initial.unwrap().hits[0].title, "old port evidence");

        // Reserve a different dynamic endpoint while the old listener is still
        // bound, then restart with the same persisted Ed25519 identity.
        let listen_b_new = available_loopback_address();
        assert_ne!(listen_b_new, listen_b_old);
        handle_b_old.shutdown().await.unwrap();
        task_b_old.await.unwrap();
        wait_for_disconnected(&mut events_a, peer_b).await;

        let identity_b_restarted = NodeIdentity::load_or_create(&identity_store_b).unwrap();
        assert_eq!(identity_b_restarted.peer_id(), peer_b);
        let (handle_b_new, _events_b_new, task_b_new) = spawn_network(
            NetworkConfig {
                listen_address: listen_b_new.clone(),
                ..NetworkConfig::default()
            },
            identity_b_restarted,
            access_b,
            Arc::new(FixtureBackend {
                collection,
                title: "new port evidence",
            }),
        )
        .unwrap();
        record_discovered_address(&handle_a, peer_b, listen_b_new).await;

        let (recovered, ()) = tokio::join!(
            handle_a.search_peers(SearchRequest::new(
                "responsable",
                airwiki_types::SearchPurpose::LocalAssistant,
                5,
            )),
            wait_for_connected(&mut events_a, peer_b),
        );
        let recovered = recovered.unwrap();
        assert_eq!(recovered.hits.len(), 1);
        assert_eq!(recovered.hits[0].title, "new port evidence");
        assert_eq!(recovered.hits[0].node_id, peer_b.to_string());

        handle_a.shutdown().await.unwrap();
        handle_b_new.shutdown().await.unwrap();
        task_a.await.unwrap();
        task_b_new.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn revocation_retires_in_flight_and_redial_searches_until_repaired() {
        let collection = uuid::Uuid::new_v4();
        let identity_a =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let identity_b =
            NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let peer_a = identity_a.peer_id();
        let peer_b = identity_b.peer_id();
        let access_a = AccessControl::default();
        let access_b = AccessControl::default();
        let listen_b = available_loopback_address();
        let calls = Arc::new(AtomicUsize::new(0));
        let first_call_started = Arc::new(Notify::new());
        let release_first_call = Arc::new(Notify::new());

        let (handle_a, mut events_a, task_a) = spawn_network(
            NetworkConfig {
                node_name: "revoking node".to_owned(),
                listen_address: available_loopback_address(),
                ..NetworkConfig::default()
            },
            identity_a,
            access_a.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "local",
            }),
        )
        .unwrap();
        let (handle_b, mut events_b, task_b) = spawn_network(
            NetworkConfig {
                node_name: "evidence node".to_owned(),
                listen_address: listen_b.clone(),
                ..NetworkConfig::default()
            },
            identity_b,
            access_b.clone(),
            Arc::new(GatedFixtureBackend {
                collection,
                title: "authorized remote evidence",
                calls: Arc::clone(&calls),
                first_call_started: Arc::clone(&first_call_started),
                release_first_call: Arc::clone(&release_first_call),
            }),
        )
        .unwrap();
        let mut dial_b = listen_b;
        dial_b.push(Protocol::P2p(peer_b));

        handle_a.dial(dial_b.clone()).await.unwrap();
        tokio::join!(
            wait_for_connected(&mut events_a, peer_b),
            wait_for_connected(&mut events_b, peer_a)
        );
        handle_a.begin_pairing(peer_b).await.unwrap();
        let (sas_a, sas_b) = tokio::join!(
            wait_for_pairing_code(&mut events_a, peer_b),
            wait_for_pairing_code(&mut events_b, peer_a)
        );
        assert_eq!(sas_a, sas_b);
        handle_a.confirm_pairing(peer_b).await.unwrap();
        handle_b.confirm_pairing(peer_a).await.unwrap();
        tokio::join!(
            wait_for_trusted(&mut events_a, peer_b),
            wait_for_trusted(&mut events_b, peer_a)
        );
        access_b.grant(peer_a, collection).unwrap();

        let searching_handle = handle_a.clone();
        let in_flight = tokio::spawn(async move {
            searching_handle
                .search_peers(SearchRequest::new(
                    "responsable",
                    airwiki_types::SearchPurpose::LocalAssistant,
                    5,
                ))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), first_call_started.notified())
            .await
            .expect("the remote backend should receive the first request");

        handle_a.revoke_peer(peer_b).await.unwrap();
        let revoked = tokio::time::timeout(Duration::from_secs(1), in_flight)
            .await
            .expect("revocation should finish an in-flight search immediately")
            .unwrap()
            .unwrap();
        assert!(revoked.hits.is_empty());
        assert!(revoked.partial);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        release_first_call.notify_one();
        tokio::join!(
            wait_for_disconnected(&mut events_a, peer_b),
            wait_for_disconnected(&mut events_b, peer_a)
        );
        let mut rediscovered_b = dial_b.clone();
        let _ = rediscovered_b.pop();
        record_discovered_address(&handle_a, peer_b, rediscovered_b).await;
        handle_a.begin_pairing(peer_b).await.unwrap();
        let (sas_a, sas_b) = tokio::join!(
            wait_for_pairing_code(&mut events_a, peer_b),
            wait_for_pairing_code(&mut events_b, peer_a)
        );
        assert_eq!(sas_a, sas_b);
        handle_a.confirm_pairing(peer_b).await.unwrap();
        handle_b.confirm_pairing(peer_a).await.unwrap();
        tokio::join!(
            wait_for_trusted(&mut events_a, peer_b),
            wait_for_trusted(&mut events_b, peer_a)
        );

        let repaired = handle_a
            .search_peers(SearchRequest::new(
                "responsable",
                airwiki_types::SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap();
        assert_eq!(repaired.hits.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        handle_a
            .command(Command::DisconnectPeer { peer: peer_b })
            .await
            .unwrap();
        tokio::join!(
            wait_for_disconnected(&mut events_a, peer_b),
            wait_for_disconnected(&mut events_b, peer_a)
        );

        let (reply, redial_result) = oneshot::channel();
        handle_a
            .command(Command::SearchThenRevokeAndBeginPairing {
                peer: peer_b,
                request: SearchRequest::new(
                    "responsable",
                    airwiki_types::SearchPurpose::LocalAssistant,
                    5,
                ),
                reply,
            })
            .await
            .unwrap();

        let retired_redial = tokio::time::timeout(Duration::from_secs(1), redial_result)
            .await
            .expect("revocation should retire a search waiting for redial")
            .unwrap()
            .unwrap();
        assert!(retired_redial.hits.is_empty());
        assert!(retired_redial.partial);

        // The search-triggered dial may race the pairing attempt. Normalize
        // either outcome through the same unsuccessful terminal and verify that
        // the durable block is restored before retrying with fresh discovery.
        handle_a.cancel_pairing(peer_b).await.unwrap();
        wait_for_cancelled(&mut events_a, peer_b).await;
        handle_b.cancel_pairing(peer_a).await.unwrap();
        let pairing_access = access_a.state(&peer_b);
        assert!(pairing_access.blocked);
        assert!(!pairing_access.trusted);
        assert!(!pairing_attempt_active(&handle_a, peer_b).await);

        let before_sas_confirmation = handle_a
            .search_peers(SearchRequest::new(
                "responsable",
                airwiki_types::SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap();
        assert!(before_sas_confirmation.hits.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // Discovery may safely provide a fresh address after revocation. A new
        // explicit pairing attempt can then recover without resurrecting the
        // retired query.
        let mut rediscovered_b = dial_b;
        let _ = rediscovered_b.pop();
        record_discovered_address(&handle_a, peer_b, rediscovered_b).await;
        handle_a.begin_pairing(peer_b).await.unwrap();
        let (sas_a, sas_b) = tokio::join!(
            wait_for_pairing_code(&mut events_a, peer_b),
            wait_for_pairing_code(&mut events_b, peer_a)
        );
        assert_eq!(sas_a, sas_b);
        handle_a.confirm_pairing(peer_b).await.unwrap();
        handle_b.confirm_pairing(peer_a).await.unwrap();
        tokio::join!(
            wait_for_trusted(&mut events_a, peer_b),
            wait_for_trusted(&mut events_b, peer_a)
        );
        let recovered = handle_a
            .search_peers(SearchRequest::new(
                "responsable",
                airwiki_types::SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap();
        assert_eq!(recovered.hits.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        handle_a.shutdown().await.unwrap();
        handle_b.shutdown().await.unwrap();
        task_a.await.unwrap();
        task_b.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trusted_peer_without_a_known_address_is_reported_offline() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let unavailable_peer = PeerId::random();
        let access = AccessControl::default();
        access.mark_trusted(unavailable_peer);
        let config = NetworkConfig {
            listen_address: available_loopback_address(),
            ..NetworkConfig::default()
        };
        let (handle, _events, task) = spawn_network(
            config,
            identity,
            access,
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();

        let response = tokio::time::timeout(
            Duration::from_secs(1),
            handle.search_peers(SearchRequest::new(
                "responsable",
                airwiki_types::SearchPurpose::LocalAssistant,
                5,
            )),
        )
        .await
        .expect("a missing address should fail without waiting for the search deadline")
        .unwrap();

        assert_eq!(
            (
                response.hits.len(),
                response.offline_nodes,
                response.partial
            ),
            (0, vec![unavailable_peer.to_string()], true)
        );

        handle.shutdown().await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rejecting_mismatched_sas_blocks_peer_without_trusting() {
        let collection = uuid::Uuid::new_v4();
        let identity = NodeIdentity::load_or_create(&crate::MemorySecretStore::default()).unwrap();
        let access = AccessControl::default();
        let rejected_peer = PeerId::random();
        let config = NetworkConfig {
            listen_address: available_loopback_address(),
            ..NetworkConfig::default()
        };
        let (handle, mut events, task) = spawn_network(
            config,
            identity,
            access.clone(),
            Arc::new(FixtureBackend {
                collection,
                title: "unused",
            }),
        )
        .unwrap();
        handle.reject_pairing(rejected_peer).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let NetworkEvent::PairingCancelled {
                    peer,
                    blocked: true,
                } = events.recv().await.unwrap()
                    && peer == rejected_peer
                {
                    return;
                }
            }
        })
        .await
        .expect("rejection event should be emitted");

        let state = access.state(&rejected_peer);
        assert!(state.blocked);
        assert!(!state.trusted);
        assert!(state.grants.is_empty());

        handle.shutdown().await.unwrap();
        task.await.unwrap();
    }
}
