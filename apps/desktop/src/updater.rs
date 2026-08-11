//! Signed desktop update checks and explicitly confirmed installation.
//!
//! Network and installer operations in this module are blocking. Callers must run
//! them behind the desktop worker's blocking boundary, never in a Tauri command
//! or directly on a Tokio executor thread.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "windows")]
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    mem::{size_of, size_of_val},
    os::windows::ffi::{OsStrExt, OsStringExt},
    os::windows::fs::OpenOptionsExt,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
};

#[cfg(target_os = "windows")]
use airwiki_windows_firewall::{
    PublisherTrustError, verify_open_artifact_publisher_matches_current_executable,
};
use async_trait::async_trait;
use semver::Version;
use tauri::AppHandle;
use tauri_plugin_updater::{Error as TauriUpdaterError, Update as TauriUpdate, UpdaterExt};
use thiserror::Error;
use url::Url;
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{
        CloseHandle, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS,
        SetHandleInformation,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_TYPE_DISK,
        GetFileInformationByHandle, GetFileType,
    },
    System::{
        ApplicationInstallationAndServicing::{
            MSIDBOPEN_READONLY, MSIHANDLE, MsiCloseHandle, MsiDatabaseOpenViewW, MsiOpenDatabaseW,
            MsiRecordGetStringW, MsiViewExecute, MsiViewFetch,
        },
        SystemInformation::GetSystemDirectoryW,
        Threading::{
            CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
            InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTUPINFOEXW, STARTUPINFOW,
            UpdateProcThreadAttribute,
        },
    },
};
#[cfg(all(target_os = "windows", test))]
use windows::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::WaitForSingleObject,
};
#[cfg(target_os = "windows")]
use windows::core::{PCWSTR, PWSTR};

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(10 * 60);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CHECK_JITTER: Duration = Duration::from_secs(30 * 60);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RELEASE_NOTES_CHARS: usize = 4_096;
#[cfg(any(target_os = "windows", test))]
const WINDOWS_INSTALLER_ARGS: [&str; 4] = [
    "/passive",
    "/norestart",
    "AUTOLAUNCHAPP=1",
    "LAUNCHAPPARGS=/AIRWIKIUPDATE",
];

