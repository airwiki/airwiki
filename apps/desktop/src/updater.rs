//! Signed desktop update checks and explicitly confirmed installation.
//!
//! Network and installer operations in this module are blocking. Callers must run
//! them behind the desktop worker's blocking boundary, never on the egui thread
//! or directly on a Tokio executor thread.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "windows")]
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    mem::{size_of, size_of_val},
    os::windows::ffi::OsStrExt,
    os::windows::fs::OpenOptionsExt,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
};

#[cfg(target_os = "windows")]
use airwiki_windows_firewall::{
    PublisherTrustError, verify_open_artifact_publisher_matches_current_executable,
};
use cargo_packager_updater::{
    Config as PackagerConfig, Error as PackagerError, Update as PackagerUpdate, UpdaterBuilder,
    semver::Version, url::Url,
};
use thiserror::Error;
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation},
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_TYPE_DISK,
        FILE_VER_GET_NEUTRAL, GetFileInformationByHandle, GetFileType, GetFileVersionInfoExW,
        GetFileVersionInfoSizeExW, VS_FFI_SIGNATURE, VS_FFI_STRUCVERSION, VS_FIXEDFILEINFO,
        VerQueryValueW,
    },
    System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTUPINFOEXW, STARTUPINFOW,
        UpdateProcThreadAttribute,
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
const WINDOWS_INSTALLER_ARGS: [&str; 3] = ["/P", "/R", "/AIRWIKIUPDATE"];
#[cfg(target_os = "windows")]
const MAX_WINDOWS_VERSION_INFO_BYTES: u32 = 1024 * 1024;

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

pub(crate) trait UpdateBackend {
    fn check(&mut self) -> Result<Option<UpdateSummary>, UpdateIssue>;
    fn download(&mut self, expected_version: &str) -> Result<(), UpdateIssue>;
    fn install(&mut self, expected_version: &str) -> Result<(), UpdateIssue>;
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
struct WindowsExecutableVersion {
    major: u16,
    minor: u16,
    build: u16,
    private: u16,
}

#[cfg(any(target_os = "windows", test))]
impl WindowsExecutableVersion {
    fn from_stable_semver(version: &Version) -> Result<Self, WindowsVersionValidationError> {
        if !version.pre.is_empty() {
            return Err(WindowsVersionValidationError::InvalidManifest);
        }
        let private = if version.build.is_empty() {
            0
        } else {
            version
                .build
                .as_str()
                .parse::<u16>()
                .map_err(|_| WindowsVersionValidationError::InvalidManifest)?
        };
        Ok(Self {
            major: u16::try_from(version.major)
                .map_err(|_| WindowsVersionValidationError::InvalidManifest)?,
            minor: u16::try_from(version.minor)
                .map_err(|_| WindowsVersionValidationError::InvalidManifest)?,
            build: u16::try_from(version.patch)
                .map_err(|_| WindowsVersionValidationError::InvalidManifest)?,
            private,
        })
    }

