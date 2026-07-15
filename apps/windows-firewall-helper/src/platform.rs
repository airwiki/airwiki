use std::{
    ffi::OsString,
    fs::{self, File},
    io::ErrorKind,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    ptr::null_mut,
    slice,
};

use windows::{
    Win32::{
        Foundation::{
            CloseHandle, HANDLE, HWND, VARIANT_FALSE, VARIANT_TRUE, WAIT_ABANDONED, WAIT_OBJECT_0,
        },
        NetworkManagement::WindowsFirewall::{
            INetFwPolicy2, INetFwRule, INetFwRule2, INetFwRules, NET_FW_ACTION_ALLOW,
            NET_FW_EDGE_TRAVERSAL_TYPE_DENY, NET_FW_IP_PROTOCOL_TCP, NET_FW_IP_PROTOCOL_UDP,
            NET_FW_MODIFY_STATE_GP_OVERRIDE, NET_FW_MODIFY_STATE_INBOUND_BLOCKED,
            NET_FW_MODIFY_STATE_OK, NET_FW_PROFILE_TYPE2, NET_FW_PROFILE2_DOMAIN,
            NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_RULE_DIR_IN, NetFwPolicy2,
            NetFwRule,
        },
        Security::Cryptography::{
            CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG, CTL_USAGE, CertGetEnhancedKeyUsage,
        },
        Security::WinTrust::{
            WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
            WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_NONE,
            WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
            WTD_UICONTEXT_EXECUTE, WTHelperGetProvCertFromChain, WTHelperGetProvSignerFromChain,
            WTHelperProvDataFromStateData, WinVerifyTrust,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IDispatch,
        },
        System::{
            Ole::IEnumVARIANT,
            Threading::{CreateMutexW, INFINITE, ReleaseMutex, WaitForSingleObject},
            Variant::VARIANT,
        },
    },
    core::{BSTR, Interface, PCWSTR},
};

use super::{
    ActiveFirewallEnforcement, DESKTOP_BASENAME, FIREWALL_HELPER_BASENAME, FirewallBackend,
    FirewallDiagnostic, FirewallDiagnosticStatus, FirewallHelperTrustStatus, FirewallProfiles,
    FirewallRuleSpec, HelperCommand, HelperError, InstalledFirewallRule, LocalPolicyState,
    PublisherTrustError, RuleProtocol, SignerEvidence, diagnose_with_backend, expected_local_ports,
    explicit_service_name, install_with_backend, managed_rule_specs, paths_match,
    remove_with_backend, sibling_desktop_path, signers_have_same_identity,
};

impl From<windows::core::Error> for HelperError {
    fn from(_error: windows::core::Error) -> Self {
        Self::Backend
    }
}

const HRESULT_FILE_NOT_FOUND: i32 = 0x8007_0002_u32 as i32;
const MAX_EKU_BUFFER_BYTES: usize = 64 * 1024;
const MAX_EKU_COUNT: usize = 64;
const MAX_ENUMERATED_FIREWALL_RULES: usize = 65_536;
const FIREWALL_OPERATION_MUTEX: &str = "Global\\AirWiki.WindowsFirewall.v1";

fn firewall_program_path(canonical_path: &Path) -> Result<PathBuf, HelperError> {
    const VERBATIM_PREFIX: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];

    let encoded = canonical_path.as_os_str().encode_wide().collect::<Vec<_>>();
    if !encoded.starts_with(&VERBATIM_PREFIX) {
        return Ok(canonical_path.to_path_buf());
    }

    let path = &encoded[VERBATIM_PREFIX.len()..];
    let has_drive_prefix = path.len() >= 3
        && (path[0] >= u16::from(b'A') && path[0] <= u16::from(b'Z')
            || path[0] >= u16::from(b'a') && path[0] <= u16::from(b'z'))
        && path[1] == u16::from(b':')
        && path[2] == u16::from(b'\\');
    if !has_drive_prefix {
        return Err(HelperError::InvalidLayout);
    }

    Ok(PathBuf::from(OsString::from_wide(path)))
}

struct FirewallOperationLock(HANDLE);

impl Drop for FirewallOperationLock {
    fn drop(&mut self) {
        // SAFETY: This guard owns the acquired mutex and releases it exactly once.
        let _ = unsafe { ReleaseMutex(self.0) };
        // SAFETY: This guard owns the handle and closes it exactly once after release.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn acquire_firewall_operation_lock() -> Result<FirewallOperationLock, HelperError> {
    acquire_named_mutex(&wide_null(FIREWALL_OPERATION_MUTEX))
}

fn acquire_named_mutex(name: &[u16]) -> Result<FirewallOperationLock, HelperError> {
    if name.is_empty() || name.last() != Some(&0) || name[..name.len() - 1].contains(&0) {
        return Err(HelperError::Backend);
    }

    // SAFETY: The validated slice is non-empty, has one terminal NUL and no
    // interior NUL. It remains alive for this synchronous call, and null
    // security attributes make the returned handle non-inheritable.
    let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }
        .map_err(|_| HelperError::Backend)?;
    // SAFETY: `handle` is valid and owned here. Waiting indefinitely ensures
    // only one elevated reconciliation reaches COM at a time.
    match unsafe { WaitForSingleObject(handle, INFINITE) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(FirewallOperationLock(handle)),
        _ => {
            // SAFETY: Ownership was not acquired, but this process owns the handle.
            let _ = unsafe { CloseHandle(handle) };
            Err(HelperError::Backend)
        }
    }
}