const COMPILED_ENDPOINT: Option<&str> = option_env!("AIRWIKI_UPDATE_ENDPOINT");
const COMPILED_PUBLIC_KEY: Option<&str> = option_env!("AIRWIKI_UPDATER_PUBLIC_KEY");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdaterDisabledReason {
    NotConfigured,
    InvalidEndpoint,
    InvalidPublicKey,
    InvalidCurrentVersion,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdaterBuildConfig {
    endpoint: Url,
    public_key: String,
}

impl UpdaterBuildConfig {
    pub(crate) fn from_compile_time() -> Result<Self, UpdaterDisabledReason> {
        Self::from_values(
            COMPILED_ENDPOINT,
            COMPILED_PUBLIC_KEY,
            cfg!(any(target_os = "macos", target_os = "windows")),
        )
    }

    fn from_values(
        endpoint: Option<&str>,
        public_key: Option<&str>,
        supported_platform: bool,
    ) -> Result<Self, UpdaterDisabledReason> {
        if !supported_platform {
            return Err(UpdaterDisabledReason::UnsupportedPlatform);
        }

        let endpoint = endpoint
            .filter(|value| !value.trim().is_empty())
            .ok_or(UpdaterDisabledReason::NotConfigured)?;
        let endpoint = Url::parse(endpoint).map_err(|_| UpdaterDisabledReason::InvalidEndpoint)?;
        if endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(UpdaterDisabledReason::InvalidEndpoint);
        }

        let public_key = public_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(UpdaterDisabledReason::InvalidPublicKey)?;

        Ok(Self {
            endpoint,
            public_key: public_key.to_owned(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpdateSchedule {
    enabled: bool,
    next_check_at: Option<Instant>,
}

impl UpdateSchedule {
    pub(crate) fn new(now: Instant, enabled: bool) -> Self {
        Self {
            enabled,
            next_check_at: enabled.then(|| add_duration(now, FIRST_CHECK_DELAY)),
        }
    }

    pub(crate) fn is_due(self, now: Instant) -> bool {
        self.next_check_at.is_some_and(|deadline| now >= deadline)
    }

    #[cfg(test)]
    pub(crate) fn time_until_next_check(self, now: Instant) -> Option<Duration> {
        self.next_check_at
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    pub(crate) fn record_attempt(&mut self, now: Instant, jitter: Duration) {
        if !self.enabled {
            return;
        }
        let bounded_jitter = jitter.min(MAX_CHECK_JITTER);
        self.next_check_at = Some(add_duration(
            now,
            CHECK_INTERVAL.saturating_add(bounded_jitter),
        ));
    }

    pub(crate) fn set_enabled(&mut self, now: Instant, enabled: bool) {
        if enabled == self.enabled {
            return;
        }
        self.enabled = enabled;
        self.next_check_at = enabled.then(|| add_duration(now, FIRST_CHECK_DELAY));
    }
}

fn add_duration(now: Instant, duration: Duration) -> Instant {
    now.checked_add(duration).unwrap_or(now)
}

pub(crate) fn schedule_jitter(now: SystemTime) -> Duration {
    let elapsed = now.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let Ok(upper_bound_millis) = u64::try_from(MAX_CHECK_JITTER.as_millis()) else {
        return Duration::ZERO;
    };
    if upper_bound_millis == 0 {
        return Duration::ZERO;
    }
    let bounded = elapsed.as_millis() % u128::from(upper_bound_millis);
    let Ok(bounded) = u64::try_from(bounded) else {
        return Duration::ZERO;
    };
    Duration::from_millis(bounded)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateSummary {
    pub(crate) version: String,
    pub(crate) release_notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateIssueCode {
    Offline,
    InvalidManifest,
    InvalidSignature,
    Unsupported,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpdateIssue {
    pub(crate) code: UpdateIssueCode,
    pub(crate) retryable: bool,
}

impl UpdateIssue {
    fn new(code: UpdateIssueCode) -> Self {
        Self {
            code,
            retryable: matches!(code, UpdateIssueCode::Offline | UpdateIssueCode::Internal),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdaterStatus {
    Idle,
    Checking,
    UpToDate,
    Available(UpdateSummary),
    Downloading(UpdateSummary),
    ReadyToInstall(UpdateSummary),
    Installing(UpdateSummary),
    Installed(UpdateSummary),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdaterView {
    pub(crate) status: UpdaterStatus,
    pub(crate) last_issue: Option<UpdateIssue>,
}

#[async_trait]
pub(crate) trait UpdateBackend: Send {
    async fn check(&mut self) -> Result<Option<UpdateSummary>, UpdateIssue>;
    async fn download(&mut self, expected_version: &str) -> Result<(), UpdateIssue>;
    async fn install(&mut self, expected_version: &str) -> Result<(), UpdateIssue>;
}

#[cfg(any(target_os = "windows", test))]
trait UpdatePackageVerifier<P> {
    fn verify(&self, package: &P) -> Result<(), UpdateIssue>;
}

#[cfg(any(target_os = "windows", test))]
trait UpdatePackageLauncher<P> {
    fn launch(&self, package: P) -> Result<(), UpdateIssue>;
}

#[cfg(any(target_os = "windows", test))]
fn verify_and_launch_package<P, V, L>(
    package: P,
    verifier: &V,
    launcher: &L,
) -> Result<(), UpdateIssue>
where
    V: UpdatePackageVerifier<P>,
    L: UpdatePackageLauncher<P>,
{
    verifier.verify(&package)?;
    launcher.launch(package)
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowsMsiVersion {
    major: u8,
    minor: u8,
    build: u16,
}

#[cfg(any(target_os = "windows", test))]
impl WindowsMsiVersion {
    fn from_stable_semver(version: &Version) -> Result<Self, WindowsVersionValidationError> {
        if !version.pre.is_empty() || !version.build.is_empty() {
            return Err(WindowsVersionValidationError::InvalidManifest);
        }
        Ok(Self {
            major: u8::try_from(version.major)
                .map_err(|_| WindowsVersionValidationError::InvalidManifest)?,
            minor: u8::try_from(version.minor)
                .map_err(|_| WindowsVersionValidationError::InvalidManifest)?,
            build: u16::try_from(version.patch)
                .map_err(|_| WindowsVersionValidationError::InvalidManifest)?,
        })
    }

    fn parse(value: &str) -> Result<Self, WindowsVersionValidationError> {
        let version = Version::parse(value)
            .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)?;
        Self::from_stable_semver(&version)
            .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsVersionValidationError {
    InvalidManifest,
    NotNewer,
    InvalidEmbeddedResource,
    VersionMismatch,
}

#[cfg(any(target_os = "windows", test))]
fn expected_windows_update_version(
    manifest_version: &str,
    current_version: &str,
) -> Result<WindowsMsiVersion, WindowsVersionValidationError> {
    let expected = Version::parse(manifest_version)
        .map_err(|_| WindowsVersionValidationError::InvalidManifest)?;
    let current = Version::parse(current_version)
        .map_err(|_| WindowsVersionValidationError::InvalidManifest)?;
    if !expected.pre.is_empty() || !expected.build.is_empty() {
        return Err(WindowsVersionValidationError::InvalidManifest);
    }
    if expected <= current {
        return Err(WindowsVersionValidationError::NotNewer);
    }
    WindowsMsiVersion::from_stable_semver(&expected)
}

#[cfg(any(target_os = "windows", test))]
fn validate_windows_msi_version(
    expected: WindowsMsiVersion,
    embedded: &str,
) -> Result<(), WindowsVersionValidationError> {
    if WindowsMsiVersion::parse(embedded)? != expected {
        return Err(WindowsVersionValidationError::VersionMismatch);
    }
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn windows_version_issue(error: WindowsVersionValidationError) -> UpdateIssue {
    let code = match error {
        WindowsVersionValidationError::InvalidManifest
        | WindowsVersionValidationError::NotNewer => UpdateIssueCode::InvalidManifest,
        WindowsVersionValidationError::InvalidEmbeddedResource
        | WindowsVersionValidationError::VersionMismatch => UpdateIssueCode::InvalidSignature,
    };
    UpdateIssue::new(code)
}

#[cfg(target_os = "windows")]
struct LockedWindowsUpdatePackage {
    installer: File,
    directory_guard: File,
    directory: tempfile::TempDir,
    installer_path: PathBuf,
}

#[cfg(target_os = "windows")]
impl LockedWindowsUpdatePackage {
    fn stage(package: &[u8]) -> Result<Self, UpdateIssue> {
        let directory = tempfile::Builder::new()
            .prefix("airwiki-update-")
            .tempdir()
            .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
        let directory_guard = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
            .open(directory.path())
            .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
        validate_windows_file_handle(&directory_guard, true)?;
        let installer_path = directory.path().join("airwiki-update.msi");
        let mut writable_installer = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(&installer_path)
            .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
        validate_windows_file_handle(&writable_installer, false)?;
        writable_installer
            .write_all(package)
            .and_then(|()| writable_installer.sync_all())
            .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
        drop(writable_installer);

        // Reopen the completed artifact without write/delete sharing. The exact
        // bytes are compared after the only handle capable of writing is gone;
        // this read-only handle then remains locked through trust verification.
        let mut installer = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&installer_path)
            .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
        validate_windows_file_handle(&installer, false)?;
        compare_staged_package(&mut installer, package)?;

        Ok(Self {
            installer,
            directory_guard,
            directory,
            installer_path,
        })
    }

    fn file(&self) -> &File {
        &self.installer
    }

    fn path(&self) -> &Path {
        &self.installer_path
    }

    fn preserve_after_launch(self) -> PathBuf {
        let Self {
            installer,
            directory_guard,
            directory,
            installer_path: _,
        } = self;
        drop(installer);
        drop(directory_guard);
        directory.keep()
    }
}

#[cfg(target_os = "windows")]
fn compare_staged_package(installer: &mut File, expected: &[u8]) -> Result<(), UpdateIssue> {
    let expected_len =
        u64::try_from(expected.len()).map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
    let actual_len = installer
        .metadata()
        .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?
        .len();
    if actual_len != expected_len {
        return Err(UpdateIssue::new(UpdateIssueCode::InvalidSignature));
    }

    installer
        .seek(SeekFrom::Start(0))
        .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
    let mut offset = 0;
    let mut buffer = [0_u8; 64 * 1024];
    while offset < expected.len() {
        let remaining = expected.len() - offset;
        let chunk_len = remaining.min(buffer.len());
        installer
            .read_exact(&mut buffer[..chunk_len])
            .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
        if buffer[..chunk_len] != expected[offset..offset + chunk_len] {
            return Err(UpdateIssue::new(UpdateIssueCode::InvalidSignature));
        }
        offset += chunk_len;
    }
    installer
        .seek(SeekFrom::Start(0))
        .map(|_| ())
        .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))
}

#[cfg(target_os = "windows")]
fn validate_windows_file_handle(file: &File, expected_directory: bool) -> Result<(), UpdateIssue> {
    let handle = HANDLE(file.as_raw_handle());
    // SAFETY: `handle` is borrowed from a live `File` for these synchronous,
    // read-only queries and remains valid until both calls return.
    if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
        return Err(UpdateIssue::new(UpdateIssueCode::Internal));
    }
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `information` is writable for the call and `handle` remains live.
    unsafe { GetFileInformationByHandle(handle, &mut information) }
        .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
    let attributes = information.dwFileAttributes;
    let is_directory = attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    let is_reparse_point = attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
    if is_directory != expected_directory || is_reparse_point {
        return Err(UpdateIssue::new(UpdateIssueCode::Internal));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn read_locked_windows_msi_version(
    package: &LockedWindowsUpdatePackage,
) -> Result<String, WindowsVersionValidationError> {
    const PRODUCT_VERSION_QUERY: &str =
        "SELECT `Value` FROM `Property` WHERE `Property` = 'ProductVersion'";
    const MAX_PRODUCT_VERSION_UNITS: usize = 64;

    // Windows Installer opens by path. The live file and directory handles deny
    // write, delete and parent replacement, so this path remains bound to the
    // already-compared MSI for the complete query.
    if !package.path().is_absolute() {
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }
    let path = nul_terminated_windows_path(package.path())
        .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)?;
    let query = nul_terminated_windows_text(PRODUCT_VERSION_QUERY)
        .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)?;
    let database = open_msi_handle(|handle| {
        // SAFETY: both PCWSTR values are NUL-terminated and live for the call;
        // handle is writable and becomes owned only on ERROR_SUCCESS.
        unsafe { MsiOpenDatabaseW(PCWSTR(path.as_ptr()), MSIDBOPEN_READONLY, handle) }
    })?;
    let view = open_msi_handle(|handle| {
        // SAFETY: database is a live MSI database handle, query is NUL-terminated,
        // and handle is writable and owned only on ERROR_SUCCESS.
        unsafe { MsiDatabaseOpenViewW(database.raw(), PCWSTR(query.as_ptr()), handle) }
    })?;
    // SAFETY: view is a live SELECT view and a null record is required for a
    // query without parameters.
    ensure_msi_success(unsafe { MsiViewExecute(view.raw(), MSIHANDLE(0)) })?;
    let record = open_msi_handle(|handle| {
        // SAFETY: view is live and handle receives the first fetched record.
        unsafe { MsiViewFetch(view.raw(), handle) }
    })?;
    let mut buffer = [0_u16; MAX_PRODUCT_VERSION_UNITS];
    let mut length = u32::try_from(buffer.len())
        .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)?;
    // SAFETY: record is live, field 1 is the selected Value column, and buffer
    // has `length` writable UTF-16 units.
    ensure_msi_success(unsafe {
        MsiRecordGetStringW(
            record.raw(),
            1,
            Some(PWSTR(buffer.as_mut_ptr())),
            Some(&mut length),
        )
    })?;
    let length = usize::try_from(length)
        .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)?;
    if length == 0 || length >= buffer.len() {
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }
    let mut unexpected_record = MSIHANDLE(0);
    // SAFETY: view remains live; a second successful row would make the property
    // query ambiguous and is rejected below.
    let second_status = unsafe { MsiViewFetch(view.raw(), &mut unexpected_record) };
    if second_status == ERROR_SUCCESS.0 {
        // SAFETY: a successful fetch returns a caller-owned MSI record handle.
        let _ = unsafe { MsiCloseHandle(unexpected_record) };
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }
    if second_status != ERROR_NO_MORE_ITEMS.0 {
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }
    String::from_utf16(&buffer[..length])
        .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)
}

#[cfg(target_os = "windows")]
struct OwnedMsiHandle(MSIHANDLE);

#[cfg(target_os = "windows")]
impl OwnedMsiHandle {
    fn raw(&self) -> MSIHANDLE {
        self.0
    }
}

#[cfg(target_os = "windows")]
impl Drop for OwnedMsiHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper is the sole owner of the nonzero MSI handle.
        let _ = unsafe { MsiCloseHandle(self.0) };
    }
}

#[cfg(target_os = "windows")]
fn open_msi_handle(
    open: impl FnOnce(*mut MSIHANDLE) -> u32,
) -> Result<OwnedMsiHandle, WindowsVersionValidationError> {
    let mut handle = MSIHANDLE(0);
    ensure_msi_success(open(&mut handle))?;
    if handle.0 == 0 {
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }
    Ok(OwnedMsiHandle(handle))
}

#[cfg(target_os = "windows")]
fn ensure_msi_success(status: u32) -> Result<(), WindowsVersionValidationError> {
    if status == ERROR_SUCCESS.0 {
        Ok(())
    } else {
        Err(WindowsVersionValidationError::InvalidEmbeddedResource)
    }
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct NativeWindowsUpdateVerifier {
    expected_version: WindowsMsiVersion,
}

#[cfg(target_os = "windows")]
impl UpdatePackageVerifier<LockedWindowsUpdatePackage> for NativeWindowsUpdateVerifier {
    fn verify(&self, package: &LockedWindowsUpdatePackage) -> Result<(), UpdateIssue> {
        verify_open_artifact_publisher_matches_current_executable(package.file(), package.path())
            .map_err(publisher_trust_issue)?;
        let embedded_version =
            read_locked_windows_msi_version(package).map_err(windows_version_issue)?;
        validate_windows_msi_version(self.expected_version, &embedded_version)
            .map_err(windows_version_issue)
    }
}

#[cfg(target_os = "windows")]
#[derive(Debug, Default)]
struct DirectWindowsUpdateLauncher;

#[cfg(target_os = "windows")]
impl UpdatePackageLauncher<LockedWindowsUpdatePackage> for DirectWindowsUpdateLauncher {
    fn launch(&self, package: LockedWindowsUpdatePackage) -> Result<(), UpdateIssue> {
        let installer = package.path().as_os_str();
        let arguments = std::iter::once(OsStr::new("/i"))
            .chain(std::iter::once(installer))
            .chain(WINDOWS_INSTALLER_ARGS.iter().map(OsStr::new))
            .collect::<Vec<_>>();
        let msiexec = trusted_windows_installer_path()?;
        let child = launch_locked_windows_process(&package, &msiexec, &arguments)?;
        let _persisted_directory = package.preserve_after_launch();
        drop(child);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn launch_locked_windows_process(
    package: &LockedWindowsUpdatePackage,
    application_path: &Path,
    arguments: &[&OsStr],
) -> Result<WindowsChildProcess, UpdateIssue> {
    let application = nul_terminated_windows_path(application_path)?;
    let mut command_line = windows_command_line(application_path, arguments)?;
    let inherited_handles =
        InheritableWindowsHandles::new(package.file(), &package.directory_guard)?;
    let attribute_list = ProcThreadAttributeList::new(inherited_handles.as_slice())?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>())
        .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
    startup.lpAttributeList = attribute_list.raw();
    let mut process_information = PROCESS_INFORMATION::default();

    // SAFETY: every pointer references initialized storage that remains live and
    // unmoved for the call. The mutable command line is NUL-terminated as required
    // by CreateProcessW. Only the two validated, explicitly inheritable package
    // handles are present in the process attribute list.
    unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            true,
            EXTENDED_STARTUPINFO_PRESENT,
            None,
            PCWSTR::null(),
            std::ptr::from_ref(&startup).cast::<STARTUPINFOW>(),
            &mut process_information,
        )
    }
    .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;

    let process = OwnedWindowsHandle(process_information.hProcess);
    let thread = OwnedWindowsHandle(process_information.hThread);
    drop(thread);
    Ok(WindowsChildProcess { _process: process })
}

#[cfg(target_os = "windows")]
struct InheritableWindowsHandles<'a> {
    handles: Box<[HANDLE; 2]>,
    _installer: &'a File,
    _directory: &'a File,
}

#[cfg(target_os = "windows")]
impl<'a> InheritableWindowsHandles<'a> {
    fn new(installer: &'a File, directory: &'a File) -> Result<Self, UpdateIssue> {
        let handles = Box::new([
            HANDLE(installer.as_raw_handle()),
            HANDLE(directory.as_raw_handle()),
        ]);
        for (enabled, handle) in handles.iter().copied().enumerate() {
            if set_windows_handle_inheritance(handle, true).is_err() {
                for enabled_handle in handles[..enabled].iter().copied() {
                    let _ = set_windows_handle_inheritance(enabled_handle, false);
                }
                return Err(UpdateIssue::new(UpdateIssueCode::Internal));
            }
        }
        Ok(Self {
            handles,
            _installer: installer,
            _directory: directory,
        })
    }

    fn as_slice(&self) -> &[HANDLE] {
        self.handles.as_slice()
    }
}

#[cfg(target_os = "windows")]
impl Drop for InheritableWindowsHandles<'_> {
    fn drop(&mut self) {
        for handle in self.handles.iter().copied() {
            let _ = set_windows_handle_inheritance(handle, false);
        }
    }
}

#[cfg(target_os = "windows")]
fn set_windows_handle_inheritance(handle: HANDLE, enabled: bool) -> Result<(), UpdateIssue> {
    let flags = if enabled {
        HANDLE_FLAG_INHERIT
    } else {
        HANDLE_FLAGS::default()
    };
    // SAFETY: the handle comes from a live File held by
    // InheritableWindowsHandles for the entire mutation interval.
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, flags) }
        .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))
}

#[cfg(target_os = "windows")]
struct ProcThreadAttributeList {
    raw: LPPROC_THREAD_ATTRIBUTE_LIST,
    _storage: Box<[usize]>,
}

#[cfg(target_os = "windows")]
impl ProcThreadAttributeList {
    fn new(handles: &[HANDLE]) -> Result<Self, UpdateIssue> {
        let mut required_bytes = 0_usize;
        // SAFETY: a null first call is the documented size query; required_bytes
        // is writable for the duration of the call.
        let _size_query =
            unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut required_bytes) };
        if required_bytes == 0 {
            return Err(UpdateIssue::new(UpdateIssueCode::Internal));
        }
        let words = required_bytes.div_ceil(size_of::<usize>());
        let mut storage = vec![0_usize; words].into_boxed_slice();
        let raw = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        // SAFETY: storage is suitably aligned, sized from the preceding API query,
        // and remains pinned in its Box until the attribute list is deleted.
        unsafe { InitializeProcThreadAttributeList(Some(raw), 1, None, &mut required_bytes) }
            .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
        let list = Self {
            raw,
            _storage: storage,
        };
        // SAFETY: handles points to a stable Box owned by the inheritance guard and
        // remains live until after CreateProcessW returns. The byte count exactly
        // describes the handle slice.
        unsafe {
            UpdateProcThreadAttribute(
                list.raw,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                size_of_val(handles),
                None,
                None,
            )
        }
        .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
        Ok(list)
    }

    fn raw(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.raw
    }
}

