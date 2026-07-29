use std::collections::HashSet;
use std::fmt;
use std::future;
use std::sync::Arc;
use std::time::Duration;

use airwiki_types::{
    DisclosureLease, PUBLIC_BROWSE_PROTOCOL, PUBLIC_SEARCH_PROTOCOL, PublicBrowsePage,
    PublicBrowseRequest, PublicSearchRequest, PublicSearchResponse,
};
use async_trait::async_trait;
use libp2p::core::transport::ListenerId;
use libp2p::request_response::{self, ProtocolSupport, ResponseChannel};
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{Multiaddr, StreamProtocol, SwarmBuilder};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::Instant as TokioInstant;
use tokio_util::sync::CancellationToken;

use crate::{NetworkError, NodeIdentity, PeerRateLimiter};

const PUBLIC_REQUEST_BYTES: u64 = 16 * 1024;
const PUBLIC_RESPONSE_BYTES: u64 = 256 * 1024;
const PUBLIC_CONCURRENT_STREAMS: usize = 64;
const PUBLIC_INBOUND_TASKS: usize = 32;
const PUBLIC_LISTEN_RETRY: Duration = Duration::from_millis(250);
const PUBLIC_RELAY_RETRY_MAX: Duration = Duration::from_secs(10);
const PUBLIC_LISTENER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const PUBLIC_LISTENER_RELEASE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PublicSearchWireResponse {
    Success(PublicSearchResponse),
    Rejected(PublicSourceRejection),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PublicBrowseWireResponse {
    Success(PublicBrowsePage),
    Rejected(PublicSourceRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicSourceRejection {
    Invalid,
    NotPublic,
    Busy,
    Unavailable,
}

#[derive(Debug, Error)]
pub enum PublicSourceBackendError {
    #[error("public request is invalid")]
    Invalid,
    #[error("collection is not public")]
    NotPublic,
    #[error("public source is busy")]
    Busy,
    #[error("public source is unavailable")]
    Unavailable,
}

impl PublicSourceBackendError {
    const fn rejection(&self) -> PublicSourceRejection {
        match self {
            Self::Invalid => PublicSourceRejection::Invalid,
            Self::NotPublic => PublicSourceRejection::NotPublic,
            Self::Busy => PublicSourceRejection::Busy,
            Self::Unavailable => PublicSourceRejection::Unavailable,
        }
    }
}

pub struct PublicSearchDelivery {
    response: PublicSearchResponse,
    _lease: DisclosureLease,
}

impl PublicSearchDelivery {
    pub fn new(response: PublicSearchResponse, lease: DisclosureLease) -> Self {
        Self {
            response,
            _lease: lease,
        }
    }
}

pub struct PublicBrowseDelivery {
    page: PublicBrowsePage,
    _lease: DisclosureLease,
}

impl PublicBrowseDelivery {
    pub fn new(page: PublicBrowsePage, lease: DisclosureLease) -> Self {
        Self {
            page,
            _lease: lease,
        }
    }
}

#[async_trait]
pub trait PublicSourceBackend: Send + Sync + 'static {
    async fn search(
        &self,
        request: PublicSearchRequest,
    ) -> Result<PublicSearchDelivery, PublicSourceBackendError>;

    async fn browse(
        &self,
        request: PublicBrowseRequest,
    ) -> Result<PublicBrowseDelivery, PublicSourceBackendError>;
}

#[derive(Debug, Clone)]
pub struct PublicSourceServerConfig {
    pub listen_addresses: Vec<Multiaddr>,
    pub relay_addresses: Vec<Multiaddr>,
    pub request_timeout: Duration,
    pub relay_readiness: Option<watch::Sender<PublicRelayReadiness>>,
}

impl PublicSourceServerConfig {
    pub fn new(listen_addresses: Vec<Multiaddr>) -> Self {
        Self {
            listen_addresses,
            relay_addresses: Vec::new(),
            request_timeout: Duration::from_millis(800),
            relay_readiness: None,
        }
    }
}

/// Current relay reservations that have produced a usable circuit-listen
/// address.
///
/// The addresses stay process-local and must not be written to normal logs.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PublicRelayReadiness {
    ready_addresses: Vec<Multiaddr>,
}

impl PublicRelayReadiness {
    /// Returns the number of distinct relay reservations that are ready.
    pub fn ready_relay_count(&self) -> usize {
        self.ready_addresses.len()
    }

    /// Returns the ready circuit-listen addresses in configured order.
    pub fn ready_relay_addresses(&self) -> &[Multiaddr] {
        &self.ready_addresses
    }

    fn from_listeners(relay_listeners: &[RelayListener]) -> Self {
        let mut ready_addresses = Vec::with_capacity(relay_listeners.len());
        for listener in relay_listeners.iter().filter(|listener| listener.ready) {
            if !ready_addresses.contains(&listener.address) {
                ready_addresses.push(listener.address.clone());
            }
        }
        Self { ready_addresses }
    }
}

impl fmt::Debug for PublicRelayReadiness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicRelayReadiness")
            .field("ready_relay_count", &self.ready_relay_count())
            .finish()
    }
}

