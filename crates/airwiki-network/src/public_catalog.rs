use std::{collections::BTreeMap, sync::Arc, time::Duration};

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
const PUBLIC_RELAY_SUMMARY_INTERVAL: Duration = Duration::from_secs(15 * 60);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PublicRelayClass {
    ReservationAccepted,
    ReservationRenewed,
    ReservationAcceptFailed,
    ReservationDenied,
    ReservationDenyFailed,
    ReservationClosed,
    ReservationTimedOut,
    CircuitDenied,
    CircuitDenyFailed,
    CircuitOutboundConnectFailed,
    CircuitAcceptFailed,
    CircuitConnectionRefused,
    CircuitConnectionAborted,
    CircuitConnectionReset,
    CircuitNotConnected,
    CircuitBrokenPipe,
    CircuitTimedOut,
    CircuitWriteZero,
    CircuitUnexpectedEof,
    CircuitPermissionDenied,
    CircuitFailed,
}

impl PublicRelayClass {
    const fn name(self) -> &'static str {
        match self {
            Self::ReservationAccepted => "public_relay_reservation_accepted",
            Self::ReservationRenewed => "public_relay_reservation_renewed",
            Self::ReservationAcceptFailed => "public_relay_reservation_accept_failed",
            Self::ReservationDenied => "public_relay_reservation_denied",
            Self::ReservationDenyFailed => "public_relay_reservation_deny_failed",
            Self::ReservationClosed => "public_relay_reservation_closed",
            Self::ReservationTimedOut => "public_relay_reservation_timed_out",
            Self::CircuitDenied => "public_relay_circuit_denied",
            Self::CircuitDenyFailed => "public_relay_circuit_deny_failed",
            Self::CircuitOutboundConnectFailed => "public_relay_circuit_outbound_connect_failed",
            Self::CircuitAcceptFailed => "public_relay_circuit_accept_failed",
            Self::CircuitConnectionRefused => "public_relay_circuit_connection_refused",
            Self::CircuitConnectionAborted => "public_relay_circuit_connection_aborted",
            Self::CircuitConnectionReset => "public_relay_circuit_connection_reset",
            Self::CircuitNotConnected => "public_relay_circuit_not_connected",
            Self::CircuitBrokenPipe => "public_relay_circuit_broken_pipe",
            Self::CircuitTimedOut => "public_relay_circuit_timed_out",
            Self::CircuitWriteZero => "public_relay_circuit_write_zero",
            Self::CircuitUnexpectedEof => "public_relay_circuit_unexpected_eof",
            Self::CircuitPermissionDenied => "public_relay_circuit_permission_denied",
            Self::CircuitFailed => "public_relay_circuit_failed",
        }
    }
}

#[derive(Default)]
struct PublicRelayCounters {
    counts: BTreeMap<PublicRelayClass, u64>,
}

impl PublicRelayCounters {
    fn record(&mut self, event: &libp2p::relay::Event) {
        let Some(class) = classify_public_relay_event(event) else {
            return;
        };
        let count = self.counts.entry(class).or_default();
        *count = count.saturating_add(1);
    }

    fn take_snapshot(&mut self) -> BTreeMap<PublicRelayClass, u64> {
        std::mem::take(&mut self.counts)
    }
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
    let mut relay_counters = PublicRelayCounters::default();
    let mut relay_summary = tokio::time::interval_at(
        tokio::time::Instant::now() + PUBLIC_RELAY_SUMMARY_INTERVAL,
        PUBLIC_RELAY_SUMMARY_INTERVAL,
    );
    relay_summary.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                emit_public_relay_summary(relay_counters.take_snapshot());
                return Ok(());
            },
            _ = relay_summary.tick() => {
                emit_public_relay_summary(relay_counters.take_snapshot());
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
                match event {
                    SwarmEvent::Behaviour(CatalogBehaviourEvent::Catalog(
                        request_response::Event::Message {
                            peer,
                            message:
                                request_response::Message::Request {
                                    request, channel, ..
                                },
                            ..
                        }
                    )) => {
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
                    SwarmEvent::Behaviour(CatalogBehaviourEvent::Relay(event)) => {
                        relay_counters.record(&event);
                    }
                    _ => {}
                }
            }
        }
    }
}

