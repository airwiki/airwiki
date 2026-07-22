use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;

use airwiki_federation_index::{CatalogBackend, CatalogStore};
use airwiki_network::{
    FileSecretStore, Multiaddr, NodeIdentity, PublicCatalogServerConfig, run_public_catalog_server,
};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let mut args = env::args_os().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!(
            "usage: airwiki-federation-index <database-path> [--print-peer-id | listen-multiaddr ...]"
        );
        return ExitCode::FAILURE;
    };
    let remaining_args = args.collect::<Vec<_>>();
    let print_peer_id = remaining_args.len() == 1 && remaining_args[0] == "--print-peer-id";
    let mut listen_addresses = Vec::new();
    for address in remaining_args {
        if address == "--print-peer-id" {
            eprintln!("--print-peer-id cannot be combined with listen addresses");
            return ExitCode::FAILURE;
        }
        let Ok(address) = address.into_string() else {
            eprintln!("listen multiaddress is not valid UTF-8");
            return ExitCode::FAILURE;
        };
        let Ok(address) = Multiaddr::from_str(&address) else {
            eprintln!("listen multiaddress is invalid");
            return ExitCode::FAILURE;
        };
        listen_addresses.push(address);
    }
    if listen_addresses.is_empty() {
        for address in ["/ip4/0.0.0.0/udp/42042/quic-v1", "/ip4/0.0.0.0/tcp/42042"] {
            match Multiaddr::from_str(address) {
                Ok(address) => listen_addresses.push(address),
                Err(_) => return ExitCode::FAILURE,
            }
        }
    }
    let store = match CatalogStore::open(&path) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let secrets = FileSecretStore::new(path.with_extension("secrets"));
    let identity = match NodeIdentity::load_or_create_at(&secrets, "federation-index-ed25519-v1") {
        Ok(identity) => identity,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if print_peer_id {
        println!("{}", identity.peer_id());
        return ExitCode::SUCCESS;
    }
    let cancellation = CancellationToken::new();
    let server = run_public_catalog_server(
        identity,
        PublicCatalogServerConfig::new(listen_addresses),
        Arc::new(CatalogBackend::new(store)),
        cancellation.clone(),
    );
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        signal = tokio::signal::ctrl_c() => {
            if signal.is_err() {
                return ExitCode::FAILURE;
            }
            cancellation.cancel();
            match server.await {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
