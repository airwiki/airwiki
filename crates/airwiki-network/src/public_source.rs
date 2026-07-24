use std::collections::HashSet;
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
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::{NetworkError, NodeIdentity, PeerRateLimiter};

const PUBLIC_REQUEST_BYTES: u64 = 16 * 1024;
const PUBLIC_RESPONSE_BYTES: u64 = 256 * 1024;
const PUBLIC_CONCURRENT_STREAMS: usize = 64;
const PUBLIC_INBOUND_TASKS: usize = 32;
const PUBLIC_LISTEN_RETRY: Duration = Duration::from_millis(250);
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
}

impl PublicSourceServerConfig {
    pub fn new(listen_addresses: Vec<Multiaddr>) -> Self {
        Self {
            listen_addresses,
            relay_addresses: Vec::new(),
            request_timeout: Duration::from_millis(800),
        }
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

pub async fn run_public_source_server(
    identity: NodeIdentity,
    config: PublicSourceServerConfig,
    backend: Arc<dyn PublicSourceBackend>,
    cancellation: CancellationToken,
) -> Result<(), NetworkError> {
    if config.listen_addresses.is_empty() {
        return Err(NetworkError::Listen(
            "no public source listen address".to_owned(),
        ));
    }
    let request_timeout = config.request_timeout;
    let listen_addresses = config.listen_addresses;
    let addresses = listen_addresses
        .iter()
        .cloned()
        .chain(config.relay_addresses)
        .collect::<Vec<_>>();
    let mut retry_count = 0_u32;
    let (mut swarm, mut listeners) = loop {
        let mut swarm = public_source_swarm(&identity, request_timeout)?;
        let mut listeners = Vec::with_capacity(addresses.len());
        let mut retry = false;
        for address in &addresses {
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
    let limiter = PeerRateLimiter::new(60, Duration::from_secs(60));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                let pending_listeners = listeners
                    .drain(..)
                    .filter(|listener| swarm.remove_listener(*listener))
                    .collect::<HashSet<_>>();
                let connected_peers = swarm.connected_peers().copied().collect::<Vec<_>>();
                for peer in connected_peers {
                    let _ = swarm.disconnect_peer_id(peer);
                }
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                await_swarm_shutdown(&mut swarm, pending_listeners).await;
                drop(swarm);
                await_listener_release(&listen_addresses).await;
                return Ok(());
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
                                    Completion::Search { channel, result: backend.search(request).await }
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
                                    Completion::Browse { channel, result: backend.browse(request).await }
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
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
