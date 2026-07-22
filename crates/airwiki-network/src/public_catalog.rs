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
            request_timeout: Duration::from_millis(800),
        }
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
