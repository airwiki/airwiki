use std::sync::Arc;
use std::time::Duration;

use airwiki_types::{
    PUBLIC_CATALOG_PROTOCOL, PublicCatalogQuery, SignedPublicCollectionManifest,
    SignedPublicCollectionTombstone,
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

const CATALOG_REQUEST_BYTES: u64 = 128 * 1024;
const CATALOG_RESPONSE_BYTES: u64 = 512 * 1024;
const CATALOG_CONCURRENT_STREAMS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CatalogWireRequest {
    Register(SignedPublicCollectionManifest),
    Withdraw(SignedPublicCollectionTombstone),
    Query(PublicCatalogQuery),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CatalogWireResponse {
    Accepted,
    Results(Vec<SignedPublicCollectionManifest>),
    Rejected(CatalogRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogRejection {
    Invalid,
    Stale,
    Busy,
    Internal,
}

#[derive(Debug, Error)]
pub enum PublicCatalogBackendError {
    #[error("catalog update is invalid")]
    Invalid,
    #[error("catalog update is stale")]
    Stale,
    #[error("catalog is busy")]
    Busy,
    #[error("catalog operation failed")]
    Internal,
}

impl PublicCatalogBackendError {
    const fn rejection(&self) -> CatalogRejection {
        match self {
            Self::Invalid => CatalogRejection::Invalid,
            Self::Stale => CatalogRejection::Stale,
            Self::Busy => CatalogRejection::Busy,
            Self::Internal => CatalogRejection::Internal,
        }
    }
}

#[async_trait]
pub trait PublicCatalogBackend: Send + Sync + 'static {
    async fn register(
        &self,
        manifest: SignedPublicCollectionManifest,
    ) -> Result<(), PublicCatalogBackendError>;

    async fn withdraw(
        &self,
        tombstone: SignedPublicCollectionTombstone,
    ) -> Result<(), PublicCatalogBackendError>;

    async fn query(
        &self,
        query: PublicCatalogQuery,
    ) -> Result<Vec<SignedPublicCollectionManifest>, PublicCatalogBackendError>;
}

#[derive(Debug, Clone)]
pub struct PublicCatalogServerConfig {
    pub listen_addresses: Vec<Multiaddr>,
    pub external_addresses: Vec<Multiaddr>,
    pub request_timeout: Duration,
}

#[derive(NetworkBehaviour)]
struct CatalogBehaviour {
    catalog: request_response::Behaviour<
        request_response::cbor::codec::Codec<CatalogWireRequest, CatalogWireResponse>,
    >,
    relay: libp2p::relay::Behaviour,
    limits: libp2p::connection_limits::Behaviour,
}

struct CatalogCompletion {
    channel: ResponseChannel<CatalogWireResponse>,
    response: CatalogWireResponse,
}

impl PublicCatalogServerConfig {
    pub fn new(listen_addresses: Vec<Multiaddr>) -> Self {
        Self {
            listen_addresses,
            external_addresses: Vec::new(),
            request_timeout: Duration::from_millis(800),
        }
    }

    pub fn with_external_addresses(mut self, external_addresses: Vec<Multiaddr>) -> Self {
        self.external_addresses = external_addresses;
        self
    }
}

pub async fn run_public_catalog_server(
    identity: NodeIdentity,
    config: PublicCatalogServerConfig,
    backend: Arc<dyn PublicCatalogBackend>,
    cancellation: CancellationToken,
) -> Result<(), NetworkError> {
    if config.listen_addresses.is_empty() {
        return Err(NetworkError::Listen(
            "no public catalog listen address".to_owned(),
        ));
    }
    let protocol = StreamProtocol::try_from_owned(PUBLIC_CATALOG_PROTOCOL.to_owned())
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    let codec =
        request_response::cbor::codec::Codec::<CatalogWireRequest, CatalogWireResponse>::default()
            .set_request_size_maximum(CATALOG_REQUEST_BYTES)
            .set_response_size_maximum(CATALOG_RESPONSE_BYTES);
    let catalog = request_response::Behaviour::with_codec(
        codec,
        [(protocol, ProtocolSupport::Full)],
        request_response::Config::default()
            .with_request_timeout(config.request_timeout)
            .with_max_concurrent_streams(CATALOG_CONCURRENT_STREAMS),
    );
    let behaviour = CatalogBehaviour {
        catalog,
        relay: libp2p::relay::Behaviour::new(identity.peer_id(), libp2p::relay::Config::default()),
        limits: libp2p::connection_limits::Behaviour::new(
            libp2p::connection_limits::ConnectionLimits::default()
                .with_max_pending_incoming(Some(128))
                .with_max_pending_outgoing(Some(64))
                .with_max_established_incoming(Some(384))
                .with_max_established_outgoing(Some(128))
                .with_max_established(Some(512))
                .with_max_established_per_peer(Some(4)),
        ),
    };
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
        .with_behaviour(|_| behaviour)
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .build();
    for address in config.listen_addresses {
        swarm
            .listen_on(address)
            .map_err(|error| NetworkError::Listen(error.to_string()))?;
    }
    for address in config.external_addresses {
        validate_public_relay_external_address(&address)?;
        swarm.add_external_address(address);
    }
    let limiter = PeerRateLimiter::new(120, Duration::from_secs(60));
    let mut tasks = JoinSet::<CatalogCompletion>::new();
    loop {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return Ok(());
            },
            completion = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Ok(completion)) = completion
                    && swarm
                        .behaviour_mut()
                        .catalog
                        .send_response(completion.channel, completion.response)
                        .is_err()
                {
                    tracing::debug!("public catalog response channel closed");
                }
            }
            event = futures::StreamExt::select_next_some(&mut swarm) => {
                if let SwarmEvent::Behaviour(CatalogBehaviourEvent::Catalog(
                    request_response::Event::Message { peer, message, .. }
                )) = event
                    && let request_response::Message::Request { request, channel, .. } = message
                {
                    if limiter.check(peer) && tasks.len() < CATALOG_CONCURRENT_STREAMS {
                        let backend = Arc::clone(&backend);
                        tasks.spawn(async move {
                            CatalogCompletion {
                                channel,
                                response: handle_request(backend.as_ref(), request).await,
                            }
                        });
                    } else {
                        let _ = swarm.behaviour_mut().catalog.send_response(
                            channel,
                            CatalogWireResponse::Rejected(CatalogRejection::Busy),
                        );
                    }
                }
            }
        }
    }
}