#[expect(
    deprecated,
    reason = "the pinned relay exposes failure events required for sanitized diagnosis"
)]
fn classify_public_relay_event(event: &libp2p::relay::Event) -> Option<PublicRelayClass> {
    match event {
        libp2p::relay::Event::ReservationReqAccepted { renewed: false, .. } => {
            Some(PublicRelayClass::ReservationAccepted)
        }
        libp2p::relay::Event::ReservationReqAccepted { renewed: true, .. } => {
            Some(PublicRelayClass::ReservationRenewed)
        }
        libp2p::relay::Event::ReservationReqAcceptFailed { .. } => {
            Some(PublicRelayClass::ReservationAcceptFailed)
        }
        libp2p::relay::Event::ReservationReqDenied { .. } => {
            Some(PublicRelayClass::ReservationDenied)
        }
        libp2p::relay::Event::ReservationReqDenyFailed { .. } => {
            Some(PublicRelayClass::ReservationDenyFailed)
        }
        libp2p::relay::Event::ReservationClosed { .. } => Some(PublicRelayClass::ReservationClosed),
        libp2p::relay::Event::ReservationTimedOut { .. } => {
            Some(PublicRelayClass::ReservationTimedOut)
        }
        libp2p::relay::Event::CircuitReqDenied { .. } => Some(PublicRelayClass::CircuitDenied),
        libp2p::relay::Event::CircuitReqDenyFailed { .. } => {
            Some(PublicRelayClass::CircuitDenyFailed)
        }
        libp2p::relay::Event::CircuitReqAccepted { .. } => None,
        libp2p::relay::Event::CircuitReqOutboundConnectFailed { .. } => {
            Some(PublicRelayClass::CircuitOutboundConnectFailed)
        }
        libp2p::relay::Event::CircuitReqAcceptFailed { .. } => {
            Some(PublicRelayClass::CircuitAcceptFailed)
        }
        libp2p::relay::Event::CircuitClosed { error: None, .. } => None,
        libp2p::relay::Event::CircuitClosed {
            error: Some(error), ..
        } => Some(classify_public_relay_io_error(error)),
    }
}

fn classify_public_relay_io_error(error: &std::io::Error) -> PublicRelayClass {
    match error.kind() {
        std::io::ErrorKind::ConnectionRefused => PublicRelayClass::CircuitConnectionRefused,
        std::io::ErrorKind::ConnectionAborted => PublicRelayClass::CircuitConnectionAborted,
        std::io::ErrorKind::ConnectionReset => PublicRelayClass::CircuitConnectionReset,
        std::io::ErrorKind::NotConnected => PublicRelayClass::CircuitNotConnected,
        std::io::ErrorKind::BrokenPipe => PublicRelayClass::CircuitBrokenPipe,
        std::io::ErrorKind::TimedOut => PublicRelayClass::CircuitTimedOut,
        std::io::ErrorKind::WriteZero => PublicRelayClass::CircuitWriteZero,
        std::io::ErrorKind::UnexpectedEof => PublicRelayClass::CircuitUnexpectedEof,
        std::io::ErrorKind::PermissionDenied => PublicRelayClass::CircuitPermissionDenied,
        _ => PublicRelayClass::CircuitFailed,
    }
}