pub(super) fn run(command: HelperCommand) -> Result<(), HelperError> {
    let precheck_helper = std::env::current_exe().map_err(|_| HelperError::InvalidLayout)?;
    let precheck_desktop = sibling_desktop_path(&precheck_helper)?;
    verify_same_publisher(&precheck_helper, &precheck_desktop)?;

    let _operation_lock = acquire_firewall_operation_lock()?;

    let helper = std::env::current_exe().map_err(|_| HelperError::InvalidLayout)?;
    let desktop = sibling_desktop_path(&helper)?;
    verify_same_publisher(&helper, &desktop)?;
    let desktop = firewall_program_path(&desktop)?;
    let expected = managed_rule_specs(&desktop);
    let mut backend = WindowsFirewallBackend::new()?;
    match command {
        HelperCommand::Install => install_with_backend(&mut backend, &expected),
        HelperCommand::Remove => remove_with_backend(&mut backend, &expected),
    }
}

pub(super) fn diagnose() -> FirewallDiagnostic {
    let desktop = match std::env::current_exe().and_then(|path| path.canonicalize()) {
        Ok(path) => path,
        Err(_) => {
            return FirewallDiagnostic {
                status: FirewallDiagnosticStatus::Error,
                exact_rule_count: 0,
                required_rule_count: 2,
            };
        }
    };
    let desktop = match firewall_program_path(&desktop) {
        Ok(path) => path,
        Err(_) => {
            return FirewallDiagnostic {
                status: FirewallDiagnosticStatus::Error,
                exact_rule_count: 0,
                required_rule_count: 2,
            };
        }
    };
    let expected = managed_rule_specs(&desktop);
    match WindowsFirewallBackend::new() {
        Ok(mut backend) => diagnose_with_backend(&mut backend, &expected),
        Err(_) => FirewallDiagnostic {
            status: FirewallDiagnosticStatus::Error,
            exact_rule_count: 0,
            required_rule_count: 2,
        },
    }
}

pub(super) fn diagnose_helper_trust() -> FirewallHelperTrustStatus {
    let (desktop, helper) = match fixed_sibling_helper_paths() {
        Ok(paths) => paths,
        Err(status) => return status,
    };

    match verify_same_durable_publisher(&desktop, &helper) {
        Ok(()) => FirewallHelperTrustStatus::Verified,
        Err(PublisherTrustError::Untrusted | PublisherTrustError::InvalidLayout) => {
            FirewallHelperTrustStatus::Untrusted
        }
        Err(PublisherTrustError::PublisherMismatch) => FirewallHelperTrustStatus::PublisherMismatch,
        Err(PublisherTrustError::Unsupported | PublisherTrustError::InspectionFailed) => {
            FirewallHelperTrustStatus::Error
        }
    }
}

pub(super) fn verify_open_artifact_publisher(
    artifact: &File,
    artifact_path: &Path,
) -> Result<(), PublisherTrustError> {
    let desktop = fixed_running_desktop_path()?;
    let artifact_path = regular_file_path(artifact_path)?;
    if !artifact
        .metadata()
        .map_err(|_| PublisherTrustError::InvalidLayout)?
        .is_file()
    {
        return Err(PublisherTrustError::InvalidLayout);
    }

    let expected_signer = trusted_signer_evidence(&desktop)?;
    let candidate_signer = trusted_open_signer_evidence(artifact, &artifact_path)?;
    compare_durable_publishers(&expected_signer, &candidate_signer)
}

fn fixed_running_desktop_path() -> Result<PathBuf, PublisherTrustError> {
    let desktop = std::env::current_exe().map_err(|_| PublisherTrustError::InvalidLayout)?;
    let desktop = regular_file_path(&desktop)?;
    let name_matches = desktop
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(DESKTOP_BASENAME));
    if !name_matches {
        return Err(PublisherTrustError::InvalidLayout);
    }
    Ok(desktop)
}

fn regular_file_path(path: &Path) -> Result<PathBuf, PublisherTrustError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| PublisherTrustError::InvalidLayout)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PublisherTrustError::InvalidLayout);
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| PublisherTrustError::InvalidLayout)?;
    if !canonical.is_file() {
        return Err(PublisherTrustError::InvalidLayout);
    }
    Ok(canonical)
}

fn verify_same_durable_publisher(
    expected: &Path,
    candidate: &Path,
) -> Result<(), PublisherTrustError> {
    let expected = regular_file_path(expected)?;
    let candidate = regular_file_path(candidate)?;
    let expected_signer = trusted_signer_evidence(&expected)?;
    let candidate_signer = trusted_signer_evidence(&candidate)?;
    compare_durable_publishers(&expected_signer, &candidate_signer)
}

fn compare_durable_publishers(
    expected_signer: &SignerEvidence,
    candidate_signer: &SignerEvidence,
) -> Result<(), PublisherTrustError> {
    if signers_have_same_identity(expected_signer, candidate_signer) {
        Ok(())
    } else {
        Err(PublisherTrustError::PublisherMismatch)
    }
}

fn trusted_signer_evidence(path: &Path) -> Result<SignerEvidence, PublisherTrustError> {
    validate_trusted_signer(verified_signer_evidence(path))
}

fn trusted_open_signer_evidence(
    artifact: &File,
    artifact_path: &Path,
) -> Result<SignerEvidence, PublisherTrustError> {
    validate_trusted_signer(verified_open_signer_evidence(artifact, artifact_path))
}

fn validate_trusted_signer(
    signer: Result<SignerEvidence, HelperError>,
) -> Result<SignerEvidence, PublisherTrustError> {
    let signer = signer.map_err(|error| match error {
        HelperError::InvalidSignature => PublisherTrustError::Untrusted,
        HelperError::InvalidLayout => PublisherTrustError::InvalidLayout,
        HelperError::Unsupported => PublisherTrustError::Unsupported,
        HelperError::Backend
        | HelperError::InvalidArguments
        | HelperError::ManagedPolicy
        | HelperError::InboundBlocked
        | HelperError::Conflict(_) => PublisherTrustError::InspectionFailed,
    })?;
    if signer.durable_public_trust_identity().is_none() {
        return Err(PublisherTrustError::Untrusted);
    }
    Ok(signer)
}

