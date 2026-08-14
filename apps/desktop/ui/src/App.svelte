<script lang="ts">
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import BookOpen from '@lucide/svelte/icons/book-open';
  import CheckCircle2 from '@lucide/svelte/icons/circle-check-big';
  import FileText from '@lucide/svelte/icons/file-text';
  import History from '@lucide/svelte/icons/history';
  import Plus from '@lucide/svelte/icons/plus';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Settings2 from '@lucide/svelte/icons/settings-2';
  import Sparkles from '@lucide/svelte/icons/sparkles';
  import { listen } from '@tauri-apps/api/event';
  import { onMount, tick } from 'svelte';
  import { addFederationIndex, addWiki, allowPeerPairingAgain, approveReview, browsePublicWiki, cancelModelInstall, checkUpdates, configureFirewall, confirmPairing, connect, deleteWiki, dialPeer, downloadUpdate, executeComputation, executeGuidedWikiRepair, hideToTray, importOkf, installModels, installUpdate, loadReviewEvidence, loadWikiBundle, loadWikiPage, manageIntegration, openExternalLink, openSystemDestination, pairPeer, pickOkfImport, pickWikiFolder, prepareGuidedWikiRepair, quitCompletely, reanalyzeReview, refreshApplicationAccess, refreshAutostart, refreshComputations, refreshConnectivity, refreshWikiHealth, rejectComputation, rejectReview, relinkWiki, removeFederationIndex, rescanWiki, revokePeer, saveComputationResult, searchKnowledge, setApplicationWikiRole, setAutostart, setPublicPublisherBlocked, setWikiGrant, setWikiIndexing, updatePreferences, updatePublicWikiProfile, updateWikiPolicy, validateOkfImport, verifyWikiConcept, type AppSnapshot, type ApplicationWikiRoleInput, type CloseBehavior, type EnrichmentDraft, type FolderSelection, type IntegrationActionInput, type IntegrationClient, type KnowledgeConceptSummary, type KnowledgePageInput, type LanPreference, type LocalePreference, type OkfImportSummary, type PublicConceptSummaryDto, type ReviewSummary, type SearchCoverage, type SearchHitSummary, type SourceIssueSummary, type SystemDestination, type ThemePreference, type WikiPolicyInput, type WikiSummary } from './api';
  import KnowledgeGraph from './KnowledgeGraph.svelte';
  import ConnectionAdvanced from './ConnectionAdvanced.svelte';
  import GlobalSearch from './GlobalSearch.svelte';
  import OnboardingFlow from './OnboardingFlow.svelte';
  import PublicWikiViewer from './PublicWikiViewer.svelte';
  import SystemStatusBar from './SystemStatusBar.svelte';
  import WikiTable from './WikiTable.svelte';
  import Checkbox from './components/controls/Checkbox.svelte';
  import SelectField from './components/controls/SelectField.svelte';
  import Switch from './components/controls/Switch.svelte';
  import TextField from './components/controls/TextField.svelte';
  import { message, resolveLocale, type MessageArgs } from './i18n';

  type Destination = 'home' | 'wikis' | 'shared' | 'search' | 'system';
  type SystemSection = 'models' | 'preferences' | 'updates' | 'connectivity' | 'devices' | 'integrations';
  type ServiceTarget = 'knowledge' | 'connections' | 'apps';

  const destinations = [{ id: 'wikis', labelId: 'desktop-nav-wikis' }] as const;
  const systemSections = [
    { id: 'models', labelId: 'settings-local-ai' },
    { id: 'preferences', labelId: 'desktop-preferences' },
    { id: 'updates', labelId: 'updates-title' },
    { id: 'connectivity', labelId: 'connectivity-title' },
    { id: 'devices', labelId: 'devices-title' },
    { id: 'integrations', labelId: 'integrations-title' }
  ] as const;

  let destination: Destination = 'wikis';
  let systemSection: SystemSection = 'preferences';
  let runtimeMessageId = 'status-working';
  let snapshot: AppSnapshot | null = null;
  let folderSelection: FolderSelection | null = null;
  let relinkSelection: FolderSelection | null = null;
  let wikiName = '';
  let continuousIndexing = true;
  let newWikiMenuOpen = false;
  let okfImportSelection: FolderSelection | null = null;
  let okfImportSummary: OkfImportSummary | null = null;
  let editingWikiId: string | null = null;
  let detailsWikiId: string | null = null;
  let wikiPolicy: WikiPolicyInput = { localOnly: true, peerShareable: false, allowExternalAi: false, internetPublic: false };
  let publicDescription = '';
  let publicLanguages = '';
  let question = '';
  let includePublic = false;
  let actionMessage = '';
  let actionMessageTimeout: number | null = null;
  let actionBusy = false;
  let selectedReview: ReviewSummary | null = null;
  let editDraft: EnrichmentDraft | null = null;
  let selectedWikiId: string | null = null;
  let knowledgeMode: 'document' | 'graph' = 'document';
  let wikiTab: 'content' | 'pending' = 'content';
  let sharedTab: 'owned' | 'public' = 'owned';
  let createWikiOpen = false;
  let connectionsOpen = false;
  let locale: LocalePreference = 'system';
  let theme: ThemePreference = 'system';
  let lanPreference: LanPreference = 'undecided';
  let closeBehavior: CloseBehavior = 'ask';
  let automaticUpdateChecks = false;
  let syncedPreferences: NonNullable<AppSnapshot['preferences']> | null = null;
  let closeChoiceRequired = false;
  let modelLicensesConfirmed = false;
  let autostartBusy = false;
  let autostartRequestId: string | null = null;
  let wikiHealthRequestId: string | null = null;
  let connectivityRequestId: string | null = null;
  let peerActionId: string | null = null;
  let integrationRequestId: string | null = null;
  let updaterRequestId: string | null = null;
  let confirmUpdateInstall = false;
  let federationPeerId = '';
  let federationAddress = '';
  let manualPeerAddress = '';
  let publicBrowseRequestId: string | null = null;
  let publicBrowseOpen = false;
  let publicBrowseLoading = false;
  let pendingSearchConcept: { wikiId: string; conceptId: string } | null = null;
  let guidedRepairRequestId: string | null = null;
  let guidedRepairConfirmed = false;
  let computationSaveTargets: Record<string, string> = {};
  let mainScrollRegion: HTMLElement | null = null;
  let orderedWikis: WikiSummary[];
  let attentionWikis: WikiSummary[];
  let selectedWiki: WikiSummary | null;
  let selectedWikiReviews: ReviewSummary[];
  let sharedWikis: WikiSummary[];
  type DialogId = 'create-wiki' | 'wiki-details' | 'wiki-access' | 'review' | 'connections' | 'close-choice' | null;
  let activeDialogId: DialogId;
  let dialogFocusGeneration = 0;
  const dialogFocusState: { activeId: DialogId; returnTarget: HTMLElement | null } = { activeId: null, returnTarget: null };

  function actionMessageTone(): 'success' | 'progress' | 'error' {
    if (actionMessage === t('notice-operation-complete')) return 'success';
    if (actionMessage === t('status-working') || actionMessage === t('review-evidence-loading')) return 'progress';
    return 'error';
  }

  function showOperationComplete() {
    const completedMessage = t('notice-operation-complete');
    actionMessage = completedMessage;
    if (actionMessageTimeout !== null) window.clearTimeout(actionMessageTimeout);
    actionMessageTimeout = window.setTimeout(() => {
      if (actionMessage === completedMessage) actionMessage = '';
      actionMessageTimeout = null;
    }, 3200);
  }

  function applyPreferences(preferences: NonNullable<AppSnapshot['preferences']>) {
    locale = preferences.locale;
    theme = preferences.theme;
    lanPreference = preferences.lanPreference;
    closeBehavior = preferences.closeBehavior;
    automaticUpdateChecks = preferences.automaticUpdateChecks;
  }

  function samePreferences(
    left: NonNullable<AppSnapshot['preferences']>,
    right: NonNullable<AppSnapshot['preferences']>
  ): boolean {
    return left.locale === right.locale
      && left.theme === right.theme
      && left.lanPreference === right.lanPreference
      && left.closeBehavior === right.closeBehavior
      && left.automaticUpdateChecks === right.automaticUpdateChecks;
  }

  function formMatches(preferences: NonNullable<AppSnapshot['preferences']>): boolean {
    return preferences.locale === locale
      && preferences.theme === theme
      && preferences.lanPreference === lanPreference
      && preferences.closeBehavior === closeBehavior
      && preferences.automaticUpdateChecks === automaticUpdateChecks;
  }

  function syncPreferences(preferences: AppSnapshot['preferences']) {
    if (!preferences) return;
    const formChanged = syncedPreferences !== null && !formMatches(syncedPreferences);
    const serverChanged = syncedPreferences === null || !samePreferences(preferences, syncedPreferences);
    if (formChanged && !serverChanged) return;
    applyPreferences(preferences);
    syncedPreferences = { ...preferences };
  }

  function scrollMainTo(top: number) {
    const target = Math.max(0, top);
    void tick().then(() => {
      mainScrollRegion?.scrollTo({ top: target, left: 0, behavior: 'auto' });
    });
  }

  function dialogFocusableElements(node: HTMLElement | null): HTMLElement[] {
    if (!node) return [];
    return Array.from(node.querySelectorAll<HTMLElement>(
      'button:not([disabled]), a[href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])'
    )).filter((element) => {
      if (element.closest('[hidden], [inert], [aria-hidden="true"]')) return false;
      const closedDisclosure = element.closest<HTMLDetailsElement>('details:not([open])');
      if (closedDisclosure?.querySelector(':scope > summary') !== element) return false;
      return true;
    });
  }

  function dialogElement(dialogId: Exclude<DialogId, null>): HTMLElement | null {
    const labelIds: Record<Exclude<DialogId, null>, string> = {
      'create-wiki': 'create-wiki-title',
      'wiki-details': 'details-title',
      'wiki-access': 'share-title',
      review: 'review-title',
      connections: 'connections-title',
      'close-choice': 'close-title'
    };
    return document.querySelector<HTMLElement>(`[role="dialog"][aria-labelledby="${labelIds[dialogId]}"]`);
  }

  function topDialogElement(): HTMLElement | null {
    return Array.from(document.querySelectorAll<HTMLElement>('[role="dialog"]'))
      .filter((dialog) => !dialog.closest('[hidden], [inert], [aria-hidden="true"]'))
      .at(-1) ?? null;
  }

  function focusDialog(dialogId: Exclude<DialogId, null>) {
    const generation = ++dialogFocusGeneration;
    void tick().then(() => {
      if (generation !== dialogFocusGeneration) return;
      const dialog = dialogElement(dialogId);
      const preferredTarget = dialogId === 'close-choice'
        ? dialog?.querySelector<HTMLElement>('.primary')
        : dialog?.querySelector<HTMLElement>('.icon-button');
      (preferredTarget ?? dialogFocusableElements(dialog).at(0))?.focus();
    });
  }

  function pushHash(hash: string) {
    if (window.location.hash !== hash) window.history.pushState(null, '', hash);
  }

  function openSystemSection(event: MouseEvent, section: SystemSection) {
    event.preventDefault();
    systemSection = section;
    pushHash(`#system/${section}`);
    scrollMainTo(0);
  }

  function translate(id: string, args?: MessageArgs): string {
    return message(locale, id, args);
  }

  function translatorFor(_locale: LocalePreference): typeof translate {
    void _locale;
    return translate;
  }

  let t = translate;
  $: t = translatorFor(locale);
  $: orderedWikis = [...(snapshot?.wikis ?? [])].sort((left, right) => {
    const leftAttention = Number(left.failedCount > 0 || left.maintenanceRequired || left.needsReviewCount > 0);
    const rightAttention = Number(right.failedCount > 0 || right.maintenanceRequired || right.needsReviewCount > 0);
    return rightAttention - leftAttention || left.name.localeCompare(right.name, resolveLocale(locale));
  });
  $: attentionWikis = orderedWikis.filter((wiki) =>
    wiki.failedCount > 0
    || wiki.maintenanceRequired
    || wiki.needsReviewCount > 0
    || (snapshot?.sourceIssues.some((issue) => issue.wikiId === wiki.id) ?? false)
  );
  $: selectedWiki = snapshot?.wikis.find((wiki) => wiki.id === selectedWikiId) ?? null;
  $: selectedWikiReviews = snapshot?.reviews.filter((review) => review.wikiId === selectedWikiId) ?? [];
  $: sharedWikis = orderedWikis.filter((wiki) => wiki.peerShareable || wiki.allowExternalAi || wiki.internetPublic);

  const sourceIssueCodes: Record<string, string> = {
    FileTooLarge: 'file-too-large',
    Unreadable: 'unreadable',
    InvalidUtf8: 'invalid-utf8',
    InvalidPdf: 'invalid-pdf',
    EncryptedPdf: 'encrypted-pdf',
    TooManyPages: 'too-many-pages',
    NoTextLayer: 'no-text-layer',
    TooManyCharacters: 'too-many-characters',
    SourceMissing: 'source-missing',
    PermissionDenied: 'permission-denied',
    ProcessingFailed: 'processing-failed'
  };

  function sourceIssueLabel(issue: SourceIssueSummary): string {
    const code = sourceIssueCodes[issue.code];
    return t(code ? `review-issue-cause-${code}` : 'review-issue-cause-unknown');
  }

  function sourceIssueActionLabel(issue: SourceIssueSummary): string {
    const code = sourceIssueCodes[issue.code];
    return t(code ? `source-issue-action-${code}` : 'source-issue-action-unknown');
  }

  function searchCoverageMessage(coverage: SearchCoverage): string {
    const labels: Partial<Record<SearchCoverage, string>> = {
      federationDisabled: 'search-coverage-federation-disabled',
      offlineDevices: 'search-coverage-offline-devices',
      publicNetworkOffline: 'search-coverage-public-offline',
      partial: 'search-coverage-partial'
    };
    const label = labels[coverage];
    return label ? t(label) : '';
  }

  function wikiAttentionSummary(wiki: WikiSummary): string {
    const summaries: string[] = [];
    const issueCount = snapshot?.sourceIssues.filter((issue) => issue.wikiId === wiki.id).length ?? 0;
    const failedCount = Math.max(issueCount, wiki.failedCount);
    if (failedCount > 0) summaries.push(t('desktop-attention-files-summary', { count: failedCount }));
    if (wiki.maintenanceRequired) summaries.push(t('desktop-attention-maintenance-summary'));
    if (wiki.needsReviewCount > 0) summaries.push(t('desktop-wiki-review-count', { count: wiki.needsReviewCount }));
    return summaries.join(' · ');
  }

  $: if (typeof document !== 'undefined') {
    document.documentElement.lang = resolveLocale(locale);
    document.documentElement.dataset.theme = theme;
    if (snapshot) document.documentElement.dataset.platform = snapshot.platform;
    document.documentElement.style.colorScheme = theme === 'system' ? 'light dark' : theme;
  }

  $: activeDialogId = closeChoiceRequired ? 'close-choice'
    : selectedReview !== null ? 'review'
      : editingWikiId !== null ? 'wiki-access'
        : detailsWikiId !== null ? 'wiki-details'
          : createWikiOpen ? 'create-wiki'
            : connectionsOpen ? 'connections'
              : null;

  $: {
    const dialogId = activeDialogId;
    if (typeof document !== 'undefined' && dialogId !== dialogFocusState.activeId) {
      if (dialogId !== null) {
        if (dialogFocusState.activeId === null) {
          dialogFocusState.returnTarget = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        }
        focusDialog(dialogId);
      } else {
        const returnTarget = dialogFocusState.returnTarget;
        dialogFocusState.returnTarget = null;
        requestAnimationFrame(() => returnTarget?.focus());
      }
      dialogFocusState.activeId = dialogId;
    }
  }

  onMount(() => {
    const syncRoute = () => {
      const [rawRoute, section, detail] = window.location.hash.slice(1).split('/');
      const route = rawRoute === 'library' || rawRoute === 'review' || rawRoute === 'home' || rawRoute === 'shared' || rawRoute === '' ? 'wikis' : rawRoute;
      const matchedDestination = destinations.find((candidate) => candidate.id === route);
      if (matchedDestination) {
        destination = matchedDestination.id;
        if (route !== 'search') {
          publicBrowseOpen = false;
          publicBrowseLoading = false;
        }
        if (route === 'wikis' && section) {
          selectedWikiId = section;
          wikiTab = detail === 'pending' ? 'pending' : 'content';
        } else if (route === 'wikis') {
          selectedWikiId = null;
        }
        scrollMainTo(0);
      }
      const matchedSection = systemSections.find((candidate) => candidate.id === section);
      if (route === 'system' && matchedSection) {
        if (section === 'connectivity' || section === 'devices' || section === 'integrations') {
          destination = 'wikis';
          connectionsOpen = true;
          pushHash('#wikis');
          if (section === 'integrations') void runIntegrationAction({ kind: 'refresh' });
          else void runConnectivityAction('refresh');
          return;
        }
        destination = 'system';
        systemSection = matchedSection.id;
        scrollMainTo(0);
      } else if (route === 'system') {
        destination = 'system';
        systemSection = 'preferences';
        scrollMainTo(0);
      } else if (route === 'search') {
        destination = 'search';
      }
    };
    syncRoute();
    window.addEventListener('hashchange', syncRoute);
    window.addEventListener('popstate', syncRoute);
    const handleShortcut = (event: KeyboardEvent) => {
      const dialog = topDialogElement();
      if (dialog !== null && event.key === 'Tab') {
        const elements = dialogFocusableElements(dialog);
        const first = elements.at(0);
        const last = elements.at(-1);
        const initial = dialog.getAttribute('aria-labelledby') === 'close-title'
          ? dialog?.querySelector<HTMLElement>('.primary') ?? first
          : dialog?.querySelector<HTMLElement>('.icon-button') ?? first;
        if (!first || !last) {
          event.preventDefault();
          return;
        }
        if (event.shiftKey && (document.activeElement === first || document.activeElement === initial)) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          initial?.focus();
        }
        return;
      }
      if (dialog !== null && event.key !== 'Escape') return;
      const command = event.metaKey || event.ctrlKey;
      if (command && event.key === '1') {
        event.preventDefault();
        select('wikis');
      } else if (command && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        destination = 'search';
        pushHash('#search');
        requestAnimationFrame(() => document.querySelector<HTMLInputElement>('#global-search')?.focus());
      } else if (command && event.key === ',') {
        event.preventDefault();
        select('system');
      } else if (event.key === 'Escape') {
        confirmUpdateInstall = false;
        editingWikiId = null;
        detailsWikiId = null;
        relinkSelection = null;
        createWikiOpen = false;
        connectionsOpen = false;
        selectedReview = null;
        if (closeChoiceRequired) closeChoiceRequired = false;
      }
    };
    window.addEventListener('keydown', handleShortcut);
    const unlistenClose = '__TAURI_INTERNALS__' in window
      ? listen('close-choice-required', () => {
        closeChoiceRequired = true;
        focusDialog('close-choice');
      })
      : Promise.resolve(() => {});
    connect((event) => {
      snapshot = event.snapshot;
      void openPendingSearchConcept(event.snapshot);
      if (event.snapshot.model?.licenseAccepted) modelLicensesConfirmed = true;
      syncPreferences(event.snapshot.preferences);
      if (selectedReview) {
        const currentReview = event.snapshot.reviews.find((review) => review.conceptId === selectedReview?.conceptId);
        if (!currentReview || currentReview.sourceRevision !== selectedReview.sourceRevision) {
          selectedReview = null;
          editDraft = null;
        }
      }
      if (event.snapshot.search?.status !== 'searching') actionBusy = false;
      if (event.requestId && event.requestId === autostartRequestId) {
        autostartBusy = false;
        autostartRequestId = null;
      }
      if (event.requestId && event.requestId === wikiHealthRequestId) wikiHealthRequestId = null;
      if (event.requestId && event.requestId === connectivityRequestId) connectivityRequestId = null;
      if (event.requestId && event.requestId === integrationRequestId) integrationRequestId = null;
      if (event.requestId && event.requestId === updaterRequestId) updaterRequestId = null;
      if (event.requestId && event.requestId === publicBrowseRequestId) {
        publicBrowseRequestId = null;
        publicBrowseLoading = false;
      }
      if (event.requestId && event.requestId === guidedRepairRequestId) guidedRepairRequestId = null;
      runtimeMessageId = event.snapshot.phase === 'ready' ? 'status-ready' : 'status-working';
    }).then(async (initial) => {
      const connected = snapshot && snapshot.sequence > initial.sequence ? snapshot : initial;
      snapshot = connected;
      if (connected.model?.licenseAccepted) modelLicensesConfirmed = true;
      syncPreferences(connected.preferences);
      runtimeMessageId = connected.phase === 'ready' ? 'status-ready' : runtimeMessageId;
      await tick();
      syncRoute();
    }).catch(() => { runtimeMessageId = 'error-generic'; });
    if (destination === 'system') {
      void refreshAutostartState();
      void runConnectivityAction('refresh');
      void runIntegrationAction({ kind: 'refresh' });
    }
    return () => {
      window.removeEventListener('hashchange', syncRoute);
      window.removeEventListener('popstate', syncRoute);
      window.removeEventListener('keydown', handleShortcut);
      void unlistenClose.then((unlisten) => unlisten());
    };
  });

  function select(next: Destination) {
    actionMessage = '';
    destination = next;
    if (next !== 'search') {
      publicBrowseOpen = false;
      publicBrowseLoading = false;
    }
    scrollMainTo(0);
    if (next === 'system') {
      systemSection = 'preferences';
      pushHash('#system/preferences');
      void refreshAutostartState();
      void runConnectivityAction('refresh');
      void runIntegrationAction({ kind: 'refresh' });
    } else {
      pushHash(`#${next}`);
    }
    if (next === 'wikis') void refreshHealth();
  }

  function openServiceStatus(target: ServiceTarget) {
    actionMessage = '';
    if (target === 'knowledge' && (!snapshot?.model?.active || snapshot.model.degraded || snapshot.modelInstall)) {
      destination = 'system';
      systemSection = 'models';
      pushHash('#system/models');
      scrollMainTo(0);
      return;
    }
    destination = 'wikis';
    selectedWikiId = null;
    connectionsOpen = target === 'connections' || target === 'apps';
    pushHash('#wikis');
    scrollMainTo(0);
    if (target === 'connections') void runConnectivityAction('refresh');
    if (target === 'apps' && !snapshot?.integrations && !integrationRequestId) {
      void runIntegrationAction({ kind: 'refresh' });
    }
    if (target === 'knowledge') void refreshHealth();
  }

  function openGlobalSearch() {
    if (destination !== 'search') {
      destination = 'search';
      pushHash('#search');
      scrollMainTo(0);
    }
  }

  async function submitGlobalSearch() {
    openGlobalSearch();
    await submitSearch();
  }

  function openSharedTab(tab: 'owned' | 'public') {
    sharedTab = tab;
    pushHash(`#shared/${tab}`);
    if (tab === 'public') includePublic = true;
    scrollMainTo(0);
  }

  function openWikiTab(tab: 'content' | 'pending') {
    if (!selectedWikiId) return;
    wikiTab = tab;
    pushHash(`#wikis/${selectedWikiId}${tab === 'pending' ? '/pending' : ''}`);
    if (tab === 'pending' && selectedWikiReviews[0] && !selectedReview) void openReview(selectedWikiReviews[0]);
  }

  function setKnowledgeMode(mode: 'document' | 'graph') {
    const currentTop = mainScrollRegion?.scrollTop ?? 0;
    knowledgeMode = mode;
    scrollMainTo(currentTop);
  }

  function integrationName(client: IntegrationClient): string {
    if (client === 'chatGptDesktop') return 'ChatGPT Desktop / Work';
    if (client === 'claudeDesktop') return 'Claude Desktop';
    if (client === 'geminiCli') return 'Gemini CLI';
    return t('integrations-generic-mcp');
  }

  function integrationState(status: string): string {
    const labels: Record<string, string> = {
      notInstalled: 'integration-status-not-installed', available: 'integration-status-available',
      awaitingClientApproval: 'integration-status-awaiting-approval', configured: 'integration-status-configured',
      updateAvailable: 'integration-status-update-available', conflict: 'integration-status-conflict',
      unsupported: 'integration-status-unsupported', error: 'integration-status-error'
    };
    return t(labels[status] ?? 'error-generic');
  }

  async function runIntegrationAction(action: IntegrationActionInput) {
    if (integrationRequestId !== null) return;
    const requestId = crypto.randomUUID();
    integrationRequestId = requestId;
    try {
      await manageIntegration(requestId, action);
    } catch {
      integrationRequestId = null;
      actionMessage = t('error-chat');
    }
  }

  function mcpSetupText(command: string, args: string[]): string {
    return JSON.stringify({ command, args }, null, 2);
  }

  async function copyMcpSetup(command: string, args: string[]) {
    try {
      await navigator.clipboard.writeText(mcpSetupText(command, args));
      actionMessage = t('integrations-generic-copied');
    } catch {
      actionMessage = t('integrations-generic-copy-failed');
    }
  }

  async function decideComputation(runId: string, decision: 'execute' | 'reject') {
    actionBusy = true;
    actionMessage = '';
    try {
      if (decision === 'execute') await executeComputation(runId);
      else await rejectComputation(runId);
      await refreshComputations();
      showOperationComplete();
    } catch {
      actionMessage = t('desktop-computation-action-failed');
    } finally {
      actionBusy = false;
    }
  }

  async function saveAcceptedComputation(runId: string) {
    const targetWikiId = computationSaveTargets[runId];
    if (!targetWikiId) return;
    actionBusy = true;
    actionMessage = '';
    try {
      await saveComputationResult(runId, targetWikiId);
      await refreshComputations();
      showOperationComplete();
    } catch {
      actionMessage = t('desktop-computation-save-failed');
    } finally {
      actionBusy = false;
    }
  }

  async function changeApplicationGrant(appId: string, wikiId: string, role: ApplicationWikiRoleInput | null) {
    actionBusy = true;
    actionMessage = '';
    try {
      await setApplicationWikiRole(appId, wikiId, role);
      await refreshApplicationAccess();
      showOperationComplete();
    } catch {
      actionMessage = t('desktop-application-grant-failed');
    } finally {
      actionBusy = false;
    }
  }

  function updaterLabel(currentLocale: LocalePreference): string {
    const localize = (id: string, args?: MessageArgs) => message(currentLocale, id, args);
    const updater = snapshot?.updater;
    if (!updater) return localize('updates-loading');
    if (updater.issue === 'offline') return localize('updates-issue-offline');
    if (updater.issue === 'invalidSignature') return localize('updates-issue-signature');
    if (updater.issue === 'invalidManifest') return localize('updates-issue-manifest');
    if (updater.status === 'disabled') return localize(updater.issue === 'notConfigured' ? 'updates-disabled-not-configured' : 'updates-disabled-platform');
    if (updater.status === 'available') return localize('updates-available', { version: updater.version ?? '' });
    if (updater.status === 'downloading') return localize('updates-downloading', { version: updater.version ?? '' });
    if (updater.status === 'readyToInstall') return localize('updates-ready-install', { version: updater.version ?? '' });
    if (updater.status === 'installing') return localize('updates-installing', { version: updater.version ?? '' });
    if (updater.status === 'installed') return localize('updates-installed', { version: updater.version ?? '' });
    const labels: Record<string, string> = { idle: 'updates-idle', checking: 'updates-checking', upToDate: 'updates-current' };
    return localize(labels[updater.status] ?? 'error-update');
  }

  async function runUpdaterAction(action: 'check' | 'download' | 'install') {
    const requestId = crypto.randomUUID();
    updaterRequestId = requestId;
    confirmUpdateInstall = false;
    try {
      if (action === 'check') await checkUpdates(requestId);
      else if (action === 'download') await downloadUpdate(requestId);
      else await installUpdate(requestId);
    } catch {
      updaterRequestId = null;
      actionMessage = t('error-update');
    }
  }

  async function openVerifiedExternalLink(url: string) {
    try {
      await openExternalLink(url);
    } catch {
      actionMessage = t('error-generic');
    }
  }

  function connectivityLabel(currentLocale: LocalePreference): string {
    if (snapshot?.lanRuntime?.listener === 'listening') return message(currentLocale, 'connectivity-active');
    if (snapshot?.lanRuntime?.listener === 'starting') return message(currentLocale, 'connectivity-starting');
    if (snapshot?.connectivity?.networkProfile === 'public') return message(currentLocale, 'connectivity-public-network');
    if (snapshot?.connectivity?.firewall === 'rulesMissing') return message(currentLocale, 'connectivity-firewall-needed');
    if (snapshot?.connectivity?.firewall === 'conflict' || snapshot?.connectivity?.firewall === 'legacyExposure') return message(currentLocale, 'connectivity-admin-needed');
    if (snapshot?.lanRuntime?.listener === 'failed') return message(currentLocale, 'connectivity-failed');
    if (lanPreference === 'disabled') return message(currentLocale, 'connectivity-disabled');
    return message(currentLocale, 'connectivity-not-ready');
  }

  function firewallGuidanceLabel(): string {
    const firewall = snapshot?.connectivity?.firewall;
    if (firewall === 'firewallDisabled') return t('connectivity-firewall-disabled');
    if (firewall === 'blockAllInbound') return t('connectivity-firewall-block-all-inbound');
    if (firewall === 'legacyExposure') return t('connectivity-firewall-legacy-exposure');
    return t('connectivity-admin-needed');
  }

  async function runConnectivityAction(action: 'refresh' | 'install' | 'remove' | SystemDestination) {
    const requestId = crypto.randomUUID();
    connectivityRequestId = requestId;
    try {
      if (action === 'refresh') await refreshConnectivity(requestId);
      if (action === 'install') await configureFirewall(requestId, true);
      if (action === 'remove') await configureFirewall(requestId, false);
      if (action === 'networkSettings' || action === 'advancedFirewall' || action === 'localNetworkPrivacy') {
        await openSystemDestination(requestId, action);
      }
    } catch {
      connectivityRequestId = null;
      actionMessage = t('error-connectivity');
    }
  }

  function openNetworkPreferences() {
    connectionsOpen = false;
    select('system');
  }

  function shortPeerId(peerId: string): string {
    return peerId.length > 18 ? `${peerId.slice(0, 9)}…${peerId.slice(-7)}` : peerId;
  }

  async function runPeerAction(peerId: string, action: 'pair' | 'accept' | 'reject' | 'revoke' | 'allowAgain') {
    peerActionId = peerId;
    try {
      if (action === 'pair') await pairPeer(peerId);
      if (action === 'accept') await confirmPairing(peerId, true);
      if (action === 'reject') await confirmPairing(peerId, false);
      if (action === 'revoke') await revokePeer(peerId);
      if (action === 'allowAgain') await allowPeerPairingAgain(peerId);
    } catch {
      actionMessage = t('error-connectivity');
    } finally {
      peerActionId = null;
    }
  }

  async function changeGrant(peerId: string, wikiId: string, granted: boolean) {
    peerActionId = peerId;
    try {
      await setWikiGrant(peerId, wikiId, granted);
    } catch {
      actionMessage = t('error-connectivity');
    } finally {
      peerActionId = null;
    }
  }

  async function refreshHealth() {
    const requestId = crypto.randomUUID();
    wikiHealthRequestId = requestId;
    try {
      await refreshWikiHealth(requestId);
    } catch {
      wikiHealthRequestId = null;
      actionMessage = t('home-wiki-failed');
    }
  }

  async function openAttentionWiki() {
    const wikiId = snapshot?.wikiHealth?.attentionWikiId;
    if (wikiId) await openWiki(wikiId);
  }

  function autostartLabel(currentLocale: LocalePreference): string {
    const status = snapshot?.autostart;
    const labels = { enabled: 'autostart-enabled', disabled: 'autostart-disabled', requiresApproval: 'autostart-needs-approval', conflict: 'autostart-conflict', unsupported: 'autostart-unsupported' } as const;
    const statusLabel = status ? message(currentLocale, labels[status]) : message(currentLocale, 'autostart-checking');
    return message(currentLocale, 'settings-login-status', { status: statusLabel });
  }

  async function refreshAutostartState() {
    autostartBusy = true;
    const requestId = crypto.randomUUID();
    autostartRequestId = requestId;
    try {
      await refreshAutostart(requestId);
    } catch {
      actionMessage = t('error-generic');
      autostartBusy = false;
      autostartRequestId = null;
    }
  }

  async function changeAutostart(enabled: boolean) {
    autostartBusy = true;
    const requestId = crypto.randomUUID();
    autostartRequestId = requestId;
    try {
      await setAutostart(requestId, enabled);
      showOperationComplete();
    } catch {
      actionMessage = t('error-generic');
      autostartBusy = false;
      autostartRequestId = null;
    }
  }

  function pageKey(page: KnowledgePageInput): string {
    return page.kind === 'concept' ? `concept:${page.path}` : page.kind;
  }

  function compatibilityLabel(wiki: WikiSummary): string {
    return t(`desktop-okf-compatibility-${wiki.okfCompatibility.kind}`);
  }

  function assuranceLabel(concept: KnowledgeConceptSummary): string {
    if (concept.assurance.verificationOutdated) return t('desktop-assurance-outdated');
    return t(`desktop-trust-${concept.assurance.trust}`);
  }

  function searchAssuranceLabel(hit: SearchHitSummary): string | null {
    if (!hit.assurance) return null;
    if (hit.assurance.verificationOutdated) return t('desktop-assurance-outdated');
    const trust = t(`desktop-trust-${hit.assurance.trust}`);
    if (hit.assurance.freshness === 'stale' || hit.assurance.freshness === 'invalid') {
      return `${trust} · ${t(`desktop-freshness-${hit.assurance.freshness}`)}`;
    }
    return trust;
  }

  function publicConceptMetadata(concept: PublicConceptSummaryDto): string {
    if (!concept.assurance) return t('desktop-public-metadata-unavailable');
    const trust = concept.assurance.verificationOutdated
      ? t('desktop-assurance-outdated')
      : t(`desktop-trust-${concept.assurance.trust}`);
    const freshness = t(`desktop-freshness-${concept.assurance.freshness}`);
    return concept.lifecycle ? `${trust} · ${freshness} · ${concept.lifecycle}` : `${trust} · ${freshness}`;
  }

  function canVerifyConcept(wiki: WikiSummary, concept: KnowledgeConceptSummary): boolean {
    return wiki.origin !== 'folder'
      && wiki.restrictions.length === 0
      && concept.assurance.trust !== 'humanReviewed';
  }

  async function verifyConcept(wiki: WikiSummary, concept: KnowledgeConceptSummary) {
    if (concept.page.kind !== 'concept') return;
    actionBusy = true;
    actionMessage = '';
    try {
      await verifyWikiConcept(wiki.id, concept.page.path, concept.fingerprint);
      await loadWikiBundle(wiki.id);
    } catch {
      actionMessage = t('desktop-concept-verify-failed');
    } finally {
      actionBusy = false;
    }
  }

  function applicationGrantRole(appId: string, wikiId: string): ApplicationWikiRoleInput | 'none' {
    const role = snapshot?.applicationAccess
      .find((application) => application.appId === appId)
      ?.grants.find((grant) => grant.wikiId === wikiId)?.role ?? 'none';
    return role === 'reader' || role === 'editor' ? role : 'none';
  }

  function wikiNameFor(wikiId: string): string {
    return snapshot?.wikis.find((wiki) => wiki.id === wikiId)?.name ?? t('desktop-wiki-unknown');
  }

  function peerNameFor(nodeId: string): string | null {
    const peer = snapshot?.peers.find((candidate) => candidate.peerId === nodeId && candidate.trust === 'trusted');
    return peer ? (peer.deviceName ?? t('devices-nearby')) : null;
  }

  function searchOriginFor(hit: SearchHitSummary): string {
    if (hit.nodeId === snapshot?.nodeId) return wikiNameFor(hit.wikiId);
    return peerNameFor(hit.nodeId) ?? t('desktop-public-network');
  }

  function searchSourceFor(hit: SearchHitSummary): string {
    if (hit.nodeId === snapshot?.nodeId) return t('desktop-search-local-source');
    return peerNameFor(hit.nodeId) ? t('desktop-search-nearby-source') : t('desktop-public-network');
  }

  function isPublicSearchHit(hit: SearchHitSummary): boolean {
    return hit.nodeId !== snapshot?.nodeId && peerNameFor(hit.nodeId) === null;
  }

  function wikiPeers(wikiId: string): string[] {
    return snapshot?.peers
      .filter((peer) => peer.trust === 'trusted' && peer.grantedWikiIds.includes(wikiId))
      .map((peer) => peer.deviceName ?? t('devices-nearby')) ?? [];
  }

  async function chooseFolder() {
    actionMessage = '';
    newWikiMenuOpen = false;
    try {
      folderSelection = await pickWikiFolder();
      if (folderSelection) createWikiOpen = true;
    } catch {
      actionMessage = t('error-collection');
    }
  }

  async function chooseOkfImport(zip: boolean) {
    actionMessage = '';
    newWikiMenuOpen = false;
    actionBusy = true;
    try {
      okfImportSelection = await pickOkfImport(zip);
      if (!okfImportSelection) return;
      okfImportSummary = await validateOkfImport(okfImportSelection.token);
      wikiName = '';
    } catch (error) {
      actionMessage = importErrorMessage(error);
      okfImportSelection = null;
      okfImportSummary = null;
    } finally {
      actionBusy = false;
    }
  }

  async function confirmOkfImport() {
    if (!okfImportSelection || !okfImportSummary || !wikiName.trim()) return;
    actionBusy = true;
    try {
      await importOkf(wikiName, okfImportSelection.token);
      wikiName = '';
      okfImportSelection = null;
      okfImportSummary = null;
      showOperationComplete();
    } catch (error) {
      actionMessage = importErrorMessage(error);
    } finally {
      actionBusy = false;
    }
  }

  function importErrorMessage(error: unknown): string {
    if (typeof error === 'object' && error !== null && 'messageKey' in error) {
      const messageKey = (error as { messageKey?: unknown }).messageKey;
      if (messageKey === 'folderSelectionExpired') return t('desktop-folder-selection-expired');
    }
    return t('desktop-okf-import-invalid');
  }

  async function createWiki() {
    if (!folderSelection) return;
    actionBusy = true;
    try {
      await addWiki(wikiName, folderSelection.token, continuousIndexing);
      wikiName = '';
      continuousIndexing = true;
      folderSelection = null;
      createWikiOpen = false;
      showOperationComplete();
    } catch {
      actionMessage = t('error-collection');
      folderSelection = null;
    } finally {
      actionBusy = false;
    }
  }

  function wikiScanState(wikiId: string) {
    return snapshot?.wikiScans.find((scan) => scan.wikiId === wikiId)?.state ?? null;
  }

  async function scanWiki(wikiId: string) {
    if (snapshot?.wikis.find((wiki) => wiki.id === wikiId)?.restrictions.length !== 0) return;
    try {
      await rescanWiki(wikiId);
      showOperationComplete();
    } catch {
      actionMessage = t('error-collection');
    }
  }

  async function changeWikiIndexing(wikiId: string, continuous: boolean) {
    actionBusy = true;
    try {
      await setWikiIndexing(wikiId, continuous);
      showOperationComplete();
    } catch {
      actionMessage = t('error-collection');
    } finally {
      actionBusy = false;
    }
  }

  function editWiki(wiki: WikiSummary) {
    if (wiki.restrictions.length !== 0) return;
    editingWikiId = wiki.id;
    wikiPolicy = {
      localOnly: wiki.localOnly,
      peerShareable: wiki.peerShareable,
      allowExternalAi: wiki.allowExternalAi,
      internetPublic: wiki.internetPublic
    };
    publicDescription = wiki.publicDescription;
    publicLanguages = wiki.publicLanguages;
  }

  function showWikiDetails(wikiId: string) {
    detailsWikiId = wikiId;
    relinkSelection = null;
  }

  async function chooseRelinkFolder() {
    try {
      relinkSelection = await pickWikiFolder();
    } catch {
      actionMessage = t('error-collection');
    }
  }

  async function applyRelink() {
    if (!detailsWikiId || !relinkSelection) return;
    actionBusy = true;
    try {
      await relinkWiki(detailsWikiId, relinkSelection.token);
      relinkSelection = null;
      showOperationComplete();
    } catch {
      relinkSelection = null;
      actionMessage = t('error-collection');
    } finally {
      actionBusy = false;
    }
  }

  async function removeWiki(wikiId: string) {
    actionBusy = true;
    try {
      await deleteWiki(wikiId);
      detailsWikiId = null;
      selectedWikiId = null;
      window.location.hash = '#wikis';
    } catch {
      actionMessage = t('error-collection');
    } finally {
      actionBusy = false;
    }
  }

  async function saveWikiPolicy() {
    if (!editingWikiId) return;
    actionBusy = true;
    try {
      const policy = {
        ...wikiPolicy,
        localOnly: !wikiPolicy.peerShareable && !wikiPolicy.allowExternalAi && !wikiPolicy.internetPublic
      };
      await updateWikiPolicy(editingWikiId, policy);
      wikiPolicy = policy;
      showOperationComplete();
    } catch {
      actionMessage = t('error-collection');
    } finally {
      actionBusy = false;
    }
  }

  async function savePublicProfile() {
    if (!editingWikiId) return;
    actionBusy = true;
    const languages = publicLanguages.split(',').map((language) => language.trim()).filter(Boolean);
    try {
      await updatePublicWikiProfile(editingWikiId, publicDescription.trim(), languages);
      showOperationComplete();
    } catch {
      actionMessage = t('error-collection');
    } finally {
      actionBusy = false;
    }
  }

  async function prepareRepair(wikiId: string) {
    guidedRepairConfirmed = false;
    try {
      guidedRepairRequestId = await prepareGuidedWikiRepair(wikiId);
      actionMessage = t('status-working');
    } catch {
      guidedRepairRequestId = null;
      actionMessage = t('error-collection');
    }
  }

  async function executeRepair(wikiId: string) {
    if (!guidedRepairConfirmed) return;
    try {
      guidedRepairRequestId = await executeGuidedWikiRepair(wikiId);
      guidedRepairConfirmed = false;
      showOperationComplete();
    } catch {
      guidedRepairRequestId = null;
      actionMessage = t('error-collection');
    }
  }

  function modelInstallLabel(currentLocale: LocalePreference): string {
    const status = snapshot?.modelInstall?.status;
    const labels: Record<string, string> = {
      queued: 'models-install-queued',
      downloading: 'models-install-downloading',
      verifying: 'models-install-verifying',
      extracting: 'models-install-extracting'
    };
    return message(currentLocale, labels[status ?? ''] ?? 'models-install-activating');
  }

  async function submitSearch() {
    actionMessage = '';
    if (!snapshot?.model?.active) return;
    publicBrowseOpen = false;
    publicBrowseLoading = false;
    actionBusy = true;
    try {
      await searchKnowledge(question, includePublic);
    } catch {
      actionMessage = t('search-error-title');
      actionBusy = false;
    }
  }

  async function openSearchHit(hit: SearchHitSummary) {
    const localWiki = snapshot?.wikis.some((wiki) => wiki.id === hit.wikiId);
    if (localWiki && hit.nodeId === snapshot?.nodeId) {
      destination = 'wikis';
      selectedWikiId = hit.wikiId;
      pushHash(`#wikis/${hit.wikiId}`);
      knowledgeMode = 'document';
      pendingSearchConcept = { wikiId: hit.wikiId, conceptId: hit.conceptId };
      await loadWikiBundle(hit.wikiId);
      await openPendingSearchConcept(snapshot);
      return;
    }
    destination = 'search';
    pushHash('#search');
    scrollMainTo(0);
    publicBrowseOpen = true;
    publicBrowseLoading = true;
    actionMessage = '';
    try {
      publicBrowseRequestId = await browsePublicWiki(hit.nodeId, hit.wikiId);
      if (snapshot?.publicBrowse?.requestId === publicBrowseRequestId) {
        publicBrowseRequestId = null;
        publicBrowseLoading = false;
      }
    } catch {
      publicBrowseRequestId = null;
      publicBrowseLoading = false;
      actionMessage = t('search-coverage-public-offline');
    }
  }

  function closePublicBrowse() {
    publicBrowseOpen = false;
    publicBrowseLoading = false;
    publicBrowseRequestId = null;
    scrollMainTo(0);
  }

  async function openPendingSearchConcept(current: AppSnapshot | null) {
    const pending = pendingSearchConcept;
    if (!pending || current?.knowledge?.wikiId !== pending.wikiId || current.knowledge.status !== 'ready') return;
    const concept = current.knowledge.concepts.find((candidate) => candidate.conceptId === pending.conceptId);
    if (!concept) {
      pendingSearchConcept = null;
      actionMessage = t('search-local-unavailable');
      return;
    }
    pendingSearchConcept = null;
    await loadWikiPage(pending.wikiId, concept.page);
  }

  async function loadMorePublicConcepts() {
    const browse = snapshot?.publicBrowse;
    if (!browse?.publisherId || !browse.wikiId || !browse.nextCursor) return;
    try {
      publicBrowseRequestId = await browsePublicWiki(browse.publisherId, browse.wikiId, browse.nextCursor);
    } catch {
      publicBrowseRequestId = null;
      actionMessage = t('search-error-title');
    }
  }

  async function changePublisherBlock(publisherId: string, blocked: boolean) {
    try {
      await setPublicPublisherBlocked(publisherId, blocked);
      showOperationComplete();
    } catch {
      actionMessage = t('search-error-title');
    }
  }

  async function saveFederationIndex(remove = false) {
    try {
      if (remove) await removeFederationIndex(federationPeerId.trim());
      else await addFederationIndex(federationPeerId.trim(), federationAddress.trim());
      showOperationComplete();
      if (!remove) {
        federationPeerId = '';
        federationAddress = '';
      }
    } catch {
      actionMessage = t('search-error-title');
    }
  }

  async function connectManualPeer() {
    try {
      await dialPeer(manualPeerAddress.trim());
      showOperationComplete();
      manualPeerAddress = '';
    } catch {
      actionMessage = t('devices-manual-invalid');
    }
  }

  function formatBytes(bytes: number): string {
    return `${(bytes / 1073741824).toFixed(1)} GiB`;
  }

  function repairChangeLabel(change: string): string {
    const labels: Record<string, string> = {
      withdraw_concept: 'knowledge-repair-change-withdraw',
      remove_orphan: 'knowledge-repair-change-orphan',
      regenerate_index: 'knowledge-repair-change-index',
      append_deprecation_history: 'knowledge-repair-change-history'
    };
    return t(labels[change] ?? 'knowledge-repair-error-generic');
  }

  async function openReview(review: ReviewSummary) {
    selectedReview = review;
    editDraft = structuredClone(review.draft);
    actionBusy = true;
    actionMessage = t('review-evidence-loading');
    try {
      await loadReviewEvidence(review);
      actionBusy = false;
    } catch {
      actionMessage = t('review-evidence-approval-blocked');
      actionBusy = false;
    }
  }

  async function loadMoreEvidence() {
    if (!selectedReview || snapshot?.reviewEvidence?.nextOrdinal == null) return;
    actionBusy = true;
    try {
      await loadReviewEvidence(selectedReview, snapshot.reviewEvidence.nextOrdinal);
    } catch {
      actionMessage = t('review-evidence-unavailable');
      actionBusy = false;
    }
  }

  function evidenceIsCurrent(): boolean {
    return snapshot?.reviewEvidence?.status === 'ready'
      && snapshot.reviewEvidence.conceptId === selectedReview?.conceptId
      && snapshot.reviewEvidence.sourceRevision === selectedReview.sourceRevision;
  }

  function selectedReviewIsReadOnly(): boolean {
    if (!selectedReview) return true;
    return snapshot?.wikis
      .find((wiki) => wiki.id === selectedReview?.wikiId)
      ?.restrictions.length !== 0;
  }

  async function decideReview(decision: 'approve' | 'reject' | 'reanalyze') {
    if (!selectedReview || ((decision === 'approve' || decision === 'reanalyze') && selectedReviewIsReadOnly())) return;
    if (decision === 'approve' && (!editDraft || !evidenceIsCurrent())) return;
    actionBusy = true;
    try {
      if (decision === 'approve' && editDraft) await approveReview(selectedReview.conceptId, selectedReview.sourceRevision, editDraft);
      if (decision === 'reject') await rejectReview(selectedReview.conceptId);
      if (decision === 'reanalyze') await reanalyzeReview(selectedReview.conceptId);
      showOperationComplete();
    } catch {
      actionMessage = t('review-evidence-approval-blocked');
    } finally {
      actionBusy = false;
    }
  }

  async function openWiki(wikiId: string, tab: 'content' | 'pending' = 'content') {
    destination = 'wikis';
    selectedWikiId = wikiId;
    wikiTab = tab;
    pushHash(`#wikis/${wikiId}${tab === 'pending' ? '/pending' : ''}`);
    knowledgeMode = 'document';
    actionBusy = true;
    actionMessage = '';
    try {
      await loadWikiBundle(wikiId);
    } catch {
      actionMessage = t('home-wiki-failed');
      actionBusy = false;
    }
  }

  async function openKnowledgePage(page: KnowledgePageInput) {
    if (!selectedWikiId) return;
    actionBusy = true;
    try {
      await loadWikiPage(selectedWikiId, page);
    } catch {
      actionMessage = t('search-local-unavailable');
      actionBusy = false;
    }
  }

  async function selectGraphPage(page: KnowledgePageInput) {
    await openKnowledgePage(page);
  }

  async function savePreferences(completeOnboarding = false) {
    actionBusy = true;
    try {
      await updatePreferences({ locale, theme, lanPreference, closeBehavior, automaticUpdateChecks, completeOnboarding });
      showOperationComplete();
    } catch {
      actionMessage = t('error-generic');
      actionBusy = false;
    }
  }

  async function applyCloseChoice(choice: 'hide' | 'quit' | 'cancel') {
    closeChoiceRequired = false;
    if (choice === 'hide') await hideToTray();
    if (choice === 'quit') await quitCompletely();
  }

  async function prepareLocalModel() {
    actionBusy = true;
    try {
      await installModels();
      actionMessage = t('models-downloading', { artifact: snapshot?.model?.displayName ?? t('component-local-ai') });
    } catch {
      actionMessage = t('error-local-ai');
      actionBusy = false;
    }
  }
