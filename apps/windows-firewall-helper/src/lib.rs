//! Narrow policy boundary for the elevated Windows Firewall helper.
//!
//! The platform-independent reconciliation code is deliberately separate from
//! the COM backend so CI can prove fail-closed behavior without modifying a
//! host firewall.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use thiserror::Error;

#[cfg(any(windows, test))]
const DESKTOP_BASENAME: &str = "airwiki.exe";
const GROUP_NAME: &str = "AirWiki";
const ALL_ADDRESSES: &str = "*";
const ALL_INTERFACES: &str = "All";
const ALL_PORTS: &str = "*";
#[cfg(any(windows, test))]
const ARTIFACT_SIGNING_EKU_PREFIX: &str = "1.3.6.1.4.1.311.97.";
#[cfg(any(windows, test))]
const ARTIFACT_SIGNING_GENERIC_EKU: &str = "1.3.6.1.4.1.311.97.1.0";
#[cfg(any(windows, test))]
const CODE_SIGNING_EKU: &str = "1.3.6.1.5.5.7.3.3";
#[cfg(windows)]
const FIREWALL_HELPER_BASENAME: &str = "airwiki-windows-firewall-helper.exe";
const LOCAL_SUBNET: &str = "LocalSubnet";
const NO_SERVICE: &str = "";
const TCP_RULE_NAME: &str = "AirWiki LAN (TCP)";
const UDP_RULE_NAME: &str = "AirWiki Discovery (mDNS)";

#[cfg(any(windows, test))]
fn explicit_service_name(service_name: &str) -> Option<&str> {
    (!service_name.is_empty()).then_some(service_name)
}

/// Stable process exit codes consumed by the desktop application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HelperExitCode {
    /// The requested state was reached.
    Success = 0,
    /// Group policy prevents an effective local firewall change.
    ManagedPolicy = 10,
    /// A rule with a managed name exists but is not owned by this helper.
    Conflict = 11,
    /// The executable layout or Authenticode publisher is invalid.
    InvalidLayoutOrSignature = 12,
    /// The operating system cannot provide the Windows backend.
    Unsupported = 13,
    /// A platform API failed unexpectedly.
    InternalError = 14,
    /// Windows policy blocks unsolicited inbound traffic for the active profile.
    InboundBlocked = 15,
    /// The command line was not exactly `install` or `remove`.
    InvalidArguments = 64,
}

impl HelperExitCode {
    /// Returns the numeric process status without exposing platform APIs.
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// The only operations accepted by the privileged process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperCommand {
    /// Install or verify the two managed rules.
    Install,
    /// Remove only exact managed rules.
    Remove,
}

/// Transport protocol for a managed inbound rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleProtocol {
    /// TCP listener whose port is selected at runtime.
    Tcp,
    /// UDP discovery traffic on a fixed port.
    Udp,
    /// Any protocol not managed by AirWiki.
    Other(i32),
}

/// Pure description of one firewall rule managed by AirWiki.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirewallRuleSpec {
    /// Stable unique display name used for exact lookup and removal.
    pub name: &'static str,
    /// Human-readable explanation shown by Windows Firewall.
    pub description: &'static str,
    /// Executable to which Windows scopes the exception.
    pub program: PathBuf,
    /// Inbound IP protocol.
    pub protocol: RuleProtocol,
    /// `None` means all local ports, represented as `*` in Windows Firewall.
    pub local_port: Option<u16>,
    /// Local addresses on which the rule accepts traffic.
    pub local_addresses: &'static str,
    /// Remote source ports accepted by the rule.
    pub remote_ports: &'static str,
    /// Only hosts on the local subnet may connect.
    pub remote_addresses: &'static str,
    /// Network interface types on which the rule applies.
    pub interface_types: &'static str,
    /// Windows service scope; empty means the executable itself.
    pub service_name: &'static str,
    /// Domain and Private profiles are enabled; Public is always absent.
    pub profiles: FirewallProfiles,
    /// Edge traversal remains disabled.
    pub edge_traversal_blocked: bool,
    /// Stable grouping label in Windows Firewall management UI.
    pub grouping: &'static str,
    /// Rule is active after installation.
    pub enabled: bool,
    /// Rule applies to inbound traffic only.
    pub inbound: bool,
    /// Matching traffic is allowed.
    pub allow: bool,
}

/// Owned state read from Windows for exact comparison before mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledFirewallRule {
    /// Rule display name.
    pub name: String,
    /// Rule description.
    pub description: String,
    /// Executable scope.
    pub program: PathBuf,
    /// IP protocol.
    pub protocol: RuleProtocol,
    /// Exact Windows text; managed rules use `*` for TCP and `5353` for discovery.
    pub local_ports: String,
    /// Local addresses on which the rule accepts traffic.
    pub local_addresses: String,
    /// Remote source ports accepted by the rule.
    pub remote_ports: String,
    /// Remote address scope.
    pub remote_addresses: String,
    /// Network interface types on which the rule applies.
    pub interface_types: String,
    /// Windows service scope; empty means the executable itself.
    pub service_name: String,
    /// Enabled profiles.
    pub profiles: FirewallProfiles,
    /// Whether all edge traversal modes are blocked.
    pub edge_traversal_blocked: bool,
    /// Rule grouping.
    pub grouping: String,
    /// Whether the installed rule is active.
    pub enabled: bool,
    /// Whether the installed rule is inbound.
    pub inbound: bool,
    /// Whether matching traffic is allowed.
    pub allow: bool,
}

impl InstalledFirewallRule {
    fn exactly_matches(&self, expected: &FirewallRuleSpec) -> bool {
        self.name == expected.name
            && self.description == expected.description
            && paths_match(&self.program, &expected.program)
            && self.protocol == expected.protocol
            && self.local_ports == expected_local_ports(expected)
            && self
                .local_addresses
                .eq_ignore_ascii_case(expected.local_addresses)
            && self
                .remote_ports
                .eq_ignore_ascii_case(expected.remote_ports)
            && self
                .remote_addresses
                .eq_ignore_ascii_case(expected.remote_addresses)
            && self
                .interface_types
                .eq_ignore_ascii_case(expected.interface_types)
            && self.service_name == expected.service_name
            && self.profiles == expected.profiles
            && self.edge_traversal_blocked == expected.edge_traversal_blocked
            && self.grouping == expected.grouping
            && self.enabled == expected.enabled
            && self.inbound == expected.inbound
            && self.allow == expected.allow
    }

    fn is_broad_legacy_exposure(&self, expected: &[FirewallRuleSpec; 2]) -> bool {
        let [tcp, udp] = expected;
        if !self.enabled
            || !self.inbound
            || !self.allow
            || !paths_match(&self.program, &tcp.program)
            || self.exactly_matches(tcp)
            || self.exactly_matches(udp)
        {
            return false;
        }

        self.profiles.includes_public()
            || !self.edge_traversal_blocked
            || !remote_scope_is_exactly_local_subnet(&self.remote_addresses)
            || match self.protocol {
                // The managed TCP listener selects a dynamic port, so another
                // TCP rule cannot widen its local-port set beyond that rule.
                RuleProtocol::Tcp => false,
                // mDNS needs exactly one UDP port. Any other UDP port set
                // expands the executable's exposure beyond discovery traffic.
                RuleProtocol::Udp => self.local_ports.trim() != "5353",
                // AirWiki manages only TCP and UDP. A rule for any or a
                // different IP protocol expands that protocol set.
                RuleProtocol::Other(_) => true,
            }
    }
}