fn fixed_sibling_helper_paths() -> Result<(PathBuf, PathBuf), FirewallHelperTrustStatus> {
    let desktop = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|_| FirewallHelperTrustStatus::Error)?;
    let desktop_name_matches = desktop
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(DESKTOP_BASENAME));
    if !desktop_name_matches || !desktop.is_file() {
        return Err(FirewallHelperTrustStatus::Error);
    }
    let directory = desktop.parent().ok_or(FirewallHelperTrustStatus::Error)?;
    let helper_candidate = directory.join(FIREWALL_HELPER_BASENAME);
    let helper_metadata = match fs::symlink_metadata(&helper_candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(FirewallHelperTrustStatus::Missing);
        }
        Err(_) => return Err(FirewallHelperTrustStatus::Error),
    };
    if helper_metadata.file_type().is_symlink() || !helper_metadata.is_file() {
        return Err(FirewallHelperTrustStatus::Untrusted);
    }
    let helper = helper_candidate
        .canonicalize()
        .map_err(|_| FirewallHelperTrustStatus::Error)?;
    let helper_name_matches = helper
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(FIREWALL_HELPER_BASENAME));
    if helper.parent() != Some(directory) || !helper_name_matches {
        return Err(FirewallHelperTrustStatus::Untrusted);
    }

    Ok((desktop, helper))
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, HelperError> {
        // SAFETY: This process owns its main thread and balances every successful
        // initialization with `CoUninitialize` from `Drop` on the same thread.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|_| HelperError::Backend)?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: `ComApartment` is created and dropped on the helper's main thread.
        unsafe { CoUninitialize() };
    }
}

struct WindowsFirewallBackend {
    rules: INetFwRules,
    policy: INetFwPolicy2,
    _apartment: ComApartment,
}

impl WindowsFirewallBackend {
    fn new() -> Result<Self, HelperError> {
        let apartment = ComApartment::initialize()?;
        // SAFETY: COM is initialized for this thread and both CLSIDs are fixed
        // Windows Firewall objects created in-process with no aggregation.
        let policy: INetFwPolicy2 = unsafe {
            CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)
                .map_err(|_| HelperError::Backend)?
        };
        // SAFETY: `policy` is a valid `INetFwPolicy2` returned above.
        let rules = unsafe { policy.Rules() }.map_err(|_| HelperError::Backend)?;
        Ok(Self {
            rules,
            policy,
            _apartment: apartment,
        })
    }

    fn read_rule(&self, name: &str) -> Result<Option<InstalledFirewallRule>, HelperError> {
        let name = BSTR::from(name);
        // SAFETY: `rules` is a live COM interface and `name` owns its BSTR.
        let rule = match unsafe { self.rules.Item(&name) } {
            Ok(rule) => rule,
            Err(error) if error.code().0 == HRESULT_FILE_NOT_FOUND => return Ok(None),
            Err(_) => return Err(HelperError::Backend),
        };
        read_installed_rule(&rule).map(Some)
    }

    fn read_rules_for_program(
        &self,
        program: &Path,
    ) -> Result<Vec<InstalledFirewallRule>, HelperError> {
        // SAFETY: `rules` is a live COM collection and `_NewEnum` is read-only.
        let unknown = unsafe { self.rules._NewEnum() }.map_err(|_| HelperError::Backend)?;
        let enumerator: IEnumVARIANT = unknown.cast().map_err(|_| HelperError::Backend)?;
        let mut matching = Vec::new();

        for _ in 0..MAX_ENUMERATED_FIREWALL_RULES {
            let mut variants = [VARIANT::default()];
            let mut fetched = 0;
            // SAFETY: The one-element VARIANT buffer and fetched counter remain
            // valid for this synchronous call. VARIANT owns and clears any COM
            // value returned by the enumerator.
            unsafe { enumerator.Next(&mut variants, &mut fetched) }
                .ok()
                .map_err(|_| HelperError::Backend)?;
            if fetched == 0 {
                return Ok(matching);
            }
            if fetched != 1 {
                return Err(HelperError::Backend);
            }

            let dispatch = IDispatch::try_from(&variants[0]).map_err(|_| HelperError::Backend)?;
            let rule: INetFwRule = dispatch.cast().map_err(|_| HelperError::Backend)?;
            // Read the application path first so unrelated host rules do not
            // require parsing all of their optional properties.
            let installed_program = unsafe { rule.ApplicationName() };
            let installed_program = PathBuf::from(bstr_to_string(installed_program)?);
            if paths_match(&installed_program, program) {
                matching.push(read_installed_rule(&rule)?);
            }
        }

        // A malformed or unexpectedly huge collection must fail closed without
        // exposing rule names, paths or platform error text.
        Err(HelperError::Backend)
    }
}

impl FirewallBackend for WindowsFirewallBackend {
    fn active_profile_enforcement(&mut self) -> Result<ActiveFirewallEnforcement, HelperError> {
        // SAFETY: `policy` remains valid for the lifetime of the backend and
        // these properties are read-only. The pure reducer below queries only
        // active Private or Domain profiles; Public never becomes eligible.
        let active_profiles =
            unsafe { self.policy.CurrentProfileTypes() }.map_err(|_| HelperError::Backend)?;
        classify_active_profile_enforcement(active_profiles, |profile| {
            // SAFETY: `profile` is one of the two fixed profile constants and
            // `policy` remains valid for each synchronous property read.
            let enabled = unsafe { self.policy.get_FirewallEnabled(profile) }
                .map_err(|_| HelperError::Backend)?
                != VARIANT_FALSE;
            // SAFETY: Same fixed profile and live COM interface as above.
            let block_all = unsafe { self.policy.get_BlockAllInboundTraffic(profile) }
                .map_err(|_| HelperError::Backend)?
                != VARIANT_FALSE;
            Ok((enabled, block_all))
        })
    }

