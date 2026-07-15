//! Blocking, platform-specific connectivity diagnostics and firewall actions.
//!
//! Callers must run these functions behind the desktop worker's blocking
//! boundary. The DTOs intentionally contain no executable paths, network
//! names, addresses or raw operating-system errors.

use thiserror::Error;

#[cfg(any(target_os = "windows", test))]
const ADVANCED_FIREWALL_CONSOLE: &str = "wf.msc";

/// State of the operating system's local-network permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Some variants are constructed only by the target-specific Windows backend.
#[allow(dead_code)]
pub(crate) enum SystemPermissionState {
    /// The platform does not expose this permission model.
    NotApplicable,
    /// The application cannot determine the decision without attempting LAN I/O.
    Unknown,
    /// Runtime connectivity proves that the permission is available.
    Granted,
    /// Runtime connectivity or an explicit system result proves denial.
    Denied,
}

/// Active network category as reported by the operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum NetworkProfileState {
    /// The platform does not use Windows network profiles.
    NotApplicable,
    /// No connected profile could be determined safely.
    Unknown,
    /// A connected Windows network is private.
    Private,
    /// A connected Windows network is domain-authenticated.
    Domain,
    /// At least one connected Windows network is public.
    Public,
}

/// Read-only state of the two exact Windows Firewall rules managed by the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum FirewallDiagnosticState {
    /// The platform does not require the Windows helper.
    NotApplicable,
    /// The state has not been determined.
    Unknown,
    /// Both rules exactly match the expected restricted configuration.
    Ready,
    /// Windows Firewall is disabled for an active Private or Domain profile.
    FirewallDisabled,
    /// Windows is ignoring all inbound exceptions for an active profile.
    BlockAllInbound,
    /// One or both expected rules are absent.
    RulesMissing,
    /// A rule name is occupied by different settings.
    Conflict,
    /// Another enabled inbound rule exposes this executable more broadly than
    /// the two managed Private/Domain + LocalSubnet rules.
    LegacyExposure,
    /// Group policy prevents effective local rule changes.
    ManagedPolicy,
    /// The installed platform cannot provide the supported helper backend.
    Unsupported,
    /// An operating-system query failed without exposing sensitive details.
    Error,
}

/// Trust state of the fixed elevated helper installed next to the desktop app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum FirewallHelperState {
    /// This platform does not use the Windows helper.
    NotApplicable,
    /// Helper and desktop have valid code-signing chains and the same identity.
    Verified,
    /// The helper is absent from the installed layout.
    Missing,
    /// One of the binaries is unsigned or has an invalid code-signing chain.
    Untrusted,
    /// Both binaries are trusted but belong to different publishers.
    PublisherMismatch,
    /// The platform cannot provide the supported verification backend.
    Unsupported,
    /// The fixed layout could not be inspected safely.
    Error,
}

impl FirewallHelperState {
    /// Elevation is offered only after the same trust check that the elevated
    /// helper will repeat independently.
    pub(crate) const fn can_request_elevation(self) -> bool {
        matches!(self, Self::Verified)
    }
}

/// Sanitized platform snapshot combined later with runtime LAN state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectivityPlatformSnapshot {
    pub system_permission: SystemPermissionState,
    pub network_profile: NetworkProfileState,
    pub firewall: FirewallDiagnosticState,
    pub firewall_helper: FirewallHelperState,
}

/// Effective desktop decision for starting the optional LAN runtime.
///
/// Consent and operating-system readiness are intentionally separate. A user
/// may keep LAN enabled while Windows is still on a Public network or while its
/// restricted firewall rules are being repaired; local search and MCP continue
/// running in either case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanRuntimePolicy {
    /// The user has not enabled LAN sharing.
    DisabledByPreference,
    /// The platform snapshot has not arrived yet.
    WaitingForDiagnostic,
    /// The operating-system prerequisites permit starting libp2p.
    Allowed,
    /// Windows is connected through a Public or indeterminate profile.
    BlockedByNetworkProfile,
    /// Windows does not have both exact restricted rules ready.
    BlockedByFirewall,
    /// The current platform is outside the supported desktop matrix.
    Unsupported,
}

impl LanRuntimePolicy {
    pub(crate) const fn should_run(self) -> bool {
        matches!(self, Self::Allowed)
    }
}

/// Reduces current consent and platform state into the desired LAN runtime.
pub(crate) fn lan_runtime_policy(
    enabled_by_user: bool,
    snapshot: Option<ConnectivityPlatformSnapshot>,
) -> LanRuntimePolicy {
    lan_runtime_policy_for(CurrentPlatform::CURRENT, enabled_by_user, snapshot)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "all platform variants are exercised by policy tests but only one is built per target"
    )
)]
enum CurrentPlatform {
    MacOs,
    Windows,
    Other,
}