/// Rejects advertised relay routes that are malformed or not publicly routable.
pub fn validate_public_relay_external_address(address: &Multiaddr) -> Result<(), NetworkError> {
    use libp2p::multiaddr::Protocol;

    let protocols = address.iter().collect::<Vec<_>>();
    let valid = match protocols.as_slice() {
        [host, Protocol::Tcp(port)] if *port != 0 => relay_host_is_publicly_routable(host),
        [host, Protocol::Udp(port), Protocol::QuicV1] if *port != 0 => {
            relay_host_is_publicly_routable(host)
        }
        _ => false,
    };
    if !valid {
        return Err(NetworkError::Listen(
            "invalid public relay external address".to_owned(),
        ));
    }
    Ok(())
}

fn relay_host_is_publicly_routable(host: &libp2p::multiaddr::Protocol<'_>) -> bool {
    match host {
        libp2p::multiaddr::Protocol::Ip4(ip) => ipv4_is_publicly_routable(*ip),
        libp2p::multiaddr::Protocol::Ip6(ip) => ipv6_is_publicly_routable(*ip),
        libp2p::multiaddr::Protocol::Dns(_)
        | libp2p::multiaddr::Protocol::Dns4(_)
        | libp2p::multiaddr::Protocol::Dns6(_)
        | libp2p::multiaddr::Protocol::Dnsaddr(_) => true,
        _ => false,
    }
}