    fn local_policy_state(&mut self) -> Result<LocalPolicyState, HelperError> {
        // SAFETY: `policy` remains valid for the lifetime of the backend.
        let state =
            unsafe { self.policy.LocalPolicyModifyState() }.map_err(|_| HelperError::Backend)?;
        match state {
            NET_FW_MODIFY_STATE_OK => Ok(LocalPolicyState::Effective),
            NET_FW_MODIFY_STATE_GP_OVERRIDE => Ok(LocalPolicyState::Managed),
            NET_FW_MODIFY_STATE_INBOUND_BLOCKED => Ok(LocalPolicyState::InboundBlocked),
            _ => Err(HelperError::Backend),
        }
    }

    fn rule(&mut self, name: &str) -> Result<Option<InstalledFirewallRule>, HelperError> {
        self.read_rule(name)
    }

    fn rules_for_program(
        &mut self,
        program: &Path,
    ) -> Result<Vec<InstalledFirewallRule>, HelperError> {
        self.read_rules_for_program(program)
    }

    fn add_rule(&mut self, spec: &FirewallRuleSpec) -> Result<(), HelperError> {
        // SAFETY: COM is initialized and the fixed firewall rule CLSID is created
        // in-process without aggregation.
        let rule: INetFwRule2 = unsafe {
            CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER)
                .map_err(|_| HelperError::Backend)?
        };
        configure_rule(&rule, spec)?;
        let base: INetFwRule = rule.cast().map_err(|_| HelperError::Backend)?;
        // SAFETY: The rule was fully initialized and is passed as a live COM interface.
        unsafe { self.rules.Add(&base) }.map_err(|_| HelperError::Backend)
    }

    fn remove_rule(&mut self, name: &str) -> Result<(), HelperError> {
        let name = BSTR::from(name);
        // SAFETY: Conflict preflight already proved that this exact managed rule exists.
        unsafe { self.rules.Remove(&name) }.map_err(|_| HelperError::Backend)
    }
}

fn classify_active_profile_enforcement(
    active_profiles: i32,
    mut read_profile: impl FnMut(NET_FW_PROFILE_TYPE2) -> Result<(bool, bool), HelperError>,
) -> Result<ActiveFirewallEnforcement, HelperError> {
    let mut found_relevant_profile = false;
    let mut disabled = false;
    let mut block_all_inbound = false;

    for profile in [NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE] {
        if active_profiles & profile.0 == 0 {
            continue;
        }
        found_relevant_profile = true;
        let (enabled, blocks_all_inbound) = read_profile(profile)?;
        disabled |= !enabled;
        block_all_inbound |= blocks_all_inbound;
    }

    if !found_relevant_profile {
        return Err(HelperError::Backend);
    }
    if disabled {
        Ok(ActiveFirewallEnforcement::Disabled)
    } else if block_all_inbound {
        Ok(ActiveFirewallEnforcement::BlockAllInbound)
    } else {
        Ok(ActiveFirewallEnforcement::Enforced)
    }
}

fn configure_rule(rule: &INetFwRule2, spec: &FirewallRuleSpec) -> Result<(), HelperError> {
    let name = BSTR::from(spec.name);
    let description = BSTR::from(spec.description);
    let application = BSTR::from(spec.program.to_string_lossy().as_ref());
    let local_addresses = BSTR::from(spec.local_addresses);
    let local_ports = BSTR::from(expected_local_ports(spec));
    let remote_ports = BSTR::from(spec.remote_ports);
    let remote_addresses = BSTR::from(spec.remote_addresses);
    let interface_types = BSTR::from(spec.interface_types);
    let service_name = explicit_service_name(spec.service_name).map(BSTR::from);
    let grouping = BSTR::from(spec.grouping);
    let protocol = match spec.protocol {
        RuleProtocol::Tcp => NET_FW_IP_PROTOCOL_TCP.0,
        RuleProtocol::Udp => NET_FW_IP_PROTOCOL_UDP.0,
        RuleProtocol::Other(_) => return Err(HelperError::Backend),
    };
    let profiles = NET_FW_PROFILE2_DOMAIN.0 | NET_FW_PROFILE2_PRIVATE.0;

    // SAFETY: All BSTR values live through these synchronous COM calls. The
    // remaining arguments are fixed values from the pure, prevalidated spec.
    unsafe {
        rule.SetName(&name)?;
        rule.SetDescription(&description)?;
        rule.SetApplicationName(&application)?;
        rule.SetProtocol(protocol)?;
        rule.SetLocalPorts(&local_ports)?;
        rule.SetLocalAddresses(&local_addresses)?;
        rule.SetRemotePorts(&remote_ports)?;
        rule.SetRemoteAddresses(&remote_addresses)?;
        rule.SetInterfaceTypes(&interface_types)?;
        // Windows 10 rejects `SetServiceName("")` with `E_INVALIDARG`; leaving
        // the property unset still reads back as the required empty value.
        if let Some(service_name) = service_name.as_ref() {
            rule.SetServiceName(service_name)?;
        }
        rule.SetDirection(NET_FW_RULE_DIR_IN)?;
        rule.SetEnabled(VARIANT_TRUE)?;
        rule.SetGrouping(&grouping)?;
        rule.SetProfiles(profiles)?;
        rule.SetEdgeTraversal(VARIANT_FALSE)?;
        rule.SetEdgeTraversalOptions(NET_FW_EDGE_TRAVERSAL_TYPE_DENY.0)?;
        rule.SetAction(NET_FW_ACTION_ALLOW)?;
    }
    Ok(())
}

