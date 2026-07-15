//! Pure readiness projection for the desktop dashboard.
//!
//! This module deliberately contains no I/O or UI code. Platform probes and
//! worker snapshots are reduced here into a small, stable vocabulary that the
//! presentation layer can translate without exposing technical implementation
//! details.

use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadinessStatus {
    Ready,
    Working,
    NeedsPermission,
    NeedsAttention,
    OptionalDisabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadinessComponent {
    LocalAi,
    Collections,
    Review,
    Wiki,
    Lan,
    Chat,
    Background,
    Updates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ComponentReadinessView {
    pub component: ReadinessComponent,
    pub status: ReadinessStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectivityPreference {
    Undecided,
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Platform probes populate the full vocabulary incrementally. Keeping the
// closed enum now prevents stringly-typed permission handling in UI code.
#[allow(dead_code)]
pub(crate) enum SystemPermission {
    Unknown,
    NotRequired,
    Pending,
    Granted,
    Denied,
    Restricted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum NetworkProfile {
    Unknown,
    NotApplicable,
    Private,
    Domain,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum FirewallState {
    Unknown,
    NotRequired,
    Ready,
    Disabled,
    BlockAllInbound,
    RulesMissing,
    LegacyExposure,
    Unsupported,
    Error,
    Blocked,
    HelperUnavailable,
    Managed,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ListenerState {
    Stopped,
    Starting,
    Listening,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum DiscoveryState {
    Disabled,
    Starting,
    Active,
    PermissionDenied,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecommendedAction {
    PrepareLocalAi,
    ResolveLocalAiIssue,
    AddKnowledgeFolder,
    ResolveCollectionIssue,
    ReviewPendingKnowledge,
    InspectWikiHealth,
    ExplainLan,
    RequestSystemPermission,
    ChangeNetworkProfile,
    ConfigureFirewall,
    OpenFirewallSettings,
    ReviewLegacyFirewallRules,
    RepairConnectivityInstallation,
    ContactAdministrator,
    RetryConnectivity,
    ResolveChatIssue,
    ResolveBackgroundIssue,
    ResolveUpdateIssue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectivityView {
    pub preference: ConnectivityPreference,
    pub system_permission: SystemPermission,
    pub network_profile: NetworkProfile,
    pub firewall: FirewallState,
    pub listener: ListenerState,
    pub discovery: DiscoveryState,
    pub peer_count: usize,
    pub recommended_action: Option<RecommendedAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeReadinessView {
    pub components: [ComponentReadinessView; 8],
    pub primary_action: Option<RecommendedAction>,
    pub last_checked_at: SystemTime,
}

impl NodeReadinessView {
    pub(crate) fn status(&self, component: ReadinessComponent) -> ReadinessStatus {
        // `components` is constructed below from every enum variant exactly
        // once, so the fallback is unreachable for values produced here. It is
        // kept total to avoid panicking if that representation changes later.
        self.components
            .iter()
            .find(|view| view.component == component)
            .map_or(ReadinessStatus::NeedsAttention, |view| view.status)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectivityInput {
    pub preference: ConnectivityPreference,
    pub system_permission: SystemPermission,
    pub network_profile: NetworkProfile,
    pub firewall: FirewallState,
    pub listener: ListenerState,
    pub discovery: DiscoveryState,
    pub peer_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OptionalFeatureState {
    Ready,
    Working,
    NeedsPermission,
    NeedsAttention,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReadinessInput {
    pub models_ready: bool,
    pub models_working: bool,
    pub model_issue_count: usize,
    pub models_need_permission: bool,
    pub collection_count: usize,
    pub collections_working: bool,
    pub collection_issue_count: usize,
    pub pending_review_count: usize,
    pub wiki_working: bool,
    pub wiki_issue_count: usize,
    pub connectivity: ConnectivityInput,
    pub chat: OptionalFeatureState,
    pub background: OptionalFeatureState,
    pub updates: OptionalFeatureState,
    pub last_checked_at: SystemTime,
}

pub(crate) fn derive_connectivity(input: ConnectivityInput) -> ConnectivityView {
    let recommended_action = connectivity_action(input);
    ConnectivityView {
        preference: input.preference,
        system_permission: input.system_permission,
        network_profile: input.network_profile,
        firewall: input.firewall,
        listener: input.listener,
        discovery: input.discovery,
        peer_count: input.peer_count,
        recommended_action,
    }
}

pub(crate) fn derive_readiness(input: ReadinessInput) -> NodeReadinessView {
    let connectivity = derive_connectivity(input.connectivity);
    let local_ai = local_ai_status(input);
    let collections = collections_status(input);
    let review = if input.pending_review_count == 0 {
        ReadinessStatus::Ready
    } else {
        ReadinessStatus::NeedsAttention
    };
    let wiki = if input.wiki_issue_count > 0 {
        ReadinessStatus::NeedsAttention
    } else if input.wiki_working {
        ReadinessStatus::Working
    } else if input.collection_count == 0 {
        ReadinessStatus::OptionalDisabled
    } else {
        ReadinessStatus::Ready
    };
    let lan = connectivity_status(&connectivity);
    let chat = optional_status(input.chat);
    let background = optional_status(input.background);
    let updates = optional_status(input.updates);

    let components = [
        ComponentReadinessView {
            component: ReadinessComponent::LocalAi,
            status: local_ai,
        },
        ComponentReadinessView {
            component: ReadinessComponent::Collections,
            status: collections,
        },
        ComponentReadinessView {
            component: ReadinessComponent::Review,
            status: review,
        },
        ComponentReadinessView {
            component: ReadinessComponent::Wiki,
            status: wiki,
        },
        ComponentReadinessView {
            component: ReadinessComponent::Lan,
            status: lan,
        },
        ComponentReadinessView {
            component: ReadinessComponent::Chat,
            status: chat,
        },
        ComponentReadinessView {
            component: ReadinessComponent::Background,
            status: background,
        },
        ComponentReadinessView {
            component: ReadinessComponent::Updates,
            status: updates,
        },
    ];

    NodeReadinessView {
        primary_action: primary_action(input, &connectivity),
        components,
        last_checked_at: input.last_checked_at,
    }
}

fn local_ai_status(input: ReadinessInput) -> ReadinessStatus {
    if input.models_need_permission {
        ReadinessStatus::NeedsPermission
    } else if input.model_issue_count > 0 {
        ReadinessStatus::NeedsAttention
    } else if input.models_working {
        ReadinessStatus::Working
    } else if input.models_ready {
        ReadinessStatus::Ready
    } else {
        ReadinessStatus::NeedsAttention
    }
}

fn collections_status(input: ReadinessInput) -> ReadinessStatus {
    if input.collection_issue_count > 0 {
        ReadinessStatus::NeedsAttention
    } else if input.collections_working {
        ReadinessStatus::Working
    } else if input.collection_count == 0 {
        ReadinessStatus::NeedsAttention
    } else {
        ReadinessStatus::Ready
    }
}

fn optional_status(state: OptionalFeatureState) -> ReadinessStatus {
    match state {
        OptionalFeatureState::Ready => ReadinessStatus::Ready,
        OptionalFeatureState::Working => ReadinessStatus::Working,
        OptionalFeatureState::NeedsPermission => ReadinessStatus::NeedsPermission,
        OptionalFeatureState::NeedsAttention => ReadinessStatus::NeedsAttention,
        OptionalFeatureState::Disabled => ReadinessStatus::OptionalDisabled,
    }
}

fn connectivity_status(view: &ConnectivityView) -> ReadinessStatus {
    match view.preference {
        ConnectivityPreference::Undecided | ConnectivityPreference::Disabled => {
            ReadinessStatus::OptionalDisabled
        }
        ConnectivityPreference::Enabled => {
            if matches!(
                view.system_permission,
                SystemPermission::Denied | SystemPermission::Restricted
            ) || view.network_profile == NetworkProfile::Public
            {
                ReadinessStatus::NeedsPermission
            } else if matches!(
                view.system_permission,
                SystemPermission::Unknown | SystemPermission::Pending
            ) || view.firewall == FirewallState::Unknown
                || view.listener == ListenerState::Starting
                || view.discovery == DiscoveryState::Starting
            {
                ReadinessStatus::Working
            } else if matches!(
                view.firewall,
                FirewallState::Blocked
                    | FirewallState::Disabled
                    | FirewallState::BlockAllInbound
                    | FirewallState::RulesMissing
                    | FirewallState::LegacyExposure
                    | FirewallState::Unsupported
                    | FirewallState::Error
                    | FirewallState::HelperUnavailable
                    | FirewallState::Managed
                    | FirewallState::Conflict
            ) || view.network_profile == NetworkProfile::Unknown
                || matches!(
                    view.listener,
                    ListenerState::Stopped | ListenerState::Failed
                )
                || matches!(
                    view.discovery,
                    DiscoveryState::Disabled
                        | DiscoveryState::PermissionDenied
                        | DiscoveryState::Failed
                )
            {
                ReadinessStatus::NeedsAttention
            } else {
                ReadinessStatus::Ready
            }
        }
    }
}

fn connectivity_action(input: ConnectivityInput) -> Option<RecommendedAction> {
    match input.preference {
        ConnectivityPreference::Undecided => Some(RecommendedAction::ExplainLan),
        ConnectivityPreference::Disabled => None,
        ConnectivityPreference::Enabled => {
            if matches!(
                input.system_permission,
                SystemPermission::Denied | SystemPermission::Restricted
            ) || input.discovery == DiscoveryState::PermissionDenied
            {
                Some(RecommendedAction::RequestSystemPermission)
            } else if input.network_profile == NetworkProfile::Public {
                Some(RecommendedAction::ChangeNetworkProfile)
            } else if input.network_profile == NetworkProfile::Unknown {
                Some(RecommendedAction::RetryConnectivity)
            } else {
                match input.firewall {
                    FirewallState::HelperUnavailable | FirewallState::Unsupported => {
                        Some(RecommendedAction::RepairConnectivityInstallation)
                    }
                    FirewallState::Disabled | FirewallState::BlockAllInbound => {
                        Some(RecommendedAction::OpenFirewallSettings)
                    }
                    FirewallState::LegacyExposure => {
                        Some(RecommendedAction::ReviewLegacyFirewallRules)
                    }
                    FirewallState::RulesMissing => Some(RecommendedAction::ConfigureFirewall),
                    FirewallState::Managed | FirewallState::Conflict => {
                        Some(RecommendedAction::ContactAdministrator)
                    }
                    FirewallState::Error | FirewallState::Blocked => {
                        Some(RecommendedAction::RetryConnectivity)
                    }
                    _ if matches!(
                        input.listener,
                        ListenerState::Stopped | ListenerState::Failed
                    ) || matches!(
                        input.discovery,
                        DiscoveryState::Disabled | DiscoveryState::Failed
                    ) =>
                    {
                        Some(RecommendedAction::RetryConnectivity)
                    }
                    _ => None,
                }
            }
        }
    }
}

fn primary_action(
    input: ReadinessInput,
    connectivity: &ConnectivityView,
) -> Option<RecommendedAction> {
    if input.models_need_permission || input.model_issue_count > 0 {
        return Some(RecommendedAction::ResolveLocalAiIssue);
    }
    if !input.models_ready && !input.models_working {
        return Some(RecommendedAction::PrepareLocalAi);
    }
    if input.collection_issue_count > 0 {
        return Some(RecommendedAction::ResolveCollectionIssue);
    }
    if input.collection_count == 0 && !input.collections_working {
        return Some(RecommendedAction::AddKnowledgeFolder);
    }
    if input.pending_review_count > 0 {
        return Some(RecommendedAction::ReviewPendingKnowledge);
    }
    if input.wiki_issue_count > 0 {
        return Some(RecommendedAction::InspectWikiHealth);
    }
    if let Some(action) = connectivity.recommended_action {
        return Some(action);
    }
    if matches!(
        input.chat,
        OptionalFeatureState::NeedsPermission | OptionalFeatureState::NeedsAttention
    ) {
        return Some(RecommendedAction::ResolveChatIssue);
    }
    if matches!(
        input.background,
        OptionalFeatureState::NeedsPermission | OptionalFeatureState::NeedsAttention
    ) {
        return Some(RecommendedAction::ResolveBackgroundIssue);
    }
    if matches!(
        input.updates,
        OptionalFeatureState::NeedsPermission | OptionalFeatureState::NeedsAttention
    ) {
        return Some(RecommendedAction::ResolveUpdateIssue);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_connectivity() -> ConnectivityInput {
        ConnectivityInput {
            preference: ConnectivityPreference::Enabled,
            system_permission: SystemPermission::Granted,
            network_profile: NetworkProfile::Private,
            firewall: FirewallState::Ready,
            listener: ListenerState::Listening,
            discovery: DiscoveryState::Active,
            peer_count: 1,
        }
    }

    fn ready_input() -> ReadinessInput {
        ReadinessInput {
            models_ready: true,
            models_working: false,
            model_issue_count: 0,
            models_need_permission: false,
            collection_count: 1,
            collections_working: false,
            collection_issue_count: 0,
            pending_review_count: 0,
            wiki_working: false,
            wiki_issue_count: 0,
            connectivity: ready_connectivity(),
            chat: OptionalFeatureState::Ready,
            background: OptionalFeatureState::Ready,
            updates: OptionalFeatureState::Ready,
            last_checked_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn all_ready_has_no_primary_action() {
        let view = derive_readiness(ready_input());

        assert_eq!(view.primary_action, None);
        assert!(
            view.components
                .iter()
                .all(|component| component.status == ReadinessStatus::Ready)
        );
    }

    #[test]
    fn core_attention_order_is_ai_collection_review_then_wiki() {
        let mut input = ready_input();
        input.model_issue_count = 1;
        input.collection_issue_count = 1;
        input.pending_review_count = 2;
        input.wiki_issue_count = 1;
        assert_eq!(
            derive_readiness(input).primary_action,
            Some(RecommendedAction::ResolveLocalAiIssue)
        );

        input.model_issue_count = 0;
        assert_eq!(
            derive_readiness(input).primary_action,
            Some(RecommendedAction::ResolveCollectionIssue)
        );

        input.collection_issue_count = 0;
        assert_eq!(
            derive_readiness(input).primary_action,
            Some(RecommendedAction::ReviewPendingKnowledge)
        );

        input.pending_review_count = 0;
        assert_eq!(
            derive_readiness(input).primary_action,
            Some(RecommendedAction::InspectWikiHealth)
        );
    }

    #[test]
    fn active_work_does_not_create_a_fake_action() {
        let mut input = ready_input();
        input.models_ready = false;
        input.models_working = true;
        input.collections_working = true;

        let view = derive_readiness(input);

        assert_eq!(
            view.status(ReadinessComponent::LocalAi),
            ReadinessStatus::Working
        );
        assert_eq!(
            view.status(ReadinessComponent::Collections),
            ReadinessStatus::Working
        );
        assert_eq!(view.primary_action, None);
    }

    #[test]
    fn empty_collection_is_the_next_action_after_models() {
        let mut input = ready_input();
        input.collection_count = 0;

        let view = derive_readiness(input);

        assert_eq!(
            view.status(ReadinessComponent::Collections),
            ReadinessStatus::NeedsAttention
        );
        assert_eq!(
            view.status(ReadinessComponent::Wiki),
            ReadinessStatus::OptionalDisabled
        );
        assert_eq!(
            view.primary_action,
            Some(RecommendedAction::AddKnowledgeFolder)
        );
    }

    #[test]
    fn disabled_lan_is_optional_and_has_no_action() {
        let mut input = ready_input();
        input.connectivity.preference = ConnectivityPreference::Disabled;
        input.connectivity.listener = ListenerState::Stopped;
        input.connectivity.discovery = DiscoveryState::Disabled;

        let view = derive_readiness(input);

        assert_eq!(
            view.status(ReadinessComponent::Lan),
            ReadinessStatus::OptionalDisabled
        );
        assert_eq!(view.primary_action, None);
    }

    #[test]
    fn undecided_lan_recommends_an_explanation_without_enabling_it() {
        let mut input = ready_input();
        input.connectivity.preference = ConnectivityPreference::Undecided;
        input.connectivity.listener = ListenerState::Stopped;
        input.connectivity.discovery = DiscoveryState::Disabled;

        let view = derive_readiness(input);

        assert_eq!(
            view.status(ReadinessComponent::Lan),
            ReadinessStatus::OptionalDisabled
        );
        assert_eq!(view.primary_action, Some(RecommendedAction::ExplainLan));
    }

    #[test]
    fn connectivity_permission_precedes_profile_and_firewall() {
        let mut input = ready_connectivity();
        input.system_permission = SystemPermission::Denied;
        input.network_profile = NetworkProfile::Public;
        input.firewall = FirewallState::Blocked;

        let view = derive_connectivity(input);

        assert_eq!(
            view.recommended_action,
            Some(RecommendedAction::RequestSystemPermission)
        );
        assert_eq!(connectivity_status(&view), ReadinessStatus::NeedsPermission);
    }

    #[test]
    fn public_profile_precedes_firewall_configuration() {
        let mut input = ready_connectivity();
        input.network_profile = NetworkProfile::Public;
        input.firewall = FirewallState::Blocked;

        let view = derive_connectivity(input);

        assert_eq!(
            view.recommended_action,
            Some(RecommendedAction::ChangeNetworkProfile)
        );
    }

    #[test]
    fn unknown_profile_needs_attention_instead_of_waiting_indefinitely() {
        let mut input = ready_connectivity();
        input.network_profile = NetworkProfile::Unknown;

        let view = derive_connectivity(input);

        assert_eq!(connectivity_status(&view), ReadinessStatus::NeedsAttention);
        assert_eq!(
            view.recommended_action,
            Some(RecommendedAction::RetryConnectivity)
        );
    }

    #[test]
    fn disabled_and_block_all_firewall_use_safe_settings_recovery() {
        for firewall in [FirewallState::Disabled, FirewallState::BlockAllInbound] {
            let mut input = ready_connectivity();
            input.firewall = firewall;

            let view = derive_connectivity(input);

            assert_eq!(
                view.recommended_action,
                Some(RecommendedAction::OpenFirewallSettings)
            );
            assert_eq!(connectivity_status(&view), ReadinessStatus::NeedsAttention);
        }
    }

    #[test]
    fn unavailable_helper_recommends_reinstall_instead_of_elevation() {
        let mut input = ready_connectivity();
        input.firewall = FirewallState::HelperUnavailable;

        let view = derive_connectivity(input);

        assert_eq!(
            view.recommended_action,
            Some(RecommendedAction::RepairConnectivityInstallation)
        );
    }

    #[test]
    fn firewall_conditions_recommend_their_actual_recovery_path() {
        let cases = [
            (
                FirewallState::RulesMissing,
                RecommendedAction::ConfigureFirewall,
            ),
            (
                FirewallState::LegacyExposure,
                RecommendedAction::ReviewLegacyFirewallRules,
            ),
            (
                FirewallState::Unsupported,
                RecommendedAction::RepairConnectivityInstallation,
            ),
            (FirewallState::Error, RecommendedAction::RetryConnectivity),
            (
                FirewallState::Managed,
                RecommendedAction::ContactAdministrator,
            ),
            (
                FirewallState::Conflict,
                RecommendedAction::ContactAdministrator,
            ),
        ];

        for (firewall, expected) in cases {
            let mut input = ready_connectivity();
            input.firewall = firewall;

            let view = derive_connectivity(input);

            assert_eq!(view.recommended_action, Some(expected));
            assert_eq!(connectivity_status(&view), ReadinessStatus::NeedsAttention);
        }
    }

    #[test]
    fn optional_feature_attention_follows_core_and_connectivity() {
        let mut input = ready_input();
        input.chat = OptionalFeatureState::NeedsAttention;
        input.background = OptionalFeatureState::NeedsPermission;
        input.updates = OptionalFeatureState::NeedsAttention;
        assert_eq!(
            derive_readiness(input).primary_action,
            Some(RecommendedAction::ResolveChatIssue)
        );

        input.chat = OptionalFeatureState::Ready;
        assert_eq!(
            derive_readiness(input).primary_action,
            Some(RecommendedAction::ResolveBackgroundIssue)
        );

        input.background = OptionalFeatureState::Disabled;
        assert_eq!(
            derive_readiness(input).primary_action,
            Some(RecommendedAction::ResolveUpdateIssue)
        );
    }

    #[test]
    fn disabled_optional_features_are_not_reported_as_failures() {
        let mut input = ready_input();
        input.chat = OptionalFeatureState::Disabled;
        input.background = OptionalFeatureState::Disabled;
        input.updates = OptionalFeatureState::Disabled;

        let view = derive_readiness(input);

        assert_eq!(
            view.status(ReadinessComponent::Chat),
            ReadinessStatus::OptionalDisabled
        );
        assert_eq!(
            view.status(ReadinessComponent::Background),
            ReadinessStatus::OptionalDisabled
        );
        assert_eq!(
            view.status(ReadinessComponent::Updates),
            ReadinessStatus::OptionalDisabled
        );
        assert_eq!(view.primary_action, None);
    }
}
