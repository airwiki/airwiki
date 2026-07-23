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
use tokio::signal;
use tokio_util::sync::CancellationToken;

enum LaunchMode {
    PrintPeerId,
    Listen {
        listen_addresses: Vec<Multiaddr>,
        external_addresses: Vec<Multiaddr>,
    },
}

fn parse_launch_mode(args: Vec<OsString>) -> Result<LaunchMode, String> {
    if args.len() == 1 && args[0] == "--print-peer-id" {
        return Ok(LaunchMode::PrintPeerId);
    }

    let mut listen_addresses = Vec::new();
    let mut external_addresses = Vec::new();
    let mut args = args.into_iter();
    while let Some(address) = args.next() {
        if address == "--print-peer-id" {
            return Err("--print-peer-id cannot be combined with listen addresses".to_owned());
        }
        if address == "--external-address" {
            let address = args
                .next()
                .ok_or_else(|| "--external-address requires a multiaddress".to_owned())?;
            external_addresses.push(parse_multiaddr(address, "external")?);
            continue;
        }
        listen_addresses.push(parse_multiaddr(address, "listen")?);
    }
    if listen_addresses.is_empty() {
        for address in ["/ip4/0.0.0.0/udp/42042/quic-v1", "/ip4/0.0.0.0/tcp/42042"] {
            let address = Multiaddr::from_str(address)
                .map_err(|_| "default listen multiaddress is invalid".to_owned())?;
            listen_addresses.push(address);
        }
    }
    Ok(LaunchMode::Listen {
        listen_addresses,
        external_addresses,
    })
}

fn parse_multiaddr(address: OsString, kind: &str) -> Result<Multiaddr, String> {
    let address = address
        .into_string()
        .map_err(|_| format!("{kind} multiaddress is not valid UTF-8"))?;
    Multiaddr::from_str(&address).map_err(|_| format!("{kind} multiaddress is invalid"))
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate = signal::unix::signal(signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        signal::ctrl_c().await
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| {
                    tracing_subscriber::EnvFilter::new(
                        "warn,airwiki_federation_index=info,airwiki_network=info,libp2p=off,libp2p_swarm=off",
                    )
                }),
        )
        .init();
    let mut args = env::args_os().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!(
            "usage: airwiki-federation-index <database-path> [--print-peer-id | [--external-address multiaddr]... [listen-multiaddr]...]"
        );
        return ExitCode::FAILURE;
    };
    let (print_peer_id, listen_addresses, external_addresses) =
        match parse_launch_mode(args.collect()) {
            Ok(LaunchMode::PrintPeerId) => (true, Vec::new(), Vec::new()),
            Ok(LaunchMode::Listen {
                listen_addresses,
                external_addresses,
            }) => (false, listen_addresses, external_addresses),
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
        PublicCatalogServerConfig::new(listen_addresses)
            .with_external_addresses(external_addresses),
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
        signal = shutdown_signal() => {
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

        let Ok(LaunchMode::Listen {
            listen_addresses,
            external_addresses,
        }) = mode
        else {
            panic!("expected listen mode");
        };
        assert_eq!(listen_addresses.len(), 2);
        assert!(external_addresses.is_empty());
    }

    #[test]
    fn external_addresses_are_separate_from_bind_addresses() {
        let mode = parse_launch_mode(vec![
            OsString::from("--external-address"),
            OsString::from("/ip4/203.0.113.10/tcp/42042"),
            OsString::from("/ip4/0.0.0.0/tcp/42042"),
        ]);

        let Ok(LaunchMode::Listen {
            listen_addresses,
            external_addresses,
        }) = mode
        else {
            panic!("expected listen mode");
        };
        assert_eq!(listen_addresses.len(), 1);
        assert_eq!(external_addresses.len(), 1);
    }
}