fn read_installed_rule(rule: &INetFwRule) -> Result<InstalledFirewallRule, HelperError> {
    let rule2: INetFwRule2 = rule.cast().map_err(|_| HelperError::Backend)?;
    // SAFETY: Every getter is invoked synchronously on a live COM rule interface.
    let (
        name,
        description,
        program,
        protocol,
        local_ports,
        local_addresses,
        remote_ports,
        remote_addresses,
        interface_types,
        service_name,
        profiles,
        edge_traversal,
        edge_options,
        grouping,
        enabled,
        direction,
        action,
    ) = unsafe {
        (
            rule.Name(),
            rule.Description(),
            rule.ApplicationName(),
            rule.Protocol(),
            rule.LocalPorts(),
            rule.LocalAddresses(),
            rule.RemotePorts(),
            rule.RemoteAddresses(),
            rule.InterfaceTypes(),
            rule.ServiceName(),
            rule.Profiles(),
            rule.EdgeTraversal(),
            rule2.EdgeTraversalOptions(),
            rule.Grouping(),
            rule.Enabled(),
            rule.Direction(),
            rule.Action(),
        )
    };

    let profile_mask = profiles.map_err(|_| HelperError::Backend)?;
    Ok(InstalledFirewallRule {
        name: bstr_to_string(name)?,
        description: bstr_to_string(description)?,
        program: PathBuf::from(bstr_to_string(program)?),
        protocol: match protocol.map_err(|_| HelperError::Backend)? {
            value if value == NET_FW_IP_PROTOCOL_TCP.0 => RuleProtocol::Tcp,
            value if value == NET_FW_IP_PROTOCOL_UDP.0 => RuleProtocol::Udp,
            value => RuleProtocol::Other(value),
        },
        local_ports: bstr_to_string(local_ports)?,
        local_addresses: bstr_to_string(local_addresses)?,
        remote_ports: bstr_to_string(remote_ports)?,
        remote_addresses: bstr_to_string(remote_addresses)?,
        interface_types: bstr_to_string(interface_types)?,
        service_name: bstr_to_string(service_name)?,
        profiles: FirewallProfiles::from_membership(
            profile_mask & NET_FW_PROFILE2_DOMAIN.0 != 0,
            profile_mask & NET_FW_PROFILE2_PRIVATE.0 != 0,
            profile_mask & NET_FW_PROFILE2_PUBLIC.0 != 0,
        ),
        edge_traversal_blocked: edge_traversal.map_err(|_| HelperError::Backend)? == VARIANT_FALSE
            && edge_options.map_err(|_| HelperError::Backend)? == NET_FW_EDGE_TRAVERSAL_TYPE_DENY.0,
        grouping: bstr_to_string(grouping)?,
        enabled: enabled.map_err(|_| HelperError::Backend)? == VARIANT_TRUE,
        inbound: direction.map_err(|_| HelperError::Backend)? == NET_FW_RULE_DIR_IN,
        allow: action.map_err(|_| HelperError::Backend)? == NET_FW_ACTION_ALLOW,
    })
}

fn bstr_to_string(value: windows::core::Result<BSTR>) -> Result<String, HelperError> {
    String::try_from(value.map_err(|_| HelperError::Backend)?).map_err(|_| HelperError::Backend)
}

fn verify_same_publisher(helper: &Path, desktop: &Path) -> Result<(), HelperError> {
    verify_same_durable_publisher(helper, desktop).map_err(|error| match error {
        PublisherTrustError::InvalidLayout => HelperError::InvalidLayout,
        PublisherTrustError::Unsupported => HelperError::Unsupported,
        PublisherTrustError::InspectionFailed => HelperError::Backend,
        PublisherTrustError::Untrusted | PublisherTrustError::PublisherMismatch => {
            HelperError::InvalidSignature
        }
    })
}

fn verified_signer_evidence(path: &Path) -> Result<SignerEvidence, HelperError> {
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: size_of_u32::<WINTRUST_FILE_INFO>()?,
        pcwszFilePath: PCWSTR(path.as_ptr()),
        hFile: HANDLE(null_mut()),
        pgKnownSubject: null_mut(),
    };
    verified_signer_evidence_with_file_info(&mut file_info)
}

fn verified_open_signer_evidence(
    artifact: &File,
    artifact_path: &Path,
) -> Result<SignerEvidence, HelperError> {
    let path: Vec<u16> = artifact_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: size_of_u32::<WINTRUST_FILE_INFO>()?,
        pcwszFilePath: PCWSTR(path.as_ptr()),
        hFile: HANDLE(artifact.as_raw_handle()),
        pgKnownSubject: null_mut(),
    };
    verified_signer_evidence_with_file_info(&mut file_info)
}