fn remote_scope_is_exactly_local_subnet(remote_addresses: &str) -> bool {
    let remote_addresses = remote_addresses.trim();
    !remote_addresses.contains(',') && remote_addresses.eq_ignore_ascii_case(LOCAL_SUBNET)
}

impl From<&FirewallRuleSpec> for InstalledFirewallRule {
    fn from(rule: &FirewallRuleSpec) -> Self {
        Self {
            name: rule.name.to_owned(),
            description: rule.description.to_owned(),
            program: rule.program.clone(),
            protocol: rule.protocol,
            local_ports: expected_local_ports(rule),
            local_addresses: rule.local_addresses.to_owned(),
            remote_ports: rule.remote_ports.to_owned(),
            remote_addresses: rule.remote_addresses.to_owned(),
            interface_types: rule.interface_types.to_owned(),
            service_name: rule.service_name.to_owned(),
            profiles: rule.profiles,
            edge_traversal_blocked: rule.edge_traversal_blocked,
            grouping: rule.grouping.to_owned(),
            enabled: rule.enabled,
            inbound: rule.inbound,
            allow: rule.allow,
        }
    }
}

fn expected_local_ports(spec: &FirewallRuleSpec) -> String {
    spec.local_port
        .map_or_else(|| ALL_PORTS.to_owned(), |port| port.to_string())
}

fn paths_match(actual: &Path, expected: &Path) -> bool {
    #[cfg(windows)]
    {
        actual
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .eq_ignore_ascii_case(expected.to_string_lossy().trim_start_matches(r"\\?\"))
    }
    #[cfg(not(windows))]
    {
        actual == expected
    }
}

/// Profile mask deliberately excluding the Public profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirewallProfiles(u8);

impl FirewallProfiles {
    const DOMAIN: u8 = 0b01;
    const PRIVATE: u8 = 0b10;
    const PUBLIC: u8 = 0b100;

    /// Domain and Private, the only allowed profiles for managed rules.
    pub const DOMAIN_AND_PRIVATE: Self = Self(Self::DOMAIN | Self::PRIVATE);

    /// Returns whether the Domain profile is present.
    pub const fn includes_domain(self) -> bool {
        self.0 & Self::DOMAIN != 0
    }

    /// Returns whether the Private profile is present.
    pub const fn includes_private(self) -> bool {
        self.0 & Self::PRIVATE != 0
    }

    /// Returns whether the disallowed Public profile is present.
    pub const fn includes_public(self) -> bool {
        self.0 & Self::PUBLIC != 0
    }

    #[cfg(windows)]
    const fn from_membership(domain: bool, private: bool, public: bool) -> Self {
        let mut value = 0;
        if domain {
            value |= Self::DOMAIN;
        }
        if private {
            value |= Self::PRIVATE;
        }
        if public {
            value |= Self::PUBLIC;
        }
        Self(value)
    }
}

/// Read-only status suitable for a desktop connectivity diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallDiagnosticStatus {
    /// Both exact rules are installed, enforced and local policy permits changes.
    Ready,
    /// Windows Firewall is disabled for at least one active Private or Domain profile.
    FirewallDisabled,
    /// At least one active Private or Domain profile ignores inbound allow rules.
    BlockAllInbound,
    /// At least one managed rule is absent.
    RulesMissing,
    /// A managed rule name is occupied by different settings.
    Conflict,
    /// An additional active inbound allow rule exposes this executable more broadly.
    LegacyExposure,
    /// Group policy prevents an effective local change.
    ManagedPolicy,
    /// This platform has no Windows Firewall backend.
    Unsupported,
    /// A platform API failed while reading the state.
    Error,
}

/// Effective read-only state of Windows Firewall for active LAN-capable profiles.
///
/// This is deliberately separate from the managed rules: exact exceptions do
/// not protect or enable the listener when Windows Firewall is disabled, and
/// they cannot receive traffic while Windows is configured to block every
/// inbound connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveFirewallEnforcement {
    /// Firewall is enabled and inbound allow rules may be evaluated.
    Enforced,
    /// Firewall is disabled for at least one active Private or Domain profile.
    Disabled,
    /// Firewall is enabled but all inbound exceptions are ignored.
    BlockAllInbound,
}

/// Effect of local firewall-rule changes under the current Windows policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalPolicyState {
    /// Local rule changes take effect.
    Effective,
    /// Group Policy overrides local rule changes.
    Managed,
    /// The active policy rejects unsolicited inbound traffic.
    InboundBlocked,
}

/// Sanitized firewall readiness snapshot; it contains no local paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirewallDiagnostic {
    /// Overall state.
    pub status: FirewallDiagnosticStatus,
    /// Number of exact managed rules currently installed.
    pub exact_rule_count: u8,
    /// Constant number of rules required by AirWiki.
    pub required_rule_count: u8,
}

/// Read-only trust state of the fixed firewall helper installed next to the desktop app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallHelperTrustStatus {
    /// The helper has a valid code-signing chain and the same durable signer identity.
    Verified,
    /// The fixed sibling helper file is not installed.
    Missing,
    /// At least one binary is not validly code-signed.
    Untrusted,
    /// Both binaries are validly signed, but their signer identities differ.
    PublisherMismatch,
    /// The operating system does not provide the Windows verification backend.
    Unsupported,
    /// The installed layout or a platform API could not be inspected safely.
    Error,
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SignerEvidence {
    enhanced_key_usage_oids: Vec<String>,
}

#[cfg(any(windows, test))]
impl SignerEvidence {
    fn has_code_signing_eku(&self) -> bool {
        self.enhanced_key_usage_oids
            .iter()
            .any(|oid| oid == CODE_SIGNING_EKU)
    }

    fn unique_artifact_signing_subscriber_eku(&self) -> Option<&str> {
        let mut selected = None;
        for oid in self.enhanced_key_usage_oids.iter().filter(|oid| {
            oid.starts_with(ARTIFACT_SIGNING_EKU_PREFIX)
                && oid.as_str() != ARTIFACT_SIGNING_GENERIC_EKU
        }) {
            if selected.is_some() {
                return None;
            }
            selected = Some(oid.as_str());
        }
        selected
    }

    fn durable_public_trust_identity(&self) -> Option<&str> {
        if !self.has_code_signing_eku()
            || !self
                .enhanced_key_usage_oids
                .iter()
                .any(|oid| oid == ARTIFACT_SIGNING_GENERIC_EKU)
        {
            return None;
        }
        self.unique_artifact_signing_subscriber_eku()
    }
}

#[cfg(any(windows, test))]
fn signers_have_same_identity(left: &SignerEvidence, right: &SignerEvidence) -> bool {
    match (
        left.durable_public_trust_identity(),
        right.durable_public_trust_identity(),
    ) {
        (Some(left_oid), Some(right_oid)) => left_oid == right_oid,
        _ => false,
    }
}