#[derive(NetworkBehaviour)]
struct SourceBehaviour {
    search: request_response::cbor::Behaviour<PublicSearchRequest, PublicSearchWireResponse>,
    browse: request_response::cbor::Behaviour<PublicBrowseRequest, PublicBrowseWireResponse>,
    relay: libp2p::relay::client::Behaviour,
    dcutr: libp2p::dcutr::Behaviour,
    autonat: libp2p::autonat::Behaviour,
    limits: libp2p::connection_limits::Behaviour,
}

enum Completion {
    Search {
        channel: ResponseChannel<PublicSearchWireResponse>,
        result: Result<PublicSearchDelivery, PublicSourceBackendError>,
    },
    Browse {
        channel: ResponseChannel<PublicBrowseWireResponse>,
        result: Result<PublicBrowseDelivery, PublicSourceBackendError>,
    },
}

struct RelayListener {
    address: Multiaddr,
    listener_id: Option<ListenerId>,
    ready: bool,
    retry_count: u32,
    retry_at: Option<TokioInstant>,
}

impl RelayListener {
    fn new(address: Multiaddr) -> Self {
        Self {
            address,
            listener_id: None,
            ready: false,
            retry_count: 0,
            retry_at: Some(TokioInstant::now()),
        }
    }

    fn schedule_retry(&mut self) -> Duration {
        self.listener_id = None;
        self.ready = false;
        self.retry_count = self.retry_count.saturating_add(1);
        let delay = relay_retry_delay(self.retry_count);
        self.retry_at = Some(TokioInstant::now() + delay);
        delay
    }

    fn mark_ready(&mut self) {
        self.ready = true;
        self.retry_count = 0;
        self.retry_at = None;
    }
}

struct RelayReadinessPublisher {
    sender: Option<watch::Sender<PublicRelayReadiness>>,
}

impl RelayReadinessPublisher {
    fn new(sender: Option<watch::Sender<PublicRelayReadiness>>) -> Self {
        let publisher = Self { sender };
        publisher.clear();
        publisher
    }

    fn publish(&self, relay_listeners: &[RelayListener]) {
        let next = PublicRelayReadiness::from_listeners(relay_listeners);
        if let Some(sender) = &self.sender {
            sender.send_if_modified(|current| {
                if *current == next {
                    false
                } else {
                    *current = next;
                    true
                }
            });
        }
    }

    fn clear(&self) {
        if let Some(sender) = &self.sender {
            sender.send_if_modified(|current| {
                if current.ready_addresses.is_empty() {
                    false
                } else {
                    current.ready_addresses.clear();
                    true
                }
            });
        }
    }
}

impl Drop for RelayReadinessPublisher {
    fn drop(&mut self) {
        self.clear();
    }
}