fn verified_signer_evidence_with_file_info(
    file_info: &mut WINTRUST_FILE_INFO,
) -> Result<SignerEvidence, HelperError> {
    let mut trust_data = WINTRUST_DATA {
        cbStruct: size_of_u32::<WINTRUST_DATA>()?,
        pPolicyCallbackData: null_mut(),
        pSIPClientData: null_mut(),
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: file_info },
        dwStateAction: WTD_STATEACTION_VERIFY,
        hWVTStateData: HANDLE(null_mut()),
        pwszURLReference: windows::core::PWSTR(null_mut()),
        // Runtime verification must remain usable offline and must not create
        // implicit network traffic. Public release gates separately require
        // SignTool chain, timestamp and revocation-policy validation before an
        // artifact can enter the signed update manifest.
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE,
        dwUIContext: WTD_UICONTEXT_EXECUTE,
        pSignatureSettings: null_mut(),
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: The caller keeps the path buffer, optional file handle, file info
    // and trust data valid for the complete verification/close pair; the API
    // receives no UI-capable flags.
    let status = unsafe {
        WinVerifyTrust(
            HWND(null_mut()),
            &mut action,
            (&mut trust_data as *mut WINTRUST_DATA).cast(),
        )
    };
    if status != 0 {
        close_trust_state(&mut trust_data, &mut action);
        return Err(HelperError::InvalidSignature);
    }

    let evidence = extract_signer_evidence(trust_data.hWVTStateData);
    close_trust_state(&mut trust_data, &mut action);
    evidence
}

