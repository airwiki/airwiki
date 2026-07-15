use std::{env, ffi::OsString, process::ExitCode, str::FromStr};

use airwiki_mcp::McpClientKind;

const USAGE: &str =
    "usage: airwiki-mcp-bridge --client <chatgpt-desktop|claude-desktop|gemini-cli>";

#[tokio::main]
async fn main() -> ExitCode {
    let client = match parse_client(env::args_os().skip(1)) {
        Ok(client) => client,
        Err(()) => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match airwiki_mcp::run_stdio_bridge(client).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Display is intentionally stable and excludes protocol payloads and paths.
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_client(args: impl IntoIterator<Item = OsString>) -> Result<McpClientKind, ()> {
    let mut args = args.into_iter();
    let Some(flag) = args.next() else {
        return Err(());
    };
    if flag != "--client" {
        return Err(());
    }
    let Some(value) = args.next() else {
        return Err(());
    };
    if args.next().is_some() {
        return Err(());
    }
    let value = value.into_string().map_err(|_| ())?;
    McpClientKind::from_str(&value).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_argument_accepts_only_supported_clients() {
        for (value, expected) in [
            ("chatgpt-desktop", McpClientKind::ChatGptDesktop),
            ("claude-desktop", McpClientKind::ClaudeDesktop),
            ("gemini-cli", McpClientKind::GeminiCli),
        ] {
            let parsed = parse_client([OsString::from("--client"), OsString::from(value)])
                .expect("supported client");
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn client_argument_rejects_missing_unknown_and_extra_values() {
        for args in [
            Vec::new(),
            vec![OsString::from("--client")],
            vec![OsString::from("--client"), OsString::from("unknown")],
            vec![
                OsString::from("--client"),
                OsString::from("gemini-cli"),
                OsString::from("extra"),
            ],
        ] {
            assert!(parse_client(args).is_err());
        }
    }
}