fn ipv4_is_publicly_routable(ip: std::net::Ipv4Addr) -> bool {
    let [first, second, third, fourth] = ip.octets();
    !(first == 0
        || ip.is_private()
        || (first == 100 && (64..=127).contains(&second))
        || ip.is_loopback()
        || ip.is_link_local()
        || (first == 192 && second == 0 && third == 0 && !matches!(fourth, 9 | 10))
        || ip.is_documentation()
        || (first == 198 && matches!(second, 18 | 19))
        || first >= 240
        || ip.is_broadcast()
        || ip.is_multicast())
}

fn ipv6_is_publicly_routable(ip: std::net::Ipv6Addr) -> bool {
    let segments = ip.segments();
    let is_global_unicast = segments[0] & 0xe000 == 0x2000;
    let is_documentation =
        matches!(segments, [0x2001, 0xdb8, ..]) || matches!(segments, [0x3fff, 0..=0x0fff, ..]);
    let is_special_2001 = matches!(segments, [0x2001, second, ..] if second < 0x0200)
        && !(u128::from_be_bytes(ip.octets()) == 0x2001_0001_0000_0000_0000_0000_0000_0001
            || u128::from_be_bytes(ip.octets()) == 0x2001_0001_0000_0000_0000_0000_0000_0002
            || matches!(segments, [0x2001, 3, ..])
            || matches!(segments, [0x2001, 4, 0x0112, ..])
            || matches!(segments, [0x2001, 0x20..=0x3f, ..]));
    is_global_unicast && !is_documentation && !is_special_2001 && !matches!(segments, [0x2002, ..])
}

async fn handle_request(
    backend: &dyn PublicCatalogBackend,
    request: CatalogWireRequest,
) -> CatalogWireResponse {
    let result = match request {
        CatalogWireRequest::Register(manifest) => backend.register(manifest).await.map(|()| None),
        CatalogWireRequest::Withdraw(tombstone) => backend.withdraw(tombstone).await.map(|()| None),
        CatalogWireRequest::Query(query) => backend.query(query).await.map(Some),
    };
    match result {
        Ok(Some(manifests)) => CatalogWireResponse::Results(manifests),
        Ok(None) => CatalogWireResponse::Accepted,
        Err(error) => CatalogWireResponse::Rejected(error.rejection()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_external_address_rejects_non_public_hosts() {
        let wildcard = "/ip4/0.0.0.0/tcp/42042".parse().unwrap();
        let loopback = "/ip4/127.0.0.1/tcp/42042".parse().unwrap();
        let private = "/ip4/192.168.1.10/tcp/42042".parse().unwrap();
        let documentation = "/ip6/2001:db8::10/udp/42042/quic-v1".parse().unwrap();

        assert!(validate_public_relay_external_address(&wildcard).is_err());
        assert!(validate_public_relay_external_address(&loopback).is_err());
        assert!(validate_public_relay_external_address(&private).is_err());
        assert!(validate_public_relay_external_address(&documentation).is_err());
    }

    #[test]
    fn relay_external_address_rejects_incomplete_or_extended_transports() {
        let quic_without_udp = "/dns4/relay.example.org/quic-v1".parse().unwrap();
        let tcp_with_peer = format!(
            "/dns4/relay.example.org/tcp/42042/p2p/{}",
            libp2p::PeerId::random()
        )
        .parse()
        .unwrap();

        assert!(validate_public_relay_external_address(&quic_without_udp).is_err());
        assert!(validate_public_relay_external_address(&tcp_with_peer).is_err());
    }

    #[test]
    fn relay_external_address_accepts_direct_tcp_and_quic_routes() {
        let tcp = "/dns4/relay.example.org/tcp/42042".parse().unwrap();
        let quic = "/ip6/2606:4700:4700::1111/udp/42042/quic-v1"
            .parse()
            .unwrap();

        assert!(validate_public_relay_external_address(&tcp).is_ok());
        assert!(validate_public_relay_external_address(&quic).is_ok());
    }
}