</script>

<svelte:head><meta name="theme-color" content="#0b1118" /></svelte:head>

{#if !snapshot || snapshot.phase !== 'ready' || !snapshot.preferences}
  <main class="onboarding startup" aria-busy="true">
    <div class="onboarding-mark">A</div>
    <p class="eyebrow">AirWiki</p>
    <h1>{t('status-working')}</h1>
    <p class="lede" aria-live="polite">{t(runtimeMessageId)}</p>
  </main>
{:else if snapshot.preferences.completedOnboardingVersion == null}
  <OnboardingFlow {snapshot} bind:locale bind:lanPreference bind:closeBehavior bind:modelLicensesConfirmed {actionBusy} {actionMessage} onprepare={prepareLocalModel} onfinish={() => savePreferences(true)} />
{:else}
<div class="shell drive-shell" inert={activeDialogId !== null} aria-hidden={activeDialogId !== null ? 'true' : undefined}>
  <main class="drive-main">
    <header class="top-bar">
      <button class="top-brand" onclick={() => select('wikis')} aria-label={t('desktop-nav-wikis')}><span class="brand-mark" aria-hidden="true">A</span><span>AirWiki</span></button>
      <GlobalSearch
        {question}
        {includePublic}
        busy={actionBusy}
        ready={snapshot.model?.active === true}
        platform={snapshot.platform}
        {t}
        onquestion={(value) => { question = value; }}
        onpublic={(value) => { includePublic = value; }}
        onsearch={submitGlobalSearch}
        onopen={openGlobalSearch}
      />
      <div class="top-actions"><button class="secondary" aria-expanded={newWikiMenuOpen} onclick={() => { newWikiMenuOpen = !newWikiMenuOpen; }}><Plus size={17} aria-hidden="true" />{t('desktop-new-wiki')}</button><button class="icon-button" class:active={destination === 'system'} aria-label={t('desktop-nav-system')} onclick={() => select('system')}><Settings2 size={19} aria-hidden="true" /></button></div>
    </header>

    <section class="drive-page" aria-live="polite" bind:this={mainScrollRegion}>
      {#key `${destination}:${selectedWikiId ?? ''}:${wikiTab}:${sharedTab}:${systemSection}`}
        <div class="route-page drive-route" data-route={destination}>
          {#if destination === 'home'}
            <header class="page-heading">
              <div><h1>{t('desktop-page-home-title')}</h1><p>{t('desktop-page-home-body')}</p></div>
            </header>

            <section class="home-section" aria-labelledby="attention-title">
              <div class="section-heading"><div><h2 id="attention-title">{t('desktop-home-attention')}</h2><p>{t('desktop-home-attention-body')}</p></div><button class="text-action" onclick={refreshHealth} disabled={wikiHealthRequestId !== null}>{wikiHealthRequestId ? t('home-wiki-checking') : t('updates-check-now')}</button></div>
              {#if attentionWikis.length}
                <div class="attention-list">
                  {#each attentionWikis as wiki (wiki.id)}
                    <button onclick={() => openWiki(wiki.id, !wiki.maintenanceRequired && wiki.failedCount === 0 && wiki.needsReviewCount > 0 ? 'pending' : 'content')}>
                      <span class:warning={wiki.failedCount > 0 || wiki.maintenanceRequired} class="attention-icon"><AlertTriangle size={18} aria-hidden="true" /></span>
                      <span><strong>{wiki.name}</strong><small>{wikiAttentionSummary(wiki)}</small></span>
                      <span>{t('desktop-attention-see-actions')}</span>
                    </button>
                  {/each}
                </div>
              {:else}
                <div class="all-clear"><CheckCircle2 size={22} aria-hidden="true" /><div><strong>{t('desktop-home-clear-title')}</strong><p>{t('desktop-home-clear-body')}</p></div></div>
              {/if}
              {#if snapshot.wikiHealth?.attentionWikiId}
                <div class="repair-summary">
                  <div><strong>{t('knowledge-recovery-guided')}</strong><p>{t('knowledge-repair-review-help')}</p></div>
                  <div class="row-actions"><button class="text-action" onclick={openAttentionWiki}>{t('action-open')}</button><button class="secondary" onclick={() => prepareRepair(snapshot!.wikiHealth!.attentionWikiId!)} disabled={guidedRepairRequestId !== null}>{t('knowledge-repair-review-action')}</button></div>
                  {#if snapshot.guidedRepair?.wikiId === snapshot.wikiHealth.attentionWikiId && snapshot.guidedRepair.status === 'prepared'}
                    <div class="repair-preview"><ul>{#each snapshot.guidedRepair.files as file, fileIndex (fileIndex)}<li><code>{file.page.kind}</code><span>{repairChangeLabel(file.change)}</span></li>{/each}</ul><Checkbox label={t('knowledge-repair-confirm-warning')} bind:checked={guidedRepairConfirmed} /><button class="danger" onclick={() => executeRepair(snapshot!.guidedRepair!.wikiId)} disabled={!guidedRepairConfirmed}>{t('knowledge-repair-confirm-action')}</button></div>
                  {/if}
                </div>
              {/if}
            </section>

            <section class="home-section" aria-labelledby="your-wikis-title">
              <div class="section-heading"><h2 id="your-wikis-title">{t('desktop-home-your-wikis')}</h2><button class="text-action" onclick={() => select('wikis')}>{t('desktop-view-all')}</button></div>
              <WikiTable wikis={orderedWikis.slice(0, 6)} scans={snapshot.wikiScans} {t} onopen={openWiki} />
            </section>
          {:else if destination === 'wikis' && !selectedWiki}
            <header class="page-heading">
              <div><h1>{t('desktop-page-wikis-title')}</h1><p>{t('desktop-page-wikis-body')}</p></div>
            </header>
            <WikiTable wikis={orderedWikis} scans={snapshot.wikiScans} {t} onopen={openWiki} />
            {#if snapshot.pendingComputations.length > 0}
              <section class="workspace-section" aria-labelledby="computations-title">
                <div class="section-heading"><div><h2 id="computations-title">{t('desktop-computation-requests-title')}</h2><p>{t('desktop-computation-requests-body')}</p></div><button class="text-action" onclick={refreshComputations}>{t('action-refresh')}</button></div>
                <div class="computation-list">
                  {#each snapshot.pendingComputations as computation (computation.runId)}
                    <article>
                      <span class="pending-icon"><Sparkles size={17} aria-hidden="true" /></span>
                      <div><strong>{t('desktop-computation-request-title', { application: computation.applicationName })}</strong><p>{t('desktop-computation-request-body', { wiki: computation.wikiName, path: computation.logicalPath })}</p>{#if computation.parameters.length > 0}<ul class="computation-parameters">{#each computation.parameters as parameter (`${parameter.name}:${parameter.parameterType}`)}<li><code>{parameter.name}</code><span>{parameter.parameterType}</span></li>{/each}</ul>{/if}</div>
                      <div class="row-actions"><button class="secondary" disabled={actionBusy} onclick={() => decideComputation(computation.runId, 'reject')}>{t('action-reject')}</button><button class="primary" disabled={actionBusy} onclick={() => decideComputation(computation.runId, 'execute')}>{t('desktop-computation-review-run')}</button></div>
                    </article>
                  {/each}
                </div>
              </section>
            {/if}
            {#if snapshot.completedComputations.length > 0}
              <section class="workspace-section" aria-labelledby="completed-computations-title">
                <div class="section-heading"><div><h2 id="completed-computations-title">{t('desktop-computation-results-title')}</h2><p>{t('desktop-computation-results-body')}</p></div><button class="text-action" onclick={refreshComputations}>{t('action-refresh')}</button></div>
                <div class="computation-list">
                  {#each snapshot.completedComputations as computation (computation.runId)}
                    <article>
                      <span class="pending-icon"><CheckCircle2 size={17} aria-hidden="true" /></span>
                      <div><strong>{t('desktop-computation-result-title', { application: computation.applicationName })}</strong><p>{t('desktop-computation-result-body', { wiki: computation.wikiName, path: computation.logicalPath })}</p></div>
                      {#if computation.verdict === 'accepted'}
                        <div class="row-actions computation-save-actions">
                          <SelectField label={t('desktop-computation-save-target')} value={computationSaveTargets[computation.runId] ?? ''} onchange={(value) => { computationSaveTargets = { ...computationSaveTargets, [computation.runId]: value }; }} options={[{ value: '', label: t('desktop-computation-save-select') }, ...snapshot.wikis.filter((wiki) => wiki.origin === 'aiMemory').map((wiki) => ({ value: wiki.id, label: wiki.name }))]} />
                          <button class="primary" disabled={actionBusy || !computationSaveTargets[computation.runId]} onclick={() => saveAcceptedComputation(computation.runId)}>{t('desktop-computation-save')}</button>
                        </div>
                      {:else}
                        <span class="status-pill warning">{t('desktop-computation-rejected')}</span>
                      {/if}
                    </article>
                  {/each}
                </div>
              </section>
            {/if}
            {#if attentionWikis.length}
              <section class="workspace-section" aria-labelledby="attention-title">
                <div class="section-heading"><div><h2 id="attention-title">{t('desktop-home-attention')}</h2><p>{t('desktop-home-attention-body')}</p></div><button class="text-action" onclick={refreshHealth} disabled={wikiHealthRequestId !== null}>{wikiHealthRequestId ? t('home-wiki-checking') : t('updates-check-now')}</button></div>
                <div class="attention-list">
                  {#each attentionWikis as wiki (wiki.id)}<button onclick={() => openWiki(wiki.id, !wiki.maintenanceRequired && wiki.failedCount === 0 && wiki.needsReviewCount > 0 ? 'pending' : 'content')}><span class:warning={wiki.failedCount > 0 || wiki.maintenanceRequired} class="attention-icon"><AlertTriangle size={18} aria-hidden="true" /></span><span><strong>{wiki.name}</strong><small>{wikiAttentionSummary(wiki)}</small></span><span>{t('desktop-attention-see-actions')}</span></button>{/each}
                </div>
                {#if snapshot.wikiHealth?.attentionWikiId}
                  <div class="repair-summary">
                    <div><strong>{t('knowledge-recovery-guided')}</strong><p>{t('knowledge-repair-review-help')}</p></div>
                    <div class="row-actions"><button class="text-action" onclick={openAttentionWiki}>{t('action-open')}</button><button class="secondary" onclick={() => prepareRepair(snapshot!.wikiHealth!.attentionWikiId!)} disabled={guidedRepairRequestId !== null}>{t('knowledge-repair-review-action')}</button></div>
                    {#if snapshot.guidedRepair?.wikiId === snapshot.wikiHealth.attentionWikiId && snapshot.guidedRepair.status === 'prepared'}
                      <div class="repair-preview"><ul>{#each snapshot.guidedRepair.files as file, fileIndex (fileIndex)}<li><code>{file.page.kind}</code><span>{repairChangeLabel(file.change)}</span></li>{/each}</ul><Checkbox label={t('knowledge-repair-confirm-warning')} bind:checked={guidedRepairConfirmed} /><button class="danger" onclick={() => executeRepair(snapshot!.guidedRepair!.wikiId)} disabled={!guidedRepairConfirmed}>{t('knowledge-repair-confirm-action')}</button></div>
                    {/if}
                  </div>
                {/if}
              </section>
            {/if}
            <section class="workspace-section public-discovery" id="public-wikis" aria-labelledby="public-wikis-title">
              <div class="section-heading"><div><h2 id="public-wikis-title">{t('desktop-public-network')}</h2><p>{t('desktop-public-discover-body')}</p></div><button class="secondary" onclick={() => { connectionsOpen = true; }}>{t('desktop-connections')}</button></div>
              {#if snapshot?.search?.hits.some(isPublicSearchHit)}<div class="search-results">{#each snapshot.search.hits.filter(isPublicSearchHit) as hit (`${hit.nodeId}:${hit.wikiId}:${hit.conceptId}:${hit.rank}`)}<article><small>{searchOriginFor(hit)}</small><h3>{hit.title}</h3><p>{hit.snippet}</p><button class="text-action" onclick={() => openSearchHit(hit)}>{t('action-open')}</button></article>{/each}</div>{:else}<div class="inline-empty"><BookOpen size={20} aria-hidden="true" /><span><strong>{t('desktop-public-empty-title')}</strong><small>{t('desktop-public-search-help')}</small></span></div>{/if}
              {#if snapshot.publicBrowse}<div class="public-browse-detail"><div class="section-heading"><div><h3>{snapshot.publicBrowse.wikiName ?? t('desktop-public-origin-missing')}</h3><p>{snapshot.publicBrowse.description ?? ''}</p><small>{snapshot.publicBrowse.okfCompatibility ? t(`desktop-okf-compatibility-${snapshot.publicBrowse.okfCompatibility.kind}`) : t('desktop-public-metadata-unavailable')}</small></div>{#if snapshot.publicBrowse.publisherId}<button class="danger" onclick={() => changePublisherBlock(snapshot!.publicBrowse!.publisherId!, true)}>{t('search-public-block-publisher')}</button>{/if}</div>{#each snapshot.publicBrowse.concepts as concept (`${concept.conceptId}:${concept.sourceRevision}`)}<article><small>{concept.conceptType} · {concept.language}</small><h3>{concept.title}</h3><p>{concept.summary}</p><span>{publicConceptMetadata(concept)}</span></article>{/each}{#if snapshot.publicBrowse.nextCursor}<button class="secondary" onclick={loadMorePublicConcepts}>{t('search-public-browse-more')}</button>{/if}</div>{/if}
            </section>
          {:else if destination === 'wikis' && selectedWiki}
            {@const selectedWikiIssues = snapshot.sourceIssues.filter((issue) => issue.wikiId === selectedWiki.id)}
            <header class="page-heading wiki-heading">
              <div><nav class="breadcrumb" aria-label={t('desktop-nav-wikis')}><button onclick={() => { selectedWikiId = null; pushHash('#wikis'); }}>{t('desktop-nav-wikis')}</button><span aria-hidden="true">/</span><span>{selectedWiki.name}</span></nav><h1>{selectedWiki.name}</h1><p>{t('desktop-wiki-detail-body', { published: selectedWiki.publishedCount })}</p></div>
              <div class="heading-actions">{#if selectedWiki.origin === 'folder' && selectedWiki.restrictions.length === 0}<button class="secondary" onclick={() => scanWiki(selectedWiki.id)} disabled={wikiScanState(selectedWiki.id) !== null}><RefreshCw size={16} aria-hidden="true" />{t('action-refresh')}</button>{/if}{#if selectedWiki.restrictions.length === 0}<button class="primary" onclick={() => editWiki(selectedWiki)}>{t('desktop-share-action')}</button>{/if}</div>
            </header>

            <section class="wiki-access-strip" aria-label={t('desktop-wiki-access-title')}>
              <strong>{t('desktop-wiki-access-title')}</strong>
              <div>{#if !selectedWiki.peerShareable && !selectedWiki.allowExternalAi && !selectedWiki.internetPublic}<span>{t('desktop-wiki-private')}</span>{/if}{#if selectedWiki.peerShareable}<span>{t('desktop-share-nearby')}</span>{/if}{#if selectedWiki.allowExternalAi}<span>{t('desktop-share-ai-apps')}</span>{/if}{#if selectedWiki.internetPublic}<span>{t('desktop-share-public')}</span>{/if}</div>
              <small>{wikiPeers(selectedWiki.id).length > 0 ? wikiPeers(selectedWiki.id).join(' · ') : t('desktop-wiki-no-specific-access')}</small>
              {#if selectedWiki.restrictions.length === 0}<button class="text-action" onclick={() => { connectionsOpen = true; }}>{t('desktop-manage-access')}</button>{/if}
            </section>

            {#if selectedWiki.okfCompatibility.kind === 'futureRestricted' || selectedWiki.okfCompatibility.kind === 'legacyV01' || selectedWiki.staleConceptCount > 0 || selectedWiki.outdatedVerificationCount > 0 || selectedWiki.metadataWarningCount > 0}
              <section class="wiki-assurance-strip" aria-label={t('desktop-okf-status-title')}>
                <div><strong>{compatibilityLabel(selectedWiki)}</strong><small>{selectedWiki.okfCompatibility.kind === 'futureRestricted' ? t('desktop-okf-future-restriction-body') : selectedWiki.okfCompatibility.kind === 'legacyV01' ? t('desktop-okf-legacy-restriction-body') : t('desktop-okf-status-summary', { stale: selectedWiki.staleConceptCount, outdated: selectedWiki.outdatedVerificationCount, warnings: selectedWiki.metadataWarningCount })}</small></div>
                <button class="text-action" onclick={() => showWikiDetails(selectedWiki.id)}>{t('desktop-details')}</button>
              </section>
            {/if}

            {#if selectedWikiIssues.length > 0 || selectedWiki.failedCount > 0 || selectedWiki.maintenanceRequired || selectedWiki.needsReviewCount > 0}
              <section class="wiki-interventions" aria-labelledby="wiki-interventions-title">
                <div class="wiki-interventions-heading">
                  <AlertTriangle size={19} aria-hidden="true" />
                  <div><h2 id="wiki-interventions-title">{t('desktop-attention-title')}</h2><p>{t('desktop-attention-body')}</p></div>
                </div>
                <div class="wiki-intervention-list">
                  {#if selectedWikiIssues.length > 0 || selectedWiki.failedCount > 0}
                    {@const affectedFileCount = Math.max(selectedWikiIssues.length, selectedWiki.failedCount)}
                    <article>
                      <div><strong>{t('desktop-attention-files-title', { count: affectedFileCount })}</strong><p>{t('desktop-attention-files-body')}</p></div>
                      <button class="secondary" onclick={() => showWikiDetails(selectedWiki.id)}>{t('desktop-attention-files-action')}</button>
                    </article>
                  {/if}
                  {#if selectedWiki.maintenanceRequired}
                    <article>
                      <div><strong>{t('desktop-attention-maintenance-title')}</strong><p>{snapshot.wikiHealth?.attentionWikiId === selectedWiki.id ? t('desktop-attention-repair-body') : t('desktop-attention-maintenance-body')}</p></div>
                      {#if snapshot.wikiHealth?.attentionWikiId === selectedWiki.id}
                        <button class="secondary" onclick={() => prepareRepair(selectedWiki.id)} disabled={guidedRepairRequestId !== null}>{t('knowledge-repair-review-action')}</button>
                      {:else}
                        <button class="secondary" onclick={() => scanWiki(selectedWiki.id)} disabled={wikiScanState(selectedWiki.id) !== null}>{t('desktop-attention-check-source')}</button>
                      {/if}
                    </article>
                  {/if}
                  {#if selectedWiki.needsReviewCount > 0}
                    <article>
                      <div><strong>{t('desktop-attention-reviews-title', { count: selectedWiki.needsReviewCount })}</strong><p>{t('desktop-attention-reviews-body')}</p></div>
                      <button class="secondary" onclick={() => openWikiTab('pending')}>{t('desktop-attention-reviews-action')}</button>
                    </article>
                  {/if}
                </div>
                {#if snapshot.guidedRepair?.wikiId === selectedWiki.id && snapshot.guidedRepair.status === 'prepared'}
                  <div class="repair-preview"><ul>{#each snapshot.guidedRepair.files as file, fileIndex (fileIndex)}<li><code>{file.page.kind}</code><span>{repairChangeLabel(file.change)}</span></li>{/each}</ul><Checkbox label={t('knowledge-repair-confirm-warning')} bind:checked={guidedRepairConfirmed} /><button class="danger" onclick={() => executeRepair(snapshot!.guidedRepair!.wikiId)} disabled={!guidedRepairConfirmed}>{t('knowledge-repair-confirm-action')}</button></div>
                {/if}
              </section>
            {/if}

            <div class="content-tabs-bar">
              <div class="content-tabs" role="tablist" aria-label={t('desktop-wiki-sections')}>
                <button role="tab" aria-selected={wikiTab === 'content'} class:active={wikiTab === 'content'} onclick={() => openWikiTab('content')}>{t('desktop-wiki-content-tab')}</button>
                <button role="tab" aria-selected={wikiTab === 'pending'} class:active={wikiTab === 'pending'} onclick={() => openWikiTab('pending')}>{t('desktop-wiki-pending-tab')}<span>{selectedWikiReviews.length}</span></button>
              </div>
              <button class="details-tab" onclick={() => showWikiDetails(selectedWiki.id)}>{t('desktop-details')}</button>
            </div>

            {#if wikiTab === 'content'}
              <div class="wiki-toolbar"><div class="view-switch" aria-label={t('desktop-view-mode')}><button class:active={knowledgeMode === 'document'} onclick={() => setKnowledgeMode('document')}>{t('desktop-list-view')}</button><button class:active={knowledgeMode === 'graph'} onclick={() => setKnowledgeMode('graph')}>{t('knowledge-tab-graph')}</button></div></div>
              {#if knowledgeMode === 'graph' && snapshot.knowledge?.wikiId === selectedWiki.id && snapshot.knowledge.status === 'ready'}
                <section class="graph-view">{#key `${snapshot.knowledge.wikiId}:${snapshot.knowledge.version}`}<KnowledgeGraph bundle={snapshot.knowledge} onselect={selectGraphPage} {locale} />{/key}</section>
              {:else}
                <div class="file-browser">
                  <aside class="file-list" aria-label={t('knowledge-pages')}>
                    {#if snapshot.knowledge?.wikiId === selectedWiki.id}
                      <button onclick={() => openKnowledgePage({ kind: 'index' })}><BookOpen size={17} aria-hidden="true" /><span><strong>{t('knowledge-index-title')}</strong><small>index.md</small></span></button>
                      <button onclick={() => openKnowledgePage({ kind: 'log' })}><History size={17} aria-hidden="true" /><span><strong>{t('knowledge-recovery-history')}</strong><small>log.md</small></span></button>
                      {#each snapshot.knowledge.concepts as concept (pageKey(concept.page))}<button onclick={() => openKnowledgePage(concept.page)}><FileText size={17} aria-hidden="true" /><span><strong>{concept.title}</strong><small>{concept.page.kind === 'concept' ? concept.page.path : concept.description}</small></span></button>{/each}
                    {/if}
                  </aside>
                  <section class="file-preview" aria-live="polite">
                    {#if snapshot.knowledge?.status === 'updating'}<p class="loading"><RefreshCw size={17} />{t('knowledge-updating-title')}</p>
                    {:else if snapshot.knowledgePage?.wikiId === selectedWiki.id && snapshot.knowledgePage.status === 'ready'}
                      <header><p class="section-label">{t('desktop-verified-page')}</p><h2>{snapshot.knowledgePage.title}</h2></header>
                      {@const concept = snapshot.knowledgePage.concept}
                      {#if concept}
                        <aside class="concept-assurance" aria-label={t('desktop-concept-assurance-title')}>
                          <div><span>{t('desktop-concept-type')}</span><strong>{concept.conceptType}</strong></div>
                          <div><span>{t('desktop-concept-trust')}</span><strong>{assuranceLabel(concept)}</strong></div>
                          <div><span>{t('desktop-concept-freshness')}</span><strong>{t(`desktop-freshness-${concept.assurance.freshness}`)}</strong></div>
                          <div><span>{t('desktop-concept-lifecycle')}</span><strong>{concept.lifecycle}</strong></div>
                          {#if concept.generatedBy}<div><span>{t('desktop-concept-generated-by')}</span><strong>{concept.generatedBy}</strong></div>{/if}
                          {#if concept.sources.length > 0}<details><summary>{t('desktop-concept-sources', { count: concept.sources.length })}</summary><ul>{#each concept.sources as source, sourceIndex (source.id ?? source.resource ?? sourceIndex)}<li><strong>{source.title ?? source.id ?? t('desktop-concept-source-unnamed')}</strong>{#if source.author}<small>{source.author}</small>{/if}{#if source.lastModified}<small>{source.lastModified}</small>{/if}</li>{/each}</ul></details>{/if}
                          {#if concept.warnings.length > 0}<p class="metadata-warning"><AlertTriangle size={15} aria-hidden="true" />{t('desktop-concept-metadata-warning', { count: concept.warnings.length })}</p>{/if}
                          {#if canVerifyConcept(selectedWiki, concept)}<button class="secondary concept-verify" onclick={() => verifyConcept(selectedWiki, concept)} disabled={actionBusy}>{t('desktop-concept-verify')}</button>{/if}
                        </aside>
                      {/if}
                      {#if snapshot.knowledgePage.truncated}<p class="evidence-warning">{t('knowledge-page-truncated')}</p>{/if}
                      <div class="knowledge-blocks">{#each snapshot.knowledgePage.blocks as block, blockIndex (blockIndex)}{#if block.kind === 'heading'}<h3 class:minor={block.level > 2}>{block.text}</h3>{:else if block.kind === 'paragraph'}<p>{block.text}</p>{:else if block.kind === 'listItem'}<div class="safe-list-item"><span>{block.ordered ? '—' : '•'}</span><p>{block.text}</p></div>{:else if block.kind === 'code'}<pre><code>{block.text}</code></pre>{:else if block.kind === 'quote'}<blockquote>{block.text}</blockquote>{:else}<hr />{/if}{/each}</div>
                    {:else}<div class="file-empty"><BookOpen size={28} aria-hidden="true" /><h2>{t('knowledge-select-page')}</h2><p>{t('desktop-verified-only')}</p></div>{/if}
                  </section>
                </div>
              {/if}
            {:else}
              <section class="pending-list" aria-label={t('desktop-wiki-pending-tab')}>
                {#each selectedWikiReviews as review (`${review.conceptId}:${review.sourceRevision}`)}<button onclick={() => openReview(review)}><span class="pending-icon"><Sparkles size={17} aria-hidden="true" /></span><span><strong>{review.sourceName}</strong><small>{review.draft.summary}</small></span><span>{t('review-revision', { revision: review.sourceRevision })}</span></button>{:else}<div class="table-empty"><CheckCircle2 size={28} aria-hidden="true" /><strong>{t('review-empty-title')}</strong><p>{t('review-empty-body')}</p></div>{/each}
              </section>
            {/if}
          {:else if destination === 'shared'}
            <header class="page-heading"><div><h1>{t('desktop-page-shared-title')}</h1><p>{t('desktop-page-shared-body')}</p></div><button class="secondary" onclick={() => { connectionsOpen = true; }}>{t('desktop-connections')}</button></header>
            <div class="content-tabs" role="tablist" aria-label={t('desktop-page-shared-title')}><button role="tab" aria-selected={sharedTab === 'owned'} class:active={sharedTab === 'owned'} onclick={() => openSharedTab('owned')}>{t('desktop-shared-by-you')}</button><button role="tab" aria-selected={sharedTab === 'public'} class:active={sharedTab === 'public'} onclick={() => openSharedTab('public')}>{t('desktop-public-network')}</button></div>
            {#if sharedTab === 'owned'}
              <section class="shared-section"><div class="section-heading"><div><h2>{t('desktop-shared-owned-title')}</h2><p>{t('desktop-shared-owned-body')}</p></div></div><WikiTable wikis={sharedWikis} scans={snapshot!.wikiScans} {t} onopen={(wikiId) => { const wiki = snapshot?.wikis.find((item) => item.id === wikiId); if (wiki) editWiki(wiki); }} /></section>
            {:else}
              <section class="public-discovery"><header><h2>{t('desktop-public-discover-title')}</h2><p>{t('desktop-public-discover-body')}</p></header><p>{t('desktop-public-search-help')}</p>{#if snapshot?.search?.hits.some(isPublicSearchHit)}<div class="search-results">{#each snapshot.search.hits.filter(isPublicSearchHit) as hit (`${hit.nodeId}:${hit.wikiId}:${hit.conceptId}:${hit.rank}`)}<article><small>{searchOriginFor(hit)}</small><h3>{hit.title}</h3><p>{hit.snippet}</p><button class="text-action" onclick={() => openSearchHit(hit)}>{t('action-open')}</button></article>{/each}</div>{/if}{#if snapshot.publicBrowse}<div class="public-browse-detail"><div class="section-heading"><div><h3>{snapshot.publicBrowse.wikiName ?? t('desktop-public-origin-missing')}</h3><p>{snapshot.publicBrowse.description ?? ''}</p><small>{snapshot.publicBrowse.okfCompatibility ? t(`desktop-okf-compatibility-${snapshot.publicBrowse.okfCompatibility.kind}`) : t('desktop-public-metadata-unavailable')}</small></div>{#if snapshot.publicBrowse.publisherId}<button class="danger" onclick={() => changePublisherBlock(snapshot!.publicBrowse!.publisherId!, true)}>{t('search-public-block-publisher')}</button>{/if}</div>{#each snapshot.publicBrowse.concepts as concept (`${concept.conceptId}:${concept.sourceRevision}`)}<article><small>{concept.conceptType} · {concept.language}</small><h3>{concept.title}</h3><p>{concept.summary}</p><span>{publicConceptMetadata(concept)}</span></article>{/each}{#if snapshot.publicBrowse.nextCursor}<button class="secondary" onclick={loadMorePublicConcepts}>{t('search-public-browse-more')}</button>{/if}</div>{/if}</section>
            {/if}
          {:else if destination === 'search'}
            <header class="page-heading"><div><h1>{t('desktop-page-search-title')}</h1><p>{t('desktop-page-search-body')}</p></div></header>
            {#if publicBrowseOpen}<PublicWikiViewer browse={publicBrowseLoading ? null : snapshot.publicBrowse} loading={publicBrowseLoading} {t} metadata={publicConceptMetadata} onback={closePublicBrowse} onmore={loadMorePublicConcepts} onblock={(publisherId) => changePublisherBlock(publisherId, true)} />{:else if !snapshot.model?.active}<div class="search-welcome" role="status"><Sparkles size={32} aria-hidden="true" /><h2>{t('desktop-search-preparing-title')}</h2><p>{t('desktop-search-preparing-body')}</p><button class="secondary" onclick={() => openServiceStatus('knowledge')}>{t('desktop-search-preparing-action')}</button></div>{:else if snapshot.search}<div class="search-results" aria-live="polite">{#if snapshot.search.status === 'searching'}<div class="search-state working" role="status"><span class="status-dot working" aria-hidden="true"></span><span>{t('search-running')}</span></div>{:else if snapshot.search.status === 'failed'}<div class="search-state error" role="alert"><AlertTriangle size={17} aria-hidden="true" /><span>{t('search-error-title')}</span></div>{:else if snapshot.search.hits.length > 0}<p class="section-label">{t('desktop-search-found')}</p>{/if}{#if snapshot.search.status === 'complete' && snapshot.search.coverage !== 'complete' && snapshot.search.hits.length > 0}<div class="search-state warning" role="status"><AlertTriangle size={17} aria-hidden="true" /><span>{searchCoverageMessage(snapshot.search.coverage)}</span></div>{/if}{#each snapshot.search.hits as hit (`${hit.nodeId}:${hit.wikiId}:${hit.conceptId}:${hit.rank}`)}<article><small>{searchOriginFor(hit)} · {hit.headingOrPage}</small><h3>{hit.title}</h3><p>{hit.snippet}</p><div class="citation-row"><span>{searchSourceFor(hit)}</span><span>{t('search-revision', { revision: hit.sourceRevision })}</span>{#if searchAssuranceLabel(hit)}<span>{searchAssuranceLabel(hit)}</span>{/if}</div>{#if hit.nodeId === snapshot.nodeId || isPublicSearchHit(hit)}<button class="text-action" onclick={() => openSearchHit(hit)} disabled={publicBrowseRequestId !== null}>{t('action-open')}</button>{/if}</article>{:else}{#if snapshot.search.status === 'complete'}<div class="table-empty"><strong>{snapshot.search.coverage === 'complete' ? t('search-empty-title') : t('search-coverage-incomplete-title')}</strong><p>{snapshot.search.coverage === 'complete' ? t(includePublic ? 'search-empty-public-body' : 'search-empty-local-body') : searchCoverageMessage(snapshot.search.coverage)}</p></div>{/if}{/each}</div>{:else}<div class="search-welcome"><BookOpen size={32} aria-hidden="true" /><h2>{t('desktop-search-welcome-title')}</h2><p>{t('desktop-search-welcome-body')}</p></div>{/if}
          {:else if destination === 'system'}
            <header class="page-heading"><div><h1>{t('desktop-page-system-title')}</h1><p>{t('desktop-page-system-body')}</p></div></header>
            <nav class="settings-nav" aria-label={t('desktop-page-system-title')}>{#each systemSections.slice(0, 3) as section (section.id)}<a href={`#system/${section.id}`} class:active={systemSection === section.id} onclick={(event) => openSystemSection(event, section.id)}>{t(section.labelId)}</a>{/each}</nav>
            <div class="settings-page">
              {#if systemSection === 'models'}<section id="system-models"><p class="section-label">{t('settings-local-ai')}</p><h2>{snapshot.model?.displayName ?? t('component-local-ai')}</h2><p>{snapshot.model?.active ? t('desktop-model-ready') : t('desktop-model-needs-setup')}</p>{#if snapshot.modelInstall}<progress max={snapshot.modelInstall.totalBytes} value={snapshot.modelInstall.downloaded}></progress><p>{modelInstallLabel(locale)}</p><button class="secondary" onclick={cancelModelInstall}>{t('action-cancel')}</button>{:else if !snapshot.model?.active}<button class="primary" onclick={prepareLocalModel} disabled={actionBusy}>{t('models-install')}</button>{/if}{#if snapshot.model?.licenseUrl}<button class="text-action" onclick={() => openVerifiedExternalLink(snapshot!.model!.licenseUrl!)}>{t('models-license-open')}</button>{/if}</section>{/if}
              {#if systemSection === 'preferences'}<section id="system-preferences"><p class="section-label">{t('desktop-preferences')}</p><h2>{t('desktop-preferences')}</h2><div class="settings-form"><SelectField label={t('settings-language')} value={locale} onchange={(value) => { locale = value as LocalePreference; }} options={[{ value: 'system', label: t('language-system') }, { value: 'en', label: 'English' }, { value: 'es', label: 'Español' }]} /><SelectField label={t('settings-theme')} value={theme} onchange={(value) => { theme = value as ThemePreference; }} options={[{ value: 'system', label: t('theme-system') }, { value: 'light', label: t('theme-light') }, { value: 'dark', label: t('theme-dark') }]} /><SelectField label={t('desktop-lan')} value={lanPreference} onchange={(value) => { lanPreference = value as LanPreference; }} options={[{ value: 'undecided', label: t('settings-lan-undecided') }, { value: 'disabled', label: t('onboarding-lan-disable') }, { value: 'enabled', label: t('onboarding-lan-enable') }]} /><SelectField label={t('desktop-close')} value={closeBehavior} onchange={(value) => { closeBehavior = value as CloseBehavior; }} options={[{ value: 'ask', label: t('desktop-ask') }, { value: 'hide', label: t('desktop-hide-tray') }, { value: 'quit', label: t('desktop-quit') }]} /><Switch label={t('updates-automatic')} checked={automaticUpdateChecks} onchange={(checked) => { automaticUpdateChecks = checked; }} /><button class="primary" onclick={() => savePreferences()} disabled={actionBusy}>{t('desktop-save-preferences')}</button></div></section><section><p class="section-label">{t('settings-login-title')}</p><h2>{t('settings-login-heading')}</h2><p>{autostartLabel(locale)}</p><div class="row-actions"><button class="secondary" onclick={() => changeAutostart(true)} disabled={autostartBusy}>{t('action-enable')}</button><button class="text-action" onclick={refreshAutostartState} disabled={autostartBusy}>{t('action-refresh')}</button></div></section>{/if}
              {#if systemSection === 'updates'}<section id="system-updates"><p class="section-label">{t('updates-title')}</p><h2>{t('updates-stable-title')}</h2><p>{updaterLabel(locale)}</p><div class="row-actions"><button class="secondary" onclick={() => runUpdaterAction('check')} disabled={updaterRequestId !== null}>{t('updates-check-now')}</button>{#if snapshot.updater?.status === 'available'}<button class="primary" onclick={() => runUpdaterAction('download')}>{t('updates-download')}</button>{:else if snapshot.updater?.status === 'readyToInstall'}<button class="primary" onclick={() => { confirmUpdateInstall = true; }}>{t('updates-install')}</button>{/if}</div>{#if confirmUpdateInstall}<div class="install-confirmation"><p>{t('updates-install-confirm')}</p><button class="primary" onclick={() => runUpdaterAction('install')}>{t('updates-install')}</button><button class="secondary" onclick={() => { confirmUpdateInstall = false; }}>{t('action-cancel')}</button></div>{/if}</section>{/if}
              <details class="advanced-disclosure"><summary>{t('desktop-advanced-details')}</summary><dl>{#if snapshot.nodeId}<div><dt>{t('desktop-network-identity')}</dt><dd><code>{shortPeerId(snapshot.nodeId)}</code></dd></div>{/if}{#if snapshot.mcpUrl}<div><dt>{t('diagnostics-local-mcp')}</dt><dd><code>{snapshot.mcpUrl}</code></dd></div>{/if}{#if snapshot.hardware}<div><dt>{t('desktop-memory-installed')}</dt><dd>{formatBytes(snapshot.hardware.totalMemoryBytes)}</dd></div><div><dt>{t('desktop-disk-available')}</dt><dd>{formatBytes(snapshot.hardware.availableDiskBytes)}</dd></div>{/if}</dl></details>
            </div>
          {/if}
          {#if actionMessage}<p class={`action-message ${actionMessageTone()}`} aria-live="polite">{actionMessage}</p>{/if}
        </div>
      {/key}
    </section>
  </main>
  <SystemStatusBar {snapshot} {t} onselect={openServiceStatus} />
</div>

<div class="dialog-layer" inert={closeChoiceRequired} aria-hidden={closeChoiceRequired ? 'true' : undefined}>
{#if createWikiOpen && folderSelection}
  <div class="modal-backdrop" role="presentation"><div class="create-wiki-dialog" role="dialog" aria-modal="true" aria-labelledby="create-wiki-title"><form onsubmit={(event) => { event.preventDefault(); createWiki(); }}><p class="section-label">{t('desktop-new-wiki')}</p><h2 id="create-wiki-title">{t('desktop-name-wiki')}</h2><p>{folderSelection.displayName}</p><TextField label={t('desktop-wiki-name')} bind:value={wikiName} maxlength={120} required /><Switch label={t('desktop-continuous-indexing')} description={t('desktop-continuous-indexing-body')} bind:checked={continuousIndexing} /><div class="row-actions"><button type="button" class="secondary" onclick={() => { createWikiOpen = false; folderSelection = null; continuousIndexing = true; }}>{t('action-cancel')}</button><button class="primary" disabled={actionBusy || !wikiName.trim()}>{t('desktop-create-wiki')}</button></div></form></div></div>
{/if}

{#if newWikiMenuOpen}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.currentTarget === event.target) newWikiMenuOpen = false; }}><div class="create-wiki-dialog source-choice" role="dialog" aria-modal="true" aria-labelledby="new-wiki-source-title"><p class="section-label">{t('desktop-new-wiki')}</p><h2 id="new-wiki-source-title">{t('desktop-new-wiki-source')}</h2><button class="source-choice-item" onclick={chooseFolder}><strong>{t('desktop-new-wiki-folder')}</strong><span>{t('desktop-new-wiki-folder-body')}</span></button><button class="source-choice-item" onclick={() => chooseOkfImport(false)}><strong>{t('desktop-import-okf-folder')}</strong><span>{t('desktop-import-okf-body')}</span></button><button class="source-choice-item" onclick={() => chooseOkfImport(true)}><strong>{t('desktop-import-okf-zip')}</strong><span>{t('desktop-import-okf-body')}</span></button><button class="text-action" onclick={() => { newWikiMenuOpen = false; }}>{t('action-cancel')}</button></div></div>
{/if}

{#if okfImportSelection && okfImportSummary}
  <div class="modal-backdrop" role="presentation">
    <div class="create-wiki-dialog" role="dialog" aria-modal="true" aria-labelledby="import-okf-title">
      <form onsubmit={(event) => { event.preventDefault(); confirmOkfImport(); }}>
        <p class="section-label">{okfImportSummary.declaredOkfVersion ? `OKF ${okfImportSummary.declaredOkfVersion}` : t('desktop-okf-version-undeclared')}</p>
        <h2 id="import-okf-title">{t('desktop-import-okf-confirm')}</h2>
        <p>{okfImportSelection.displayName}</p>
        <p class:warning-text={okfImportSummary.restrictions.length > 0}>{t(`desktop-okf-compatibility-${okfImportSummary.compatibility.kind}`)}</p>
        <dl class="import-summary">
          <div><dt>{t('desktop-import-okf-concepts')}</dt><dd>{okfImportSummary.conceptCount}</dd></div>
          <div><dt>{t('desktop-import-okf-files')}</dt><dd>{okfImportSummary.entryCount}</dd></div>
          <div><dt>{t('desktop-import-okf-warnings')}</dt><dd>{okfImportSummary.warningCount}</dd></div>
        </dl>
        {#if okfImportSummary.restrictions.length > 0}
          <div class="import-warning-list" aria-labelledby="import-restrictions-title">
            <strong id="import-restrictions-title">{t('desktop-import-okf-restrictions')}</strong>
            <ul>{#each okfImportSummary.restrictions as restriction (restriction)}<li>{t(`desktop-okf-restriction-${restriction}`)}</li>{/each}</ul>
          </div>
        {/if}
        {#if okfImportSummary.warnings.length > 0}
          <details class="advanced-disclosure">
            <summary>{t('desktop-import-okf-warning-details')}</summary>
            <ul class="import-warning-list">
              {#each okfImportSummary.warnings as warning (`${warning.code}:${warning.logicalPath}:${warning.field ?? ''}`)}
                <li><strong>{t(`desktop-okf-warning-${warning.code}`)}</strong><code>{warning.logicalPath}</code>{#if warning.field}<span>{warning.field}</span>{/if}</li>
              {/each}
            </ul>
          </details>
        {/if}
        <TextField label={t('desktop-wiki-name')} bind:value={wikiName} maxlength={120} required />
        <div class="row-actions"><button type="button" class="secondary" onclick={() => { okfImportSelection = null; okfImportSummary = null; wikiName = ''; }}>{t('action-cancel')}</button><button class="primary" disabled={actionBusy || !wikiName.trim()}>{t('desktop-import-okf-action')}</button></div>
      </form>
    </div>
  </div>
{/if}

{#if detailsWikiId}
  {@const detailsWiki = snapshot.wikis.find((wiki) => wiki.id === detailsWikiId)}
  {@const detailsIssues = snapshot.sourceIssues.filter((issue) => issue.wikiId === detailsWikiId)}
  <div class="drawer-backdrop" role="presentation" onclick={(event) => { if (event.currentTarget === event.target) { detailsWikiId = null; relinkSelection = null; } }}><div class="side-drawer details-drawer" role="dialog" aria-modal="true" aria-labelledby="details-title"><header><div><p class="section-label">{t('desktop-details')}</p><h2 id="details-title">{detailsWiki?.name}</h2></div><button class="icon-button" aria-label={t('action-close')} onclick={() => { detailsWikiId = null; relinkSelection = null; }}>×</button></header><p>{t('desktop-wiki-details-body')}</p>{#if detailsWiki}<dl class="wiki-details-list"><div><dt>{t('desktop-wiki-source-health')}</dt><dd class:warning-text={detailsWiki.maintenanceRequired || detailsWiki.failedCount > 0}>{detailsWiki.maintenanceRequired || detailsWiki.failedCount > 0 ? t('status-needs-attention') : t('desktop-wiki-health-ready')}</dd></div><div><dt>{t('desktop-wiki-documents')}</dt><dd>{detailsWiki.documentCount}</dd></div><div><dt>{t('desktop-wiki-published')}</dt><dd>{detailsWiki.publishedCount}</dd></div><div><dt>{t('desktop-wiki-pending')}</dt><dd>{detailsWiki.needsReviewCount}</dd></div><div><dt>{t('desktop-wiki-failed')}</dt><dd>{detailsWiki.failedCount}</dd></div></dl>{/if}<section class="details-section"><div><h3>{t('desktop-wiki-source-issues')}</h3><p>{t('desktop-folder-privacy')}</p></div>{#if detailsWiki?.origin === 'folder' && detailsWiki.restrictions.length === 0}<Switch label={t('desktop-continuous-indexing')} description={t('desktop-continuous-indexing-body')} checked={detailsWiki.indexingMode === 'continuous'} onchange={(checked) => changeWikiIndexing(detailsWiki.id, checked)} />{/if}{#if detailsIssues.length > 0}<ul class="source-issue-list">{#each detailsIssues as issue (`${issue.sourceName}:${issue.code}`)}<li><AlertTriangle size={16} aria-hidden="true" /><span><strong>{issue.sourceName}</strong><small>{sourceIssueLabel(issue)}</small><small class="source-issue-action">{t('source-issue-next-step')} {sourceIssueActionLabel(issue)}</small></span></li>{/each}</ul>{:else if detailsWiki?.maintenanceRequired}<div class="inline-empty compact warning-empty"><AlertTriangle size={18} aria-hidden="true" /><span><strong>{t('desktop-wiki-maintenance-required')}</strong><small>{t('desktop-wiki-maintenance-required-body')}</small></span></div>{:else}<div class="inline-empty compact"><CheckCircle2 size={18} aria-hidden="true" /><span><strong>{t('desktop-wiki-no-source-issues')}</strong></span></div>{/if}{#if detailsWiki?.origin === 'folder' && detailsWiki.restrictions.length === 0}<div class="drawer-actions"><button class="secondary" onclick={() => scanWiki(detailsWiki.id)} disabled={wikiScanState(detailsWiki.id) !== null}><RefreshCw size={16} aria-hidden="true" />{t('action-refresh')}</button><button class="secondary" onclick={chooseRelinkFolder}>{t('desktop-wiki-relink')}</button></div>{/if}{#if relinkSelection}<div class="relink-confirmation"><p>{relinkSelection.displayName}</p><button class="primary" onclick={applyRelink} disabled={actionBusy}>{t('action-confirm')}</button></div>{/if}</section>{#if detailsWiki}<details class="advanced-disclosure"><summary>{t('desktop-advanced-details')}</summary><dl><div><dt>{t('desktop-wiki-id')}</dt><dd><code>{detailsWiki.id}</code></dd></div><div><dt>OKF</dt><dd>{detailsWiki.okfVersion}</dd></div></dl></details><section class="danger-zone"><h3>{t('desktop-delete-wiki')}</h3><p>{detailsWiki.origin === 'folder' ? t('desktop-delete-folder-wiki-body') : t('desktop-delete-managed-wiki-body')}</p><button class="danger" disabled={actionBusy} onclick={() => removeWiki(detailsWiki.id)}>{t('desktop-delete-wiki')}</button></section>{/if}</div></div>
{/if}

{#if editingWikiId}
  {@const activeWiki = snapshot.wikis.find((wiki) => wiki.id === editingWikiId)}
  <div class="drawer-backdrop" role="presentation" onclick={(event) => { if (event.currentTarget === event.target) editingWikiId = null; }}><div class="side-drawer" role="dialog" aria-modal="true" aria-labelledby="share-title"><header><div><p class="section-label">{t('desktop-share-action')}</p><h2 id="share-title">{activeWiki?.name}</h2></div><button class="icon-button" aria-label={t('action-close')} onclick={() => { editingWikiId = null; }}>×</button></header><p>{t('desktop-share-drawer-body')}</p><div class="policy-list"><Switch label={t('desktop-share-nearby')} description={t('desktop-share-nearby-body')} bind:checked={wikiPolicy.peerShareable} /><Switch label={t('desktop-share-ai-apps')} description={t('desktop-share-ai-apps-body')} bind:checked={wikiPolicy.allowExternalAi} /><Switch label={t('desktop-share-public')} description={t('desktop-share-public-body')} bind:checked={wikiPolicy.internetPublic} /></div>{#if wikiPolicy.internetPublic}<TextField label={t('desktop-wiki-public-description')} bind:value={publicDescription} maxlength={2048} rows={3} multiline /><TextField label={t('desktop-wiki-public-languages')} bind:value={publicLanguages} maxlength={300} placeholder="es, en" />{/if}<div class="drawer-actions"><button class="primary" onclick={saveWikiPolicy} disabled={actionBusy}>{t('action-save')}</button>{#if wikiPolicy.internetPublic}<button class="secondary" onclick={savePublicProfile} disabled={actionBusy}>{t('desktop-wiki-public-profile-save')}</button>{/if}</div></div></div>
{/if}

{#if selectedReview && editDraft}
  <div class="drawer-backdrop" role="presentation" onclick={(event) => { if (event.currentTarget === event.target) selectedReview = null; }}><div class="side-drawer review-drawer" role="dialog" aria-modal="true" aria-labelledby="review-title"><header><div><p class="section-label">{t('desktop-wiki-pending-tab')}</p><h2 id="review-title">{selectedReview.sourceName}</h2></div><button class="icon-button" aria-label={t('action-close')} onclick={() => { selectedReview = null; editDraft = null; }}>×</button></header><ol class="review-steps"><li class="active"><span>1</span>{t('desktop-evidence')}</li><li><span>2</span>{t('desktop-proposal')}</li><li><span>3</span>{t('desktop-decision')}</li></ol><section><h3>{t('desktop-evidence')}</h3>{#if snapshot.reviewEvidence?.status === 'ready' && snapshot.reviewEvidence.conceptId === selectedReview.conceptId}<div class="evidence-list">{#each snapshot.reviewEvidence.excerpts as line (line.ordinal)}<blockquote>{line.text}</blockquote>{/each}</div>{#if snapshot.reviewEvidence.nextOrdinal != null}<button class="text-action" onclick={loadMoreEvidence}>{t('desktop-load-more')}</button>{/if}{:else}<p class="evidence-warning">{t('review-evidence-approval-blocked')}</p>{/if}</section><section><h3>{t('desktop-proposal')}</h3><TextField label={t('review-edit-title')} bind:value={editDraft.title} maxlength={200} disabled={selectedReviewIsReadOnly()} /><TextField label={t('review-edit-summary')} bind:value={editDraft.summary} maxlength={2000} rows={5} multiline disabled={selectedReviewIsReadOnly()} /></section>{#if selectedReviewIsReadOnly()}<p class="evidence-warning" role="status">{t('review-okf-read-only')}</p>{/if}<footer><button class="danger" onclick={() => decideReview('reject')} disabled={actionBusy}>{t('review-reject')}</button><button class="secondary" onclick={() => decideReview('reanalyze')} disabled={actionBusy || selectedReviewIsReadOnly()}>{t('review-reanalyze')}</button><button class="primary" onclick={() => decideReview('approve')} disabled={actionBusy || selectedReviewIsReadOnly() || !evidenceIsCurrent()}>{t('review-approve')}</button></footer></div></div>
{/if}

{#if connectionsOpen}
  <div class="drawer-backdrop" role="presentation" onclick={(event) => { if (event.currentTarget === event.target) connectionsOpen = false; }}><div class="side-drawer connections-drawer" role="dialog" aria-modal="true" aria-labelledby="connections-title"><header><div><p class="section-label">{t('desktop-page-shared-title')}</p><h2 id="connections-title">{t('desktop-connections')}</h2></div><button class="icon-button" aria-label={t('action-close')} onclick={() => { connectionsOpen = false; }}>×</button></header><section><div class="section-heading"><div><h3>{t('desktop-known-devices', { count: snapshot.peers.length })}</h3><p>{connectivityLabel(locale)}</p></div><button class="text-action" onclick={() => runConnectivityAction('refresh')}>{t('action-refresh')}</button></div>{#if lanPreference !== 'enabled'}<div class="connection-guidance"><p>{lanPreference === 'undecided' ? t('connectivity-undecided') : t('connectivity-disabled')}</p><button class="secondary" onclick={openNetworkPreferences}>{t('desktop-preferences')}</button></div>{:else if snapshot.connectivity?.networkProfile === 'public'}<div class="connection-guidance"><p>{t('connectivity-public-network')}</p><button class="secondary" onclick={() => runConnectivityAction('networkSettings')}>{t('connectivity-open-network-settings')}</button></div>{:else if snapshot.connectivity?.firewall === 'rulesMissing' && snapshot.connectivity.firewallHelper === 'verified'}<div class="connection-guidance"><p>{t('connectivity-firewall-needed')}</p><button class="secondary" onclick={() => runConnectivityAction('install')}>{t('connectivity-configure-firewall')}</button></div>{:else if snapshot.connectivity?.firewall === 'rulesMissing'}<div class="connection-guidance"><p>{t('connectivity-firewall-helper-repair')}</p></div>{:else if snapshot.connectivity?.firewall === 'conflict' || snapshot.connectivity?.firewall === 'legacyExposure' || snapshot.connectivity?.firewall === 'managedPolicy' || snapshot.connectivity?.firewall === 'firewallDisabled' || snapshot.connectivity?.firewall === 'blockAllInbound'}<div class="connection-guidance"><p>{firewallGuidanceLabel()}</p><button class="secondary" onclick={() => runConnectivityAction('advancedFirewall')}>{t('connectivity-open-advanced-firewall')}</button></div>{:else if snapshot.connectivity?.systemPermission === 'denied'}<div class="connection-guidance"><p>{t('connectivity-failed')}</p><button class="secondary" onclick={() => runConnectivityAction('localNetworkPrivacy')}>{t('connectivity-open-local-network-settings')}</button></div>{/if}<div class="peer-list">{#each snapshot.peers as peer (peer.peerId)}<article><div class="peer-summary"><strong>{peer.deviceName ?? t('devices-nearby')}</strong><small>{peer.trust === 'trusted' ? t('desktop-verified') : peer.trust === 'blocked' ? t('desktop-pairing-blocked') : t('desktop-unverified')}</small></div>{#if peer.sasWords}<strong class="sas-words">{peer.sasWords.join(' · ')}</strong><div class="row-actions"><button class="primary" disabled={peerActionId === peer.peerId} onclick={() => runPeerAction(peer.peerId, 'accept')}>{t('devices-code-matches')}</button><button class="danger" disabled={peerActionId === peer.peerId} onclick={() => runPeerAction(peer.peerId, 'reject')}>{t('devices-code-does-not-match')}</button></div>{:else if peer.trust === 'unpaired'}<button class="secondary" disabled={peerActionId === peer.peerId} onclick={() => runPeerAction(peer.peerId, 'pair')}>{t('desktop-verify-device')}</button>{:else if peer.trust === 'blocked'}<div class="blocked-peer-guidance"><p>{t('desktop-pairing-blocked-help')}</p><button class="secondary" disabled={peerActionId === peer.peerId} onclick={() => runPeerAction(peer.peerId, 'allowAgain')}>{t('desktop-allow-pairing-again')}</button></div>{:else if peer.trust === 'trusted'}<div class="grant-list">{#each snapshot.wikis.filter((wiki) => wiki.peerShareable) as wiki (wiki.id)}<Checkbox label={wiki.name} checked={peer.grantedWikiIds.includes(wiki.id)} onchange={(checked) => changeGrant(peer.peerId, wiki.id, checked)} />{/each}</div><button class="text-action" disabled={peerActionId === peer.peerId} onclick={() => runPeerAction(peer.peerId, 'revoke')}>{t('desktop-revoke-trust')}</button>{/if}</article>{:else}<p class="empty">{t('desktop-no-devices')}</p>{/each}</div></section><section><div class="section-heading"><div><h3>{t('desktop-ai-clients')}</h3><p>{t('desktop-integration-body')}</p></div><button class="text-action" disabled={integrationRequestId !== null} onclick={() => runIntegrationAction({ kind: 'refresh' })}>{t('action-refresh')}</button></div><div class="integration-list">{#each snapshot.integrations?.integrations ?? [] as integration (integration.client)}<article><div><strong>{integrationName(integration.client)}</strong><small>{integrationState(integration.status)}</small></div>{#if integration.status === 'available' || integration.status === 'updateAvailable'}<button class="secondary" disabled={integrationRequestId !== null} onclick={() => runIntegrationAction({ kind: 'connect', client: integration.client })}>{t('integrations-connect')}</button>{:else if integration.status === 'configured'}<button class="text-action" disabled={integrationRequestId !== null} onclick={() => runIntegrationAction({ kind: 'disconnect', client: integration.client })}>{t('integrations-disconnect')}</button>{/if}{#if integration.mcpSetup}<div class="mcp-setup"><div><strong>{t('integrations-generic-setup')}</strong><small>{t('integrations-generic-setup-help')}</small></div><pre>{mcpSetupText(integration.mcpSetup.command, integration.mcpSetup.args)}</pre><button class="secondary" onclick={() => copyMcpSetup(integration.mcpSetup?.command ?? '', integration.mcpSetup?.args ?? [])}>{t('action-copy')}</button></div>{/if}</article>{/each}</div><div class="application-access-list">{#each snapshot.applicationAccess.filter((application) => application.active) as application (application.appId)}<article><div><strong>{application.displayName}</strong><small>{t('desktop-application-quota', { count: application.ownedWikiCount, size: formatBytes(application.managedBytes) })}</small></div>{#each snapshot.wikis.filter((wiki) => wiki.origin === 'aiMemory' && application.grants.some((grant) => grant.wikiId === wiki.id && grant.role === 'owner') === false) as wiki (wiki.id)}<SelectField label={wiki.name} value={applicationGrantRole(application.appId, wiki.id)} options={[{ value: 'none', label: t('desktop-application-no-access') }, { value: 'reader', label: t('desktop-application-reader') }, { value: 'editor', label: t('desktop-application-editor') }]} onchange={(value) => changeApplicationGrant(application.appId, wiki.id, value === 'none' ? null : value as ApplicationWikiRoleInput)} disabled={actionBusy} />{/each}</article>{/each}</div></section><ConnectionAdvanced lanRuntime={snapshot.lanRuntime} peerId={federationPeerId} address={federationAddress} blockedPublishers={snapshot.blockedPublicPublishers} busy={peerActionId !== null} {t} onpeerid={(value) => { federationPeerId = value; }} onaddress={(value) => { federationAddress = value; }} onadd={() => saveFederationIndex(false)} onremove={() => saveFederationIndex(true)} onunblock={(publisherId) => changePublisherBlock(publisherId, false)} /><details class="advanced-disclosure"><summary>{t('desktop-advanced-details')}</summary><TextField label={t('desktop-address')} bind:value={manualPeerAddress} maxlength={500} /><button class="secondary" onclick={connectManualPeer}>{t('action-connect')}</button></details></div></div>
{/if}

</div>

{/if}
{#if closeChoiceRequired}
  <div class="modal-backdrop close-confirmation-backdrop" role="presentation">
    <div class="close-dialog" role="dialog" aria-modal="true" aria-labelledby="close-title">
      <p class="section-label">{t('desktop-close-eyebrow')}</p><h2 id="close-title">{t('close-dialog-title')}</h2>
      <p>{t('desktop-hide-services')}</p>
      <div><button class="primary" onclick={() => applyCloseChoice('hide')}>{t('desktop-hide-tray')}</button><button class="danger" onclick={() => applyCloseChoice('quit')}>{t('desktop-quit')}</button><button class="secondary" onclick={() => applyCloseChoice('cancel')}>{t('action-cancel')}</button></div>
    </div>
  </div>
{/if}