#[cfg(target_os = "windows")]
impl Drop for ProcThreadAttributeList {
    fn drop(&mut self) {
        // SAFETY: raw was initialized successfully and is deleted exactly once
        // before its backing storage is released.
        unsafe { DeleteProcThreadAttributeList(self.raw) };
    }
}

#[cfg(target_os = "windows")]
struct OwnedWindowsHandle(HANDLE);

#[cfg(target_os = "windows")]
impl Drop for OwnedWindowsHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: this wrapper is the sole owner of the process/thread handle
            // returned by CreateProcessW and closes it exactly once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(target_os = "windows")]
struct WindowsChildProcess {
    _process: OwnedWindowsHandle,
}

#[cfg(all(target_os = "windows", test))]
impl WindowsChildProcess {
    fn is_running(&self) -> bool {
        // SAFETY: process remains owned and live for this non-blocking wait.
        (unsafe { WaitForSingleObject(self._process.0, 0) }) == WAIT_TIMEOUT
    }

    fn wait(&self, timeout: Duration) -> Result<(), UpdateIssue> {
        let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: process remains owned and live for the bounded wait.
        let result = unsafe { WaitForSingleObject(self._process.0, milliseconds) };
        if result == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(UpdateIssue::new(UpdateIssueCode::Internal))
        }
    }
}

