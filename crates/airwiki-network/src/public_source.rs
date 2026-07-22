use std::sync::Arc;
use std::time::Duration;

use airwiki_types::{
    DisclosureLease, PUBLIC_BROWSE_PROTOCOL, PUBLIC_SEARCH_PROTOCOL, PublicBrowsePage,
    PublicBrowseRequest, PublicSearchRequest, PublicSearchResponse,
};
use async_trait::async_trait;
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
    let search = public_behaviour(PUBLIC_SEARCH_PROTOCOL, config.request_timeout)?;
    let browse = public_behaviour(PUBLIC_BROWSE_PROTOCOL, config.request_timeout)?;
    let local_peer = identity.peer_id();
    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
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
    let mut listeners = Vec::new();
    for address in config.listen_addresses {
        listeners.push(
            swarm
                .listen_on(address)
                .map_err(|error| NetworkError::Listen(error.to_string()))?,
        );
    }
    for address in config.relay_addresses {
        listeners.push(
            swarm
                .listen_on(address)
                .map_err(|error| NetworkError::Listen(error.to_string()))?,
        );
    }
    let limiter = PeerRateLimiter::new(60, Duration::from_secs(60));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                for listener in listeners.drain(..) {
                    let _ = swarm.remove_listener(listener);
                }
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
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
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

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
}