pub async fn run_public_source_server(
    identity: NodeIdentity,
    config: PublicSourceServerConfig,
    backend: Arc<dyn PublicSourceBackend>,
    cancellation: CancellationToken,
) -> Result<(), NetworkError> {
    let PublicSourceServerConfig {
        listen_addresses,
        relay_addresses,
        request_timeout,
        relay_readiness,
    } = config;
    let readiness_publisher = RelayReadinessPublisher::new(relay_readiness);
    if listen_addresses.is_empty() {
        return Err(NetworkError::Listen(
            "no public source listen address".to_owned(),
        ));
    }
    let mut retry_count = 0_u32;
    let (mut swarm, mut direct_listeners) = loop {
        let mut swarm = public_source_swarm(&identity, request_timeout)?;
        let mut listeners = Vec::with_capacity(listen_addresses.len());
        let mut retry = false;
        for address in &listen_addresses {
            match swarm.listen_on(address.clone()) {
                Ok(listener) => listeners.push(listener),
                Err(_) if listener_address_is_in_use(address) => {
                    retry = true;
                    break;
                }
                Err(_) => {
                    return Err(NetworkError::Listen(
                        "public source listener configuration is invalid".to_owned(),
                    ));
                }
            }
        }
        if !retry {
            break (swarm, listeners);
        }
        drop(swarm);
        retry_count = retry_count.saturating_add(1);
        if retry_count == 1 || retry_count.is_multiple_of(20) {
            tracing::warn!(
                retry_count,
                error_kind = "public_source_listen_retry",
                "public source listeners are temporarily unavailable"
            );
        }
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Ok(()),
            () = tokio::time::sleep(PUBLIC_LISTEN_RETRY) => {}
        }
    };
    let mut relay_listeners = relay_addresses
        .into_iter()
        .map(RelayListener::new)
        .collect::<Vec<_>>();
    readiness_publisher.publish(&relay_listeners);
    let limiter = PeerRateLimiter::new(60, Duration::from_secs(60));
    let mut tasks = JoinSet::new();
    if let Err(error) = retry_due_relay_listeners(&mut swarm, &mut relay_listeners) {
        prepare_public_source_shutdown(
            &mut swarm,
            &mut direct_listeners,
            &mut relay_listeners,
            &mut tasks,
        )
        .await;
        drop(swarm);
        await_listener_release(&listen_addresses).await;
        return Err(error);
    }
    loop {
        let relay_retry_at = relay_listeners
            .iter()
            .filter_map(|listener| listener.retry_at)
            .min();
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                prepare_public_source_shutdown(
                    &mut swarm,
                    &mut direct_listeners,
                    &mut relay_listeners,
                    &mut tasks,
                )
                .await;
                drop(swarm);
                await_listener_release(&listen_addresses).await;
                return Ok(());
            }
            () = wait_for_relay_retry(relay_retry_at) => {
                if let Err(error) =
                    retry_due_relay_listeners(&mut swarm, &mut relay_listeners)
                {
                    prepare_public_source_shutdown(
                        &mut swarm,
                        &mut direct_listeners,
                        &mut relay_listeners,
                        &mut tasks,
                    )
                    .await;
                    drop(swarm);
                    await_listener_release(&listen_addresses).await;
                    return Err(error);
                }
            }
            completion = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Ok(completion)) = completion {
                    send_completion(swarm.behaviour_mut(), completion);
                }
            }
            event = futures::StreamExt::select_next_some(&mut swarm) => {
                match event {
                    SwarmEvent::Behaviour(SourceBehaviourEvent::Search(event)) => {
                        if let request_response::Event::Message { peer, message, .. } = event
                            && let request_response::Message::Request { request, channel, .. } = message
                        {
                            if !limiter.check(peer) || tasks.len() >= PUBLIC_INBOUND_TASKS {
                                let _ = swarm.behaviour_mut().search.send_response(
                                    channel,
                                    PublicSearchWireResponse::Rejected(PublicSourceRejection::Busy),
                                );
                            } else {
                                let backend = Arc::clone(&backend);
                                tasks.spawn(async move {
                                    Completion::Search {
                                        channel,
                                        result: backend.search(request).await,
                                    }
                                });
                            }
                        }
                    }
                    SwarmEvent::Behaviour(SourceBehaviourEvent::Browse(event)) => {
                        if let request_response::Event::Message { peer, message, .. } = event
                            && let request_response::Message::Request { request, channel, .. } = message
                        {
                            if !limiter.check(peer) || tasks.len() >= PUBLIC_INBOUND_TASKS {
                                let _ = swarm.behaviour_mut().browse.send_response(
                                    channel,
                                    PublicBrowseWireResponse::Rejected(PublicSourceRejection::Busy),
                                );
                            } else {
                                let backend = Arc::clone(&backend);
                                tasks.spawn(async move {
                                    Completion::Browse {
                                        channel,
                                        result: backend.browse(request).await,
                                    }
                                });
                            }
                        }
                    }
                    SwarmEvent::NewListenAddr { listener_id, .. } => {
                        if let Some(listener) = relay_listeners
                            .iter_mut()
                            .find(|listener| listener.listener_id == Some(listener_id))
                        {
                            listener.mark_ready();
                            readiness_publisher.publish(&relay_listeners);
                            tracing::info!(
                                ready_relay_count = ready_relay_count(&relay_listeners),
                                configured_relay_count = relay_listeners.len(),
                                "public source relay reservation is ready"
                            );
                        }
                    }
                    SwarmEvent::ListenerError { listener_id, .. } => {
                        if relay_listeners
                            .iter()
                            .any(|listener| listener.listener_id == Some(listener_id))
                        {
                            tracing::warn!(
                                error_kind = "public_source_relay_listener_error",
                                ready_relay_count = ready_relay_count(&relay_listeners),
                                configured_relay_count = relay_listeners.len(),
                                "public source relay listener reported an error"
                            );
                        }
                    }
                    SwarmEvent::ListenerClosed {
                        listener_id,
                        reason,
                        ..
                    } => {
                        if let Some(index) = relay_listeners
                            .iter()
                            .position(|listener| listener.listener_id == Some(listener_id))
                        {
                            let delay = relay_listeners[index].schedule_retry();
                            readiness_publisher.publish(&relay_listeners);
                            tracing::warn!(
                                error_kind = if reason.is_err() {
                                    "public_source_relay_reservation_failed"
                                } else {
                                    "public_source_relay_reservation_closed"
                                },
                                retry_delay_ms = duration_millis(delay),
                                ready_relay_count = ready_relay_count(&relay_listeners),
                                configured_relay_count = relay_listeners.len(),
                                "public source relay reservation will be retried"
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn relay_retry_delay(retry_count: u32) -> Duration {
    let factor = 1_u32 << retry_count.saturating_sub(1).min(6);
    PUBLIC_LISTEN_RETRY
        .saturating_mul(factor)
        .min(PUBLIC_RELAY_RETRY_MAX)
}

async fn wait_for_relay_retry(retry_at: Option<TokioInstant>) {
    match retry_at {
        Some(retry_at) => tokio::time::sleep_until(retry_at).await,
        None => future::pending().await,
    }
}

fn retry_due_relay_listeners(
    swarm: &mut libp2p::Swarm<SourceBehaviour>,
    relay_listeners: &mut [RelayListener],
) -> Result<(), NetworkError> {
    let now = TokioInstant::now();
    for listener in relay_listeners {
        if listener.listener_id.is_some() || listener.retry_at.is_none_or(|retry_at| retry_at > now)
        {
            continue;
        }
        match swarm.listen_on(listener.address.clone()) {
            Ok(listener_id) => {
                listener.listener_id = Some(listener_id);
                listener.retry_at = None;
            }
            Err(_) => {
                return Err(NetworkError::Listen(
                    "public source relay listener configuration is invalid".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn ready_relay_count(relay_listeners: &[RelayListener]) -> usize {
    relay_listeners
        .iter()
        .filter(|listener| listener.ready)
        .count()
}

async fn prepare_public_source_shutdown(
    swarm: &mut libp2p::Swarm<SourceBehaviour>,
    direct_listeners: &mut Vec<ListenerId>,
    relay_listeners: &mut [RelayListener],
    tasks: &mut JoinSet<Completion>,
) {
    let relay_listener_ids = relay_listeners
        .iter_mut()
        .filter_map(|listener| listener.listener_id.take());
    let pending_listeners = direct_listeners
        .drain(..)
        .chain(relay_listener_ids)
        .filter(|listener| swarm.remove_listener(*listener))
        .collect::<HashSet<_>>();
    let connected_peers = swarm.connected_peers().copied().collect::<Vec<_>>();
    for peer in connected_peers {
        let _ = swarm.disconnect_peer_id(peer);
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    await_swarm_shutdown(swarm, pending_listeners).await;
}

async fn await_swarm_shutdown(
    swarm: &mut libp2p::Swarm<SourceBehaviour>,
    mut pending_listeners: HashSet<ListenerId>,
) {
    if pending_listeners.is_empty() && swarm.connected_peers().next().is_none() {
        return;
    }
    let shutdown = async {
        while !pending_listeners.is_empty() || swarm.connected_peers().next().is_some() {
            if let SwarmEvent::ListenerClosed { listener_id, .. } =
                futures::StreamExt::select_next_some(&mut *swarm).await
            {
                pending_listeners.remove(&listener_id);
            }
        }
    };
    if tokio::time::timeout(PUBLIC_LISTENER_SHUTDOWN_TIMEOUT, shutdown)
        .await
        .is_err()
    {
        tracing::warn!(
            pending_listener_count = pending_listeners.len(),
            pending_peer_count = swarm.connected_peers().count(),
            error_kind = "public_source_shutdown_timeout",
            "public source transport did not close before the shutdown deadline"
        );
    }
}

async fn await_listener_release(addresses: &[Multiaddr]) {
    let release = async {
        loop {
            if addresses.iter().all(listener_address_is_available) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    if tokio::time::timeout(PUBLIC_LISTENER_RELEASE_TIMEOUT, release)
        .await
        .is_err()
    {
        tracing::warn!(
            pending_listener_count = addresses
                .iter()
                .filter(|address| !listener_address_is_available(address))
                .count(),
            error_kind = "public_source_listener_release_timeout",
            "public source listener sockets remain temporarily unavailable"
        );
    }
}

fn public_source_swarm(
    identity: &NodeIdentity,
    request_timeout: Duration,
) -> Result<libp2p::Swarm<SourceBehaviour>, NetworkError> {
    let search = public_behaviour(PUBLIC_SEARCH_PROTOCOL, request_timeout)?;
    let browse = public_behaviour(PUBLIC_BROWSE_PROTOCOL, request_timeout)?;
    let local_peer = identity.peer_id();
    let swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .with_quic()
        .with_dns()
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .with_relay_client(libp2p::noise::Config::new, libp2p::yamux::Config::default)
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .with_behaviour(move |_, relay| SourceBehaviour {
            search,
            browse,
            relay,
            dcutr: libp2p::dcutr::Behaviour::new(local_peer),
            autonat: libp2p::autonat::Behaviour::new(
                local_peer,
                libp2p::autonat::Config::default(),
            ),
            limits: libp2p::connection_limits::Behaviour::new(
                libp2p::connection_limits::ConnectionLimits::default()
                    .with_max_pending_incoming(Some(32))
                    .with_max_pending_outgoing(Some(32))
                    .with_max_established_incoming(Some(64))
                    .with_max_established_outgoing(Some(32))
                    .with_max_established(Some(96))
                    .with_max_established_per_peer(Some(4)),
            ),
        })
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .build();
    Ok(swarm)
}

fn listener_address_is_in_use(address: &Multiaddr) -> bool {
    matches!(
        probe_listener_address(address),
        Some(Err(error)) if error.kind() == std::io::ErrorKind::AddrInUse
    )
}

fn listener_address_is_available(address: &Multiaddr) -> bool {
    matches!(probe_listener_address(address), Some(Ok(())))
}

fn probe_listener_address(address: &Multiaddr) -> Option<std::io::Result<()>> {
    use libp2p::multiaddr::Protocol;

    let protocols = address.iter().collect::<Vec<_>>();
    let bind = match protocols.as_slice() {
        [Protocol::Ip4(ip), Protocol::Tcp(port)] => {
            std::net::TcpListener::bind(std::net::SocketAddrV4::new(*ip, *port)).map(drop)
        }
        [Protocol::Ip6(ip), Protocol::Tcp(port)] => {
            std::net::TcpListener::bind(std::net::SocketAddrV6::new(*ip, *port, 0, 0)).map(drop)
        }
        [Protocol::Ip4(ip), Protocol::Udp(port), Protocol::QuicV1] => {
            std::net::UdpSocket::bind(std::net::SocketAddrV4::new(*ip, *port)).map(drop)
        }
        [Protocol::Ip6(ip), Protocol::Udp(port), Protocol::QuicV1] => {
            std::net::UdpSocket::bind(std::net::SocketAddrV6::new(*ip, *port, 0, 0)).map(drop)
        }
        _ => return None,
    };
    Some(bind)
}

fn public_behaviour<Request, Response>(
    protocol: &'static str,
    timeout: Duration,
) -> Result<request_response::cbor::Behaviour<Request, Response>, NetworkError>
where
    Request: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    Response: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
{
    let protocol = StreamProtocol::try_from_owned(protocol.to_owned())
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    let codec = request_response::cbor::codec::Codec::<Request, Response>::default()
        .set_request_size_maximum(PUBLIC_REQUEST_BYTES)
        .set_response_size_maximum(PUBLIC_RESPONSE_BYTES);
    Ok(request_response::Behaviour::with_codec(
        codec,
        [(protocol, ProtocolSupport::Full)],
        request_response::Config::default()
            .with_request_timeout(timeout)
            .with_max_concurrent_streams(PUBLIC_CONCURRENT_STREAMS),
    ))
}

fn send_completion(behaviour: &mut SourceBehaviour, completion: Completion) {
    match completion {
        Completion::Search { channel, result } => {
            let response = match result {
                Ok(delivery) => PublicSearchWireResponse::Success(delivery.response),
                Err(error) => PublicSearchWireResponse::Rejected(error.rejection()),
            };
            let _ = behaviour.search.send_response(channel, response);
        }
        Completion::Browse { channel, result } => {
            let response = match result {
                Ok(delivery) => PublicBrowseWireResponse::Success(delivery.page),
                Err(error) => PublicBrowseWireResponse::Rejected(error.rejection()),
            };
            let _ = behaviour.browse.send_response(channel, response);
        }
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};

    use airwiki_types::{PublicBrowseRequest, PublicSearchRequest};

    use super::*;
    use crate::MemorySecretStore;

    struct RejectingBackend;

    #[async_trait]
    impl PublicSourceBackend for RejectingBackend {
        async fn search(
            &self,
            _request: PublicSearchRequest,
        ) -> Result<PublicSearchDelivery, PublicSourceBackendError> {
            Err(PublicSourceBackendError::Unavailable)
        }

        async fn browse(
            &self,
            _request: PublicBrowseRequest,
        ) -> Result<PublicBrowseDelivery, PublicSourceBackendError> {
            Err(PublicSourceBackendError::Unavailable)
        }
    }

    #[test]
    fn relay_retry_backoff_starts_at_250_ms_and_caps_at_10_seconds() {
        let expected = [
            Duration::from_millis(250),
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(10),
            Duration::from_secs(10),
        ];

        for (retry_count, expected_delay) in (1_u32..).zip(expected) {
            assert_eq!(relay_retry_delay(retry_count), expected_delay);
        }
        assert_eq!(relay_retry_delay(u32::MAX), Duration::from_secs(10));
    }

    #[test]
    fn relay_readiness_publishes_zero_to_one_to_two_ready_reservations() {
        let (sender, mut receiver) = watch::channel(PublicRelayReadiness::default());
        let publisher = RelayReadinessPublisher::new(Some(sender));
        let mut listeners = vec![
            RelayListener::new("/memory/1".parse().expect("parse first relay address")),
            RelayListener::new("/memory/2".parse().expect("parse second relay address")),
        ];

        assert_eq!(receiver.borrow().ready_relay_count(), 0);

        listeners[0].mark_ready();
        publisher.publish(&listeners);
        assert!(receiver.has_changed().expect("read readiness channel"));
        assert_eq!(receiver.borrow_and_update().ready_relay_count(), 1);

        listeners[1].mark_ready();
        publisher.publish(&listeners);
        assert!(receiver.has_changed().expect("read readiness channel"));
        assert_eq!(receiver.borrow_and_update().ready_relay_count(), 2);
    }

    #[test]
    fn relay_readiness_publishes_two_to_one_to_zero_ready_reservations() {
        let (sender, mut receiver) = watch::channel(PublicRelayReadiness::default());
        let publisher = RelayReadinessPublisher::new(Some(sender));
        let mut listeners = vec![
            RelayListener::new("/memory/1".parse().expect("parse first relay address")),
            RelayListener::new("/memory/2".parse().expect("parse second relay address")),
        ];
        listeners.iter_mut().for_each(RelayListener::mark_ready);
        publisher.publish(&listeners);
        receiver.borrow_and_update();

        listeners[0].schedule_retry();
        publisher.publish(&listeners);
        assert!(receiver.has_changed().expect("read readiness channel"));
        assert_eq!(receiver.borrow_and_update().ready_relay_count(), 1);

        listeners[1].schedule_retry();
        publisher.publish(&listeners);
        assert!(receiver.has_changed().expect("read readiness channel"));
        assert_eq!(receiver.borrow_and_update().ready_relay_count(), 0);
    }

    #[test]
    fn dropping_relay_readiness_publisher_clears_the_ready_snapshot() {
        let (sender, mut receiver) = watch::channel(PublicRelayReadiness::default());
        let mut listeners = vec![RelayListener::new(
            "/memory/1".parse().expect("parse relay address"),
        )];
        listeners[0].mark_ready();
        {
            let publisher = RelayReadinessPublisher::new(Some(sender));
            publisher.publish(&listeners);
            receiver.borrow_and_update();
        }

        assert_eq!(
            (
                receiver.has_changed().is_err(),
                receiver.borrow().ready_relay_count(),
            ),
            (true, 0)
        );
    }

    #[tokio::test]
    async fn invalid_relay_listener_configuration_fails_closed() {
        let reservation = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("reserve an ephemeral UDP port");
        let port = reservation.local_addr().expect("read reserved port").port();
        drop(reservation);

        let identity = NodeIdentity::load_or_create(&MemorySecretStore::default())
            .expect("create test identity");
        let listen_address = format!("/ip4/127.0.0.1/udp/{port}/quic-v1")
            .parse()
            .expect("parse direct listen address");
        let mut config = PublicSourceServerConfig::new(vec![listen_address]);
        config.relay_addresses = vec![
            "/memory/1"
                .parse()
                .expect("parse unsupported relay address"),
        ];

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            run_public_source_server(
                identity,
                config,
                Arc::new(RejectingBackend),
                CancellationToken::new(),
            ),
        )
        .await
        .expect("invalid relay configuration should fail without retrying");

        assert!(matches!(
            result,
            Err(NetworkError::Listen(message))
                if message == "public source relay listener configuration is invalid"
        ));
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
            .expect("failure should release the direct listener");
    }

    #[tokio::test]
    async fn cancellation_releases_quic_listener() {
        let reservation = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("reserve an ephemeral UDP port");
        let port = reservation.local_addr().expect("read reserved port").port();
        drop(reservation);

        let identity = NodeIdentity::load_or_create(&MemorySecretStore::default())
            .expect("create test identity");
        let listen_address = format!("/ip4/127.0.0.1/udp/{port}/quic-v1")
            .parse()
            .expect("parse test listen address");
        let cancellation = CancellationToken::new();
        let server_cancellation = cancellation.clone();
        let server = tokio::spawn(run_public_source_server(
            identity,
            PublicSourceServerConfig::new(vec![listen_address]),
            Arc::new(RejectingBackend),
            server_cancellation,
        ));

        let socket_address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match UdpSocket::bind(socket_address) {
                    Ok(probe) => {
                        drop(probe);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => break,
                    Err(error) => panic!("unexpected UDP bind failure: {error}"),
                }
            }
        })
        .await
        .expect("public QUIC listener should bind");

        cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("public source server should stop promptly")
            .expect("public source task should not panic")
            .expect("public source server should stop cleanly");

        UdpSocket::bind(socket_address).expect("cancellation should release the QUIC listener");
    }

    #[tokio::test]
    async fn cancellation_releases_tcp_and_quic_listeners() {
        let tcp_reservation = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("reserve an ephemeral TCP port");
        let tcp_port = tcp_reservation
            .local_addr()
            .expect("read reserved TCP port")
            .port();
        let udp_reservation = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("reserve an ephemeral UDP port");
        let udp_port = udp_reservation
            .local_addr()
            .expect("read reserved UDP port")
            .port();
        drop((tcp_reservation, udp_reservation));

        let identity = NodeIdentity::load_or_create(&MemorySecretStore::default())
            .expect("create test identity");
        let listen_addresses = vec![
            format!("/ip4/127.0.0.1/udp/{udp_port}/quic-v1")
                .parse()
                .expect("parse QUIC listen address"),
            format!("/ip4/127.0.0.1/tcp/{tcp_port}")
                .parse()
                .expect("parse TCP listen address"),
        ];
        let cancellation = CancellationToken::new();
        let server = tokio::spawn(run_public_source_server(
            identity,
            PublicSourceServerConfig::new(listen_addresses),
            Arc::new(RejectingBackend),
            cancellation.clone(),
        ));

        let tcp_address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, tcp_port);
        let udp_address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, udp_port);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let tcp_busy = TcpListener::bind(tcp_address).is_err();
                let udp_busy = UdpSocket::bind(udp_address).is_err();
                if tcp_busy && udp_busy {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("public TCP and QUIC listeners should bind");

        cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("public source server should stop promptly")
            .expect("public source task should not panic")
            .expect("public source server should stop cleanly");

        TcpListener::bind(tcp_address)
            .expect("cancellation should release the public TCP listener");
        UdpSocket::bind(udp_address).expect("cancellation should release the public QUIC listener");
    }

    #[tokio::test]
    async fn busy_tcp_and_quic_ports_are_retried_until_available() {
        let tcp_reservation = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("reserve an ephemeral TCP port");
        let tcp_port = tcp_reservation
            .local_addr()
            .expect("read reserved TCP port")
            .port();
        let udp_reservation = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("reserve an ephemeral UDP port");
        let udp_port = udp_reservation
            .local_addr()
            .expect("read reserved UDP port")
            .port();

        let identity = NodeIdentity::load_or_create(&MemorySecretStore::default())
            .expect("create test identity");
        let listen_addresses = vec![
            format!("/ip4/127.0.0.1/tcp/{tcp_port}")
                .parse()
                .expect("parse TCP listen address"),
            format!("/ip4/127.0.0.1/udp/{udp_port}/quic-v1")
                .parse()
                .expect("parse QUIC listen address"),
        ];

        let cancellation = CancellationToken::new();
        let server = tokio::spawn(run_public_source_server(
            identity,
            PublicSourceServerConfig::new(listen_addresses),
            Arc::new(RejectingBackend),
            cancellation.clone(),
        ));

        tokio::time::sleep(Duration::from_millis(100)).await;
        if server.is_finished() {
            panic!(
                "public source server should wait for busy listeners: {:?}",
                server.await
            );
        }
        drop((tcp_reservation, udp_reservation));

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let tcp_busy =
                    TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, tcp_port)).is_err();
                let udp_busy =
                    UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, udp_port)).is_err();
                if tcp_busy && udp_busy {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("public source server should claim released listeners");

        cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("public source server should stop promptly")
            .expect("public source task should not panic")
            .expect("public source server should stop cleanly");
    }

    #[tokio::test]
    async fn unsupported_listener_returns_an_error_without_retrying() {
        let identity = NodeIdentity::load_or_create(&MemorySecretStore::default())
            .expect("create test identity");
        let unsupported = "/memory/1".parse().expect("parse unsupported address");

        let result = tokio::time::timeout(
            Duration::from_secs(2),
            run_public_source_server(
                identity,
                PublicSourceServerConfig::new(vec![unsupported]),
                Arc::new(RejectingBackend),
                CancellationToken::new(),
            ),
        )
        .await
        .expect("unsupported listener should fail without retrying");

        assert!(matches!(result, Err(NetworkError::Listen(_))));
    }
}