impl CurrentPlatform {
    #[cfg(target_os = "macos")]
    const CURRENT: Self = Self::MacOs;
    #[cfg(target_os = "windows")]
    const CURRENT: Self = Self::Windows;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    const CURRENT: Self = Self::Other;
}

fn lan_runtime_policy_for(
    platform: CurrentPlatform,
    enabled_by_user: bool,
    snapshot: Option<ConnectivityPlatformSnapshot>,
) -> LanRuntimePolicy {
    if !enabled_by_user {
        return LanRuntimePolicy::DisabledByPreference;
    }
    let Some(snapshot) = snapshot else {
        return LanRuntimePolicy::WaitingForDiagnostic;
    };
    match platform {
        CurrentPlatform::MacOs => LanRuntimePolicy::Allowed,
        CurrentPlatform::Windows => {
            if !matches!(
                snapshot.network_profile,
                NetworkProfileState::Private | NetworkProfileState::Domain
            ) {
                LanRuntimePolicy::BlockedByNetworkProfile
            } else if snapshot.firewall != FirewallDiagnosticState::Ready {
                LanRuntimePolicy::BlockedByFirewall
            } else {
                LanRuntimePolicy::Allowed
            }
        }
        CurrentPlatform::Other => LanRuntimePolicy::Unsupported,
    }
}

/// Stable, user-actionable result of invoking the elevated Windows helper.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum FirewallActionError {
    /// The user cancelled the UAC prompt.
    #[error("firewall elevation was cancelled")]
    Cancelled,
    /// Group policy prevents effective local rule changes.
    #[error("firewall changes are managed by policy")]
    ManagedPolicy,
    /// Windows policy rejects unsolicited inbound traffic.
    #[error("firewall policy blocks inbound connections")]
    InboundBlocked,
    /// A managed rule name is occupied by a different configuration.
    #[error("a conflicting firewall rule exists")]
    Conflict,
    /// The signed helper/application layout could not be verified.
    #[error("the firewall helper installation could not be verified")]
    InvalidLayoutOrSignature,
    /// This operating system has no supported firewall helper.
    #[error("firewall integration is unsupported")]
    Unsupported,
    /// Consent, network profile or diagnosed rule state changed before elevation.
    #[error("firewall prerequisites changed before elevation")]
    StateChanged,
    /// The operating-system action failed without exposing raw details.
    #[error("firewall configuration failed")]
    Internal,
}

/// Performs a read-only platform diagnostic.
///
/// This function is blocking on Windows because it initializes COM and reads
/// Windows Firewall policy. macOS deliberately returns an unknown permission:
/// only a contextual LAN attempt may establish the user's Local Network choice.
pub(crate) fn diagnose() -> ConnectivityPlatformSnapshot {
    platform::diagnose()
}

/// Installs or verifies the two exact restricted Windows Firewall rules.
///
/// No path, port or arbitrary rule is accepted from the caller.
pub(crate) fn install_firewall_rules() -> Result<(), FirewallActionError> {
    platform::run_firewall_action(FirewallOperation::Install)
}

/// Removes only exact rules previously managed by AirWiki.
pub(crate) fn remove_firewall_rules() -> Result<(), FirewallActionError> {
    platform::run_firewall_action(FirewallOperation::Remove)
}