    #[cfg(target_os = "windows")]
    fn from_fixed_words(most_significant: u32, least_significant: u32) -> Self {
        Self {
            major: (most_significant >> 16) as u16,
            minor: most_significant as u16,
            build: (least_significant >> 16) as u16,
            private: least_significant as u16,
        }
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmbeddedWindowsVersions {
    file: WindowsExecutableVersion,
    product: WindowsExecutableVersion,
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
) -> Result<WindowsExecutableVersion, WindowsVersionValidationError> {
    let expected = Version::parse(manifest_version)
        .map_err(|_| WindowsVersionValidationError::InvalidManifest)?;
    let current = Version::parse(current_version)
        .map_err(|_| WindowsVersionValidationError::InvalidManifest)?;
    if !expected.pre.is_empty() {
        return Err(WindowsVersionValidationError::InvalidManifest);
    }
    if expected <= current {
        return Err(WindowsVersionValidationError::NotNewer);
    }
    WindowsExecutableVersion::from_stable_semver(&expected)
}

#[cfg(any(target_os = "windows", test))]
fn validate_embedded_windows_versions(
    expected: WindowsExecutableVersion,
    embedded: EmbeddedWindowsVersions,
) -> Result<(), WindowsVersionValidationError> {
    if embedded.file != expected || embedded.product != expected {
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
        let installer_path = directory.path().join("airwiki-update.exe");
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
fn read_locked_windows_versions(
    package: &LockedWindowsUpdatePackage,
) -> Result<EmbeddedWindowsVersions, WindowsVersionValidationError> {
    // Version.dll is path-based. The final file handle denies write/delete sharing
    // and the directory guard denies replacement of its parent while this borrow
    // is live, so the queried path remains bound to the already-compared artifact.
    if !package.path().is_absolute() {
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }
    let path = nul_terminated_windows_path(package.path())
        .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)?;
    let path = PCWSTR(path.as_ptr());
    let mut ignored_handle = 0_u32;
    // SAFETY: path is NUL-terminated and remains live for this size query.
    let version_size =
        unsafe { GetFileVersionInfoSizeExW(FILE_VER_GET_NEUTRAL, path, &mut ignored_handle) };
    if version_size == 0 || version_size > MAX_WINDOWS_VERSION_INFO_BYTES {
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }
    let buffer_size = usize::try_from(version_size)
        .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)?;
    let mut buffer = vec![0_u8; buffer_size];
    // SAFETY: buffer has exactly version_size writable bytes and path remains
    // NUL-terminated and live. The same neutral-resource flags are used as in
    // the preceding size query.
    unsafe {
        GetFileVersionInfoExW(
            FILE_VER_GET_NEUTRAL,
            path,
            None,
            version_size,
            buffer.as_mut_ptr().cast(),
        )
    }
    .map_err(|_| WindowsVersionValidationError::InvalidEmbeddedResource)?;

    const ROOT_SUBBLOCK: [u16; 2] = [b'\\' as u16, 0];
    let mut fixed_info_pointer = std::ptr::null_mut();
    let mut fixed_info_length = 0_u32;
    // SAFETY: buffer is initialized by GetFileVersionInfoExW, ROOT_SUBBLOCK is
    // NUL-terminated, and both output pointers are writable for the call.
    let found = unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            PCWSTR(ROOT_SUBBLOCK.as_ptr()),
            &mut fixed_info_pointer,
            &mut fixed_info_length,
        )
    }
    .as_bool();
    let fixed_info_size = size_of::<VS_FIXEDFILEINFO>();
    if !found
        || fixed_info_pointer.is_null()
        || usize::try_from(fixed_info_length).ok() != Some(fixed_info_size)
    {
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }
    let buffer_start = buffer.as_ptr() as usize;
    let buffer_end = buffer_start
        .checked_add(buffer.len())
        .ok_or(WindowsVersionValidationError::InvalidEmbeddedResource)?;
    let fixed_info_start = fixed_info_pointer as usize;
    let fixed_info_end = fixed_info_start
        .checked_add(fixed_info_size)
        .ok_or(WindowsVersionValidationError::InvalidEmbeddedResource)?;
    if fixed_info_start < buffer_start || fixed_info_end > buffer_end {
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }
    // SAFETY: the range check above proves that a complete VS_FIXEDFILEINFO lies
    // inside buffer. read_unaligned avoids assuming alignment of the byte buffer.
    let fixed_info =
        unsafe { std::ptr::read_unaligned(fixed_info_pointer.cast::<VS_FIXEDFILEINFO>()) };
    if fixed_info.dwSignature != VS_FFI_SIGNATURE as u32
        || fixed_info.dwStrucVersion != VS_FFI_STRUCVERSION as u32
    {
        return Err(WindowsVersionValidationError::InvalidEmbeddedResource);
    }