/// Fail-closed result of checking an Authenticode artifact against the running desktop.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PublisherTrustError {
    /// This operation is only available on Windows.
    #[error("publisher verification is unsupported on this platform")]
    Unsupported,
    /// The current executable or candidate is not a regular file in the expected layout.
    #[error("the executable layout is invalid")]
    InvalidLayout,
    /// At least one executable lacks a valid Public Trust code-signing identity.
    #[error("the executable signature is not trusted")]
    Untrusted,
    /// Both signatures are valid but belong to different durable publisher identities.
    #[error("the executable publisher does not match AirWiki")]
    PublisherMismatch,
    /// Windows could not inspect the signature without exposing platform details.
    #[error("the executable signature could not be inspected")]
    InspectionFailed,
}

/// Typed failures mapped to stable helper exit codes.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HelperError {
    /// The command line did not match the closed command set.
    #[error("expected exactly one command: install or remove")]
    InvalidArguments,
    /// The helper is not running on Windows.
    #[error("Windows Firewall integration is unsupported on this platform")]
    Unsupported,
    /// The helper and desktop executable layout is invalid.
    #[error("the helper executable layout is invalid")]
    InvalidLayout,
    /// One or both binaries are unsigned or their verified publishers differ.
    #[error("the helper and desktop signatures are not valid for the same publisher")]
    InvalidSignature,
    /// Local rules would be overridden by system policy.
    #[error("Windows Firewall is managed by policy")]
    ManagedPolicy,
    /// The active Windows Firewall policy blocks unsolicited inbound traffic.
    #[error("Windows Firewall blocks unsolicited inbound traffic")]
    InboundBlocked,
    /// A differently configured rule uses a managed name.
    #[error("a conflicting firewall rule exists: {0}")]
    Conflict(String),
    /// A Windows API failed without exposing sensitive details.
    #[error("Windows Firewall operation failed")]
    Backend,
}

impl HelperError {
    /// Maps internal failures to the stable process contract.
    pub const fn exit_code(&self) -> HelperExitCode {
        match self {
            Self::InvalidArguments => HelperExitCode::InvalidArguments,
            Self::Unsupported => HelperExitCode::Unsupported,
            Self::InvalidLayout | Self::InvalidSignature => {
                HelperExitCode::InvalidLayoutOrSignature
            }
            Self::ManagedPolicy => HelperExitCode::ManagedPolicy,
            Self::InboundBlocked => HelperExitCode::InboundBlocked,
            Self::Conflict(_) => HelperExitCode::Conflict,
            Self::Backend => HelperExitCode::InternalError,
        }
    }
}

/// Minimal backend surface required by the pure reconciliation algorithm.
pub trait FirewallBackend {
    /// Reads effective enforcement for the active Private or Domain profiles.
    fn active_profile_enforcement(&mut self) -> Result<ActiveFirewallEnforcement, HelperError>;

    /// Reads why local rule changes do or do not take effect.
    fn local_policy_state(&mut self) -> Result<LocalPolicyState, HelperError>;

    /// Reads a rule by its stable managed name.
    fn rule(&mut self, name: &str) -> Result<Option<InstalledFirewallRule>, HelperError>;

    /// Enumerates rules scoped to the exact desktop executable without mutating them.
    fn rules_for_program(
        &mut self,
        program: &Path,
    ) -> Result<Vec<InstalledFirewallRule>, HelperError>;

    /// Adds a fully specified inbound rule.
    fn add_rule(&mut self, rule: &FirewallRuleSpec) -> Result<(), HelperError>;

    /// Removes a rule by its stable managed name.
    fn remove_rule(&mut self, name: &str) -> Result<(), HelperError>;
}

/// Parses the helper's closed command set.
pub fn parse_command<I, S>(arguments: I) -> Result<HelperCommand, HelperError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut arguments = arguments.into_iter();
    let command = arguments
        .next()
        .and_then(|value| value.as_ref().to_str().map(str::to_owned));
    if arguments.next().is_some() {
        return Err(HelperError::InvalidArguments);
    }
    match command.as_deref() {
        Some("install") => Ok(HelperCommand::Install),
        Some("remove") => Ok(HelperCommand::Remove),
        _ => Err(HelperError::InvalidArguments),
    }
}

#[cfg(any(windows, test))]
fn sibling_desktop_path(helper_path: &Path) -> Result<PathBuf, HelperError> {
    let helper = helper_path
        .canonicalize()
        .map_err(|_| HelperError::InvalidLayout)?;
    let directory = helper.parent().ok_or(HelperError::InvalidLayout)?;
    let desktop = directory.join(DESKTOP_BASENAME);
    let desktop = desktop
        .canonicalize()
        .map_err(|_| HelperError::InvalidLayout)?;
    if desktop.parent() != Some(directory) || !desktop.is_file() {
        return Err(HelperError::InvalidLayout);
    }
    Ok(desktop)
}

/// Returns the only two rules the helper may create.
pub fn managed_rule_specs(program: &Path) -> [FirewallRuleSpec; 2] {
    [
        FirewallRuleSpec {
            name: TCP_RULE_NAME,
            description: "Allows the AirWiki LAN listener from the local subnet",
            program: program.to_path_buf(),
            protocol: RuleProtocol::Tcp,
            local_port: None,
            local_addresses: ALL_ADDRESSES,
            remote_ports: ALL_PORTS,
            remote_addresses: LOCAL_SUBNET,
            interface_types: ALL_INTERFACES,
            service_name: NO_SERVICE,
            profiles: FirewallProfiles::DOMAIN_AND_PRIVATE,
            edge_traversal_blocked: true,
            grouping: GROUP_NAME,
            enabled: true,
            inbound: true,
            allow: true,
        },
        FirewallRuleSpec {
            name: UDP_RULE_NAME,
            description: "Allows AirWiki mDNS discovery from the local subnet",
            program: program.to_path_buf(),
            protocol: RuleProtocol::Udp,
            local_port: Some(5353),
            local_addresses: ALL_ADDRESSES,
            remote_ports: ALL_PORTS,
            remote_addresses: LOCAL_SUBNET,
            interface_types: ALL_INTERFACES,
            service_name: NO_SERVICE,
            profiles: FirewallProfiles::DOMAIN_AND_PRIVATE,
            edge_traversal_blocked: true,
            grouping: GROUP_NAME,
            enabled: true,
            inbound: true,
            allow: true,
        },
    ]
}