#[cfg(target_os = "windows")]
fn nul_terminated_windows_path(path: &Path) -> Result<Vec<u16>, UpdateIssue> {
    nul_terminated_windows_units(path.as_os_str())
}

#[cfg(target_os = "windows")]
fn nul_terminated_windows_text(value: &str) -> Result<Vec<u16>, UpdateIssue> {
    nul_terminated_windows_units(OsStr::new(value))
}

#[cfg(target_os = "windows")]
fn nul_terminated_windows_units(value: &OsStr) -> Result<Vec<u16>, UpdateIssue> {
    let mut encoded = value.encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(UpdateIssue::new(UpdateIssueCode::Internal));
    }
    encoded.push(0);
    Ok(encoded)
}

#[cfg(target_os = "windows")]
fn trusted_windows_installer_path() -> Result<PathBuf, UpdateIssue> {
    let required = unsafe { GetSystemDirectoryW(None) };
    if required == 0 {
        return Err(UpdateIssue::new(UpdateIssueCode::Internal));
    }
    let capacity = usize::try_from(required)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| UpdateIssue::new(UpdateIssueCode::Internal))?;
    let mut buffer = vec![0_u16; capacity];
    // SAFETY: buffer is writable for its complete length. The returned count
    // excludes the terminating NUL when the call succeeds.
    let written = unsafe { GetSystemDirectoryW(Some(&mut buffer)) };
    let written =
        usize::try_from(written).map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
    if written == 0 || written >= buffer.len() {
        return Err(UpdateIssue::new(UpdateIssueCode::Internal));
    }
    let system_directory = PathBuf::from(OsString::from_wide(&buffer[..written]));
    let installer = system_directory.join("msiexec.exe");
    let metadata = installer
        .metadata()
        .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?;
    if !metadata.is_file() {
        return Err(UpdateIssue::new(UpdateIssueCode::Internal));
    }
    Ok(installer)
}