    Ok(EmbeddedWindowsVersions {
        file: WindowsExecutableVersion::from_fixed_words(
            fixed_info.dwFileVersionMS,
            fixed_info.dwFileVersionLS,
        ),
        product: WindowsExecutableVersion::from_fixed_words(
            fixed_info.dwProductVersionMS,
            fixed_info.dwProductVersionLS,
        ),
    })
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct NativeWindowsUpdateVerifier {
    expected_version: WindowsExecutableVersion,
}

#[cfg(target_os = "windows")]
impl UpdatePackageVerifier<LockedWindowsUpdatePackage> for NativeWindowsUpdateVerifier {
    fn verify(&self, package: &LockedWindowsUpdatePackage) -> Result<(), UpdateIssue> {
        verify_open_artifact_publisher_matches_current_executable(package.file(), package.path())
            .map_err(publisher_trust_issue)?;
        let embedded_versions =
            read_locked_windows_versions(package).map_err(windows_version_issue)?;
        validate_embedded_windows_versions(self.expected_version, embedded_versions)
            .map_err(windows_version_issue)
    }
}

#[cfg(target_os = "windows")]
#[derive(Debug, Default)]
struct DirectWindowsUpdateLauncher;

#[cfg(target_os = "windows")]
impl UpdatePackageLauncher<LockedWindowsUpdatePackage> for DirectWindowsUpdateLauncher {
    fn launch(&self, package: LockedWindowsUpdatePackage) -> Result<(), UpdateIssue> {
        let child = launch_locked_windows_process(&package, &WINDOWS_INSTALLER_ARGS)?;
        let _persisted_directory = package.preserve_after_launch();
        drop(child);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn launch_locked_windows_process(
    package: &LockedWindowsUpdatePackage,
    arguments: &[&str],
) -> Result<WindowsChildProcess, UpdateIssue> {
    let application = nul_terminated_windows_path(package.path())?;
    let mut command_line = windows_command_line(package.path(), arguments)?;
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
    let mut encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(UpdateIssue::new(UpdateIssueCode::Internal));
    }
    encoded.push(0);
    Ok(encoded)
}

#[cfg(target_os = "windows")]
fn windows_command_line(path: &Path, arguments: &[&str]) -> Result<Vec<u16>, UpdateIssue> {
    const MAX_COMMAND_LINE_UNITS: usize = 32_767;

    let executable = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut command_line = Vec::new();
    push_quoted_windows_argument(&mut command_line, &executable)?;
    for argument in arguments {
        command_line.push(u16::from(b' '));
        let encoded = argument.encode_utf16().collect::<Vec<_>>();
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
fn install_platform_update(update: PackagerUpdate, package: Vec<u8>) -> Result<(), UpdateIssue> {
    let expected_version =
        expected_windows_update_version(&update.version, env!("CARGO_PKG_VERSION"))
            .map_err(windows_version_issue)?;
    let package = LockedWindowsUpdatePackage::stage(&package)?;
    let verifier = NativeWindowsUpdateVerifier { expected_version };
    verify_and_launch_package(package, &verifier, &DirectWindowsUpdateLauncher)
}

#[cfg(not(target_os = "windows"))]
fn install_platform_update(update: PackagerUpdate, package: Vec<u8>) -> Result<(), UpdateIssue> {
    update.install(package).map_err(packager_issue)
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

pub(crate) struct UpdaterService<B> {
    backend: B,
    generation: u64,
    view: UpdaterView,
}

impl<B: UpdateBackend> UpdaterService<B> {
    pub(crate) fn new(backend: B) -> Self {
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

    pub(crate) fn check_blocking(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.view.status = UpdaterStatus::Checking;
        self.view.last_issue = None;

        match self.backend.check() {
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

    pub(crate) fn download_blocking(
        &mut self,
        confirmation: DownloadConfirmation,
    ) -> Result<(), UpdateActionError> {
        let update =
            self.validated_available_update(confirmation.generation, &confirmation.version)?;
        self.view.status = UpdaterStatus::Downloading(update.clone());
        self.view.last_issue = None;

        match self.backend.download(&update.version) {
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

    pub(crate) fn install_blocking(
        &mut self,
        confirmation: InstallConfirmation,
    ) -> Result<(), UpdateActionError> {
        let update = self.validated_ready_update(confirmation.generation, &confirmation.version)?;
        self.view.status = UpdaterStatus::Installing(update.clone());
        self.view.last_issue = None;

        match self.backend.install(&update.version) {
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

pub(crate) struct PackagerUpdateBackend {
    current_version: Version,
    config: PackagerConfig,
    checked_update: Option<PackagerUpdate>,
    downloaded_package: Option<Vec<u8>>,
}

impl PackagerUpdateBackend {
    pub(crate) fn new(config: UpdaterBuildConfig) -> Result<Self, UpdaterDisabledReason> {
        let current_version = Version::parse(env!("CARGO_PKG_VERSION"))
            .map_err(|_| UpdaterDisabledReason::InvalidCurrentVersion)?;
        Ok(Self {
            current_version,
            config: PackagerConfig {
                endpoints: vec![config.endpoint],
                pubkey: config.public_key,
                windows: None,
            },
            checked_update: None,
            downloaded_package: None,
        })
    }

    fn checked_update(&self, expected_version: &str) -> Result<&PackagerUpdate, UpdateIssue> {
        self.checked_update
            .as_ref()
            .filter(|update| update.version == expected_version)
            .ok_or_else(|| UpdateIssue::new(UpdateIssueCode::Internal))
    }
}

impl UpdateBackend for PackagerUpdateBackend {
    fn check(&mut self) -> Result<Option<UpdateSummary>, UpdateIssue> {
        self.checked_update = None;
        self.downloaded_package = None;

        let updater = UpdaterBuilder::new(self.current_version.clone(), self.config.clone())
            .version_comparator(|current, release| {
                release.version.pre.is_empty() && release.version > current
            })
            .timeout(NETWORK_TIMEOUT)
            .build()
            .map_err(packager_issue)?;
        let update = updater.check().map_err(packager_issue)?;
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

    fn download(&mut self, expected_version: &str) -> Result<(), UpdateIssue> {
        let package = self
            .checked_update(expected_version)?
            .download()
            .map_err(packager_issue)?;
        self.downloaded_package = Some(package);
        Ok(())
    }

    fn install(&mut self, expected_version: &str) -> Result<(), UpdateIssue> {
        let update = self.checked_update(expected_version)?.clone();
        let package = self
            .downloaded_package
            .take()
            .ok_or_else(|| UpdateIssue::new(UpdateIssueCode::Internal))?;
        install_platform_update(update, package)
    }
}

fn truncate_release_notes(notes: &str) -> String {
    notes
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .take(MAX_RELEASE_NOTES_CHARS)
        .collect()
}

fn packager_issue(error: PackagerError) -> UpdateIssue {
    let code = match error {
        PackagerError::Reqwest(error) if error.is_decode() => UpdateIssueCode::InvalidManifest,
        PackagerError::Reqwest(error) if error.is_connect() || error.is_timeout() => {
            UpdateIssueCode::Offline
        }
        PackagerError::Reqwest(_) | PackagerError::Network(_) => UpdateIssueCode::Offline,
        PackagerError::Serialization(_)
        | PackagerError::ReleaseNotFound
        | PackagerError::Semver(_)
        | PackagerError::TargetNotFound(_)
        | PackagerError::UrlParse(_) => UpdateIssueCode::InvalidManifest,
        PackagerError::Minisign(_) | PackagerError::Base64(_) | PackagerError::SignatureUtf8(_) => {
            UpdateIssueCode::InvalidSignature
        }
        PackagerError::UnsupportedArch
        | PackagerError::UnsupportedOs
        | PackagerError::UnsupportedUpdateFormat => UpdateIssueCode::Unsupported,
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
        expected: WindowsExecutableVersion,
        embedded: EmbeddedWindowsVersions,
        calls: Cell<usize>,
    }

    impl UpdatePackageVerifier<Vec<u8>> for FakeEmbeddedWindowsVersionVerifier {
        fn verify(&self, _package: &Vec<u8>) -> Result<(), UpdateIssue> {
            self.calls.set(self.calls.get() + 1);
            validate_embedded_windows_versions(self.expected, self.embedded)
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

    impl UpdateBackend for FakeBackend {
        fn check(&mut self) -> Result<Option<UpdateSummary>, UpdateIssue> {
            self.checks += 1;
            if let Some(issue) = self.check_issue {
                return Err(issue);
            }
            Ok(self.available.clone())
        }

        fn download(&mut self, _expected_version: &str) -> Result<(), UpdateIssue> {
            self.downloads += 1;
            self.download_issue.map_or(Ok(()), Err)
        }

        fn install(&mut self, _expected_version: &str) -> Result<(), UpdateIssue> {
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
        assert_eq!(WINDOWS_INSTALLER_ARGS, ["/P", "/R", "/AIRWIKIUPDATE"]);
    }

    #[test]
    fn windows_manifest_version_maps_numeric_build_metadata_to_private_component() {
        let expected = expected_windows_update_version("0.2.0+5", "0.2.0").unwrap();

        assert_eq!(
            expected,
            WindowsExecutableVersion {
                major: 0,
                minor: 2,
                build: 0,
                private: 5,
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
            expected_windows_update_version("0.3.0+65536", "0.2.0"),
            Err(WindowsVersionValidationError::InvalidManifest)
        );
        assert_eq!(
            expected_windows_update_version("65536.0.0", "0.2.0"),
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
        let expected = expected_windows_update_version("999.0.0", "0.2.0").unwrap();
        let embedded = WindowsExecutableVersion {
            major: 998,
            minor: 0,
            build: 0,
            private: 0,
        };
        let verifier = FakeEmbeddedWindowsVersionVerifier {
            expected,
            embedded: EmbeddedWindowsVersions {
                file: embedded,
                product: embedded,
            },
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
    fn file_and_product_versions_must_both_match_the_manifest() {
        let expected = expected_windows_update_version("9.1.2+3", "0.2.0").unwrap();
        let older = WindowsExecutableVersion {
            major: 9,
            minor: 1,
            build: 1,
            private: 3,
        };

        assert_eq!(
            validate_embedded_windows_versions(
                expected,
                EmbeddedWindowsVersions {
                    file: older,
                    product: expected,
                },
            ),
            Err(WindowsVersionValidationError::VersionMismatch)
        );
        assert_eq!(
            validate_embedded_windows_versions(
                expected,
                EmbeddedWindowsVersions {
                    file: expected,
                    product: older,
                },
            ),
            Err(WindowsVersionValidationError::VersionMismatch)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn extracts_fixed_versions_from_a_locked_pe_copy() {
        let current_executable = std::env::current_exe().unwrap();
        let executable_bytes = std::fs::read(current_executable).unwrap();
        let package = LockedWindowsUpdatePackage::stage(&executable_bytes).unwrap();
        let embedded = read_locked_windows_versions(&package).unwrap();
        let current = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        let expected = WindowsExecutableVersion::from_stable_semver(&current).unwrap();

        assert_eq!(embedded.file, expected);
        assert_eq!(embedded.product, expected);
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
        let child = launch_locked_windows_process(
            &package,
            &["--ignored", "--exact", CHILD_TEST, "--nocapture"],
        )
        .unwrap();
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

    #[test]
    fn update_should_require_separate_download_and_install_confirmations() {
        let backend = FakeBackend {
            available: Some(available_update()),
            ..FakeBackend::default()
        };
        let mut service = UpdaterService::new(backend);

        service.check_blocking();
        let download_confirmation = service.confirm_download().unwrap();
        service.download_blocking(download_confirmation).unwrap();
        assert!(matches!(
            service.view().status,
            UpdaterStatus::ReadyToInstall(_)
        ));

        let install_confirmation = service.confirm_install().unwrap();
        service.install_blocking(install_confirmation).unwrap();
        assert!(matches!(service.view().status, UpdaterStatus::Installed(_)));
    }

    #[test]
    fn offline_check_should_be_recoverable_and_non_blocking() {
        let backend = FakeBackend {
            check_issue: Some(UpdateIssue::new(UpdateIssueCode::Offline)),
            ..FakeBackend::default()
        };
        let mut service = UpdaterService::new(backend);

        service.check_blocking();

        assert_eq!(service.view().status, UpdaterStatus::Idle);
        assert_eq!(
            service.view().last_issue,
            Some(UpdateIssue {
                code: UpdateIssueCode::Offline,
                retryable: true,
            })
        );
    }

    #[test]
    fn stale_confirmation_should_not_download_a_different_update() {
        let backend = FakeBackend {
            available: Some(available_update()),
            ..FakeBackend::default()
        };
        let mut service = UpdaterService::new(backend);

        service.check_blocking();
        let confirmation = service.confirm_download().unwrap();
        service.check_blocking();

        assert_eq!(
            service.download_blocking(confirmation),
            Err(UpdateActionError::StaleConfirmation)
        );
    }

    #[test]
    fn download_failure_should_keep_update_available_for_retry() {
        let backend = FakeBackend {
            available: Some(available_update()),
            download_issue: Some(UpdateIssue::new(UpdateIssueCode::Offline)),
            ..FakeBackend::default()
        };
        let mut service = UpdaterService::new(backend);

        service.check_blocking();
        let confirmation = service.confirm_download().unwrap();
        service.download_blocking(confirmation).unwrap();

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