/// Produces a read-only diagnostic without mutating backend state.
pub fn diagnose_with_backend(
    backend: &mut impl FirewallBackend,
    expected: &[FirewallRuleSpec; 2],
) -> FirewallDiagnostic {
    let enforcement = match backend.active_profile_enforcement() {
        Ok(value) => value,
        Err(_) => {
            return FirewallDiagnostic {
                status: FirewallDiagnosticStatus::Error,
                exact_rule_count: 0,
                required_rule_count: 2,
            };
        }
    };
    match enforcement {
        ActiveFirewallEnforcement::Enforced => {}
        ActiveFirewallEnforcement::Disabled => {
            return FirewallDiagnostic {
                status: FirewallDiagnosticStatus::FirewallDisabled,
                exact_rule_count: 0,
                required_rule_count: 2,
            };
        }
        ActiveFirewallEnforcement::BlockAllInbound => {
            return FirewallDiagnostic {
                status: FirewallDiagnosticStatus::BlockAllInbound,
                exact_rule_count: 0,
                required_rule_count: 2,
            };
        }
    }

    let policy = match backend.local_policy_state() {
        Ok(value) => value,
        Err(_) => {
            return FirewallDiagnostic {
                status: FirewallDiagnosticStatus::Error,
                exact_rule_count: 0,
                required_rule_count: 2,
            };
        }
    };
    match policy {
        LocalPolicyState::Effective => {}
        LocalPolicyState::Managed => {
            return FirewallDiagnostic {
                status: FirewallDiagnosticStatus::ManagedPolicy,
                exact_rule_count: 0,
                required_rule_count: 2,
            };
        }
        LocalPolicyState::InboundBlocked => {
            return FirewallDiagnostic {
                status: FirewallDiagnosticStatus::BlockAllInbound,
                exact_rule_count: 0,
                required_rule_count: 2,
            };
        }
    }

    let mut exact_rule_count = 0;
    let mut missing = false;
    for rule in expected {
        match backend.rule(rule.name) {
            Ok(Some(installed)) if installed.exactly_matches(rule) => exact_rule_count += 1,
            Ok(Some(_)) => {
                return FirewallDiagnostic {
                    status: FirewallDiagnosticStatus::Conflict,
                    exact_rule_count,
                    required_rule_count: 2,
                };
            }
            Ok(None) => missing = true,
            Err(_) => {
                return FirewallDiagnostic {
                    status: FirewallDiagnosticStatus::Error,
                    exact_rule_count,
                    required_rule_count: 2,
                };
            }
        }
    }
    let [desktop_rule, _] = expected;
    let program_rules = match backend.rules_for_program(&desktop_rule.program) {
        Ok(rules) => rules,
        Err(_) => {
            return FirewallDiagnostic {
                status: FirewallDiagnosticStatus::Error,
                exact_rule_count,
                required_rule_count: 2,
            };
        }
    };
    if program_rules
        .iter()
        .any(|rule| rule.is_broad_legacy_exposure(expected))
    {
        return FirewallDiagnostic {
            status: FirewallDiagnosticStatus::LegacyExposure,
            exact_rule_count,
            required_rule_count: 2,
        };
    }
    FirewallDiagnostic {
        status: if missing {
            FirewallDiagnosticStatus::RulesMissing
        } else {
            FirewallDiagnosticStatus::Ready
        },
        exact_rule_count,
        required_rule_count: 2,
    }
}

/// Reconciles the two rules, preflighting conflicts before any mutation.
pub fn install_with_backend(
    backend: &mut impl FirewallBackend,
    expected: &[FirewallRuleSpec; 2],
) -> Result<(), HelperError> {
    require_effective_local_policy(backend.local_policy_state()?)?;

    let mut missing = Vec::with_capacity(expected.len());
    for rule in expected {
        match backend.rule(rule.name)? {
            Some(installed) if installed.exactly_matches(rule) => {}
            Some(_) => return Err(HelperError::Conflict(rule.name.to_owned())),
            None => missing.push(rule),
        }
    }

    let mut added: Vec<&FirewallRuleSpec> = Vec::with_capacity(missing.len());
    for rule in missing {
        if let Err(error) = backend.add_rule(rule) {
            rollback_new_exact_rules(backend, &added);
            return Err(error);
        }
        added.push(rule);
    }

    if let Err(error) = verify_installed_postcondition(backend, expected) {
        rollback_new_exact_rules(backend, &added);
        return Err(error);
    }

    Ok(())
}

/// Removes only rules whose complete settings still match the managed spec.
pub fn remove_with_backend(
    backend: &mut impl FirewallBackend,
    expected: &[FirewallRuleSpec; 2],
) -> Result<(), HelperError> {
    require_effective_local_policy(backend.local_policy_state()?)?;

    let mut present = Vec::with_capacity(expected.len());
    for rule in expected {
        match backend.rule(rule.name)? {
            Some(installed) if installed.exactly_matches(rule) => present.push(rule),
            Some(_) => return Err(HelperError::Conflict(rule.name.to_owned())),
            None => {}
        }
    }
    for rule in present {
        match backend.rule(rule.name)? {
            Some(installed) if installed.exactly_matches(rule) => {
                backend.remove_rule(rule.name)?;
            }
            Some(_) => return Err(HelperError::Conflict(rule.name.to_owned())),
            None => {}
        }
    }

    verify_removed_postcondition(backend, expected)
}

fn verify_installed_postcondition(
    backend: &mut impl FirewallBackend,
    expected: &[FirewallRuleSpec; 2],
) -> Result<(), HelperError> {
    for rule in expected {
        match backend.rule(rule.name)? {
            Some(installed) if installed.exactly_matches(rule) => {}
            Some(_) => return Err(HelperError::Conflict(rule.name.to_owned())),
            None => return Err(HelperError::Backend),
        }
    }
    Ok(())
}

fn require_effective_local_policy(state: LocalPolicyState) -> Result<(), HelperError> {
    match state {
        LocalPolicyState::Effective => Ok(()),
        LocalPolicyState::Managed => Err(HelperError::ManagedPolicy),
        LocalPolicyState::InboundBlocked => Err(HelperError::InboundBlocked),
    }
}

fn verify_removed_postcondition(
    backend: &mut impl FirewallBackend,
    expected: &[FirewallRuleSpec; 2],
) -> Result<(), HelperError> {
    for rule in expected {
        match backend.rule(rule.name)? {
            None => {}
            Some(installed) if installed.exactly_matches(rule) => {
                return Err(HelperError::Backend);
            }
            Some(_) => return Err(HelperError::Conflict(rule.name.to_owned())),
        }
    }
    Ok(())
}

fn rollback_new_exact_rules(backend: &mut impl FirewallBackend, added: &[&FirewallRuleSpec]) {
    for rule in added.iter().rev() {
        if matches!(
            backend.rule(rule.name),
            Ok(Some(installed)) if installed.exactly_matches(rule)
        ) {
            let _ = backend.remove_rule(rule.name);
        }
    }
}

/// Executes the platform backend after deriving all paths internally.
pub fn run_platform(command: HelperCommand) -> Result<(), HelperError> {
    platform::run(command)
}

/// Reads the exact managed-rule state for the running desktop executable.
///
/// The returned value is deliberately sanitized and never contains the
/// executable path or underlying Windows API errors.
pub fn diagnose_platform() -> FirewallDiagnostic {
    platform::diagnose()
}

/// Inspects the fixed firewall helper next to the running desktop application.
///
/// This read-only operation never elevates and accepts no caller-controlled
/// paths. It can therefore be used to decide whether offering the UAC flow is
/// safe without widening the privileged helper's command surface.
pub fn diagnose_sibling_helper_trust() -> FirewallHelperTrustStatus {
    platform::diagnose_helper_trust()
}

/// Verifies an already-open Authenticode artifact against the durable Public
/// Trust publisher identity of the running AirWiki desktop executable.
///
/// `artifact` must remain open and protected against write and delete sharing
/// for the complete call. Windows verifies that exact handle; `artifact_path`
/// supplies the non-null path required by `WINTRUST_FILE_INFO` and is never
/// opened as the source of candidate bytes.
pub fn verify_open_artifact_publisher_matches_current_executable(
    artifact: &std::fs::File,
    artifact_path: &Path,
) -> Result<(), PublisherTrustError> {
    platform::verify_open_artifact_publisher(artifact, artifact_path)
}

#[cfg(not(windows))]
mod platform {
    use super::{
        FirewallDiagnostic, FirewallDiagnosticStatus, FirewallHelperTrustStatus, HelperCommand,
        HelperError, PublisherTrustError,
    };
    use std::{fs::File, path::Path};