/// Opens the fixed Windows advanced firewall console.
///
/// Callers must invoke this through the worker's blocking boundary.
pub(crate) fn open_advanced_firewall_rules() -> Result<(), FirewallActionError> {
    platform::open_advanced_firewall_rules()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirewallOperation {
    Install,
    Remove,
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{
        fs::{File, OpenOptions},
        mem::size_of,
        os::windows::{
            ffi::OsStrExt,
            fs::OpenOptionsExt,
            io::{AsRawHandle, RawHandle},
        },
        path::{Path, PathBuf},
        ptr::null_mut,
    };

    use airwiki_windows_firewall::{
        FirewallDiagnosticStatus as HelperDiagnosticStatus,
        FirewallHelperTrustStatus as HelperTrustStatus, HelperExitCode, diagnose_platform,
        diagnose_sibling_helper_trust, verify_open_artifact_publisher_matches_current_executable,
    };
    use windows::{
        Win32::{
            Foundation::{CloseHandle, HANDLE, HWND, WAIT_OBJECT_0},
            Networking::NetworkListManager::{
                INetworkListManager, NLM_ENUM_NETWORK_CONNECTED,
                NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED, NLM_NETWORK_CATEGORY_PRIVATE,
                NLM_NETWORK_CATEGORY_PUBLIC, NetworkListManager,
            },
            Storage::FileSystem::{
                FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
                FILE_TYPE_DISK, FileAttributeTagInfo, GetFileInformationByHandleEx, GetFileType,
            },
            System::{
                Com::{
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
                    COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
                },
                Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
            },
            UI::{
                Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
                WindowsAndMessaging::SW_SHOWNORMAL,
            },
        },
        core::PCWSTR,
    };

    use super::{
        ADVANCED_FIREWALL_CONSOLE, ConnectivityPlatformSnapshot, FirewallActionError,
        FirewallDiagnosticState, FirewallHelperState, FirewallOperation, NetworkProfileState,
        SystemPermissionState,
    };

    const HELPER_BASENAME: &str = "airwiki-windows-firewall-helper.exe";
    const UAC_CANCELLED_HRESULT: i32 = 0x8007_04c7_u32 as i32;
    const FIREWALL_HELPER_WAIT_MILLIS: u32 = INFINITE;

    pub(super) fn diagnose() -> ConnectivityPlatformSnapshot {
        ConnectivityPlatformSnapshot {
            system_permission: SystemPermissionState::NotApplicable,
            network_profile: diagnose_network_profile(),
            firewall: map_helper_diagnostic(diagnose_platform().status),
            firewall_helper: map_helper_trust(diagnose_sibling_helper_trust()),
        }
    }

    pub(super) fn run_firewall_action(
        operation: FirewallOperation,
    ) -> Result<(), FirewallActionError> {
        let _apartment = ComApartment::initialize_shell_action()?;
        VerifiedElevationTarget::prepare()?.run(operation)
    }

    pub(super) fn open_advanced_firewall_rules() -> Result<(), FirewallActionError> {
        let _apartment = ComApartment::initialize_shell_action()?;
        let verb = wide_null("open");
        let console = wide_null(ADVANCED_FIREWALL_CONSOLE);
        let mut execute = SHELLEXECUTEINFOW {
            cbSize: u32::try_from(size_of::<SHELLEXECUTEINFOW>())
                .map_err(|_| FirewallActionError::Internal)?,
            hwnd: HWND(null_mut()),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(console.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };

        // SAFETY: The two fixed UTF-16 buffers and initialized structure live
        // through this synchronous dispatch; callers cannot control the target.
        unsafe { ShellExecuteExW(&mut execute) }.map_err(|_| FirewallActionError::Internal)
    }

    struct LockedElevationTarget {
        // Omitting write/delete sharing prevents another process from changing
        // either the final directory entry or the helper bytes while UAC is open.
        _directory: File,
        helper: File,
        helper_path: PathBuf,
    }

    impl LockedElevationTarget {
        fn open_sibling() -> Result<Self, FirewallActionError> {
            let desktop = std::env::current_exe()
                .map_err(|_| FirewallActionError::InvalidLayoutOrSignature)?;
            let directory = desktop
                .parent()
                .ok_or(FirewallActionError::InvalidLayoutOrSignature)?;
            Self::open_in(directory)
        }

        fn open_in(directory_path: &Path) -> Result<Self, FirewallActionError> {
            let directory = open_locked_object(directory_path, LockedObjectKind::Directory)?;
            let helper_path = directory_path.join(HELPER_BASENAME);
            let helper = open_locked_object(&helper_path, LockedObjectKind::RegularFile)?;
            Ok(Self {
                _directory: directory,
                helper,
                helper_path,
            })
        }
    }

    struct VerifiedElevationTarget {
        locked: LockedElevationTarget,
    }

    impl VerifiedElevationTarget {
        fn prepare() -> Result<Self, FirewallActionError> {
            let locked = LockedElevationTarget::open_sibling()?;
            verify_open_artifact_publisher_matches_current_executable(
                &locked.helper,
                &locked.helper_path,
            )
            .map_err(|_| FirewallActionError::InvalidLayoutOrSignature)?;
            Ok(Self { locked })
        }

        fn run(self, operation: FirewallOperation) -> Result<(), FirewallActionError> {
            let helper_wide = wide_null(self.locked.helper_path.as_os_str());
            let process = self.launch(operation, &helper_wide)?;
            // The verified file and directory handles remain alive through the
            // UAC prompt and until the helper exits.
            let result = wait_for_helper(process);
            drop(self);
            result
        }

        fn launch(
            &self,
            operation: FirewallOperation,
            helper_wide: &[u16],
        ) -> Result<OwnedHandle, FirewallActionError> {
            let verb = wide_null("runas");
            let parameters = wide_null(match operation {
                FirewallOperation::Install => "install",
                FirewallOperation::Remove => "remove",
            });
            let mut execute = SHELLEXECUTEINFOW {
                cbSize: u32::try_from(size_of::<SHELLEXECUTEINFOW>())
                    .map_err(|_| FirewallActionError::Internal)?,
                fMask: SEE_MASK_NOCLOSEPROCESS,
                hwnd: HWND(null_mut()),
                lpVerb: PCWSTR(verb.as_ptr()),
                lpFile: PCWSTR(helper_wide.as_ptr()),
                lpParameters: PCWSTR(parameters.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };

            // SAFETY: Every UTF-16 buffer and the fully initialized structure live
            // through this synchronous call. `self` keeps read-only handles that
            // deny write/delete sharing for the verified helper and its directory,
            // so `lpFile` cannot be replaced while ShellExecute and UAC resolve it.
            if let Err(error) = unsafe { ShellExecuteExW(&mut execute) } {
                return if is_uac_cancelled(error.code().0) {
                    Err(FirewallActionError::Cancelled)
                } else {
                    Err(FirewallActionError::Internal)
                };
            }
            OwnedHandle::new(execute.hProcess)
        }
    }

    fn wait_for_helper(process: OwnedHandle) -> Result<(), FirewallActionError> {
        // SAFETY: `process` owns a valid handle returned with
        // `SEE_MASK_NOCLOSEPROCESS`; the helper is intentionally awaited by this
        // blocking worker operation. The worker remains authoritative until
        // the elevated helper exits and is never killed during COM work.
        let wait = unsafe { WaitForSingleObject(process.raw(), FIREWALL_HELPER_WAIT_MILLIS) };
        if wait != WAIT_OBJECT_0 {
            return Err(FirewallActionError::Internal);
        }
        let mut exit_code = 0;
        // SAFETY: The process handle remains valid until `OwnedHandle` drops.
        unsafe { GetExitCodeProcess(process.raw(), &mut exit_code) }
            .map_err(|_| FirewallActionError::Internal)?;
        map_helper_exit_code(exit_code)
    }

    #[derive(Clone, Copy)]
    enum LockedObjectKind {
        Directory,
        RegularFile,
    }

    fn open_locked_object(
        path: &Path,
        kind: LockedObjectKind,
    ) -> Result<File, FirewallActionError> {
        let flags = FILE_FLAG_OPEN_REPARSE_POINT.0
            | match kind {
                LockedObjectKind::Directory => FILE_FLAG_BACKUP_SEMANTICS.0,
                LockedObjectKind::RegularFile => 0,
            };
        let mut options = OpenOptions::new();
        options
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(flags);
        let file = options
            .open(path)
            .map_err(|_| FirewallActionError::InvalidLayoutOrSignature)?;
        validate_locked_object(&file, kind)?;
        Ok(file)
    }

    fn validate_locked_object(
        file: &File,
        kind: LockedObjectKind,
    ) -> Result<(), FirewallActionError> {
        let handle = file_handle(file);
        // SAFETY: `handle` is borrowed from `file`, which remains alive for this
        // synchronous type query.
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            return Err(FirewallActionError::InvalidLayoutOrSignature);
        }

        let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
        let info_size = u32::try_from(size_of::<FILE_ATTRIBUTE_TAG_INFO>())
            .map_err(|_| FirewallActionError::Internal)?;
        // SAFETY: `info` is a correctly sized writable buffer and `handle`
        // remains valid for the complete synchronous call.
        unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                (&raw mut info).cast(),
                info_size,
            )
        }
        .map_err(|_| FirewallActionError::InvalidLayoutOrSignature)?;
        if !locked_object_attributes_match(info.FileAttributes, kind) {
            return Err(FirewallActionError::InvalidLayoutOrSignature);
        }
        Ok(())
    }

    const fn locked_object_attributes_match(attributes: u32, kind: LockedObjectKind) -> bool {
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return false;
        }
        let is_directory = attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
        match kind {
            LockedObjectKind::Directory => is_directory,
            LockedObjectKind::RegularFile => !is_directory,
        }
    }

    fn file_handle(file: &File) -> HANDLE {
        let handle: RawHandle = file.as_raw_handle();
        HANDLE(handle)
    }

    fn diagnose_network_profile() -> NetworkProfileState {
        let _apartment = match ComApartment::initialize() {
            Ok(apartment) => apartment,
            Err(()) => return NetworkProfileState::Unknown,
        };
        // SAFETY: COM is initialized on this thread and the fixed system NLM
        // class is created in-process without aggregation.
        let manager: INetworkListManager =
            match unsafe { CoCreateInstance(&NetworkListManager, None, CLSCTX_INPROC_SERVER) } {
                Ok(manager) => manager,
                Err(_) => return NetworkProfileState::Unknown,
            };
        // SAFETY: `manager` is a live COM interface and the query is read-only.
        let networks = match unsafe { manager.GetNetworks(NLM_ENUM_NETWORK_CONNECTED) } {
            Ok(networks) => networks,
            Err(_) => return NetworkProfileState::Unknown,
        };

        let mut categories = Vec::new();
        loop {
            let mut item = [None];
            let mut fetched = 0;
            // SAFETY: `item` and `fetched` remain valid for the synchronous COM
            // call; the one-element buffer bounds the result.
            if unsafe { networks.Next(&mut item, Some(&mut fetched)) }.is_err() {
                return NetworkProfileState::Unknown;
            }
            if fetched == 0 {
                break;
            }
            let Some(network) = item[0].take() else {
                return NetworkProfileState::Unknown;
            };
            // SAFETY: `network` is a live interface returned by the enumerator.
            let category = match unsafe { network.GetCategory() } {
                Ok(category) => category.0,
                Err(_) => return NetworkProfileState::Unknown,
            };
            categories.push(category);
        }
        profile_from_categories(categories)
    }

    fn profile_from_categories(categories: impl IntoIterator<Item = i32>) -> NetworkProfileState {
        let mut profile = NetworkProfileState::Unknown;
        for category in categories {
            if category == NLM_NETWORK_CATEGORY_PUBLIC.0 {
                return NetworkProfileState::Public;
            }
            if category == NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED.0 {
                profile = NetworkProfileState::Domain;
            } else if category == NLM_NETWORK_CATEGORY_PRIVATE.0
                && profile != NetworkProfileState::Domain
            {
                profile = NetworkProfileState::Private;
            }
        }
        profile
    }

    fn map_helper_diagnostic(status: HelperDiagnosticStatus) -> FirewallDiagnosticState {
        match status {
            HelperDiagnosticStatus::Ready => FirewallDiagnosticState::Ready,
            HelperDiagnosticStatus::FirewallDisabled => FirewallDiagnosticState::FirewallDisabled,
            HelperDiagnosticStatus::BlockAllInbound => FirewallDiagnosticState::BlockAllInbound,
            HelperDiagnosticStatus::RulesMissing => FirewallDiagnosticState::RulesMissing,
            HelperDiagnosticStatus::Conflict => FirewallDiagnosticState::Conflict,
            HelperDiagnosticStatus::LegacyExposure => FirewallDiagnosticState::LegacyExposure,
            HelperDiagnosticStatus::ManagedPolicy => FirewallDiagnosticState::ManagedPolicy,
            HelperDiagnosticStatus::Unsupported => FirewallDiagnosticState::Unsupported,
            HelperDiagnosticStatus::Error => FirewallDiagnosticState::Error,
        }
    }

    const fn map_helper_trust(status: HelperTrustStatus) -> FirewallHelperState {
        match status {
            HelperTrustStatus::Verified => FirewallHelperState::Verified,
            HelperTrustStatus::Missing => FirewallHelperState::Missing,
            HelperTrustStatus::Untrusted => FirewallHelperState::Untrusted,
            HelperTrustStatus::PublisherMismatch => FirewallHelperState::PublisherMismatch,
            HelperTrustStatus::Unsupported => FirewallHelperState::Unsupported,
            HelperTrustStatus::Error => FirewallHelperState::Error,
        }
    }

    fn map_helper_exit_code(code: u32) -> Result<(), FirewallActionError> {
        match code {
            code if code == HelperExitCode::Success as u32 => Ok(()),
            code if code == HelperExitCode::ManagedPolicy as u32 => {
                Err(FirewallActionError::ManagedPolicy)
            }
            code if code == HelperExitCode::InboundBlocked as u32 => {
                Err(FirewallActionError::InboundBlocked)
            }
            code if code == HelperExitCode::Conflict as u32 => Err(FirewallActionError::Conflict),
            code if code == HelperExitCode::InvalidLayoutOrSignature as u32 => {
                Err(FirewallActionError::InvalidLayoutOrSignature)
            }
            code if code == HelperExitCode::Unsupported as u32 => {
                Err(FirewallActionError::Unsupported)
            }
            _ => Err(FirewallActionError::Internal),
        }
    }

    const fn is_uac_cancelled(hresult: i32) -> bool {
        hresult == UAC_CANCELLED_HRESULT
    }

    fn wide_null(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        value.as_ref().encode_wide().chain(Some(0)).collect()
    }

    struct ComApartment;

    impl ComApartment {
        fn initialize() -> Result<Self, ()> {
            // SAFETY: A successful initialization is balanced on this same
            // blocking thread by `Drop`.
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                .ok()
                .map_err(|_| ())?;
            Ok(Self)
        }

        fn initialize_shell_action() -> Result<Self, FirewallActionError> {
            // Shell elevation is invoked from a dedicated blocking thread. STA
            // plus OLE1 DDE suppression is the documented model for ShellExecute
            // and is balanced by `Drop` on this same thread.
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
                .ok()
                .map_err(|_| FirewallActionError::Internal)?;
            Ok(Self)
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            // SAFETY: This instance exists only after successful initialization
            // and cannot move to another thread.
            unsafe { CoUninitialize() };
        }
    }

    struct OwnedHandle(HANDLE);

    impl OwnedHandle {
        fn new(handle: HANDLE) -> Result<Self, FirewallActionError> {
            if handle.is_invalid() {
                Err(FirewallActionError::Internal)
            } else {
                Ok(Self(handle))
            }
        }

        const fn raw(&self) -> HANDLE {
            self.0
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: This wrapper exclusively owns the process handle returned
            // by ShellExecuteExW and closes it exactly once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    #[cfg(test)]
    mod tests {
        use std::{fs, io, process::Command};

        use tempfile::TempDir;
        use windows::Win32::Foundation::ERROR_SHARING_VIOLATION;

        use super::*;

        fn locked_helper_fixture() -> (TempDir, LockedElevationTarget) {
            let directory = tempfile::tempdir().expect("temporary install directory");
            fs::write(
                directory.path().join(HELPER_BASENAME),
                b"signed fixture placeholder",
            )
            .expect("helper fixture");
            let locked =
                LockedElevationTarget::open_in(directory.path()).expect("locked helper fixture");
            (directory, locked)
        }

        fn assert_sharing_violation(error: &io::Error) {
            assert_eq!(
                error.raw_os_error(),
                Some(ERROR_SHARING_VIOLATION.0 as i32),
                "unexpected Windows error: {error}"
            );
        }

        #[test]
        fn public_profile_has_fail_closed_precedence() {
            assert_eq!(
                profile_from_categories([
                    NLM_NETWORK_CATEGORY_PRIVATE.0,
                    NLM_NETWORK_CATEGORY_PUBLIC.0,
                    NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED.0,
                ]),
                NetworkProfileState::Public
            );
        }

        #[test]
        fn domain_precedes_private_and_empty_is_unknown() {
            assert_eq!(
                profile_from_categories([
                    NLM_NETWORK_CATEGORY_PRIVATE.0,
                    NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED.0,
                ]),
                NetworkProfileState::Domain
            );
            assert_eq!(profile_from_categories([]), NetworkProfileState::Unknown);
        }

        #[test]
        fn stable_helper_exit_codes_are_mapped_without_raw_details() {
            assert_eq!(map_helper_exit_code(HelperExitCode::Success as u32), Ok(()));
            assert_eq!(
                map_helper_exit_code(HelperExitCode::ManagedPolicy as u32),
                Err(FirewallActionError::ManagedPolicy)
            );
            assert_eq!(
                map_helper_exit_code(HelperExitCode::InboundBlocked as u32),
                Err(FirewallActionError::InboundBlocked)
            );
            assert_eq!(
                map_helper_exit_code(HelperExitCode::Conflict as u32),
                Err(FirewallActionError::Conflict)
            );
            assert_eq!(
                map_helper_exit_code(u32::MAX),
                Err(FirewallActionError::Internal)
            );
        }

        #[test]
        fn uac_cancellation_is_distinct_from_internal_failure() {
            assert!(is_uac_cancelled(UAC_CANCELLED_HRESULT));
            assert!(!is_uac_cancelled(0));
        }

        #[test]
        fn helper_wait_is_unbounded_and_process_authoritative() {
            assert_eq!(FIREWALL_HELPER_WAIT_MILLIS, INFINITE);
        }

        #[test]
        fn locked_helper_blocks_overwrite_until_guard_drops() {
            let (directory, locked) = locked_helper_fixture();
            let helper = directory.path().join(HELPER_BASENAME);

            let error = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&helper)
                .expect_err("locked helper must reject overwrite");
            assert_sharing_violation(&error);

            drop(locked);
            OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(helper)
                .expect("dropping guard must release overwrite lock");
        }

        #[test]
        fn locked_helper_blocks_rename_until_guard_drops() {
            let (directory, locked) = locked_helper_fixture();
            let helper = directory.path().join(HELPER_BASENAME);
            let renamed = directory.path().join("renamed-helper.exe");

            let error =
                fs::rename(&helper, &renamed).expect_err("locked helper must reject rename");
            assert_sharing_violation(&error);

            drop(locked);
            fs::rename(helper, renamed).expect("dropping guard must release rename lock");
        }

        #[test]
        fn locked_helper_blocks_delete_until_guard_drops() {
            let (directory, locked) = locked_helper_fixture();
            let helper = directory.path().join(HELPER_BASENAME);

            let error = fs::remove_file(&helper).expect_err("locked helper must reject deletion");
            assert_sharing_violation(&error);

            drop(locked);
            fs::remove_file(helper).expect("dropping guard must release deletion lock");
        }

        #[test]
        fn locked_install_directory_blocks_rename_until_guard_drops() {
            let parent = tempfile::tempdir().expect("temporary parent directory");
            let directory = parent.path().join("AirWiki");
            let renamed = parent.path().join("AirWiki moved");
            fs::create_dir(&directory).expect("install directory");
            fs::write(
                directory.join(HELPER_BASENAME),
                b"signed fixture placeholder",
            )
            .expect("helper fixture");
            let locked = LockedElevationTarget::open_in(&directory).expect("locked helper fixture");

            let error = fs::rename(&directory, &renamed)
                .expect_err("locked install directory must reject rename");
            assert_sharing_violation(&error);

            drop(locked);
            fs::rename(directory, renamed)
                .expect("dropping guard must release directory rename lock");
        }

        #[test]
        fn locked_helper_remains_executable_by_windows_loader() {
            let directory = tempfile::tempdir().expect("temporary install directory");
            let helper = directory.path().join(HELPER_BASENAME);
            fs::copy(
                std::env::current_exe().expect("test executable path"),
                &helper,
            )
            .expect("executable helper fixture");
            let _locked =
                LockedElevationTarget::open_in(directory.path()).expect("locked helper fixture");

            let output = Command::new(helper)
                .arg("--list")
                .output()
                .expect("Windows loader must open a read-locked executable");

            assert!(output.status.success());
        }

        #[test]
        fn helper_directory_is_rejected_as_non_regular() {
            let directory = tempfile::tempdir().expect("temporary install directory");
            fs::create_dir(directory.path().join(HELPER_BASENAME))
                .expect("directory-shaped helper fixture");

            let result = LockedElevationTarget::open_in(directory.path());

            assert!(matches!(
                result,
                Err(FirewallActionError::InvalidLayoutOrSignature)
            ));
        }

        #[test]
        fn reparse_attribute_is_never_accepted_as_elevation_target() {
            assert!(!locked_object_attributes_match(
                FILE_ATTRIBUTE_REPARSE_POINT.0,
                LockedObjectKind::RegularFile
            ));
            assert!(!locked_object_attributes_match(
                FILE_ATTRIBUTE_REPARSE_POINT.0 | FILE_ATTRIBUTE_DIRECTORY.0,
                LockedObjectKind::Directory
            ));
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::{
        ConnectivityPlatformSnapshot, FirewallActionError, FirewallDiagnosticState,
        FirewallHelperState, FirewallOperation, NetworkProfileState, SystemPermissionState,
    };

    pub(super) fn diagnose() -> ConnectivityPlatformSnapshot {
        #[cfg(target_os = "macos")]
        {
            ConnectivityPlatformSnapshot {
                system_permission: SystemPermissionState::Unknown,
                network_profile: NetworkProfileState::NotApplicable,
                firewall: FirewallDiagnosticState::NotApplicable,
                firewall_helper: FirewallHelperState::NotApplicable,
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            ConnectivityPlatformSnapshot {
                system_permission: SystemPermissionState::NotApplicable,
                network_profile: NetworkProfileState::NotApplicable,
                firewall: FirewallDiagnosticState::Unsupported,
                firewall_helper: FirewallHelperState::Unsupported,
            }
        }
    }

    pub(super) fn run_firewall_action(
        _operation: FirewallOperation,
    ) -> Result<(), FirewallActionError> {
        Err(FirewallActionError::Unsupported)
    }

    pub(super) fn open_advanced_firewall_rules() -> Result<(), FirewallActionError> {
        Err(FirewallActionError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advanced_firewall_action_is_fixed_and_platform_scoped() {
        assert_eq!(ADVANCED_FIREWALL_CONSOLE, "wf.msc");
    }

    fn windows_snapshot(
        network_profile: NetworkProfileState,
        firewall: FirewallDiagnosticState,
    ) -> ConnectivityPlatformSnapshot {
        ConnectivityPlatformSnapshot {
            system_permission: SystemPermissionState::NotApplicable,
            network_profile,
            firewall,
            firewall_helper: FirewallHelperState::Verified,
        }
    }

    #[test]
    fn state_dtos_do_not_require_technical_or_sensitive_details() {
        let snapshot = ConnectivityPlatformSnapshot {
            system_permission: SystemPermissionState::Granted,
            network_profile: NetworkProfileState::Private,
            firewall: FirewallDiagnosticState::Unknown,
            firewall_helper: FirewallHelperState::Verified,
        };
        assert_eq!(snapshot.system_permission, SystemPermissionState::Granted);
        assert_eq!(snapshot.network_profile, NetworkProfileState::Private);
        assert_eq!(snapshot.firewall, FirewallDiagnosticState::Unknown);
        assert_ne!(
            SystemPermissionState::Denied,
            SystemPermissionState::Unknown
        );
    }

    #[test]
    fn only_verified_helper_can_request_elevation() {
        assert_eq!(
            [
                FirewallHelperState::Verified.can_request_elevation(),
                FirewallHelperState::Untrusted.can_request_elevation(),
                FirewallHelperState::PublisherMismatch.can_request_elevation(),
                FirewallHelperState::Missing.can_request_elevation(),
            ],
            [true, false, false, false]
        );
    }

    #[test]
    fn disabled_preference_never_starts_lan() {
        let decision = lan_runtime_policy_for(
            CurrentPlatform::Windows,
            false,
            Some(windows_snapshot(
                NetworkProfileState::Private,
                FirewallDiagnosticState::Ready,
            )),
        );

        assert_eq!(decision, LanRuntimePolicy::DisabledByPreference);
    }

    #[test]
    fn windows_waits_for_diagnostic_before_starting_lan() {
        assert_eq!(
            lan_runtime_policy_for(CurrentPlatform::Windows, true, None),
            LanRuntimePolicy::WaitingForDiagnostic
        );
    }

    #[test]
    fn windows_public_profile_blocks_lan_even_with_ready_rules() {
        let decision = lan_runtime_policy_for(
            CurrentPlatform::Windows,
            true,
            Some(windows_snapshot(
                NetworkProfileState::Public,
                FirewallDiagnosticState::Ready,
            )),
        );

        assert_eq!(decision, LanRuntimePolicy::BlockedByNetworkProfile);
    }

    #[test]
    fn windows_missing_rules_block_lan_on_private_profile() {
        let decision = lan_runtime_policy_for(
            CurrentPlatform::Windows,
            true,
            Some(windows_snapshot(
                NetworkProfileState::Private,
                FirewallDiagnosticState::RulesMissing,
            )),
        );

        assert_eq!(decision, LanRuntimePolicy::BlockedByFirewall);
    }

    #[test]
    fn windows_disabled_or_block_all_firewall_never_starts_lan() {
        for firewall in [
            FirewallDiagnosticState::FirewallDisabled,
            FirewallDiagnosticState::BlockAllInbound,
        ] {
            let decision = lan_runtime_policy_for(
                CurrentPlatform::Windows,
                true,
                Some(windows_snapshot(NetworkProfileState::Private, firewall)),
            );

            assert_eq!(decision, LanRuntimePolicy::BlockedByFirewall);
        }
    }

    #[test]
    fn windows_private_profile_and_exact_rules_allow_lan() {
        let decision = lan_runtime_policy_for(
            CurrentPlatform::Windows,
            true,
            Some(windows_snapshot(
                NetworkProfileState::Private,
                FirewallDiagnosticState::Ready,
            )),
        );

        assert!(decision.should_run());
    }

    #[test]
    fn macos_allows_contextual_permission_attempt_after_consent() {
        let snapshot = ConnectivityPlatformSnapshot {
            system_permission: SystemPermissionState::Unknown,
            network_profile: NetworkProfileState::NotApplicable,
            firewall: FirewallDiagnosticState::NotApplicable,
            firewall_helper: FirewallHelperState::NotApplicable,
        };

        assert_eq!(
            lan_runtime_policy_for(CurrentPlatform::MacOs, true, Some(snapshot)),
            LanRuntimePolicy::Allowed
        );
    }

    #[test]
    fn unsupported_platform_keeps_lan_stopped() {
        let snapshot = ConnectivityPlatformSnapshot {
            system_permission: SystemPermissionState::NotApplicable,
            network_profile: NetworkProfileState::NotApplicable,
            firewall: FirewallDiagnosticState::Unsupported,
            firewall_helper: FirewallHelperState::Unsupported,
        };

        assert_eq!(
            lan_runtime_policy_for(CurrentPlatform::Other, true, Some(snapshot)),
            LanRuntimePolicy::Unsupported
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_defers_permission_to_runtime_connectivity() {
        assert_eq!(
            diagnose(),
            ConnectivityPlatformSnapshot {
                system_permission: SystemPermissionState::Unknown,
                network_profile: NetworkProfileState::NotApplicable,
                firewall: FirewallDiagnosticState::NotApplicable,
                firewall_helper: FirewallHelperState::NotApplicable,
            }
        );
        assert_eq!(
            install_firewall_rules(),
            Err(FirewallActionError::Unsupported)
        );
    }
}