#[cfg(target_os = "windows")]
fn windows_command_line(path: &Path, arguments: &[&OsStr]) -> Result<Vec<u16>, UpdateIssue> {
    const MAX_COMMAND_LINE_UNITS: usize = 32_767;

    let executable = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut command_line = Vec::new();
    push_quoted_windows_argument(&mut command_line, &executable)?;
    for argument in arguments {
        command_line.push(u16::from(b' '));
        let encoded = argument.encode_wide().collect::<Vec<_>>();
        push_quoted_windows_argument(&mut command_line, &encoded)?;
    }
    if command_line.len() >= MAX_COMMAND_LINE_UNITS {
        return Err(UpdateIssue::new(UpdateIssueCode::Internal));
    }
    command_line.push(0);
    Ok(command_line)
}

#[cfg(target_os = "windows")]
fn push_quoted_windows_argument(
    command_line: &mut Vec<u16>,
    argument: &[u16],
) -> Result<(), UpdateIssue> {
    const BACKSLASH: u16 = b'\\' as u16;
    const QUOTE: u16 = b'"' as u16;

    if argument.contains(&0) {
        return Err(UpdateIssue::new(UpdateIssueCode::Internal));
    }
    command_line.push(QUOTE);
    let mut backslashes = 0_usize;
    for unit in argument.iter().copied() {
        match unit {
            BACKSLASH => backslashes += 1,
            QUOTE => {
                command_line.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2 + 1));
                command_line.push(QUOTE);
                backslashes = 0;
            }
            _ => {
                command_line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
                command_line.push(unit);
                backslashes = 0;
            }
        }
    }
    command_line.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
    command_line.push(QUOTE);
    Ok(())
}

#[cfg(target_os = "windows")]
fn install_windows_platform_update(version: &str, package: Vec<u8>) -> Result<(), UpdateIssue> {
    let expected_version = expected_windows_update_version(version, env!("CARGO_PKG_VERSION"))
        .map_err(windows_version_issue)?;
    let package = LockedWindowsUpdatePackage::stage(&package)?;
    let verifier = NativeWindowsUpdateVerifier { expected_version };
    verify_and_launch_package(package, &verifier, &DirectWindowsUpdateLauncher)
}

#[cfg(not(target_os = "windows"))]
fn install_tauri_platform_update(update: TauriUpdate, package: Vec<u8>) -> Result<(), UpdateIssue> {
    update.install(package).map_err(tauri_updater_issue)
}

#[cfg(target_os = "windows")]
fn install_tauri_platform_update(update: TauriUpdate, package: Vec<u8>) -> Result<(), UpdateIssue> {
    install_windows_platform_update(&update.version, package)
}