fn emit_public_relay_summary(snapshot: BTreeMap<PublicRelayClass, u64>) {
    for (class, count) in snapshot {
        tracing::info!(
            relay_class = class.name(),
            count,
            "public_relay_summary relay_class={} count={}",
            class.name(),
            count
        );
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
    fn relay_lifecycle_snapshot_is_bounded_saturating_and_reset() {
        let source = libp2p::PeerId::random();
        let mut counters = PublicRelayCounters::default();
        counters
            .counts
            .insert(PublicRelayClass::ReservationAccepted, u64::MAX);
        for event in [
            libp2p::relay::Event::ReservationReqAccepted {
                src_peer_id: source,
                renewed: false,
            },
            libp2p::relay::Event::ReservationReqDenied {
                src_peer_id: source,
                status: libp2p::relay::StatusCode::ReservationRefused,
            },
            libp2p::relay::Event::ReservationClosed {
                src_peer_id: source,
            },
        ] {
            counters.record(&event);
        }
        let snapshot = counters.take_snapshot();
        let second_snapshot = counters.take_snapshot();

        assert_eq!(
            (
                snapshot
                    .get(&PublicRelayClass::ReservationAccepted)
                    .copied(),
                snapshot.get(&PublicRelayClass::ReservationDenied).copied(),
                snapshot.get(&PublicRelayClass::ReservationClosed).copied(),
                second_snapshot.is_empty(),
            ),
            (Some(u64::MAX), Some(1), Some(1), true)
        );
    }

    #[test]
    fn relay_classes_are_fixed_and_do_not_include_reader_success_activity() {
        let source = libp2p::PeerId::random();
        let destination = libp2p::PeerId::random();
        assert_eq!(
            classify_public_relay_event(&libp2p::relay::Event::CircuitReqAccepted {
                src_peer_id: source,
                dst_peer_id: destination,
            }),
            None
        );
        assert_eq!(
            classify_public_relay_event(&libp2p::relay::Event::CircuitClosed {
                src_peer_id: source,
                dst_peer_id: destination,
                error: None,
            }),
            None
        );

        let names = [
            PublicRelayClass::ReservationAccepted,
            PublicRelayClass::ReservationRenewed,
            PublicRelayClass::ReservationAcceptFailed,
            PublicRelayClass::ReservationDenied,
            PublicRelayClass::ReservationDenyFailed,
            PublicRelayClass::ReservationClosed,
            PublicRelayClass::ReservationTimedOut,
            PublicRelayClass::CircuitDenied,
            PublicRelayClass::CircuitDenyFailed,
            PublicRelayClass::CircuitOutboundConnectFailed,
            PublicRelayClass::CircuitAcceptFailed,
            PublicRelayClass::CircuitConnectionRefused,
            PublicRelayClass::CircuitConnectionAborted,
            PublicRelayClass::CircuitConnectionReset,
            PublicRelayClass::CircuitNotConnected,
            PublicRelayClass::CircuitBrokenPipe,
            PublicRelayClass::CircuitTimedOut,
            PublicRelayClass::CircuitWriteZero,
            PublicRelayClass::CircuitUnexpectedEof,
            PublicRelayClass::CircuitPermissionDenied,
            PublicRelayClass::CircuitFailed,
        ]
        .map(PublicRelayClass::name);

        assert_eq!(
            names,
            [
                "public_relay_reservation_accepted",
                "public_relay_reservation_renewed",
                "public_relay_reservation_accept_failed",
                "public_relay_reservation_denied",
                "public_relay_reservation_deny_failed",
                "public_relay_reservation_closed",
                "public_relay_reservation_timed_out",
                "public_relay_circuit_denied",
                "public_relay_circuit_deny_failed",
                "public_relay_circuit_outbound_connect_failed",
                "public_relay_circuit_accept_failed",
                "public_relay_circuit_connection_refused",
                "public_relay_circuit_connection_aborted",
                "public_relay_circuit_connection_reset",
                "public_relay_circuit_not_connected",
                "public_relay_circuit_broken_pipe",
                "public_relay_circuit_timed_out",
                "public_relay_circuit_write_zero",
                "public_relay_circuit_unexpected_eof",
                "public_relay_circuit_permission_denied",
                "public_relay_circuit_failed",
            ]
        );
    }

    #[test]
    fn relay_circuit_io_failures_use_fixed_sanitized_classes() {
        let classes = [
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::NotConnected,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::WriteZero,
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidData,
        ]
        .map(|kind| classify_public_relay_io_error(&std::io::Error::from(kind)));

        assert_eq!(
            classes,
            [
                PublicRelayClass::CircuitConnectionRefused,
                PublicRelayClass::CircuitConnectionAborted,
                PublicRelayClass::CircuitConnectionReset,
                PublicRelayClass::CircuitNotConnected,
                PublicRelayClass::CircuitBrokenPipe,
                PublicRelayClass::CircuitTimedOut,
                PublicRelayClass::CircuitWriteZero,
                PublicRelayClass::CircuitUnexpectedEof,
                PublicRelayClass::CircuitPermissionDenied,
                PublicRelayClass::CircuitFailed,
            ]
        );
    }

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
