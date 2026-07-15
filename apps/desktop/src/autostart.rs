#[cfg(any(target_os = "windows", test))]
use std::ffi::OsString;
#[cfg(any(target_os = "windows", test))]
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;

#[cfg(target_os = "windows")]
const WINDOWS_VALUE_NAME: &str = "AirWiki";
#[cfg(target_os = "macos")]
const MACOS_PLIST_NAME: &str = "io.github.airwiki.AirWiki.background.plist";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutostartStatus {
    Disabled,
    Enabled,
    #[cfg_attr(
        target_os = "windows",
        expect(dead_code, reason = "this state is reported by the macOS backend")
    )]
    RequiresApproval,
    #[cfg_attr(
        all(not(target_os = "windows"), not(test)),
        expect(dead_code, reason = "this state is reported by the Windows backend")
    )]
    Conflict,
    #[cfg_attr(
        target_os = "windows",
        expect(
            dead_code,
            reason = "this state is reported by unsupported platform backends"
        )
    )]
    Unsupported,
}

#[derive(Debug, Error)]
pub(crate) enum AutostartError {
    #[cfg(target_os = "windows")]
    #[error("the Windows autostart registry operation `{operation}` failed")]
    Registry {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[cfg(target_os = "macos")]
    #[error("the macOS autostart operation `{operation}` failed (code {code})")]
    MacOs {
        operation: &'static str,
        code: isize,
    },
    #[cfg(target_os = "macos")]
    #[error("the bundled macOS launch agent is unavailable (status {status})")]
    MacOsServiceUnavailable { status: isize },
}

pub(crate) struct AutostartManager {
    #[cfg(target_os = "windows")]
    executable: PathBuf,
}

impl AutostartManager {
    pub(crate) fn new(executable: PathBuf) -> Self {
        #[cfg(target_os = "windows")]
        {
            Self { executable }
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = executable;
            Self {}
        }
    }

    pub(crate) fn status(&self) -> Result<AutostartStatus, AutostartError> {
        platform::status(self)
    }

    pub(crate) fn enable(&self) -> Result<AutostartStatus, AutostartError> {
        platform::enable(self)
    }

    pub(crate) fn disable(&self) -> Result<AutostartStatus, AutostartError> {
        platform::disable(self)
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryState {
    Missing,
    Exact,
    Conflict,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mutation {
    None(AutostartStatus),
    Write,
    Delete,
}

#[cfg(any(target_os = "windows", test))]
fn enable_mutation(entry: EntryState) -> Mutation {
    match entry {
        EntryState::Missing => Mutation::Write,
        EntryState::Exact => Mutation::None(AutostartStatus::Enabled),
        EntryState::Conflict => Mutation::None(AutostartStatus::Conflict),
    }
}

#[cfg(any(target_os = "windows", test))]
fn disable_mutation(entry: EntryState) -> Mutation {
    match entry {
        EntryState::Missing => Mutation::None(AutostartStatus::Disabled),
        EntryState::Exact => Mutation::Delete,
        EntryState::Conflict => Mutation::None(AutostartStatus::Conflict),
    }
}

#[cfg(any(target_os = "windows", test))]
fn status_from_entry(entry: EntryState) -> AutostartStatus {
    match entry {
        EntryState::Missing => AutostartStatus::Disabled,
        EntryState::Exact => AutostartStatus::Enabled,
        EntryState::Conflict => AutostartStatus::Conflict,
    }
}

#[cfg(any(target_os = "windows", test))]
fn windows_command(executable: &Path) -> OsString {
    let mut command = OsString::from("\"");
    command.push(executable.as_os_str());
    command.push("\" --background");
    command
}

#[cfg(any(target_os = "windows", test))]
fn classify_windows_bytes(actual: Option<&[u8]>, expected: &[u8]) -> EntryState {
    match actual {
        None => EntryState::Missing,
        Some(actual) if actual == expected => EntryState::Exact,
        Some(_) => EntryState::Conflict,
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::io;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ};
    use winreg::types::ToRegValue;

    use super::{
        AutostartError, AutostartManager, AutostartStatus, EntryState, Mutation,
        WINDOWS_VALUE_NAME, classify_windows_bytes, disable_mutation, enable_mutation,
        status_from_entry, windows_command,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub(super) fn status(manager: &AutostartManager) -> Result<AutostartStatus, AutostartError> {
        let expected = windows_command(&manager.executable);
        let current_user = RegKey::predef(HKEY_CURRENT_USER);
        let key = match current_user.open_subkey_with_flags(RUN_KEY, KEY_READ) {
            Ok(key) => key,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(AutostartStatus::Disabled);
            }
            Err(source) => return Err(registry_error("read", source)),
        };
        Ok(status_from_entry(read_entry(&key, &expected)?))
    }

    pub(super) fn enable(manager: &AutostartManager) -> Result<AutostartStatus, AutostartError> {
        let expected = windows_command(&manager.executable);
        let current_user = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = current_user
            .create_subkey(RUN_KEY)
            .map_err(|source| registry_error("open_for_enable", source))?;
        match enable_mutation(read_entry(&key, &expected)?) {
            Mutation::None(status) => Ok(status),
            Mutation::Write => {
                key.set_value(WINDOWS_VALUE_NAME, &expected)
                    .map_err(|source| registry_error("enable", source))?;
                Ok(AutostartStatus::Enabled)
            }
            Mutation::Delete => Ok(AutostartStatus::Conflict),
        }
    }

    pub(super) fn disable(manager: &AutostartManager) -> Result<AutostartStatus, AutostartError> {
        let expected = windows_command(&manager.executable);
        let current_user = RegKey::predef(HKEY_CURRENT_USER);
        let key = match current_user.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE) {
            Ok(key) => key,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(AutostartStatus::Disabled);
            }
            Err(source) => return Err(registry_error("open_for_disable", source)),
        };
        match disable_mutation(read_entry(&key, &expected)?) {
            Mutation::None(status) => Ok(status),
            Mutation::Delete => {
                key.delete_value(WINDOWS_VALUE_NAME)
                    .map_err(|source| registry_error("disable", source))?;
                Ok(AutostartStatus::Disabled)
            }
            Mutation::Write => Ok(AutostartStatus::Conflict),
        }
    }

    fn read_entry(key: &RegKey, expected: &std::ffi::OsStr) -> Result<EntryState, AutostartError> {
        let raw = match key.get_raw_value(WINDOWS_VALUE_NAME) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(EntryState::Missing);
            }
            Err(source) => return Err(registry_error("read_value", source)),
        };
        if raw.vtype != REG_SZ {
            return Ok(EntryState::Conflict);
        }
        let expected = expected.to_reg_value();
        Ok(classify_windows_bytes(
            Some(raw.bytes.as_ref()),
            expected.bytes.as_ref(),
        ))
    }