    pub(super) fn run(_command: HelperCommand) -> Result<(), HelperError> {
        Err(HelperError::Unsupported)
    }

    pub(super) fn diagnose() -> FirewallDiagnostic {
        FirewallDiagnostic {
            status: FirewallDiagnosticStatus::Unsupported,
            exact_rule_count: 0,
            required_rule_count: 2,
        }
    }

    pub(super) const fn diagnose_helper_trust() -> FirewallHelperTrustStatus {
        FirewallHelperTrustStatus::Unsupported
    }

    pub(super) const fn verify_open_artifact_publisher(
        _artifact: &File,
        _artifact_path: &Path,
    ) -> Result<(), PublisherTrustError> {
        Err(PublisherTrustError::Unsupported)
    }
}

#[cfg(windows)]
mod platform;

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use super::*;

    #[test]
    fn empty_service_name_is_left_unset_for_windows_compatibility() {
        assert_eq!(explicit_service_name(""), None);
    }

    #[test]
    fn non_empty_service_name_is_configured_explicitly() {
        assert_eq!(explicit_service_name("service"), Some("service"));
    }

    struct FakeBackend {
        enforcement: ActiveFirewallEnforcement,
        local_policy: LocalPolicyState,
        rules: BTreeMap<String, InstalledFirewallRule>,
        extra_rules: Vec<InstalledFirewallRule>,
        added: Vec<&'static str>,
        removed: Vec<&'static str>,
        fail_enumeration: bool,
        fail_add: Option<&'static str>,
        mutate_before_add_failure: Option<&'static str>,
        pretend_add_success: Option<&'static str>,
        pretend_remove_success: Option<&'static str>,
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self {
                enforcement: ActiveFirewallEnforcement::Enforced,
                local_policy: LocalPolicyState::Managed,
                rules: BTreeMap::new(),
                extra_rules: Vec::new(),
                added: Vec::new(),
                removed: Vec::new(),
                fail_enumeration: false,
                fail_add: None,
                mutate_before_add_failure: None,
                pretend_add_success: None,
                pretend_remove_success: None,
            }
        }
    }

    impl FirewallBackend for FakeBackend {
        fn active_profile_enforcement(&mut self) -> Result<ActiveFirewallEnforcement, HelperError> {
            Ok(self.enforcement)
        }

        fn local_policy_state(&mut self) -> Result<LocalPolicyState, HelperError> {
            Ok(self.local_policy)
        }

        fn rule(&mut self, name: &str) -> Result<Option<InstalledFirewallRule>, HelperError> {
            Ok(self.rules.get(name).cloned())
        }

        fn rules_for_program(
            &mut self,
            program: &Path,
        ) -> Result<Vec<InstalledFirewallRule>, HelperError> {
            if self.fail_enumeration {
                return Err(HelperError::Backend);
            }
            Ok(self
                .rules
                .values()
                .chain(&self.extra_rules)
                .filter(|rule| paths_match(&rule.program, program))
                .cloned()
                .collect())
        }

        fn add_rule(&mut self, rule: &FirewallRuleSpec) -> Result<(), HelperError> {
            if self.fail_add == Some(rule.name) {
                if let Some(name) = self.mutate_before_add_failure
                    && let Some(installed) = self.rules.get_mut(name)
                {
                    installed.service_name = "changed-by-another-actor".to_owned();
                }
                return Err(HelperError::Backend);
            }
            self.added.push(rule.name);
            if self.pretend_add_success == Some(rule.name) {
                return Ok(());
            }
            self.rules
                .insert(rule.name.to_owned(), InstalledFirewallRule::from(rule));
            Ok(())
        }

        fn remove_rule(&mut self, name: &str) -> Result<(), HelperError> {
            let stable_name = match name {
                TCP_RULE_NAME => TCP_RULE_NAME,
                UDP_RULE_NAME => UDP_RULE_NAME,
                _ => return Err(HelperError::Backend),
            };
            self.removed.push(stable_name);
            if self.pretend_remove_success == Some(stable_name) {
                return Ok(());
            }
            self.rules.remove(name);
            Ok(())
        }
    }

    fn exact_tcp_rule_fixture() -> (FirewallRuleSpec, InstalledFirewallRule) {
        let [expected, _] = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let installed = InstalledFirewallRule::from(&expected);
        (expected, installed)
    }

    fn ready_backend(expected: &[FirewallRuleSpec; 2]) -> FakeBackend {
        FakeBackend {
            local_policy: LocalPolicyState::Effective,
            rules: expected
                .iter()
                .map(|rule| (rule.name.to_owned(), InstalledFirewallRule::from(rule)))
                .collect(),
            ..FakeBackend::default()
        }
    }

    fn renamed_rule(expected: &FirewallRuleSpec, name: &str) -> InstalledFirewallRule {
        let mut rule = InstalledFirewallRule::from(expected);
        rule.name = name.to_owned();
        rule
    }

    fn signer_fixture(oids: &[&str]) -> SignerEvidence {
        SignerEvidence {
            enhanced_key_usage_oids: oids.iter().map(|oid| (*oid).to_owned()).collect(),
        }
    }

    #[test]
    fn signer_identity_requires_code_signing_eku_on_both_certificates() {
        let subscriber = "1.3.6.1.4.1.311.97.100.200";
        let left = signer_fixture(&[ARTIFACT_SIGNING_GENERIC_EKU, subscriber]);
        let right = signer_fixture(&[CODE_SIGNING_EKU, ARTIFACT_SIGNING_GENERIC_EKU, subscriber]);

        assert!(!signers_have_same_identity(&left, &right));
    }

    #[test]
    fn artifact_signing_generic_marker_is_not_a_subscriber_identity() {
        let signer = signer_fixture(&[CODE_SIGNING_EKU, ARTIFACT_SIGNING_GENERIC_EKU]);

        assert_eq!(signer.unique_artifact_signing_subscriber_eku(), None);
    }

    #[test]
    fn duplicate_subscriber_eku_is_rejected_as_ambiguous() {
        let subscriber = "1.3.6.1.4.1.311.97.100.200";
        let signer = signer_fixture(&[
            CODE_SIGNING_EKU,
            ARTIFACT_SIGNING_GENERIC_EKU,
            subscriber,
            subscriber,
        ]);

        assert_eq!(signer.unique_artifact_signing_subscriber_eku(), None);
    }

    #[test]
    fn multiple_subscriber_ekus_do_not_select_an_ambiguous_identity() {
        let signer = signer_fixture(&[
            CODE_SIGNING_EKU,
            ARTIFACT_SIGNING_GENERIC_EKU,
            "1.3.6.1.4.1.311.97.100.200",
            "1.3.6.1.4.1.311.97.100.201",
        ]);

        assert_eq!(signer.unique_artifact_signing_subscriber_eku(), None);
    }

    #[test]
    fn matching_public_trust_subscriber_eku_accepts_the_publisher() {
        let subscriber = "1.3.6.1.4.1.311.97.100.200";
        let left = signer_fixture(&[CODE_SIGNING_EKU, ARTIFACT_SIGNING_GENERIC_EKU, subscriber]);
        let right = signer_fixture(&[CODE_SIGNING_EKU, ARTIFACT_SIGNING_GENERIC_EKU, subscriber]);

        assert!(signers_have_same_identity(&left, &right));
    }

    #[test]
    fn different_subscriber_ekus_reject_the_publisher() {
        let left = signer_fixture(&[
            CODE_SIGNING_EKU,
            ARTIFACT_SIGNING_GENERIC_EKU,
            "1.3.6.1.4.1.311.97.100.200",
        ]);
        let right = signer_fixture(&[
            CODE_SIGNING_EKU,
            ARTIFACT_SIGNING_GENERIC_EKU,
            "1.3.6.1.4.1.311.97.100.201",
        ]);

        assert!(!signers_have_same_identity(&left, &right));
    }

    #[test]
    fn missing_subscriber_eku_rejects_the_publisher() {
        let left = signer_fixture(&[
            CODE_SIGNING_EKU,
            ARTIFACT_SIGNING_GENERIC_EKU,
            "1.3.6.1.4.1.311.97.100.200",
        ]);
        let right = signer_fixture(&[CODE_SIGNING_EKU, ARTIFACT_SIGNING_GENERIC_EKU]);

        assert!(!signers_have_same_identity(&left, &right));
    }

    #[test]
    fn missing_public_trust_marker_rejects_the_publisher() {
        let subscriber = "1.3.6.1.4.1.311.97.100.200";
        let left = signer_fixture(&[CODE_SIGNING_EKU, subscriber]);
        let right = signer_fixture(&[CODE_SIGNING_EKU, subscriber]);

        assert!(!signers_have_same_identity(&left, &right));
    }

    #[cfg(not(windows))]
    #[test]
    fn artifact_publisher_verification_fails_closed_off_windows() {
        let artifact = tempfile::tempfile().expect("temporary artifact should be created");
        assert_eq!(
            verify_open_artifact_publisher_matches_current_executable(
                &artifact,
                Path::new("unused")
            ),
            Err(PublisherTrustError::Unsupported)
        );
    }

    #[test]
    fn parser_accepts_only_one_closed_command() {
        assert_eq!(parse_command(["install"]), Ok(HelperCommand::Install));
        assert_eq!(parse_command(["remove"]), Ok(HelperCommand::Remove));
        assert!(matches!(
            parse_command(["install", "unexpected"]),
            Err(HelperError::InvalidArguments)
        ));
        assert!(matches!(
            parse_command(["diagnose"]),
            Err(HelperError::InvalidArguments)
        ));
    }

    #[test]
    fn specifications_are_narrow_and_exclude_public_profile() {
        let rules = managed_rule_specs(Path::new(r"C:\Program Files\AirWiki\airwiki.exe"));
        assert_eq!(rules[0].protocol, RuleProtocol::Tcp);
        assert_eq!(rules[0].local_port, None);
        assert_eq!(rules[1].protocol, RuleProtocol::Udp);
        assert_eq!(rules[1].local_port, Some(5353));
        assert!(rules.iter().all(|rule| {
            rule.profiles.includes_domain()
                && rule.profiles.includes_private()
                && !rule.profiles.includes_public()
                && rule.local_addresses == ALL_ADDRESSES
                && rule.remote_ports == ALL_PORTS
                && rule.remote_addresses == LOCAL_SUBNET
                && rule.interface_types == ALL_INTERFACES
                && rule.service_name == NO_SERVICE
                && rule.edge_traversal_blocked
                && rule.grouping == GROUP_NAME
                && rule.enabled
                && rule.inbound
                && rule.allow
        }));
    }

    #[test]
    fn installed_tcp_rule_uses_explicit_all_local_ports() {
        let [tcp, _] = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));

        assert_eq!(InstalledFirewallRule::from(&tcp).local_ports, "*");
    }

    #[test]
    fn implicit_empty_tcp_ports_are_not_an_exact_managed_rule() {
        let (expected, mut installed) = exact_tcp_rule_fixture();
        installed.local_ports.clear();

        assert!(!installed.exactly_matches(&expected));
    }

    #[test]
    fn managed_local_ports_have_one_canonical_serialization() {
        let [tcp, udp] = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));

        assert_eq!(
            (expected_local_ports(&tcp), expected_local_ports(&udp)),
            (String::from("*"), String::from("5353"))
        );
    }

    #[test]
    fn exact_match_rejects_a_different_local_address_scope() {
        let (expected, mut installed) = exact_tcp_rule_fixture();
        installed.local_addresses = "192.168.0.10".to_owned();

        assert!(!installed.exactly_matches(&expected));
    }

    #[test]
    fn exact_match_rejects_restricted_remote_ports() {
        let (expected, mut installed) = exact_tcp_rule_fixture();
        installed.remote_ports = "443".to_owned();

        assert!(!installed.exactly_matches(&expected));
    }

    #[test]
    fn exact_match_rejects_restricted_interface_types() {
        let (expected, mut installed) = exact_tcp_rule_fixture();
        installed.interface_types = "Wireless".to_owned();

        assert!(!installed.exactly_matches(&expected));
    }

    #[test]
    fn exact_match_rejects_a_service_scope() {
        let (expected, mut installed) = exact_tcp_rule_fixture();
        installed.service_name = "ExampleService".to_owned();

        assert!(!installed.exactly_matches(&expected));
    }

    #[test]
    fn legacy_scope_accepts_only_semantically_exact_local_subnet() {
        for safe in ["LocalSubnet", " localsubnet ", "LOCALSUBNET"] {
            assert!(remote_scope_is_exactly_local_subnet(safe));
        }
        for unsafe_scope in [
            "",
            "*",
            "192.168.0.0/16",
            "10.0.0.7",
            "Internet",
            "LocalSubnet,10.0.0.0/8",
            "LocalSubnet,LocalSubnet",
            "LocalSubnet,",
        ] {
            assert!(!remote_scope_is_exactly_local_subnet(unsafe_scope));
        }
    }

    #[test]
    fn install_preflights_conflict_without_mutation() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut conflicting = expected[1].clone();
        conflicting.local_port = Some(9999);
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            rules: BTreeMap::from([(
                expected[1].name.to_owned(),
                InstalledFirewallRule::from(&conflicting),
            )]),
            ..FakeBackend::default()
        };

        let result = install_with_backend(&mut backend, &expected);

        assert!(matches!(
            result,
            Err(HelperError::Conflict(name)) if name == UDP_RULE_NAME
        ));
        assert!(backend.added.is_empty());
        assert!(backend.removed.is_empty());
    }

    #[test]
    fn install_rolls_back_only_rules_added_in_failed_attempt() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            fail_add: Some(UDP_RULE_NAME),
            ..FakeBackend::default()
        };

        let result = install_with_backend(&mut backend, &expected);

        assert!(matches!(result, Err(HelperError::Backend)));
        assert_eq!(backend.added, vec![TCP_RULE_NAME]);
        assert_eq!(backend.removed, vec![TCP_RULE_NAME]);
    }

    #[test]
    fn install_does_not_roll_back_a_new_rule_that_no_longer_matches() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            fail_add: Some(UDP_RULE_NAME),
            mutate_before_add_failure: Some(TCP_RULE_NAME),
            ..FakeBackend::default()
        };

        let result = install_with_backend(&mut backend, &expected);

        assert_eq!(result, Err(HelperError::Backend));
        assert!(backend.removed.is_empty());
        assert!(backend.rules.contains_key(TCP_RULE_NAME));
    }

    #[test]
    fn install_fails_and_rolls_back_when_postcondition_is_missing() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            pretend_add_success: Some(UDP_RULE_NAME),
            ..FakeBackend::default()
        };

        let result = install_with_backend(&mut backend, &expected);

        assert_eq!(result, Err(HelperError::Backend));
        assert_eq!(backend.removed, vec![TCP_RULE_NAME]);
        assert!(backend.rules.is_empty());
    }

    #[test]
    fn remove_deletes_only_exact_managed_rules() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            rules: expected
                .iter()
                .map(|rule| (rule.name.to_owned(), InstalledFirewallRule::from(rule)))
                .collect(),
            ..FakeBackend::default()
        };

        remove_with_backend(&mut backend, &expected).expect("exact rules should be removable");

        assert_eq!(backend.removed, vec![TCP_RULE_NAME, UDP_RULE_NAME]);
        assert!(backend.rules.is_empty());
    }

    #[test]
    fn remove_refuses_conflict_without_partial_removal() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut conflicting = expected[1].clone();
        conflicting.edge_traversal_blocked = false;
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            rules: BTreeMap::from([
                (
                    expected[0].name.to_owned(),
                    InstalledFirewallRule::from(&expected[0]),
                ),
                (
                    expected[1].name.to_owned(),
                    InstalledFirewallRule::from(&conflicting),
                ),
            ]),
            ..FakeBackend::default()
        };

        let result = remove_with_backend(&mut backend, &expected);

        assert!(matches!(
            result,
            Err(HelperError::Conflict(name)) if name == UDP_RULE_NAME
        ));
        assert!(backend.removed.is_empty());
    }

    #[test]
    fn remove_preserves_same_name_tcp_rule_with_implicit_ports() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut implicit_ports = InstalledFirewallRule::from(&expected[0]);
        implicit_ports.local_ports.clear();
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            rules: BTreeMap::from([(TCP_RULE_NAME.to_owned(), implicit_ports)]),
            ..FakeBackend::default()
        };

        let result = remove_with_backend(&mut backend, &expected);

        assert_eq!(
            (
                result,
                backend.removed,
                backend.rules.contains_key(TCP_RULE_NAME),
            ),
            (
                Err(HelperError::Conflict(TCP_RULE_NAME.to_owned())),
                Vec::<&'static str>::new(),
                true,
            )
        );
    }

    #[test]
    fn remove_is_idempotent_when_managed_rules_are_absent() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            ..FakeBackend::default()
        };

        let result = remove_with_backend(&mut backend, &expected);

        assert_eq!(result, Ok(()));
        assert!(backend.removed.is_empty());
    }

    #[test]
    fn remove_fails_when_postcondition_still_contains_an_exact_rule() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            rules: expected
                .iter()
                .map(|rule| (rule.name.to_owned(), InstalledFirewallRule::from(rule)))
                .collect(),
            pretend_remove_success: Some(TCP_RULE_NAME),
            ..FakeBackend::default()
        };

        let result = remove_with_backend(&mut backend, &expected);

        assert_eq!(result, Err(HelperError::Backend));
        assert!(backend.rules.contains_key(TCP_RULE_NAME));
    }

    #[test]
    fn sibling_path_rejects_desktop_outside_helper_directory() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let helper = directory.path().join("airwiki-windows-firewall-helper.exe");
        fs::write(&helper, b"fixture").expect("helper fixture should be writable");

        assert!(matches!(
            sibling_desktop_path(&helper),
            Err(HelperError::InvalidLayout)
        ));
    }

    #[test]
    fn sibling_path_returns_canonical_desktop_in_same_directory() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let helper = directory.path().join("airwiki-windows-firewall-helper.exe");
        let desktop = directory.path().join(DESKTOP_BASENAME);
        fs::write(&helper, b"helper fixture").expect("helper fixture should be writable");
        fs::write(&desktop, b"desktop fixture").expect("desktop fixture should be writable");

        let discovered =
            sibling_desktop_path(&helper).expect("sibling desktop should be discovered");

        assert_eq!(
            discovered,
            desktop
                .canonicalize()
                .expect("desktop fixture should canonicalize")
        );
    }

    #[test]
    fn diagnostic_distinguishes_ready_missing_managed_and_inbound_blocked() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut backend = ready_backend(&expected);
        assert_eq!(
            diagnose_with_backend(&mut backend, &expected).status,
            FirewallDiagnosticStatus::Ready
        );

        backend.rules.remove(UDP_RULE_NAME);
        assert_eq!(
            diagnose_with_backend(&mut backend, &expected).status,
            FirewallDiagnosticStatus::RulesMissing
        );

        backend.local_policy = LocalPolicyState::Managed;
        assert_eq!(
            diagnose_with_backend(&mut backend, &expected).status,
            FirewallDiagnosticStatus::ManagedPolicy
        );

        backend.local_policy = LocalPolicyState::InboundBlocked;
        assert_eq!(
            diagnose_with_backend(&mut backend, &expected).status,
            FirewallDiagnosticStatus::BlockAllInbound
        );
    }

    #[test]
    fn mutations_preserve_managed_and_inbound_blocked_policy_causes() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));

        for (state, error) in [
            (LocalPolicyState::Managed, HelperError::ManagedPolicy),
            (
                LocalPolicyState::InboundBlocked,
                HelperError::InboundBlocked,
            ),
        ] {
            let mut backend = FakeBackend {
                local_policy: state,
                ..FakeBackend::default()
            };

            assert_eq!(install_with_backend(&mut backend, &expected), Err(error));
            assert!(backend.added.is_empty());
            assert!(backend.removed.is_empty());
        }
    }

    #[test]
    fn diagnostic_fails_closed_when_firewall_is_disabled_or_blocks_all_inbound() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));

        for (enforcement, expected_status) in [
            (
                ActiveFirewallEnforcement::Disabled,
                FirewallDiagnosticStatus::FirewallDisabled,
            ),
            (
                ActiveFirewallEnforcement::BlockAllInbound,
                FirewallDiagnosticStatus::BlockAllInbound,
            ),
        ] {
            let mut backend = ready_backend(&expected);
            backend.enforcement = enforcement;

            let diagnostic = diagnose_with_backend(&mut backend, &expected);

            assert_eq!(diagnostic.status, expected_status);
            assert!(backend.added.is_empty());
            assert!(backend.removed.is_empty());
        }
    }

    #[test]
    fn diagnostic_ignores_managed_equivalent_inactive_and_other_program_rules() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut equivalent = renamed_rule(&expected[0], "Legacy equivalent TCP");
        equivalent.local_ports = "443".to_owned();
        let mut disabled = renamed_rule(&expected[1], "Legacy disabled public UDP");
        disabled.enabled = false;
        disabled.profiles = FirewallProfiles(
            FirewallProfiles::DOMAIN | FirewallProfiles::PRIVATE | FirewallProfiles::PUBLIC,
        );
        let mut outbound = disabled.clone();
        outbound.name = "Legacy outbound public UDP".to_owned();
        outbound.enabled = true;
        outbound.inbound = false;
        let mut denied = disabled.clone();
        denied.name = "Legacy denied public UDP".to_owned();
        denied.enabled = true;
        denied.allow = false;
        let mut other_program = disabled.clone();
        other_program.name = "Other application public UDP".to_owned();
        other_program.enabled = true;
        other_program.program = PathBuf::from(r"C:\Other\other.exe");
        let mut backend = ready_backend(&expected);
        backend.extra_rules = vec![equivalent, disabled, outbound, denied, other_program];

        let diagnostic = diagnose_with_backend(&mut backend, &expected);

        assert_eq!(diagnostic.status, FirewallDiagnosticStatus::Ready);
        assert_eq!(diagnostic.exact_rule_count, 2);
        assert!(backend.added.is_empty());
        assert!(backend.removed.is_empty());
    }

    #[test]
    fn diagnostic_reports_public_legacy_exposure_without_mutation() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut legacy = renamed_rule(&expected[0], "Legacy public rule");
        legacy.profiles = FirewallProfiles(
            FirewallProfiles::DOMAIN | FirewallProfiles::PRIVATE | FirewallProfiles::PUBLIC,
        );
        let original = legacy.clone();
        let mut backend = ready_backend(&expected);
        backend.extra_rules.push(legacy);

        let diagnostic = diagnose_with_backend(&mut backend, &expected);

        assert_eq!(diagnostic.status, FirewallDiagnosticStatus::LegacyExposure);
        assert_eq!(diagnostic.exact_rule_count, 2);
        assert_eq!(backend.extra_rules, [original]);
        assert!(backend.added.is_empty());
        assert!(backend.removed.is_empty());
    }

    #[test]
    fn diagnostic_reports_broader_scope_port_and_protocol() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut broad_scope = renamed_rule(&expected[0], "Legacy any remote");
        broad_scope.remote_addresses = "*".to_owned();
        let mut broad_udp_ports = renamed_rule(&expected[1], "Legacy broad UDP ports");
        broad_udp_ports.local_ports = "5353,5354".to_owned();
        let mut broad_protocol = renamed_rule(&expected[0], "Legacy any protocol");
        broad_protocol.protocol = RuleProtocol::Other(256);
        let mut edge_traversal = renamed_rule(&expected[0], "Legacy edge traversal");
        edge_traversal.edge_traversal_blocked = false;

        for legacy in [broad_scope, broad_udp_ports, broad_protocol, edge_traversal] {
            let mut backend = ready_backend(&expected);
            backend.extra_rules.push(legacy);

            assert_eq!(
                diagnose_with_backend(&mut backend, &expected).status,
                FirewallDiagnosticStatus::LegacyExposure
            );
            assert!(backend.added.is_empty());
            assert!(backend.removed.is_empty());
        }
    }

    #[test]
    fn diagnostic_fails_closed_for_every_non_local_subnet_remote_scope() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));

        for remote_scope in [
            "192.168.0.0/16",
            "10.0.0.7",
            "Internet",
            "LocalSubnet,10.0.0.0/8",
        ] {
            let mut legacy = renamed_rule(&expected[0], "Legacy non-local scope");
            legacy.remote_addresses = remote_scope.to_owned();
            let mut backend = ready_backend(&expected);
            backend.extra_rules.push(legacy);

            assert_eq!(
                diagnose_with_backend(&mut backend, &expected).status,
                FirewallDiagnosticStatus::LegacyExposure,
                "scope `{remote_scope}` must fail closed"
            );
            assert!(backend.added.is_empty());
            assert!(backend.removed.is_empty());
        }
    }

    #[test]
    fn diagnostic_reports_legacy_exposure_before_missing_managed_rules() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut legacy = renamed_rule(&expected[1], "Legacy UDP any port");
        legacy.local_ports = "*".to_owned();
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            extra_rules: vec![legacy],
            ..FakeBackend::default()
        };

        let diagnostic = diagnose_with_backend(&mut backend, &expected);

        assert_eq!(diagnostic.status, FirewallDiagnosticStatus::LegacyExposure);
        assert_eq!(diagnostic.exact_rule_count, 0);
    }

    #[test]
    fn diagnostic_reports_the_four_broad_windows_rules_as_legacy_exposure() {
        let expected = managed_rule_specs(Path::new(r"C:\AirWiki\airwiki.exe"));
        let mut legacy_rules = [
            renamed_rule(&expected[0], "Legacy private TCP"),
            renamed_rule(&expected[1], "Legacy private UDP"),
            renamed_rule(&expected[0], "Legacy public TCP"),
            renamed_rule(&expected[1], "Legacy public UDP"),
        ];
        for rule in &mut legacy_rules {
            rule.local_ports = "*".to_owned();
            rule.remote_addresses = "*".to_owned();
            rule.edge_traversal_blocked = false;
        }
        legacy_rules[0].profiles = FirewallProfiles(FirewallProfiles::PRIVATE);
        legacy_rules[1].profiles = FirewallProfiles(FirewallProfiles::PRIVATE);
        legacy_rules[2].profiles = FirewallProfiles(FirewallProfiles::PUBLIC);
        legacy_rules[3].profiles = FirewallProfiles(FirewallProfiles::PUBLIC);
        let mut backend = FakeBackend {
            local_policy: LocalPolicyState::Effective,
            extra_rules: Vec::from(legacy_rules),
            ..FakeBackend::default()
        };

        let diagnostic = diagnose_with_backend(&mut backend, &expected);

        assert_eq!(
            (
                diagnostic.status,
                diagnostic.exact_rule_count,
                backend.extra_rules.len(),
                backend.added.len(),
                backend.removed.len(),
            ),
            (FirewallDiagnosticStatus::LegacyExposure, 0, 4, 0, 0)
        );
    }

    #[test]
    fn diagnostic_enumeration_failure_is_sanitized_and_fails_closed() {
        let expected = managed_rule_specs(Path::new(r"C:\Sensitive\airwiki.exe"));
        let mut backend = ready_backend(&expected);
        backend.fail_enumeration = true;

        let diagnostic = diagnose_with_backend(&mut backend, &expected);
        let rendered = format!("{diagnostic:?}");

        assert_eq!(diagnostic.status, FirewallDiagnosticStatus::Error);
        assert!(!rendered.contains("Sensitive"));
        assert!(!rendered.contains("airwiki.exe"));
        assert!(backend.added.is_empty());
        assert!(backend.removed.is_empty());
    }

    #[test]
    fn every_error_has_a_stable_non_success_exit_code() {
        let cases = [
            (HelperError::ManagedPolicy, HelperExitCode::ManagedPolicy),
            (HelperError::InboundBlocked, HelperExitCode::InboundBlocked),
            (
                HelperError::Conflict(TCP_RULE_NAME.to_owned()),
                HelperExitCode::Conflict,
            ),
            (
                HelperError::InvalidSignature,
                HelperExitCode::InvalidLayoutOrSignature,
            ),
            (HelperError::Unsupported, HelperExitCode::Unsupported),
            (HelperError::Backend, HelperExitCode::InternalError),
            (
                HelperError::InvalidArguments,
                HelperExitCode::InvalidArguments,
            ),
        ];

        assert!(cases.into_iter().all(|(error, expected)| {
            error.exit_code() == expected && expected != HelperExitCode::Success
        }));
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_platform_fails_closed() {
        assert!(matches!(
            run_platform(HelperCommand::Install),
            Err(HelperError::Unsupported)
        ));
        assert_eq!(
            diagnose_platform().status,
            FirewallDiagnosticStatus::Unsupported
        );
        assert_eq!(
            diagnose_sibling_helper_trust(),
            FirewallHelperTrustStatus::Unsupported
        );
    }
}
