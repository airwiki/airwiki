use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;

use airwiki_federation_index::{CatalogBackend, CatalogStore};
use airwiki_network::{
    FileSecretStore, Multiaddr, NodeIdentity, PublicCatalogServerConfig, run_public_catalog_server,
};
use tokio_util::sync::CancellationToken;

enum LaunchMode {
    PrintPeerId,
    Listen(Vec<Multiaddr>),
}

fn parse_launch_mode(args: Vec<OsString>) -> Result<LaunchMode, String> {
    if args.len() == 1 && args[0] == "--print-peer-id" {
        return Ok(LaunchMode::PrintPeerId);
    }

    let mut listen_addresses = Vec::new();
    for address in args {
        if address == "--print-peer-id" {
            return Err("--print-peer-id cannot be combined with listen addresses".to_owned());
        }
        let address = address
            .into_string()
            .map_err(|_| "listen multiaddress is not valid UTF-8".to_owned())?;
        let address = Multiaddr::from_str(&address)
            .map_err(|_| "listen multiaddress is invalid".to_owned())?;
        listen_addresses.push(address);
    }
    if listen_addresses.is_empty() {
        for address in ["/ip4/0.0.0.0/udp/42042/quic-v1", "/ip4/0.0.0.0/tcp/42042"] {
            let address = Multiaddr::from_str(address)
                .map_err(|_| "default listen multiaddress is invalid".to_owned())?;
            listen_addresses.push(address);
        }
    }
    Ok(LaunchMode::Listen(listen_addresses))
}

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
    let (print_peer_id, listen_addresses) = match parse_launch_mode(args.collect()) {
        Ok(LaunchMode::PrintPeerId) => (true, Vec::new()),
        Ok(LaunchMode::Listen(addresses)) => (false, addresses),
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_peer_id_mode_is_accepted_on_its_own() {
        let mode = parse_launch_mode(vec![OsString::from("--print-peer-id")]);

        assert!(matches!(mode, Ok(LaunchMode::PrintPeerId)));
    }

    #[test]
    fn print_peer_id_mode_rejects_listen_addresses() {
        let mode = parse_launch_mode(vec![
            OsString::from("--print-peer-id"),
            OsString::from("/ip4/127.0.0.1/tcp/42042"),
        ]);

        assert!(mode.is_err());
    }

    #[test]
    fn empty_arguments_use_the_private_default_listeners() {
        let mode = parse_launch_mode(Vec::new());

        let Ok(LaunchMode::Listen(addresses)) = mode else {
            panic!("expected listen mode");
        };
        assert_eq!(addresses.len(), 2);
    }
}