fn extract_signer_evidence(state: HANDLE) -> Result<SignerEvidence, HelperError> {
    // SAFETY: `state` was produced by a successful `WinVerifyTrust` call and
    // remains open until `close_trust_state`. Every pointer and bound is checked
    // before copying EKU data into owned memory.
    unsafe {
        let provider = WTHelperProvDataFromStateData(state);
        if provider.is_null() {
            return Err(HelperError::InvalidSignature);
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
        if signer.is_null() {
            return Err(HelperError::InvalidSignature);
        }
        let provider_cert = WTHelperGetProvCertFromChain(signer, 0);
        if provider_cert.is_null() || (*provider_cert).pCert.is_null() {
            return Err(HelperError::InvalidSignature);
        }
        let certificate = (*provider_cert).pCert;
        let enhanced_key_usage_oids = enhanced_key_usage_oids(certificate)?;
        Ok(SignerEvidence {
            enhanced_key_usage_oids,
        })
    }
}

fn enhanced_key_usage_oids(
    certificate: *const windows::Win32::Security::Cryptography::CERT_CONTEXT,
) -> Result<Vec<String>, HelperError> {
    let flags = CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG.0;
    let mut required_bytes = 0_u32;
    // SAFETY: `certificate` is a live context from the verified provider chain;
    // the first call requests only the required buffer size.
    unsafe { CertGetEnhancedKeyUsage(certificate, flags, None, &mut required_bytes) }
        .map_err(|_| HelperError::InvalidSignature)?;
    let required_bytes =
        usize::try_from(required_bytes).map_err(|_| HelperError::InvalidSignature)?;
    if required_bytes < size_of::<CTL_USAGE>() || required_bytes > MAX_EKU_BUFFER_BYTES {
        return Err(HelperError::InvalidSignature);
    }

    let word_count = required_bytes.div_ceil(size_of::<usize>());
    let mut storage = vec![0_usize; word_count];
    let usage_pointer = storage.as_mut_ptr().cast::<CTL_USAGE>();
    let mut returned_bytes =
        u32::try_from(required_bytes).map_err(|_| HelperError::InvalidSignature)?;
    // SAFETY: `storage` is aligned for `CTL_USAGE` and has the byte capacity
    // requested by Crypt32. It remains owned until all returned pointers are copied.
    unsafe {
        CertGetEnhancedKeyUsage(certificate, flags, Some(usage_pointer), &mut returned_bytes)
    }
    .map_err(|_| HelperError::InvalidSignature)?;

    let returned_bytes =
        usize::try_from(returned_bytes).map_err(|_| HelperError::InvalidSignature)?;
    if returned_bytes < size_of::<CTL_USAGE>() || returned_bytes > required_bytes {
        return Err(HelperError::InvalidSignature);
    }
    let storage_start = storage.as_ptr() as usize;
    let storage_end = storage_start
        .checked_add(returned_bytes)
        .ok_or(HelperError::InvalidSignature)?;
    // SAFETY: `usage_pointer` is aligned and points into `storage`, and Crypt32
    // reported at least one complete `CTL_USAGE` value.
    let usage = unsafe { &*usage_pointer };
    let oid_count =
        usize::try_from(usage.cUsageIdentifier).map_err(|_| HelperError::InvalidSignature)?;
    if oid_count == 0 || oid_count > MAX_EKU_COUNT || usage.rgpszUsageIdentifier.is_null() {
        return Err(HelperError::InvalidSignature);
    }
    let pointer_bytes = oid_count
        .checked_mul(size_of::<windows::core::PSTR>())
        .ok_or(HelperError::InvalidSignature)?;
    let pointer_start = usage.rgpszUsageIdentifier as usize;
    if !range_is_inside(pointer_start, pointer_bytes, storage_start, storage_end) {
        return Err(HelperError::InvalidSignature);
    }
    // SAFETY: The pointer array range was validated inside the initialized buffer.
    let oid_pointers = unsafe { slice::from_raw_parts(usage.rgpszUsageIdentifier, oid_count) };
    let mut oids = Vec::with_capacity(oid_count);
    for oid_pointer in oid_pointers {
        let oid_start = oid_pointer.as_ptr() as usize;
        if oid_start < storage_start || oid_start >= storage_end {
            return Err(HelperError::InvalidSignature);
        }
        let remaining = storage_end - oid_start;
        // SAFETY: The start and maximum length are bounded by the initialized buffer.
        let bytes = unsafe { slice::from_raw_parts(oid_pointer.as_ptr(), remaining) };
        let oid_length = bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(HelperError::InvalidSignature)?;
        let oid_bytes = &bytes[..oid_length];
        if oid_bytes.is_empty()
            || !oid_bytes
                .iter()
                .all(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            return Err(HelperError::InvalidSignature);
        }
        let oid = std::str::from_utf8(oid_bytes)
            .map_err(|_| HelperError::InvalidSignature)?
            .to_owned();
        oids.push(oid);
    }
    Ok(oids)
}

fn range_is_inside(
    start: usize,
    length: usize,
    container_start: usize,
    container_end: usize,
) -> bool {
    start >= container_start
        && start
            .checked_add(length)
            .is_some_and(|end| end <= container_end)
}

fn close_trust_state(trust_data: &mut WINTRUST_DATA, action: &mut windows::core::GUID) {
    if trust_data.hWVTStateData.is_invalid() {
        return;
    }
    trust_data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: This is the required matching close operation for state returned
    // by `WinVerifyTrust`; all structures still outlive this synchronous call.
    let _ = unsafe {
        WinVerifyTrust(
            HWND(null_mut()),
            action,
            (trust_data as *mut WINTRUST_DATA).cast(),
        )
    };
}

fn size_of_u32<T>() -> Result<u32, HelperError> {
    u32::try_from(size_of::<T>()).map_err(|_| HelperError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_operation_lock_blocks_a_second_thread_until_release() {
        use std::{sync::mpsc, thread, time::Duration};

        let name = format!(
            "Local\\AirWiki.Firewall.Test.{}.{}",
            std::process::id(),
            line!()
        );
        let wide = wide_null(&name);
        let first = acquire_named_mutex(&wide).expect("first lock should be acquired");
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let second_name = wide.clone();
        let second = thread::spawn(move || {
            started_tx
                .send(())
                .expect("second thread should signal before acquisition");
            let guard = acquire_named_mutex(&second_name).expect("second lock should be acquired");
            acquired_tx
                .send(())
                .expect("second thread should signal acquisition");
            drop(guard);
        });

        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("second thread should start");
        assert!(
            acquired_rx
                .recv_timeout(Duration::from_millis(200))
                .is_err()
        );
        drop(first);
        acquired_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("second lock should be acquired after release");
        second.join().expect("second lock thread should finish");
    }

    #[test]
    fn named_operation_lock_rejects_invalid_names_before_win32() {
        assert!(matches!(
            acquire_named_mutex(&[]),
            Err(HelperError::Backend)
        ));
        assert!(matches!(
            acquire_named_mutex(&['x' as u16]),
            Err(HelperError::Backend)
        ));
        assert!(matches!(
            acquire_named_mutex(&['x' as u16, 0, 'y' as u16, 0]),
            Err(HelperError::Backend)
        ));
    }

    const RETAINED_OPERATION_LOCK: &str =
        "let _operation_lock = acquire_firewall_operation_lock()?;";
    const POST_LOCK_HELPER: &str =
        "let helper = std::env::current_exe().map_err(|_| HelperError::InvalidLayout)?;";
    const POST_LOCK_DESKTOP: &str = "let desktop = sibling_desktop_path(&helper)?;";
    const POST_LOCK_PUBLISHER_CHECK: &str = "verify_same_publisher(&helper, &desktop)?;";
    const POST_LOCK_RULE_BINDING: &str = "let expected = managed_rule_specs(&desktop);";
    const POST_LOCK_BACKEND: &str = "let mut backend = WindowsFirewallBackend::new()?;";
    const COMMAND_OPERATION: &str = "match command {";

    fn run_source(source: &str) -> &str {
        let run_start = source.find("pub(super) fn run(").expect("run should exist");
        let run_end = source[run_start..]
            .find("\npub(super) fn diagnose(")
            .map(|offset| run_start + offset)
            .expect("diagnose should follow run");
        &source[run_start..run_end]
    }

    fn identifier_occurrences(source: &str, identifier: &str) -> usize {
        source
            .match_indices(identifier)
            .filter(|(start, _)| {
                let before = source[..*start].chars().next_back();
                let after = source[*start + identifier.len()..].chars().next();
                let is_identifier_character =
                    |character: char| character == '_' || character.is_ascii_alphanumeric();
                !before.is_some_and(is_identifier_character)
                    && !after.is_some_and(is_identifier_character)
            })
            .count()
    }

    fn run_satisfies_operation_lock_contract(run: &str) -> bool {
        let Some(lock) = run.find(RETAINED_OPERATION_LOCK) else {
            return false;
        };
        let Some(post_lock_current_exe) = run[lock..]
            .find(POST_LOCK_HELPER)
            .map(|offset| lock + offset)
        else {
            return false;
        };
        let Some(post_lock_desktop) = run[post_lock_current_exe..]
            .find(POST_LOCK_DESKTOP)
            .map(|offset| post_lock_current_exe + offset)
        else {
            return false;
        };
        let Some(final_publisher_check) = run[post_lock_desktop..]
            .find(POST_LOCK_PUBLISHER_CHECK)
            .map(|offset| post_lock_desktop + offset)
        else {
            return false;
        };
        let Some(expected) = run[final_publisher_check..]
            .find(POST_LOCK_RULE_BINDING)
            .map(|offset| final_publisher_check + offset)
        else {
            return false;
        };
        let Some(backend) = run[expected..]
            .find(POST_LOCK_BACKEND)
            .map(|offset| expected + offset)
        else {
            return false;
        };
        let Some(command_operation) = run[backend..]
            .find(COMMAND_OPERATION)
            .map(|offset| backend + offset)
        else {
            return false;
        };
        let held_region = &run[lock + RETAINED_OPERATION_LOCK.len()..command_operation];

        lock < post_lock_current_exe
            && post_lock_current_exe < post_lock_desktop
            && post_lock_desktop < final_publisher_check
            && final_publisher_check < expected
            && expected < backend
            && backend < command_operation
            && run.matches("verify_same_publisher").count() == 2
            && identifier_occurrences(run, "_operation_lock") == 1
            && !held_region.contains('}')
            && !held_region.contains("precheck_")
            && !run.contains("managed_rule_specs(&precheck_desktop)")
    }

    #[test]
    fn run_rebinds_publisher_and_paths_under_operation_lock() {
        let run = run_source(include_str!("platform.rs"));

        assert!(run_satisfies_operation_lock_contract(run));
    }

    #[test]
    fn run_lock_contract_rejects_early_release_scope_exit_and_precheck_aliases() {
        let run = run_source(include_str!("platform.rs"));
        let unretained = run.replacen(
            RETAINED_OPERATION_LOCK,
            "acquire_firewall_operation_lock()?;",
            1,
        );
        let explicitly_dropped = run.replacen(
            COMMAND_OPERATION,
            "drop(_operation_lock);\n    match command {",
            1,
        );
        let scope_closed = run.replacen(COMMAND_OPERATION, "}\n    match command {", 1);
        let precheck_alias = run.replacen(
            POST_LOCK_PUBLISHER_CHECK,
            "verify_same_publisher(&helper, &desktop)?;\n    let desktop = precheck_desktop;",
            1,
        );
        let precheck_rules = run.replacen(
            POST_LOCK_RULE_BINDING,
            "let expected = managed_rule_specs(&precheck_desktop);",
            1,
        );

        assert!(!run_satisfies_operation_lock_contract(&unretained));
        assert!(!run_satisfies_operation_lock_contract(&explicitly_dropped));
        assert!(!run_satisfies_operation_lock_contract(&scope_closed));
        assert!(!run_satisfies_operation_lock_contract(&precheck_alias));
        assert!(!run_satisfies_operation_lock_contract(&precheck_rules));
    }

    #[test]
    fn configure_rule_overwrites_tcp_port_with_explicit_wildcard() {
        let _apartment = ComApartment::initialize().expect("COM apartment should initialize");
        // SAFETY: COM is initialized on this thread. The detached fixed rule
        // object is never inserted into an `INetFwRules` collection.
        let rule: INetFwRule2 = unsafe {
            CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER)
                .expect("detached firewall rule should be created")
        };
        let initial_ports = BSTR::from("4242");
        // SAFETY: `rule` is a live detached COM object and the BSTR remains
        // owned for this synchronous property write.
        unsafe {
            rule.SetProtocol(NET_FW_IP_PROTOCOL_TCP.0)
                .expect("TCP protocol should be configured");
            rule.SetLocalPorts(&initial_ports)
                .expect("initial local port should be configured");
        }
        let [tcp, _] = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));

        configure_rule(&rule, &tcp).expect("managed TCP rule should be configured");

        // SAFETY: `rule` remains a live detached COM object and the getter is read-only.
        let local_ports = unsafe { rule.LocalPorts() }
            .expect("configured local ports should be readable")
            .to_string();
        assert_eq!(local_ports, "*");
    }

    #[test]
    fn firewall_program_path_strips_verbatim_local_disk_prefix() {
        let path = firewall_program_path(Path::new(r"\\?\C:\Users\Rustic\AirWiki\airwiki.exe"));

        assert_eq!(
            path,
            Ok(PathBuf::from(r"C:\Users\Rustic\AirWiki\airwiki.exe"))
        );
    }

    #[test]
    fn firewall_program_path_rejects_verbatim_unc_path() {
        let path = firewall_program_path(Path::new(r"\\?\UNC\server\share\AirWiki\airwiki.exe"));

        assert_eq!(path, Err(HelperError::InvalidLayout));
    }

    #[test]
    fn path_comparison_accepts_normal_and_verbatim_local_disk_forms() {
        assert!(paths_match(
            Path::new(r"C:\AirWiki\airwiki.exe"),
            Path::new(r"\\?\C:\AirWiki\airwiki.exe"),
        ));
    }

    #[test]
    fn active_profile_probe_ignores_public_and_reads_each_relevant_profile() {
        let active =
            NET_FW_PROFILE2_DOMAIN.0 | NET_FW_PROFILE2_PRIVATE.0 | NET_FW_PROFILE2_PUBLIC.0;
        let mut read = Vec::new();

        let state = classify_active_profile_enforcement(active, |profile| {
            read.push(profile.0);
            Ok((true, false))
        });

        assert_eq!(state, Ok(ActiveFirewallEnforcement::Enforced));
        assert_eq!(read, [NET_FW_PROFILE2_DOMAIN.0, NET_FW_PROFILE2_PRIVATE.0]);
    }

    #[test]
    fn disabled_profile_takes_precedence_over_block_all_inbound() {
        let active = NET_FW_PROFILE2_DOMAIN.0 | NET_FW_PROFILE2_PRIVATE.0;

        let state = classify_active_profile_enforcement(active, |profile| {
            if profile == NET_FW_PROFILE2_DOMAIN {
                Ok((true, true))
            } else {
                Ok((false, false))
            }
        });

        assert_eq!(state, Ok(ActiveFirewallEnforcement::Disabled));
    }

    #[test]
    fn public_only_profile_fails_closed_without_querying_it() {
        let mut queried = false;

        let state = classify_active_profile_enforcement(NET_FW_PROFILE2_PUBLIC.0, |_| {
            queried = true;
            Ok((true, false))
        });

        assert_eq!(state, Err(HelperError::Backend));
        assert!(!queried);
    }

    #[test]
    fn profile_read_failure_is_sanitized_and_fails_closed() {
        let state = classify_active_profile_enforcement(NET_FW_PROFILE2_PRIVATE.0, |_| {
            Err(HelperError::Backend)
        });

        assert_eq!(state, Err(HelperError::Backend));
    }
}