    fn registry_error(operation: &'static str, source: io::Error) -> AutostartError {
        AutostartError::Registry { operation, source }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2_foundation::NSString;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    use super::{AutostartError, AutostartManager, AutostartStatus, MACOS_PLIST_NAME};

    pub(super) fn status(_manager: &AutostartManager) -> Result<AutostartStatus, AutostartError> {
        with_service(service_status)
    }

    pub(super) fn enable(_manager: &AutostartManager) -> Result<AutostartStatus, AutostartError> {
        with_service(|service| match service_status(service)? {
            AutostartStatus::Enabled | AutostartStatus::RequiresApproval => service_status(service),
            AutostartStatus::Disabled => {
                // SAFETY: `service` is retained for the call and was created by
                // ServiceManagement for the static plist name bundled by this app.
                if let Err(error) = unsafe { service.registerAndReturnError() } {
                    let observed = service_status(service)?;
                    if matches!(
                        observed,
                        AutostartStatus::Enabled | AutostartStatus::RequiresApproval
                    ) {
                        return Ok(observed);
                    }
                    return Err(AutostartError::MacOs {
                        operation: "enable",
                        code: error.code(),
                    });
                }
                service_status(service)
            }
            AutostartStatus::Conflict | AutostartStatus::Unsupported => {
                Ok(AutostartStatus::Unsupported)
            }
        })
    }

    pub(super) fn disable(_manager: &AutostartManager) -> Result<AutostartStatus, AutostartError> {
        with_service(|service| match service_status(service)? {
            AutostartStatus::Disabled => Ok(AutostartStatus::Disabled),
            AutostartStatus::Enabled | AutostartStatus::RequiresApproval => {
                // SAFETY: `service` is retained for the call and was created by
                // ServiceManagement for the static plist name bundled by this app.
                if let Err(error) = unsafe { service.unregisterAndReturnError() } {
                    if service_status(service)? == AutostartStatus::Disabled {
                        return Ok(AutostartStatus::Disabled);
                    }
                    return Err(AutostartError::MacOs {
                        operation: "disable",
                        code: error.code(),
                    });
                }
                service_status(service)
            }
            AutostartStatus::Conflict | AutostartStatus::Unsupported => {
                Ok(AutostartStatus::Unsupported)
            }
        })
    }

    fn with_service<T>(operation: impl FnOnce(&SMAppService) -> T) -> T {
        let plist_name = NSString::from_str(MACOS_PLIST_NAME);
        // SAFETY: The argument is a valid retained NSString and its static file
        // name resolves only inside Contents/Library/LaunchAgents.
        let service = unsafe { SMAppService::agentServiceWithPlistName(&plist_name) };
        operation(&service)
    }

    fn service_status(service: &SMAppService) -> Result<AutostartStatus, AutostartError> {
        // SAFETY: `service` is a live framework object, and `status` takes no
        // pointer arguments or caller-managed buffers.
        let status = unsafe { service.status() };
        match status {
            SMAppServiceStatus::NotRegistered => Ok(AutostartStatus::Disabled),
            SMAppServiceStatus::Enabled => Ok(AutostartStatus::Enabled),
            SMAppServiceStatus::RequiresApproval => Ok(AutostartStatus::RequiresApproval),
            SMAppServiceStatus::NotFound => {
                Err(AutostartError::MacOsServiceUnavailable { status: status.0 })
            }
            other => Err(AutostartError::MacOsServiceUnavailable { status: other.0 }),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::{AutostartError, AutostartManager, AutostartStatus};

    pub(super) fn status(_manager: &AutostartManager) -> Result<AutostartStatus, AutostartError> {
        Ok(AutostartStatus::Unsupported)
    }

    pub(super) fn enable(_manager: &AutostartManager) -> Result<AutostartStatus, AutostartError> {
        Ok(AutostartStatus::Unsupported)
    }

    pub(super) fn disable(_manager: &AutostartManager) -> Result<AutostartStatus, AutostartError> {
        Ok(AutostartStatus::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use super::{
        AutostartStatus, EntryState, Mutation, classify_windows_bytes, disable_mutation,
        enable_mutation, status_from_entry, windows_command,
    };

    #[test]
    fn windows_command_should_quote_unicode_path_and_add_background_argument() {
        let command = windows_command(Path::new(r"C:\Program Files\AirWiki\airwiki.exe"));

        assert_eq!(
            command,
            OsString::from(r#""C:\Program Files\AirWiki\airwiki.exe" --background"#)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_command_should_serialize_as_a_registry_string() {
        use winreg::enums::REG_SZ;
        use winreg::types::ToRegValue;

        let command = windows_command(Path::new(r"C:\Usuarios\José\AirWiki\airwiki.exe"));

        assert_eq!(command.to_reg_value().vtype, REG_SZ);
    }

    #[test]
    fn classify_windows_bytes_should_require_an_exact_serialized_value() {
        let expected = [1, 2, 0, 0];

        assert_eq!(
            classify_windows_bytes(Some(&expected), &expected),
            EntryState::Exact
        );
        assert_eq!(
            classify_windows_bytes(Some(&[1, 2, 0, 0, 0, 0]), &expected),
            EntryState::Conflict
        );
        assert_eq!(classify_windows_bytes(None, &expected), EntryState::Missing);
    }

    #[test]
    fn enable_mutation_should_be_idempotent_and_preserve_conflicts() {
        assert_eq!(enable_mutation(EntryState::Missing), Mutation::Write);
        assert_eq!(
            enable_mutation(EntryState::Exact),
            Mutation::None(AutostartStatus::Enabled)
        );
        assert_eq!(
            enable_mutation(EntryState::Conflict),
            Mutation::None(AutostartStatus::Conflict)
        );
    }

    #[test]
    fn disable_mutation_should_delete_only_an_exact_entry() {
        assert_eq!(
            disable_mutation(EntryState::Missing),
            Mutation::None(AutostartStatus::Disabled)
        );
        assert_eq!(disable_mutation(EntryState::Exact), Mutation::Delete);
        assert_eq!(
            disable_mutation(EntryState::Conflict),
            Mutation::None(AutostartStatus::Conflict)
        );
    }

    #[test]
    fn status_should_reflect_registry_authority() {
        assert_eq!(
            status_from_entry(EntryState::Missing),
            AutostartStatus::Disabled
        );
        assert_eq!(
            status_from_entry(EntryState::Exact),
            AutostartStatus::Enabled
        );
        assert_eq!(
            status_from_entry(EntryState::Conflict),
            AutostartStatus::Conflict
        );
    }

    #[test]
    fn macos_launch_agent_should_use_bundled_program_and_background_argument() {
        let plist =
            include_str!("../../../packaging/macos/io.github.airwiki.AirWiki.background.plist");

        assert!(plist.contains("<key>BundleProgram</key>"));
        assert!(plist.contains("<string>Contents/MacOS/airwiki</string>"));
        assert!(plist.contains("<string>--background</string>"));
        assert!(!plist.contains("<key>KeepAlive</key>"));
    }
}
