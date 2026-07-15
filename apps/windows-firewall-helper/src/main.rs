use std::process::ExitCode;

use airwiki_windows_firewall::{HelperExitCode, parse_command, run_platform};

fn main() -> ExitCode {
    let result = parse_command(std::env::args_os().skip(1)).and_then(run_platform);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", sanitized_message(error.exit_code()));
            ExitCode::from(error.exit_code() as u8)
        }
    }
}

const fn sanitized_message(code: HelperExitCode) -> &'static str {
    match code {
        HelperExitCode::Success => "Windows Firewall configuration completed",
        HelperExitCode::ManagedPolicy => "Windows Firewall is managed by policy",
        HelperExitCode::InboundBlocked => "Windows Firewall blocks inbound connections",
        HelperExitCode::Conflict => "A conflicting Windows Firewall rule exists",
        HelperExitCode::InvalidLayoutOrSignature => {
            "The installed AirWiki binaries could not be verified"
        }
        HelperExitCode::Unsupported => "Windows Firewall integration is unavailable",
        HelperExitCode::InternalError => "Windows Firewall configuration failed",
        HelperExitCode::InvalidArguments => "Usage: helper.exe <install|remove>",
    }
}
