use std::env;
use std::ffi::OsString;
use std::process::ExitCode;
use std::str::FromStr;

use airwiki_network::{Multiaddr, PeerId, PublicIndexEndpoint, PublicReader};
use airwiki_types::{SearchPurpose, SearchRequest};

fn parse_endpoint(args: Vec<OsString>) -> Result<PublicIndexEndpoint, String> {
    let [peer_id, address]: [OsString; 2] = args
        .try_into()
        .map_err(|_| "usage: federation-probe <peer-id> <multiaddress>".to_owned())?;
    let peer_id = peer_id
        .into_string()
        .map_err(|_| "peer id is not valid UTF-8".to_owned())?;
    let address = address
        .into_string()
        .map_err(|_| "multiaddress is not valid UTF-8".to_owned())?;
    Ok(PublicIndexEndpoint {
        peer_id: PeerId::from_str(&peer_id).map_err(|_| "peer id is invalid".to_owned())?,
        address: Multiaddr::from_str(&address).map_err(|_| "multiaddress is invalid".to_owned())?,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let endpoint = match parse_endpoint(env::args_os().skip(1).collect()) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let request = SearchRequest::new(
        "synthetic federation connectivity probe",
        SearchPurpose::LocalAssistant,
        1,
    );
    match PublicReader::new().search(&[endpoint], request).await {
        Ok(response) if response.hits.is_empty() && !response.partial => {
            println!("public catalog probe: PASS");
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("public catalog probe returned an unexpected response");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("public catalog probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_requires_exactly_two_arguments() {
        let endpoint = parse_endpoint(Vec::new());

        assert!(endpoint.is_err());
    }
}