#[cfg(target_os = "windows")]
fn publisher_trust_issue(error: PublisherTrustError) -> UpdateIssue {
    let code = match error {
        PublisherTrustError::Unsupported => UpdateIssueCode::Unsupported,
        PublisherTrustError::InvalidLayout
        | PublisherTrustError::Untrusted
        | PublisherTrustError::PublisherMismatch => UpdateIssueCode::InvalidSignature,
        PublisherTrustError::InspectionFailed => UpdateIssueCode::Internal,
    };
    UpdateIssue::new(code)
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateActionError {
    #[error("the update action is not valid in the current state")]
    InvalidState,
    #[error("the update confirmation is stale")]
    StaleConfirmation,
}

#[derive(Debug)]
pub(crate) struct DownloadConfirmation {
    generation: u64,
    version: String,
}

#[derive(Debug)]
pub(crate) struct InstallConfirmation {
    generation: u64,
    version: String,
}

pub(crate) struct UpdaterService {
    backend: Box<dyn UpdateBackend>,
    generation: u64,
    view: UpdaterView,
}

impl UpdaterService {
    #[cfg(test)]
    pub(crate) fn new(backend: impl UpdateBackend + 'static) -> Self {
        Self::from_boxed(Box::new(backend))
    }

    pub(crate) fn from_boxed(backend: Box<dyn UpdateBackend>) -> Self {
        Self {
            backend,
            generation: 0,
            view: UpdaterView {
                status: UpdaterStatus::Idle,
                last_issue: None,
            },
        }
    }

    pub(crate) fn view(&self) -> &UpdaterView {
        &self.view
    }

    pub(crate) async fn check(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.view.status = UpdaterStatus::Checking;
        self.view.last_issue = None;

        match self.backend.check().await {
            Ok(Some(update)) => self.view.status = UpdaterStatus::Available(update),
            Ok(None) => self.view.status = UpdaterStatus::UpToDate,
            Err(issue) => {
                self.view.status = UpdaterStatus::Idle;
                self.view.last_issue = Some(issue);
            }
        }
    }

    pub(crate) fn confirm_download(&self) -> Result<DownloadConfirmation, UpdateActionError> {
        let UpdaterStatus::Available(update) = &self.view.status else {
            return Err(UpdateActionError::InvalidState);
        };
        Ok(DownloadConfirmation {
            generation: self.generation,
            version: update.version.clone(),
        })
    }

    pub(crate) async fn download(
        &mut self,
        confirmation: DownloadConfirmation,
    ) -> Result<(), UpdateActionError> {
        let update =
            self.validated_available_update(confirmation.generation, &confirmation.version)?;
        self.view.status = UpdaterStatus::Downloading(update.clone());
        self.view.last_issue = None;

        match self.backend.download(&update.version).await {
            Ok(()) => self.view.status = UpdaterStatus::ReadyToInstall(update),
            Err(issue) => {
                self.view.status = UpdaterStatus::Available(update);
                self.view.last_issue = Some(issue);
            }
        }
        Ok(())
    }

    pub(crate) fn confirm_install(&self) -> Result<InstallConfirmation, UpdateActionError> {
        let UpdaterStatus::ReadyToInstall(update) = &self.view.status else {
            return Err(UpdateActionError::InvalidState);
        };
        Ok(InstallConfirmation {
            generation: self.generation,
            version: update.version.clone(),
        })
    }

    pub(crate) async fn install(
        &mut self,
        confirmation: InstallConfirmation,
    ) -> Result<(), UpdateActionError> {
        let update = self.validated_ready_update(confirmation.generation, &confirmation.version)?;
        self.view.status = UpdaterStatus::Installing(update.clone());
        self.view.last_issue = None;

        match self.backend.install(&update.version).await {
            Ok(()) => self.view.status = UpdaterStatus::Installed(update),
            Err(issue) => {
                self.view.status = UpdaterStatus::Available(update);
                self.view.last_issue = Some(issue);
            }
        }
        Ok(())
    }

    fn validated_available_update(
        &self,
        generation: u64,
        version: &str,
    ) -> Result<UpdateSummary, UpdateActionError> {
        if generation != self.generation {
            return Err(UpdateActionError::StaleConfirmation);
        }
        let UpdaterStatus::Available(update) = &self.view.status else {
            return Err(UpdateActionError::InvalidState);
        };
        if update.version != version {
            return Err(UpdateActionError::StaleConfirmation);
        }
        Ok(update.clone())
    }

    fn validated_ready_update(
        &self,
        generation: u64,
        version: &str,
    ) -> Result<UpdateSummary, UpdateActionError> {
        if generation != self.generation {
            return Err(UpdateActionError::StaleConfirmation);
        }
        let UpdaterStatus::ReadyToInstall(update) = &self.view.status else {
            return Err(UpdateActionError::InvalidState);
        };
        if update.version != version {
            return Err(UpdateActionError::StaleConfirmation);
        }
        Ok(update.clone())
    }
}

pub(crate) struct TauriUpdateBackend {
    app: AppHandle,
    config: UpdaterBuildConfig,
    checked_update: Option<TauriUpdate>,
    downloaded_package: Option<Vec<u8>>,
}

impl TauriUpdateBackend {
    pub(crate) fn new(
        app: AppHandle,
        config: UpdaterBuildConfig,
    ) -> Result<Self, UpdaterDisabledReason> {
        Version::parse(env!("CARGO_PKG_VERSION"))
            .map_err(|_| UpdaterDisabledReason::InvalidCurrentVersion)?;
        Ok(Self {
            app,
            config,
            checked_update: None,
            downloaded_package: None,
        })
    }

    fn checked_update(&self, expected_version: &str) -> Result<&TauriUpdate, UpdateIssue> {
        self.checked_update
            .as_ref()
            .filter(|update| update.version == expected_version)
            .ok_or_else(|| UpdateIssue::new(UpdateIssueCode::Internal))
    }
}

#[async_trait]
impl UpdateBackend for TauriUpdateBackend {
    async fn check(&mut self) -> Result<Option<UpdateSummary>, UpdateIssue> {
        self.checked_update = None;
        self.downloaded_package = None;

        let updater = self
            .app
            .updater_builder()
            .endpoints(vec![self.config.endpoint.clone()])
            .map_err(tauri_updater_issue)?
            .pubkey(self.config.public_key.clone())
            .version_comparator(|current, release| {
                release.version.pre.is_empty() && release.version > current
            })
            .timeout(NETWORK_TIMEOUT)
            .build()
            .map_err(tauri_updater_issue)?;
        let update = updater.check().await.map_err(tauri_updater_issue)?;
        let Some(update) = update else {
            return Ok(None);
        };
        let summary = UpdateSummary {
            version: update.version.clone(),
            release_notes: update.body.as_deref().map(truncate_release_notes),
        };
        self.checked_update = Some(update);
        Ok(Some(summary))
    }

    async fn download(&mut self, expected_version: &str) -> Result<(), UpdateIssue> {
        let package = self
            .checked_update(expected_version)?
            .download(|_, _| {}, || {})
            .await
            .map_err(tauri_updater_issue)?;
        self.downloaded_package = Some(package);
        Ok(())
    }

    async fn install(&mut self, expected_version: &str) -> Result<(), UpdateIssue> {
        let update = self.checked_update(expected_version)?.clone();
        let package = self
            .downloaded_package
            .take()
            .ok_or_else(|| UpdateIssue::new(UpdateIssueCode::Internal))?;
        tokio::task::spawn_blocking(move || install_tauri_platform_update(update, package))
            .await
            .map_err(|_| UpdateIssue::new(UpdateIssueCode::Internal))?
    }
}

fn truncate_release_notes(notes: &str) -> String {
    notes
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .take(MAX_RELEASE_NOTES_CHARS)
        .collect()
}

fn tauri_updater_issue(error: TauriUpdaterError) -> UpdateIssue {
    let code = match error {
        TauriUpdaterError::Reqwest(error) if error.is_decode() => UpdateIssueCode::InvalidManifest,
        TauriUpdaterError::Reqwest(error) if error.is_connect() || error.is_timeout() => {
            UpdateIssueCode::Offline
        }
        TauriUpdaterError::Reqwest(_) | TauriUpdaterError::Network(_) => UpdateIssueCode::Offline,
        TauriUpdaterError::Serialization(_)
        | TauriUpdaterError::ReleaseNotFound
        | TauriUpdaterError::Semver(_)
        | TauriUpdaterError::TargetNotFound(_)
        | TauriUpdaterError::TargetsNotFound(_)
        | TauriUpdaterError::UrlParse(_)
        | TauriUpdaterError::EmptyEndpoints => UpdateIssueCode::InvalidManifest,
        TauriUpdaterError::Minisign(_)
        | TauriUpdaterError::Base64(_)
        | TauriUpdaterError::SignatureUtf8(_) => UpdateIssueCode::InvalidSignature,
        TauriUpdaterError::UnsupportedArch | TauriUpdaterError::UnsupportedOs => {
            UpdateIssueCode::Unsupported
        }
        _ => UpdateIssueCode::Internal,
    };
    UpdateIssue::new(code)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    struct FakeUpdatePackageVerifier {
        result: Result<(), UpdateIssue>,
        calls: Cell<usize>,
    }

    impl UpdatePackageVerifier<Vec<u8>> for FakeUpdatePackageVerifier {
        fn verify(&self, _package: &Vec<u8>) -> Result<(), UpdateIssue> {
            self.calls.set(self.calls.get() + 1);
            self.result
        }
    }

    struct FakeEmbeddedWindowsVersionVerifier {
        expected: WindowsMsiVersion,
        embedded: String,
        calls: Cell<usize>,
    }

    impl UpdatePackageVerifier<Vec<u8>> for FakeEmbeddedWindowsVersionVerifier {
        fn verify(&self, _package: &Vec<u8>) -> Result<(), UpdateIssue> {
            self.calls.set(self.calls.get() + 1);
            validate_windows_msi_version(self.expected, &self.embedded)
                .map_err(windows_version_issue)
        }
    }

    struct FakeUpdatePackageLauncher {
        result: Result<(), UpdateIssue>,
        calls: Cell<usize>,
        received: RefCell<Option<Vec<u8>>>,
    }

    impl UpdatePackageLauncher<Vec<u8>> for FakeUpdatePackageLauncher {
        fn launch(&self, package: Vec<u8>) -> Result<(), UpdateIssue> {
            self.calls.set(self.calls.get() + 1);
            self.received.replace(Some(package));
            self.result
        }
    }

    #[derive(Default)]
    struct FakeBackend {
        available: Option<UpdateSummary>,
        check_issue: Option<UpdateIssue>,
        download_issue: Option<UpdateIssue>,
        install_issue: Option<UpdateIssue>,
        checks: usize,
        downloads: usize,
        installs: usize,
    }

    #[async_trait]
    impl UpdateBackend for FakeBackend {
        async fn check(&mut self) -> Result<Option<UpdateSummary>, UpdateIssue> {
            self.checks += 1;
            if let Some(issue) = self.check_issue {
                return Err(issue);
            }
            Ok(self.available.clone())
        }

        async fn download(&mut self, _expected_version: &str) -> Result<(), UpdateIssue> {
            self.downloads += 1;
            self.download_issue.map_or(Ok(()), Err)
        }

        async fn install(&mut self, _expected_version: &str) -> Result<(), UpdateIssue> {
            self.installs += 1;
            self.install_issue.map_or(Ok(()), Err)
        }
    }

    fn available_update() -> UpdateSummary {
        UpdateSummary {
            version: "0.2.0".to_owned(),
            release_notes: Some("Safer maintenance".to_owned()),
        }
    }

    #[test]
    fn untrusted_native_package_should_never_reach_the_launcher() {
        let issue = UpdateIssue::new(UpdateIssueCode::InvalidSignature);
        let verifier = FakeUpdatePackageVerifier {
            result: Err(issue),
            calls: Cell::new(0),
        };
        let launcher = FakeUpdatePackageLauncher {
            result: Ok(()),
            calls: Cell::new(0),
            received: RefCell::new(None),
        };

        let result = verify_and_launch_package(vec![1, 2, 3], &verifier, &launcher);

        assert_eq!(result, Err(issue));
        assert_eq!(verifier.calls.get(), 1);
        assert_eq!(launcher.calls.get(), 0);
        assert_eq!(*launcher.received.borrow(), None);
    }

    #[test]
    fn trusted_native_package_should_launch_the_exact_guard_once() {
        let verifier = FakeUpdatePackageVerifier {
            result: Ok(()),
            calls: Cell::new(0),
        };
        let launcher = FakeUpdatePackageLauncher {
            result: Ok(()),
            calls: Cell::new(0),
            received: RefCell::new(None),
        };
        let package = vec![1, 3, 3, 7];

        let result = verify_and_launch_package(package.clone(), &verifier, &launcher);

        assert_eq!(result, Ok(()));
        assert_eq!(verifier.calls.get(), 1);
        assert_eq!(launcher.calls.get(), 1);
        assert_eq!(launcher.received.into_inner(), Some(package));
    }

    #[test]
    fn launcher_failure_should_be_propagated_after_one_verification() {
        let issue = UpdateIssue::new(UpdateIssueCode::Internal);
        let verifier = FakeUpdatePackageVerifier {
            result: Ok(()),
            calls: Cell::new(0),
        };
        let launcher = FakeUpdatePackageLauncher {
            result: Err(issue),
            calls: Cell::new(0),
            received: RefCell::new(None),
        };

        let result = verify_and_launch_package(vec![2, 4], &verifier, &launcher);

        assert_eq!(result, Err(issue));
        assert!(issue.retryable);
        assert_eq!(verifier.calls.get(), 1);
        assert_eq!(launcher.calls.get(), 1);
    }

    #[test]
    fn windows_launcher_arguments_are_fixed_and_request_clean_update_shutdown() {
        assert_eq!(
            WINDOWS_INSTALLER_ARGS,
            [
                "/passive",
                "/norestart",
                "AUTOLAUNCHAPP=1",
                "LAUNCHAPPARGS=/AIRWIKIUPDATE",
            ]
        );
    }

    #[test]
    fn windows_manifest_version_maps_to_msi_product_version() {
        let expected = expected_windows_update_version("9.2.5", "9.2.4").unwrap();

        assert_eq!(
            expected,
            WindowsMsiVersion {
                major: 9,
                minor: 2,
                build: 5,
            }
        );
    }

    #[test]
    fn windows_manifest_version_rejects_prerelease_non_numeric_build_and_downgrade() {
        assert_eq!(
            expected_windows_update_version("0.3.0-rc.1", "0.2.0"),
            Err(WindowsVersionValidationError::InvalidManifest)
        );
        assert_eq!(
            expected_windows_update_version("0.3.0+public.1", "0.2.0"),
            Err(WindowsVersionValidationError::InvalidManifest)
        );
        assert_eq!(
            expected_windows_update_version("0.3.0+5", "0.2.0"),
            Err(WindowsVersionValidationError::InvalidManifest)
        );
        assert_eq!(
            expected_windows_update_version("256.0.0", "0.2.0"),
            Err(WindowsVersionValidationError::InvalidManifest)
        );
        assert_eq!(
            expected_windows_update_version("0.1.9", "0.2.0"),
            Err(WindowsVersionValidationError::NotNewer)
        );
        assert_eq!(
            windows_version_issue(WindowsVersionValidationError::InvalidEmbeddedResource).code,
            UpdateIssueCode::InvalidSignature
        );
    }

    #[test]
    fn older_embedded_windows_version_is_rejected_before_launch() {
        let expected = expected_windows_update_version("9.0.0", "0.2.0").unwrap();
        let verifier = FakeEmbeddedWindowsVersionVerifier {
            expected,
            embedded: "8.0.0".to_owned(),
            calls: Cell::new(0),
        };
        let launcher = FakeUpdatePackageLauncher {
            result: Ok(()),
            calls: Cell::new(0),
            received: RefCell::new(None),
        };

        let result = verify_and_launch_package(vec![1, 2, 3], &verifier, &launcher);

        assert_eq!(
            result,
            Err(UpdateIssue::new(UpdateIssueCode::InvalidSignature))
        );
        assert_eq!(verifier.calls.get(), 1);
        assert_eq!(launcher.calls.get(), 0);
        assert_eq!(*launcher.received.borrow(), None);
    }

    #[test]
    fn msi_product_version_must_match_the_manifest() {
        let expected = expected_windows_update_version("9.1.2", "0.2.0").unwrap();

        assert_eq!(
            validate_windows_msi_version(expected, "9.1.1"),
            Err(WindowsVersionValidationError::VersionMismatch)
        );
        assert_eq!(validate_windows_msi_version(expected, "9.1.2"), Ok(()));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn inherited_windows_package_locks_survive_until_the_child_exits() {
        const CHILD_TEST: &str = "updater::tests::windows_inherited_handle_child";

        let current_executable = std::env::current_exe().unwrap();
        let executable_bytes = std::fs::read(current_executable).unwrap();
        let package = LockedWindowsUpdatePackage::stage(&executable_bytes).unwrap();
        let installer_path = package.path().to_path_buf();
        let directory_path = package.directory.path().to_path_buf();
        let renamed_file = directory_path.join("replacement.exe");
        let renamed_directory = directory_path.with_extension("renamed");
        let child_arguments = ["--ignored", "--exact", CHILD_TEST, "--nocapture"].map(OsStr::new);
        let child =
            launch_locked_windows_process(&package, &current_executable, &child_arguments).unwrap();
        assert!(child.is_running(), "the lock-holder child did not start");
        let persisted_directory = package.preserve_after_launch();
        assert_eq!(persisted_directory, directory_path);

        assert!(
            OpenOptions::new()
                .write(true)
                .open(&installer_path)
                .is_err()
        );
        assert!(std::fs::rename(&installer_path, &renamed_file).is_err());
        assert!(std::fs::remove_file(&installer_path).is_err());
        assert!(std::fs::rename(&directory_path, &renamed_directory).is_err());

        child.wait(Duration::from_secs(20)).unwrap();
        drop(child);
        std::fs::rename(&installer_path, &renamed_file).unwrap();
        std::fs::rename(&directory_path, &renamed_directory).unwrap();
        std::fs::remove_dir_all(&renamed_directory).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "launched by the inherited-handle integration test"]
    fn windows_inherited_handle_child() {
        std::thread::sleep(Duration::from_secs(3));
    }

    #[test]
    fn schedule_should_wait_ten_minutes_before_first_check() {
        let now = Instant::now();
        let schedule = UpdateSchedule::new(now, true);

        assert!(!schedule.is_due(now + FIRST_CHECK_DELAY - Duration::from_secs(1)));
        assert!(schedule.is_due(now + FIRST_CHECK_DELAY));
    }

    #[test]
    fn schedule_should_bound_daily_jitter() {
        let now = Instant::now();
        let mut schedule = UpdateSchedule::new(now, true);

        schedule.record_attempt(now, Duration::from_secs(12 * 60 * 60));

        assert_eq!(
            schedule.time_until_next_check(now),
            Some(CHECK_INTERVAL + MAX_CHECK_JITTER)
        );
    }

    #[test]
    fn disabled_schedule_should_never_be_due() {
        let now = Instant::now();
        let schedule = UpdateSchedule::new(now, false);

        assert!(!schedule.is_due(now + Duration::from_secs(365 * 24 * 60 * 60)));
    }

    #[test]
    fn disabled_schedule_should_ignore_an_in_flight_attempt() {
        let now = Instant::now();
        let mut schedule = UpdateSchedule::new(now, true);
        schedule.set_enabled(now, false);

        schedule.record_attempt(now, Duration::ZERO);

        assert_eq!(schedule.time_until_next_check(now), None);
    }

    #[test]
    fn build_config_should_reject_insecure_or_credentialed_endpoint() {
        let insecure = UpdaterBuildConfig::from_values(
            Some("http://updates.example.test/stable.json"),
            Some("public-key"),
            true,
        );
        let credentialed = UpdaterBuildConfig::from_values(
            Some("https://secret@updates.example.test/stable.json"),
            Some("public-key"),
            true,
        );

        assert_eq!(insecure, Err(UpdaterDisabledReason::InvalidEndpoint));
        assert_eq!(credentialed, Err(UpdaterDisabledReason::InvalidEndpoint));
    }

    #[test]
    fn build_config_should_stay_disabled_when_compile_values_are_absent() {
        let result = UpdaterBuildConfig::from_values(None, None, true);

        assert_eq!(result, Err(UpdaterDisabledReason::NotConfigured));
    }

    #[tokio::test]
    async fn update_should_require_separate_download_and_install_confirmations() {
        let backend = FakeBackend {
            available: Some(available_update()),
            ..FakeBackend::default()
        };
        let mut service = UpdaterService::new(backend);

        service.check().await;
        let download_confirmation = service.confirm_download().unwrap();
        service.download(download_confirmation).await.unwrap();
        assert!(matches!(
            service.view().status,
            UpdaterStatus::ReadyToInstall(_)
        ));

        let install_confirmation = service.confirm_install().unwrap();
        service.install(install_confirmation).await.unwrap();
        assert!(matches!(service.view().status, UpdaterStatus::Installed(_)));
    }

    #[tokio::test]
    async fn offline_check_should_be_recoverable_and_non_blocking() {
        let backend = FakeBackend {
            check_issue: Some(UpdateIssue::new(UpdateIssueCode::Offline)),
            ..FakeBackend::default()
        };
        let mut service = UpdaterService::new(backend);

        service.check().await;

        assert_eq!(service.view().status, UpdaterStatus::Idle);
        assert_eq!(
            service.view().last_issue,
            Some(UpdateIssue {
                code: UpdateIssueCode::Offline,
                retryable: true,
            })
        );
    }

    #[tokio::test]
    async fn stale_confirmation_should_not_download_a_different_update() {
        let backend = FakeBackend {
            available: Some(available_update()),
            ..FakeBackend::default()
        };
        let mut service = UpdaterService::new(backend);

        service.check().await;
        let confirmation = service.confirm_download().unwrap();
        service.check().await;

        assert_eq!(
            service.download(confirmation).await,
            Err(UpdateActionError::StaleConfirmation)
        );
    }

    #[tokio::test]
    async fn download_failure_should_keep_update_available_for_retry() {
        let backend = FakeBackend {
            available: Some(available_update()),
            download_issue: Some(UpdateIssue::new(UpdateIssueCode::Offline)),
            ..FakeBackend::default()
        };
        let mut service = UpdaterService::new(backend);

        service.check().await;
        let confirmation = service.confirm_download().unwrap();
        service.download(confirmation).await.unwrap();

        assert!(matches!(service.view().status, UpdaterStatus::Available(_)));
        assert_eq!(
            service.view().last_issue.map(|issue| issue.code),
            Some(UpdateIssueCode::Offline)
        );
    }

    #[test]
    fn release_notes_should_be_bounded_without_splitting_unicode() {
        let notes = format!("{}\u{0000}", "á".repeat(MAX_RELEASE_NOTES_CHARS + 3));

        let truncated = truncate_release_notes(&notes);

        assert_eq!(truncated.chars().count(), MAX_RELEASE_NOTES_CHARS);
        assert!(!truncated.contains('\u{0000}'));
    }
}
