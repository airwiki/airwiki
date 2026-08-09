<script lang="ts">
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import BookOpen from '@lucide/svelte/icons/book-open';
  import CheckCircle2 from '@lucide/svelte/icons/circle-check-big';
  import FileText from '@lucide/svelte/icons/file-text';
  import History from '@lucide/svelte/icons/history';
  import Network from '@lucide/svelte/icons/network';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Search from '@lucide/svelte/icons/search';
  import Settings2 from '@lucide/svelte/icons/settings-2';
  import Sparkles from '@lucide/svelte/icons/sparkles';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import { addCollection, addFederationIndex, approveReview, browsePublicCollection, cancelModelInstall, checkUpdates, configureFirewall, confirmPairing, connect, dialPeer, downloadUpdate, executeGuidedWikiRepair, hideToTray, installModels, installUpdate, loadKnowledgeBundle, loadKnowledgePage, loadReviewEvidence, manageIntegration, openExternalLink, openSystemDestination, pairPeer, pickCollectionFolder, prepareGuidedWikiRepair, quitCompletely, reanalyzeReview, refreshAutostart, refreshConnectivity, refreshWikiHealth, rejectReview, relinkCollection, removeFederationIndex, rescanCollection, revokePeer, searchKnowledge, setAutostart, setCollectionGrant, setPublicPublisherBlocked, updateCollectionPolicy, updatePreferences, updatePublicCollectionProfile, type AppSnapshot, type CloseBehavior, type CollectionPolicyInput, type CollectionSummary, type EnrichmentDraft, type FolderSelection, type IntegrationActionInput, type IntegrationClient, type KnowledgePageInput, type LanPreference, type LocalePreference, type ReviewSummary, type SearchHitSummary, type SystemDestination, type ThemePreference } from './api';
  import KnowledgeGraph from './KnowledgeGraph.svelte';
  import { message, resolveLocale, type MessageArgs } from './i18n';

  type Destination = 'library' | 'review' | 'search' | 'system';
  type SystemSection = 'models' | 'preferences' | 'updates' | 'connectivity' | 'devices' | 'integrations';

  const destinations = [
    { id: 'library', labelId: 'desktop-nav-library', icon: BookOpen },
    { id: 'review', labelId: 'nav-review', icon: CheckCircle2 },
    { id: 'search', labelId: 'nav-search', icon: Search },
    { id: 'system', labelId: 'desktop-nav-system', icon: Settings2 }
  ] as const;
  const systemSections = [
    { id: 'models', labelId: 'settings-local-ai' },
    { id: 'preferences', labelId: 'desktop-preferences' },
    { id: 'updates', labelId: 'updates-title' },
    { id: 'connectivity', labelId: 'connectivity-title' },
    { id: 'devices', labelId: 'devices-title' },
    { id: 'integrations', labelId: 'integrations-title' }
  ] as const;

  let destination: Destination = 'library';
  let systemSection: SystemSection = 'preferences';
  let runtimeMessageId = 'status-working';
  let snapshot: AppSnapshot | null = null;
  let folderSelection: FolderSelection | null = null;
  let relinkSelection: FolderSelection | null = null;
  let collectionName = '';
  let editingCollectionId: string | null = null;
  let collectionPolicy: CollectionPolicyInput = { localOnly: true, peerShareable: false, allowExternalAi: false, internetPublic: false };
  let publicDescription = '';
  let publicLanguages = '';
  let question = '';
  let includePublic = false;
  let actionMessage = '';
  let actionBusy = false;
  let selectedReview: ReviewSummary | null = null;
  let editDraft: EnrichmentDraft | null = null;
  let selectedCollectionId: string | null = null;
  let knowledgeMode: 'document' | 'graph' = 'document';
  let locale: LocalePreference = 'system';
  let theme: ThemePreference = 'system';
  let lanPreference: LanPreference = 'undecided';
  let closeBehavior: CloseBehavior = 'ask';
  let automaticUpdateChecks = false;
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
  let guidedRepairRequestId: string | null = null;
  let guidedRepairConfirmed = false;
  let mainScrollRegion: HTMLElement | null = null;

  function scrollMainTo(top: number) {
    requestAnimationFrame(() => {
      mainScrollRegion?.scrollTo({ top: Math.max(0, top), left: 0, behavior: 'auto' });
    });
  }

  function scrollToSystemSection(section: SystemSection) {
    requestAnimationFrame(() => {
      const target = document.getElementById(`system-${section}`);
      if (!mainScrollRegion || !target || !mainScrollRegion.contains(target)) return;
      const regionTop = mainScrollRegion.getBoundingClientRect().top;
      const targetTop = target.getBoundingClientRect().top;
      mainScrollRegion.scrollTo({
        top: Math.max(0, mainScrollRegion.scrollTop + targetTop - regionTop - 24),
        left: 0,
        behavior: 'auto'
      });
    });
  }

  function pushHash(hash: string) {
    if (window.location.hash !== hash) window.history.pushState(null, '', hash);
  }

  function openSystemSection(event: MouseEvent, section: SystemSection) {
    event.preventDefault();
    systemSection = section;
    pushHash(`#system/${section}`);
    scrollToSystemSection(section);
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

  $: if (typeof document !== 'undefined') {
    document.documentElement.lang = resolveLocale(locale);
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme === 'system' ? 'light dark' : theme;
  }

  onMount(() => {
    const syncRoute = () => {
      const [route, section] = window.location.hash.slice(1).split('/');
      const matchedDestination = destinations.find((candidate) => candidate.id === route);
      if (matchedDestination) {
        destination = matchedDestination.id;
        if (matchedDestination.id !== 'system') scrollMainTo(0);
      }
      const matchedSection = systemSections.find((candidate) => candidate.id === section);
      if (route === 'system' && matchedSection) {
        systemSection = matchedSection.id;
        scrollToSystemSection(matchedSection.id);
      } else if (route === 'system') {
        systemSection = 'preferences';
        scrollMainTo(0);
      }
    };
    syncRoute();
    window.addEventListener('hashchange', syncRoute);
    window.addEventListener('popstate', syncRoute);
    const unlistenClose = '__TAURI_INTERNALS__' in window
      ? listen('close-choice-required', () => { closeChoiceRequired = true; })
      : Promise.resolve(() => {});
    connect((event) => {
      snapshot = event.snapshot;
      if (event.snapshot.model?.licenseAccepted) modelLicensesConfirmed = true;
      if (event.snapshot.preferences) {
        locale = event.snapshot.preferences.locale;
        theme = event.snapshot.preferences.theme;
        lanPreference = event.snapshot.preferences.lanPreference;
        closeBehavior = event.snapshot.preferences.closeBehavior;
        automaticUpdateChecks = event.snapshot.preferences.automaticUpdateChecks;
      }
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
      if (event.requestId && event.requestId === publicBrowseRequestId) publicBrowseRequestId = null;
      if (event.requestId && event.requestId === guidedRepairRequestId) guidedRepairRequestId = null;
      runtimeMessageId = event.snapshot.phase === 'ready' ? 'status-ready' : 'status-working';
    }).then((initial) => {
      snapshot = initial;
      if (initial.model?.licenseAccepted) modelLicensesConfirmed = true;
      if (initial.preferences) {
        locale = initial.preferences.locale;
        theme = initial.preferences.theme;
        lanPreference = initial.preferences.lanPreference;
        closeBehavior = initial.preferences.closeBehavior;
        automaticUpdateChecks = initial.preferences.automaticUpdateChecks;
      }
      runtimeMessageId = initial.phase === 'ready' ? 'status-ready' : runtimeMessageId;
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
      void unlistenClose.then((unlisten) => unlisten());
    };
  });

  function select(next: Destination) {
    destination = next;
    scrollMainTo(0);
    if (next === 'system') {
      systemSection = 'preferences';
      pushHash('#system/preferences');
      void refreshAutostartState();
      void runConnectivityAction('refresh');
      void runIntegrationAction({ kind: 'refresh' });
    } else {
      window.location.hash = next;
    }
    if (next === 'library') void refreshHealth();
  }

  function integrationName(client: IntegrationClient): string {
    if (client === 'chatGptDesktop') return 'ChatGPT Desktop / Work';
    if (client === 'claudeDesktop') return 'Claude Desktop';
    return 'Gemini CLI';
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
    const requestId = crypto.randomUUID();
    integrationRequestId = requestId;
    try {
      await manageIntegration(requestId, action);
    } catch {
      integrationRequestId = null;
      actionMessage = t('error-chat');
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

  function lanStateLabel(state: string): string {
    const labels: Record<string, string> = {
      stopped: 'status-optional-disabled', starting: 'status-working', listening: 'status-ready', failed: 'status-needs-attention',
      disabled: 'status-optional-disabled', active: 'status-ready'
    };
    return t(labels[state] ?? 'error-generic');
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

  function shortPeerId(peerId: string): string {
    return peerId.length > 18 ? `${peerId.slice(0, 9)}…${peerId.slice(-7)}` : peerId;
  }

  async function runPeerAction(peerId: string, action: 'pair' | 'accept' | 'reject' | 'revoke') {
    peerActionId = peerId;
    try {
      if (action === 'pair') await pairPeer(peerId);
      if (action === 'accept') await confirmPairing(peerId, true);
      if (action === 'reject') await confirmPairing(peerId, false);
      if (action === 'revoke') await revokePeer(peerId);
    } catch {
      actionMessage = t('error-connectivity');
    } finally {
      peerActionId = null;
    }
  }

  async function changeGrant(peerId: string, collectionId: string, granted: boolean) {
    peerActionId = peerId;
    try {
      await setCollectionGrant(peerId, collectionId, granted);
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

  async function openAttentionCollection() {
    const collectionId = snapshot?.wikiHealth?.attentionCollectionId;
    if (collectionId) await openKnowledge(collectionId);
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
      actionMessage = t('notice-operation-complete');
    } catch {
      actionMessage = t('error-generic');
      autostartBusy = false;
      autostartRequestId = null;
    }
  }

  function nextActionLabel(currentLocale: LocalePreference): string {
    if (destination === 'library') return message(currentLocale, 'primary-button-add-folder');
    if (destination === 'review') return message(currentLocale, snapshot?.reviews.length ? 'action-review' : 'review-empty-title');
    if (destination === 'search') return message(currentLocale, 'search-question');
    return message(currentLocale, 'action-confirm');
  }

  async function runNextAction() {
    if (destination === 'library') await chooseFolder();
    if (destination === 'review' && snapshot?.reviews[0]) await openReview(snapshot.reviews[0]);
    if (destination === 'search') document.querySelector<HTMLTextAreaElement>('#knowledge-question')?.focus();
    if (destination === 'system') await savePreferences(false);
  }

  async function chooseFolder() {
    actionMessage = '';
    try {
      folderSelection = await pickCollectionFolder();
    } catch {
      actionMessage = t('error-collection');
    }
  }

  async function createCollection() {
    if (!folderSelection) return;
    actionBusy = true;
    try {
      await addCollection(collectionName, folderSelection.token);
      collectionName = '';
      folderSelection = null;
      actionMessage = t('notice-operation-complete');
    } catch {
      actionMessage = t('error-collection');
      folderSelection = null;
    } finally {
      actionBusy = false;
    }
  }

  function collectionScanState(collectionId: string) {
    return snapshot?.collectionScans.find((scan) => scan.collectionId === collectionId)?.state ?? null;
  }

  async function scanCollection(collectionId: string) {
    try {
      await rescanCollection(collectionId);
      actionMessage = t('notice-operation-complete');
    } catch {
      actionMessage = t('error-collection');
    }
  }

  function editCollection(collection: CollectionSummary) {
    editingCollectionId = collection.id;
    relinkSelection = null;
    collectionPolicy = {
      localOnly: collection.localOnly,
      peerShareable: collection.peerShareable,
      allowExternalAi: collection.allowExternalAi,
      internetPublic: collection.internetPublic
    };
    publicDescription = collection.publicDescription;
    publicLanguages = collection.publicLanguages;
  }

  async function chooseRelinkFolder() {
    try {
      relinkSelection = await pickCollectionFolder();
    } catch {
      actionMessage = t('error-collection');
    }
  }

  async function applyRelink() {
    if (!editingCollectionId || !relinkSelection) return;
    actionBusy = true;
    try {
      await relinkCollection(editingCollectionId, relinkSelection.token);
      relinkSelection = null;
      actionMessage = t('notice-operation-complete');
    } catch {
      relinkSelection = null;
      actionMessage = t('error-collection');
    } finally {
      actionBusy = false;
    }
  }

  async function saveCollectionPolicy() {
    if (!editingCollectionId) return;
    actionBusy = true;
    try {
      const policy = {
        ...collectionPolicy,
        localOnly: !collectionPolicy.peerShareable && !collectionPolicy.allowExternalAi && !collectionPolicy.internetPublic
      };
      await updateCollectionPolicy(editingCollectionId, policy);
      collectionPolicy = policy;
      actionMessage = t('notice-operation-complete');
    } catch {
      actionMessage = t('error-collection');
    } finally {
      actionBusy = false;
    }
  }

  async function savePublicProfile() {
    if (!editingCollectionId) return;
    actionBusy = true;
    const languages = publicLanguages.split(',').map((language) => language.trim()).filter(Boolean);
    try {
      await updatePublicCollectionProfile(editingCollectionId, publicDescription.trim(), languages);
      actionMessage = t('notice-operation-complete');
    } catch {
      actionMessage = t('error-collection');
    } finally {
      actionBusy = false;
    }
  }

  async function prepareRepair(collectionId: string) {
    guidedRepairConfirmed = false;
    try {
      guidedRepairRequestId = await prepareGuidedWikiRepair(collectionId);
      actionMessage = t('status-working');
    } catch {
      guidedRepairRequestId = null;
      actionMessage = t('error-collection');
    }
  }

  async function executeRepair(collectionId: string) {
    if (!guidedRepairConfirmed) return;
    try {
      guidedRepairRequestId = await executeGuidedWikiRepair(collectionId);
      guidedRepairConfirmed = false;
      actionMessage = t('notice-operation-complete');
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
    actionBusy = true;
    try {
      await searchKnowledge(question, includePublic);
      actionMessage = t('search-running');
    } catch {
      actionMessage = t('search-error-title');
      actionBusy = false;
    }
  }

  async function openSearchHit(hit: SearchHitSummary) {
    const localCollection = snapshot?.collections.some((collection) => collection.id === hit.collectionId);
    if (localCollection && hit.nodeId === snapshot?.nodeId) {
      destination = 'library';
      window.location.hash = 'library';
      selectedCollectionId = hit.collectionId;
      await loadKnowledgeBundle(hit.collectionId);
      await loadKnowledgePage(hit.collectionId, { kind: 'concept', id: hit.conceptId });
      return;
    }
    try {
      publicBrowseRequestId = await browsePublicCollection(hit.nodeId, hit.collectionId);
      actionMessage = t('search-running');
    } catch {
      publicBrowseRequestId = null;
      actionMessage = t('search-coverage-public-offline');
    }
  }

  async function loadMorePublicConcepts() {
    const browse = snapshot?.publicBrowse;
    if (!browse?.publisherId || !browse.collectionId || !browse.nextCursor) return;
    try {
      publicBrowseRequestId = await browsePublicCollection(browse.publisherId, browse.collectionId, browse.nextCursor);
    } catch {
      publicBrowseRequestId = null;
      actionMessage = t('search-error-title');
    }
  }

  async function changePublisherBlock(publisherId: string, blocked: boolean) {
    try {
      await setPublicPublisherBlocked(publisherId, blocked);
      actionMessage = t('notice-operation-complete');
    } catch {
      actionMessage = t('search-error-title');
    }
  }

  async function saveFederationIndex(remove = false) {
    try {
      if (remove) await removeFederationIndex(federationPeerId.trim());
      else await addFederationIndex(federationPeerId.trim(), federationAddress.trim());
      actionMessage = t('notice-operation-complete');
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
      actionMessage = t('notice-operation-complete');
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

  async function decideReview(decision: 'approve' | 'reject' | 'reanalyze') {
    if (!selectedReview || (decision === 'approve' && (!editDraft || !evidenceIsCurrent()))) return;
    actionBusy = true;
    try {
      if (decision === 'approve' && editDraft) await approveReview(selectedReview.conceptId, selectedReview.sourceRevision, editDraft);
      if (decision === 'reject') await rejectReview(selectedReview.conceptId);
      if (decision === 'reanalyze') await reanalyzeReview(selectedReview.conceptId);
      actionMessage = t('notice-operation-complete');
    } catch {
      actionMessage = t('review-evidence-approval-blocked');
    } finally {
      actionBusy = false;
    }
  }

  async function openKnowledge(collectionId: string) {
    selectedCollectionId = collectionId;
    knowledgeMode = 'document';
    actionBusy = true;
    actionMessage = t('home-wiki-checking');
    try {
      await loadKnowledgeBundle(collectionId);
    } catch {
      actionMessage = t('home-wiki-failed');
      actionBusy = false;
    }
  }

  async function openKnowledgePage(page: KnowledgePageInput) {
    if (!selectedCollectionId) return;
    actionBusy = true;
    try {
      await loadKnowledgePage(selectedCollectionId, page);
    } catch {
      actionMessage = t('search-local-unavailable');
      actionBusy = false;
    }
  }

  async function selectGraphPage(page: KnowledgePageInput) {
    knowledgeMode = 'document';
    await openKnowledgePage(page);
  }

  async function savePreferences(completeOnboarding = false) {
    actionBusy = true;
    try {
      await updatePreferences({ locale, theme, lanPreference, closeBehavior, automaticUpdateChecks, completeOnboarding });
      actionMessage = t('notice-operation-complete');
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

<svelte:head><meta name="theme-color" content="#07131f" /></svelte:head>

{#if !snapshot || snapshot.phase !== 'ready' || !snapshot.preferences}
  <main class="onboarding startup" aria-busy="true">
    <div class="onboarding-mark">A</div>
    <p class="eyebrow">AirWiki</p>
    <h1>{t('status-working')}</h1>
    <p class="lede" aria-live="polite">{t(runtimeMessageId)}</p>
  </main>
{:else if snapshot.preferences.completedOnboardingVersion == null}
  <main class="onboarding">
    <div class="onboarding-mark">A</div>
    <p class="eyebrow">{t('onboarding-welcome-title')}</p>
    <h1>{t('first-knowledge-eyebrow')}<br />{t('onboarding-privacy-title')}</h1>
    <p class="lede">{t('onboarding-welcome-body')}</p>
    <div class="onboarding-steps">
      <section><span>01</span><div><h2>{t('settings-language')}</h2><p>{t('settings-subtitle')}</p></div><select bind:value={locale}><option value="system">{t('language-system')}</option><option value="es">{t('language-spanish')}</option><option value="en">{t('language-english')}</option></select></section>
      <section><span>02</span><div><h2>{t('onboarding-lan-title')}</h2><p>{t('onboarding-lan-body')}</p></div><select bind:value={lanPreference}><option value="disabled">{t('onboarding-lan-disable')}</option><option value="enabled">{t('onboarding-lan-enable')}</option></select></section>
      <section><span>03</span><div><h2>{t('onboarding-background-title')}</h2><p>{t('onboarding-background-body')}</p></div><select bind:value={closeBehavior}><option value="ask">{t('close-dialog-title')}</option><option value="hide_to_tray">{t('close-dialog-background')}</option><option value="quit">{t('tray-quit')}</option></select></section>
      {#if snapshot.model && !snapshot.model.active}<section><span>04</span><div><h2>{t('onboarding-model-title')}</h2><p>{snapshot.model.displayName ?? t('onboarding-model-recommended')} · {(snapshot.model.downloadBytes / 1073741824).toFixed(1)} GiB</p></div><label class="check"><input type="checkbox" bind:checked={modelLicensesConfirmed} /> {t('models-accept-licenses')}</label></section>{/if}
    </div>
    {#if snapshot.model && !snapshot.model.active}<button class="secondary onboarding-model" onclick={prepareLocalModel} disabled={actionBusy || (!modelLicensesConfirmed && !snapshot.model.licenseAccepted) || !snapshot.model.fitsAvailableDisk}>{t('primary-button-prepare')}</button>{/if}
    <button class="primary onboarding-action" onclick={() => savePreferences(true)} disabled={actionBusy || lanPreference === 'undecided'}>{t('onboarding-finish')}</button>
    {#if actionMessage}<p class="action-message" aria-live="polite">{actionMessage}</p>{/if}
  </main>
{:else}
<div class="shell">
  <aside class="rail" aria-label={t('nav-group-knowledge')}>
    <div class="brand"><span class="brand-mark">A</span><span>AirWiki</span></div>
    <nav>
      {#each destinations as item (item.id)}
        <button class:active={destination === item.id} onclick={() => select(item.id)}>
          <item.icon size={18} strokeWidth={1.8} aria-hidden="true" />
          <span>{t(item.labelId)}</span>
        </button>
      {/each}
    </nav>
    <div class="device-state">
      <span class="pulse" aria-hidden="true"></span>
      <div><strong>{t('nav-device-status')}</strong><small>{t(runtimeMessageId)}</small></div>
    </div>
  </aside>

  <main bind:this={mainScrollRegion}>
    <header>
      <div><p class="eyebrow">{t('dashboard-eyebrow')}</p><h1>{t('dashboard-title')}</h1></div>
      <button class="primary" onclick={runNextAction} disabled={destination === 'review' && !snapshot?.reviews.length}><Sparkles size={17} />{nextActionLabel(locale)}</button>
    </header>

    <section class="workspace" aria-live="polite">
      <div class="evidence-rail" aria-hidden="true"><i></i><i></i><i></i></div>
      <div class="content">
        <p class="section-label">{t(destinations.find((item) => item.id === destination)?.labelId ?? 'app-title')}</p>
        <h2>{destination === 'library' ? t('first-knowledge-title') : t(destinations.find((item) => item.id === destination)?.labelId ?? 'app-title')}</h2>
        <p class="lede">{t('dashboard-subtitle')}</p>

        <div class="sequence">
          <article><span>{t('journey-read')}</span><strong>{snapshot?.collections.length ?? 0} · {t('component-collections')}</strong><p>{t('onboarding-privacy-local')}</p></article>
          <article><span>{t('journey-prepare')}</span><strong>{snapshot?.collections.reduce((total, item) => total + item.documentCount, 0) ?? 0} · {t('collections-counts', { documents: snapshot?.collections.reduce((total, item) => total + item.documentCount, 0) ?? 0, published: snapshot?.collections.reduce((total, item) => total + item.publishedCount, 0) ?? 0 })}</strong><p>{t('primary-ai-explanation')}</p></article>
          <article><span>{t('journey-review')}</span><strong>{t('review-ready-summary', { count: snapshot?.reviews.length ?? 0 })}</strong><p>{t('primary-review-explanation')}</p></article>
        </div>

        {#if destination === 'library' && snapshot?.wikiHealth}
          <section class:attention={snapshot.wikiHealth.errorCount > 0} class="health-strip" aria-labelledby="health-title">
            <div><span class="health-signal" aria-hidden="true"></span><div><p class="section-label">{t('knowledge-tab-health')}</p><h3 id="health-title">{snapshot.wikiHealth.status === 'failed' ? t('home-wiki-failed') : snapshot.wikiHealth.errorCount ? t('knowledge-health-findings', { count: snapshot.wikiHealth.errorCount }) : snapshot.wikiHealth.warningCount ? t('knowledge-health-findings', { count: snapshot.wikiHealth.warningCount }) : t('knowledge-health-ready-title')}</h3><small>{snapshot.wikiHealth.updatingCount ? t('knowledge-health-updating-body') : t('knowledge-health-ready-body')}</small></div></div>
            <div class="row-actions">{#if snapshot.wikiHealth.attentionCollectionId}<button class="secondary" onclick={openAttentionCollection}>{t('action-open')}</button>{/if}<button class="text-action" onclick={refreshHealth} disabled={wikiHealthRequestId !== null}>{wikiHealthRequestId ? t('home-wiki-checking') : t('updates-check-now')}</button></div>
          </section>
        {/if}

        {#if destination === 'library' && snapshot?.wikiHealth?.attentionCollectionId}
          <section class="repair-panel" aria-labelledby="repair-title">
            <div class="settings-heading"><div><p class="section-label">{t('knowledge-repair-confirm-title')}</p><h3 id="repair-title">{t('knowledge-recovery-guided')}</h3></div><button class="secondary" onclick={() => prepareRepair(snapshot!.wikiHealth!.attentionCollectionId!)} disabled={guidedRepairRequestId !== null}>{guidedRepairRequestId ? t('knowledge-repair-working') : t('knowledge-repair-review-action')}</button></div>
            <p>{t('knowledge-repair-review-help')}</p>
            {#if snapshot.guidedRepair?.collectionId === snapshot.wikiHealth.attentionCollectionId}
              {#if snapshot.guidedRepair.status === 'prepared'}
                <div class="repair-preview">
                  <strong>{t('knowledge-repair-changes-title')} · {snapshot.guidedRepair.files.length}</strong>
                  <ul>{#each snapshot.guidedRepair.files as file, fileIndex (fileIndex)}<li><code>{file.page.kind}</code><span>{repairChangeLabel(file.change)}</span></li>{/each}</ul>
                  <label class="check"><input type="checkbox" bind:checked={guidedRepairConfirmed} /> {t('knowledge-repair-confirm-warning')}</label>
                  <button class="danger" onclick={() => executeRepair(snapshot!.guidedRepair!.collectionId)} disabled={!guidedRepairConfirmed || guidedRepairRequestId !== null}>{t('knowledge-repair-confirm-action')}</button>
                </div>
              {:else if snapshot.guidedRepair.status === 'completed'}
                <p class="verified-copy">{t('knowledge-repair-complete', { reviewed: snapshot.guidedRepair.conceptsReturnedToReview, orphans: snapshot.guidedRepair.orphanConceptsRemoved })}</p>
              {:else}
                <p class="evidence-warning">{t('knowledge-repair-error-stale')}</p>
              {/if}
            {/if}
          </section>
        {/if}

        {#if destination === 'library' && snapshot?.collections.length}
          <div class="records" aria-label={t('collections-title')}>
            {#each snapshot.collections as collection (collection.id)}
              <article><div><strong>{collection.name}</strong><small>{t('collections-counts', { documents: collection.documentCount, published: collection.publishedCount })}{#if collectionScanState(collection.id)} · {collectionScanState(collection.id) === 'queued' ? t('collections-scan-queued') : t('collections-scan-running')}{/if}</small></div><div class="row-actions"><button class="text-action" onclick={() => openKnowledge(collection.id)}>{t('action-open')}</button><button class="text-action" onclick={() => editCollection(collection)}>{t('action-configure')}</button><button class="text-action" onclick={() => scanCollection(collection.id)} disabled={collectionScanState(collection.id) !== null}>{collectionScanState(collection.id) ? t('status-working') : t('action-refresh')}</button></div></article>
            {/each}
          </div>
        {/if}

        {#if destination === 'library' && editingCollectionId}
          {@const activeCollection = snapshot?.collections.find((collection) => collection.id === editingCollectionId)}
          <section class="collection-settings" aria-labelledby="collection-settings-title">
            <div class="settings-heading"><div><p class="section-label">{t('collections-access-title')}</p><h3 id="collection-settings-title">{activeCollection?.name}</h3></div><button class="text-action" onclick={() => { editingCollectionId = null; relinkSelection = null; }}>{t('action-cancel')}</button></div>
            <div class="policy-state"><strong>{!collectionPolicy.peerShareable && !collectionPolicy.allowExternalAi && !collectionPolicy.internetPublic ? t('collections-local-only') : t('collections-access-title')}</strong><span>{t('integrations-permissions-reminder')}</span></div>
            <div class="policy-grid">
              <label class="check"><input type="checkbox" bind:checked={collectionPolicy.peerShareable} /> {t('collections-policy-peers')}</label>
              <label class="check"><input type="checkbox" bind:checked={collectionPolicy.allowExternalAi} /> {t('collections-policy-chat')}</label>
              <label class="check"><input type="checkbox" bind:checked={collectionPolicy.internetPublic} /> {t('collections-public-network')}</label>
            </div>
            <p class="guardrail">{t('collections-chat-confirm-body')}</p>
            <div class="collection-settings-actions"><button class="primary" onclick={saveCollectionPolicy} disabled={actionBusy}>{t('action-confirm')}</button><button class="secondary" onclick={chooseRelinkFolder}>{t('collections-relink')}</button>{#if relinkSelection}<span>{relinkSelection.displayPath}</span><button class="secondary" onclick={applyRelink} disabled={actionBusy}>{t('action-confirm')}</button>{/if}</div>
            {#if collectionPolicy.internetPublic}
              <div class="public-profile">
                <div><p class="section-label">{t('collections-public-description')}</p><p>{t('collections-public-confirm-body')}</p></div>
                <label><span>{t('collections-public-description')}</span><textarea bind:value={publicDescription} maxlength="2048" rows="3"></textarea></label>
                <label><span>{t('collections-public-languages')}</span><input bind:value={publicLanguages} maxlength="300" placeholder="es, en" /></label>
                <div class="row-actions"><button class="secondary" onclick={savePublicProfile} disabled={actionBusy}>{t('collections-public-profile-save')}</button>{#if activeCollection?.publicAnnouncement.status === 'advertised'}<span class="verified-copy">{t('collections-public-announcement-online', { indexes: activeCollection.publicAnnouncement.acceptedIndexes })}</span>{/if}</div>
              </div>
            {/if}
          </section>
        {/if}

        {#if destination === 'library' && snapshot?.sourceIssues.length}
          <section class="source-issues" aria-labelledby="source-issues-title">
            <div><AlertTriangle size={18} aria-hidden="true" /><div><h3 id="source-issues-title">{t('desktop-source-issues-title')}</h3><p>{t('desktop-source-issues-body')}</p></div></div>
            {#each snapshot.sourceIssues as issue (`${issue.collectionId}:${issue.sourceName}:${issue.code}`)}
              <article><strong>{issue.sourceName}</strong><span>{issue.collectionName}</span><code>{issue.code}</code></article>
            {/each}
          </section>
        {/if}

        {#if destination === 'library' && snapshot?.knowledge?.collectionId === selectedCollectionId}
          <div class="knowledge-workspace">
            <aside class="knowledge-tree" aria-label={t('knowledge-pages')}>
              <div><strong>{snapshot.knowledge.collectionName}</strong><small>{t('knowledge-concept-count', { count: snapshot.knowledge.concepts.length })}</small></div>
              <button onclick={() => openKnowledgePage({ kind: 'index' })}><BookOpen size={15} />{t('knowledge-index-title')}</button>
              <button onclick={() => openKnowledgePage({ kind: 'log' })}><History size={15} />{t('knowledge-recovery-history')}</button>
              <button class:active={knowledgeMode === 'graph'} onclick={() => { knowledgeMode = 'graph'; }}><Network size={15} />{t('knowledge-tab-graph')}</button>
              {#each snapshot.knowledge.concepts as concept (concept.title)}
                <button onclick={() => openKnowledgePage(concept.page)} title={concept.description}><FileText size={15} /><span>{concept.title}</span></button>
              {/each}
            </aside>
            <section class="knowledge-document" aria-live="polite">
              {#if knowledgeMode === 'graph' && snapshot.knowledge.status === 'ready'}
                {#key `${snapshot.knowledge.collectionId}:${snapshot.knowledge.version}`}
                  <KnowledgeGraph bundle={snapshot.knowledge} onselect={selectGraphPage} {locale} />
                {/key}
              {:else if snapshot.knowledge.status === 'updating'}
                <p class="loading"><RefreshCw size={16} /> {t('knowledge-updating-title')}</p>
              {:else if snapshot.knowledgePage?.collectionId === selectedCollectionId && snapshot.knowledgePage.status === 'ready'}
                <div class="document-heading"><p class="section-label">{t('desktop-verified-page')}</p><h3>{snapshot.knowledgePage.title}</h3></div>
                {#if snapshot.knowledgePage.truncated}<p class="evidence-warning">{t('knowledge-page-truncated')}</p>{/if}
                <div class="knowledge-blocks">
                  {#each snapshot.knowledgePage.blocks as block, blockIndex (blockIndex)}
                    {#if block.kind === 'heading'}<h4 class:minor={block.level > 2}>{block.text}</h4>
                    {:else if block.kind === 'paragraph'}<p>{block.text}</p>
                    {:else if block.kind === 'listItem'}<div class="safe-list-item"><span>{block.ordered ? '—' : '•'}</span><p>{block.text}</p></div>
                    {:else if block.kind === 'code'}<pre><code>{block.text}</code></pre>
                    {:else if block.kind === 'quote'}<blockquote>{block.text}</blockquote>
                    {:else}<hr />{/if}
                  {/each}
                </div>
              {:else if snapshot.knowledge.status === 'failed'}
                <p class="evidence-warning">{t('knowledge-bundle-error-title')}</p>
              {:else}
                <div class="review-placeholder"><BookOpen size={26} /><h3>{t('knowledge-select-page')}</h3><p>{t('desktop-verified-only')}</p></div>
              {/if}
            </section>
          </div>
        {/if}

        {#if destination === 'library'}
          <form class="action-panel" onsubmit={(event) => { event.preventDefault(); createCollection(); }}>
            <label><span>{t('collections-name')}</span><input bind:value={collectionName} maxlength="120" placeholder={t('desktop-collection-name-placeholder')} required /></label>
            <div><button type="button" class="secondary" onclick={chooseFolder}>{t('collections-choose-folder')}</button><small>{folderSelection?.displayPath ?? t('desktop-folder-privacy')}</small></div>
            <button class="primary" disabled={actionBusy || !folderSelection || !collectionName.trim()}>{t('desktop-add-library')}</button>
          </form>
        {/if}

        {#if destination === 'review' && snapshot}
          <div class="review-workspace">
            <aside class="review-queue" aria-label={t('desktop-review-queue')}>
              {#each snapshot.reviews as review (`${review.conceptId}:${review.sourceRevision}`)}
                <button class:active={selectedReview?.conceptId === review.conceptId} onclick={() => openReview(review)}>
                  <strong>{review.sourceName}</strong><small>{review.collectionName} · {t('desktop-review-revision', { revision: review.sourceRevision })}</small>
                </button>
              {:else}<p class="empty">{t('review-empty-body')}</p>{/each}
            </aside>
            {#if selectedReview && editDraft}
              <div class="review-flow">
                <section class="review-step evidence-step" aria-labelledby="evidence-title">
                  <div class="step-heading"><span>01</span><div><p>{t('review-focus-evidence')}</p><h3 id="evidence-title">{t('desktop-review-check-source')}</h3></div></div>
                  {#if snapshot.reviewEvidence?.conceptId === selectedReview.conceptId && snapshot.reviewEvidence.status === 'ready'}
                    <div class="excerpts">
                      {#each snapshot.reviewEvidence.excerpts as excerpt (excerpt.ordinal)}
                        <article><small>{excerpt.headingOrPage || t('review-evidence-untitled-section')}</small><p>{excerpt.text}</p></article>
                      {/each}
                    </div>
                    {#if snapshot.reviewEvidence.nextOrdinal != null}<button class="secondary" onclick={loadMoreEvidence} disabled={actionBusy}>{t('review-evidence-load-more')}</button>{/if}
                  {:else if snapshot.reviewEvidence?.conceptId === selectedReview.conceptId}
                    <p class="evidence-warning">{t('review-evidence-approval-blocked')}</p>
                  {:else}
                    <p class="loading"><RefreshCw size={16} /> {t('review-evidence-loading')}</p>
                  {/if}
                </section>
                <section class="review-step proposal-step" aria-labelledby="proposal-title">
                  <div class="step-heading"><span>02</span><div><p>{t('desktop-review-ai-proposal')}</p><h3 id="proposal-title">{t('desktop-review-edit')}</h3></div></div>
                  <label><span>{t('review-field-title')}</span><input bind:value={editDraft.title} maxlength="240" /></label>
                  <label><span>{t('review-field-description')}</span><textarea bind:value={editDraft.description} maxlength="1000" rows="3"></textarea></label>
                  <label><span>{t('review-field-summary')}</span><textarea bind:value={editDraft.summary} maxlength="4000" rows="6"></textarea></label>
                  <label><span>{t('review-field-tags')}</span><input value={editDraft.tags.join(', ')} onchange={(event) => { editDraft!.tags = event.currentTarget.value.split(',').map((tag) => tag.trim()).filter(Boolean); }} /></label>
                </section>
                <section class="review-step decision-step" aria-labelledby="decision-title">
                  <div class="step-heading"><span>03</span><div><p>{t('desktop-review-human-decision')}</p><h3 id="decision-title">{t('desktop-review-decide')}</h3></div></div>
                  <p>{t('desktop-review-decision-body')}</p>
                  <div class="decision-actions">
                    <button class="primary" onclick={() => decideReview('approve')} disabled={actionBusy || !evidenceIsCurrent()}>{t('desktop-review-approve')}</button>
                    <button class="secondary" onclick={() => decideReview('reanalyze')} disabled={actionBusy || !snapshot.model?.active || snapshot.reanalyzingReviewIds.includes(selectedReview.conceptId)}>{snapshot.reanalyzingReviewIds.includes(selectedReview.conceptId) ? t('review-analyzing') : t('review-reanalyze')}</button>
                    <button class="danger" onclick={() => decideReview('reject')} disabled={actionBusy}>{t('review-reject')}</button>
                  </div>
                  {#if !evidenceIsCurrent()}<small class="guardrail">{t('desktop-review-guardrail')}</small>{/if}
                </section>
              </div>
            {:else}
              <div class="review-placeholder"><CheckCircle2 size={26} /><h3>{t('desktop-review-select-title')}</h3><p>{t('desktop-review-select-body')}</p></div>
            {/if}
          </div>
        {/if}

        {#if destination === 'system' && snapshot}
          <nav class="system-subnav" aria-label={t('nav-group-system')}>
            {#each systemSections as section (section.id)}
              <a href={`#system/${section.id}`} aria-current={systemSection === section.id ? 'page' : undefined} onclick={(event) => openSystemSection(event, section.id)}>{t(section.labelId)}</a>
            {/each}
          </nav>
          <div class="system-layout">
            <section id="system-models"><p class="section-label">{t('settings-local-ai')}</p><h3>{snapshot.model?.displayName ?? t('models-profile-automatic')}</h3><p>{snapshot.model?.active ? t('models-ready') : t('models-pending')}</p>{#if snapshot.modelInstall}<progress max={snapshot.modelInstall.totalBytes || 1} value={snapshot.modelInstall.downloaded}></progress><small>{modelInstallLabel(locale)}</small><button class="secondary" onclick={cancelModelInstall}>{t('action-cancel')}</button>{:else if snapshot.model && !snapshot.model.active}<label class="check license-check"><input type="checkbox" bind:checked={modelLicensesConfirmed} /> {t('models-accept-licenses')} · {snapshot.model.license ?? t('models-license')}</label><div class="row-actions"><button class="secondary" onclick={prepareLocalModel} disabled={!modelLicensesConfirmed && !snapshot.model.licenseAccepted}>{t('models-action-download')}</button>{#if snapshot.model.licenseUrl}<button class="text-action" onclick={() => openVerifiedExternalLink(snapshot?.model?.licenseUrl ?? '')}>{t('models-license')}</button>{/if}</div>{/if}</section>
            <section id="system-preferences" class="settings-form"><p class="section-label">{t('desktop-preferences')}</p><label><span>{t('settings-language')}</span><select bind:value={locale}><option value="system">{t('language-system')}</option><option value="es">{t('language-spanish')}</option><option value="en">{t('language-english')}</option></select></label><label><span>{t('settings-theme')}</span><select bind:value={theme}><option value="system">{t('theme-system')}</option><option value="light">{t('theme-light')}</option><option value="dark">{t('theme-dark')}</option></select></label><label><span>{t('desktop-lan')}</span><select bind:value={lanPreference}><option value="disabled">{t('desktop-disabled')}</option><option value="enabled">{t('desktop-enabled')}</option></select></label><label><span>{t('desktop-close')}</span><select bind:value={closeBehavior}><option value="ask">{t('desktop-ask')}</option><option value="hide_to_tray">{t('desktop-hide-tray')}</option><option value="quit">{t('desktop-quit')}</option></select></label><label class="check"><input type="checkbox" bind:checked={automaticUpdateChecks} /> {t('updates-automatic')}</label><button class="primary" onclick={() => savePreferences(false)} disabled={actionBusy}>{t('desktop-save-preferences')}</button></section>
            <section id="system-updates" class="updater-section" aria-live="polite"><div class="settings-heading"><div><p class="section-label">{t('updates-title')}</p><h3>{t('desktop-stable-channel')}</h3></div>{#if snapshot.updater?.status !== 'disabled'}<button class="text-action" onclick={() => runUpdaterAction('check')} disabled={updaterRequestId !== null || snapshot.updater?.status === 'checking' || snapshot.updater?.status === 'downloading' || snapshot.updater?.status === 'installing'}>{t('updates-check-now')}</button>{/if}</div><p>{updaterLabel(locale)}</p>{#if snapshot.updater?.releaseNotes}<div class="release-notes"><small>{t('desktop-release-notes')}</small><p>{snapshot.updater.releaseNotes}</p></div>{/if}<div class="row-actions">{#if snapshot.updater?.status === 'available'}<button class="secondary" onclick={() => runUpdaterAction('download')} disabled={updaterRequestId !== null}>{t('desktop-update-download')}</button>{:else if snapshot.updater?.status === 'readyToInstall' && !confirmUpdateInstall}<button class="primary" onclick={() => { confirmUpdateInstall = true; }}>{t('updates-install')}</button>{:else if snapshot.updater?.status === 'readyToInstall' && confirmUpdateInstall}<div class="install-confirmation" role="alert"><p>{t('desktop-update-install-body', { version: snapshot.updater.version ?? '' })}</p><button class="primary" onclick={() => runUpdaterAction('install')} disabled={updaterRequestId !== null}>{t('updates-confirm-install')}</button><button class="secondary" onclick={() => { confirmUpdateInstall = false; }} disabled={updaterRequestId !== null}>{t('action-cancel')}</button></div>{:else if snapshot.updater?.retryable}<button class="secondary" onclick={() => runUpdaterAction('check')} disabled={updaterRequestId !== null}>{t('action-retry')}</button>{/if}</div><small>{t('desktop-update-privacy')}</small></section>
            <section><p class="section-label">{t('desktop-sign-in')}</p><h3>{t('desktop-autostart')}</h3><p>{autostartLabel(locale)}</p><div class="row-actions">{#if snapshot.autostart === 'enabled'}<button class="secondary" onclick={() => changeAutostart(false)} disabled={autostartBusy}>{t('action-disable')}</button>{:else if snapshot.autostart !== 'unsupported' && snapshot.autostart !== 'conflict'}<button class="secondary" onclick={() => changeAutostart(true)} disabled={autostartBusy}>{autostartBusy ? t('autostart-checking') : t('action-enable')}</button>{/if}<button class="text-action" onclick={refreshAutostartState} disabled={autostartBusy}>{t('settings-refresh-status')}</button></div></section>
            <section id="system-connectivity" class="connectivity-section"><p class="section-label">{t('desktop-connectivity')}</p><h3>{t('desktop-known-devices', { count: snapshot.peers.length })}</h3><p>{connectivityLabel(locale)}</p>{#if snapshot.lanRuntime}<dl><div><dt>{t('desktop-listener')}</dt><dd>{lanStateLabel(snapshot.lanRuntime.listener)}</dd></div><div><dt>{t('desktop-discovery')}</dt><dd>{lanStateLabel(snapshot.lanRuntime.discovery)}</dd></div><div><dt>{t('desktop-interfaces')}</dt><dd>{snapshot.lanRuntime.addressCount}</dd></div></dl>{/if}<div class="row-actions"><button class="secondary" onclick={() => runConnectivityAction('refresh')} disabled={connectivityRequestId !== null}>{connectivityRequestId ? t('updates-checking') : t('desktop-check')}</button>{#if snapshot.connectivity?.networkProfile === 'public'}<button class="text-action" onclick={() => runConnectivityAction('networkSettings')} disabled={connectivityRequestId !== null}>{t('desktop-network-settings')}</button>{/if}{#if snapshot.connectivity?.systemPermission === 'denied'}<button class="text-action" onclick={() => runConnectivityAction('localNetworkPrivacy')} disabled={connectivityRequestId !== null}>{t('desktop-local-network-permission')}</button>{/if}{#if snapshot.connectivity?.firewallHelper === 'verified' && snapshot.connectivity.firewall !== 'ready' && snapshot.connectivity.firewall !== 'notApplicable'}<button class="secondary" onclick={() => runConnectivityAction('install')} disabled={connectivityRequestId !== null || lanPreference !== 'enabled'}>{t('connectivity-configure-firewall')}</button>{/if}{#if snapshot.connectivity?.firewall === 'ready'}<button class="text-action" onclick={() => runConnectivityAction('remove')} disabled={connectivityRequestId !== null}>{t('desktop-firewall-remove')}</button>{/if}{#if snapshot.connectivity?.firewall === 'conflict' || snapshot.connectivity?.firewall === 'legacyExposure'}<button class="text-action" onclick={() => runConnectivityAction('advancedFirewall')} disabled={connectivityRequestId !== null}>{t('connectivity-open-advanced-firewall')}</button>{/if}</div><small>{t('desktop-sharing-guardrail')}</small></section>
            <section class="device-details"><p class="section-label">{t('desktop-this-device')}</p><h3>{t('desktop-identity-capacity')}</h3><dl>{#if snapshot.nodeId}<div><dt>{t('desktop-network-identity')}</dt><dd><code>{shortPeerId(snapshot.nodeId)}</code></dd></div>{/if}{#if snapshot.mcpUrl}<div><dt>{t('diagnostics-local-mcp')}</dt><dd><code>{snapshot.mcpUrl}</code></dd></div>{/if}{#if snapshot.hardware}<div><dt>{t('desktop-memory-installed')}</dt><dd>{formatBytes(snapshot.hardware.totalMemoryBytes)}</dd></div><div><dt>{t('desktop-disk-available')}</dt><dd>{formatBytes(snapshot.hardware.availableDiskBytes)}</dd></div><div><dt>{t('models-acceleration')}</dt><dd>{snapshot.hardware.metalAvailable ? 'Metal' : snapshot.hardware.avx2 ? 'AVX2' : t('desktop-basic-cpu')}</dd></div>{/if}</dl>{#if snapshot.hardware?.issues.length}<p class="evidence-warning">{snapshot.hardware.issues.join(' · ')}</p>{/if}</section>
            <section class="network-advanced"><p class="section-label">{t('devices-manual-advanced')}</p><h3>{t('desktop-manual-connect')}</h3><p>{t('desktop-manual-connect-body')}</p><label><span>{t('desktop-address')}</span><input bind:value={manualPeerAddress} maxlength="500" placeholder="/ip4/192.168.1.20/tcp/12345/p2p/12D3Koo…" /></label><button class="secondary" onclick={connectManualPeer} disabled={lanPreference !== 'enabled' || !manualPeerAddress.trim()}>{t('action-connect')}</button></section>
            <section id="system-devices" class="peer-trust"><p class="section-label">{t('desktop-devices-permissions')}</p><h3>{t('desktop-explicit-trust')}</h3><p>{t('desktop-explicit-trust-body')}</p><div class="peer-list">{#each snapshot.peers as peer (peer.peerId)}<article><div class="peer-heading"><div><strong>{peer.deviceName ?? t('devices-nearby')}</strong><code title={peer.peerId}>{shortPeerId(peer.peerId)}</code><small>{peer.address}</small></div><span class:verified={peer.trust === 'trusted'}>{peer.trust === 'trusted' ? t('desktop-verified') : peer.trust === 'blocked' ? t('desktop-revoked') : peer.activity === 'pairing' ? t('desktop-verifying') : t('desktop-unverified')}</span></div>{#if peer.sasWords}<div class="sas" aria-label={t('desktop-sas')}><small>{t('desktop-sas-help')}</small><strong>{peer.sasWords.join(' · ')}</strong><div><button class="primary" onclick={() => runPeerAction(peer.peerId, 'accept')} disabled={peerActionId === peer.peerId}>{t('devices-code-matches')}</button><button class="danger" onclick={() => runPeerAction(peer.peerId, 'reject')} disabled={peerActionId === peer.peerId}>{t('devices-code-does-not-match')}</button></div></div>{:else if peer.trust === 'unpaired'}<button class="secondary" onclick={() => runPeerAction(peer.peerId, 'pair')} disabled={peerActionId === peer.peerId || peer.activity === 'notObserved'}>{t('desktop-verify-device')}</button>{:else if peer.trust === 'trusted'}<div class="grant-list">{#each snapshot.collections.filter((collection) => collection.peerShareable) as collection (collection.id)}<label class="check"><input type="checkbox" checked={peer.grantedCollectionIds.includes(collection.id)} onchange={(event) => changeGrant(peer.peerId, collection.id, event.currentTarget.checked)} disabled={peerActionId === peer.peerId} /> {collection.name}</label>{:else}<small>{t('desktop-no-shareable-folders')}</small>{/each}</div><button class="danger" onclick={() => runPeerAction(peer.peerId, 'revoke')} disabled={peerActionId === peer.peerId}>{t('desktop-revoke-trust')}</button>{/if}</article>{:else}<p class="empty">{t('desktop-no-devices')}</p>{/each}</div></section>
            <section class="network-advanced"><p class="section-label">{t('desktop-public-federation')}</p><h3>{t('desktop-community-indexes')}</h3><p>{t('desktop-community-indexes-body')}</p><div class="network-fields"><label><span>{t('desktop-peer-id')}</span><input bind:value={federationPeerId} maxlength="200" /></label><label><span>{t('desktop-multiaddress')}</span><input bind:value={federationAddress} maxlength="500" /></label></div><div class="row-actions"><button class="secondary" onclick={() => saveFederationIndex(false)} disabled={!federationPeerId.trim() || !federationAddress.trim()}>{t('search-public-index-add')}</button><button class="text-action" onclick={() => saveFederationIndex(true)} disabled={!federationPeerId.trim()}>{t('search-public-index-remove')}</button></div>{#if snapshot.blockedPublicPublishers.length}<div class="blocked-publishers"><small>{t('desktop-blocked-publishers')}</small>{#each snapshot.blockedPublicPublishers as publisherId (publisherId)}<div><code>{shortPeerId(publisherId)}</code><button class="text-action" onclick={() => changePublisherBlock(publisherId, false)}>{t('search-public-unblock-publisher')}</button></div>{/each}</div>{/if}</section>
            <section id="system-integrations" class="integrations-section"><div class="settings-heading"><div><p class="section-label">{t('integrations-title')}</p><h3>{t('desktop-ai-clients')}</h3></div><button class="text-action" onclick={() => runIntegrationAction({ kind: 'refresh' })} disabled={integrationRequestId !== null}>{t('integrations-refresh')}</button></div><p>{t('desktop-integration-body')}</p>{#if snapshot.integrations?.externalAiCollectionCount}<p class="evidence-warning">{t('desktop-integration-warning', { count: snapshot.integrations.externalAiCollectionCount })}</p>{/if}<div class="integration-list">{#each snapshot.integrations?.integrations ?? [] as integration (integration.client)}<article><div><strong>{integrationName(integration.client)}</strong><small>{integrationState(integration.status)}{#if integration.detectedVersion} · {integration.detectedVersion}{/if}{#if integration.restartRequired} · {t('desktop-restart-required')}{/if}</small></div><div class="row-actions">{#if integration.status === 'available' || integration.status === 'updateAvailable'}<button class="secondary" onclick={() => runIntegrationAction({ kind: 'connect', client: integration.client })} disabled={integrationRequestId !== null}>{integration.status === 'updateAvailable' ? t('integrations-update') : t('integrations-connect')}</button>{:else if integration.status === 'configured'}<button class="danger" onclick={() => runIntegrationAction({ kind: 'disconnect', client: integration.client })} disabled={integrationRequestId !== null}>{t('integrations-disconnect')}</button>{:else if integration.status === 'awaitingClientApproval' && integration.client === 'claudeDesktop'}<button class="secondary" onclick={() => runIntegrationAction({ kind: 'openClaudeSettings' })} disabled={integrationRequestId !== null}>{t('integrations-open-claude')}</button><button class="text-action" onclick={() => runIntegrationAction({ kind: 'confirmClaudeInstalled' })} disabled={integrationRequestId !== null}>{t('integrations-installed')}</button>{/if}</div></article>{:else}<p class="empty">{t('desktop-no-integrations')}</p>{/each}</div></section>
          </div>
        {/if}

        {#if destination === 'search'}
          <form class="search-panel" onsubmit={(event) => { event.preventDefault(); submitSearch(); }}>
            <label for="knowledge-question">{t('desktop-search-question')}</label>
            <textarea id="knowledge-question" bind:value={question} maxlength="4096" rows="4" placeholder={t('desktop-search-placeholder')} required></textarea>
            <label class="check"><input type="checkbox" bind:checked={includePublic} /> {t('desktop-search-include-public')}</label>
            <button class="primary" disabled={actionBusy}>{t('desktop-search-evidence')}</button>
          </form>
          {#if snapshot?.search}
            <div class="search-results" aria-live="polite">
              <p class="section-label">{snapshot.search.status === 'searching' ? t('desktop-search-partial') : t('desktop-search-found')}</p>
              {#each snapshot.search.hits as hit (`${hit.nodeId}:${hit.collectionId}:${hit.conceptId}:${hit.rank}`)}
                <article><small>{hit.headingOrPage}</small><h3>{hit.title}</h3><p>{hit.snippet}</p><div class="citation-row"><code>{hit.logicalResourceUri}</code><span>{t('search-revision', { revision: hit.sourceRevision })} · {hit.sourceSha256.slice(0, 12)}…</span></div><button class="text-action" onclick={() => openSearchHit(hit)} disabled={publicBrowseRequestId !== null}>{hit.nodeId === snapshot.nodeId ? t('desktop-open-local-evidence') : t('desktop-open-source-collection')}</button></article>
              {:else}
                {#if snapshot.search.status === 'complete'}
                  <p class="empty">{t('search-empty-body')}</p>
                {/if}
              {/each}
            </div>
          {/if}
          {#if snapshot?.publicBrowse}
            <section class="public-browser" aria-live="polite" aria-labelledby="public-browser-title">
              <div class="settings-heading"><div><p class="section-label">{t('search-public-browse-title')}</p><h3 id="public-browser-title">{snapshot.publicBrowse.collectionName ?? t('desktop-public-origin-missing')}</h3></div>{#if snapshot.publicBrowse.publisherId}<button class="danger" onclick={() => changePublisherBlock(snapshot!.publicBrowse!.publisherId!, true)}>{t('search-public-block-publisher')}</button>{/if}</div>
              <p class:warning-copy={snapshot.publicBrowse.status === 'offline' || snapshot.publicBrowse.status === 'expired' || snapshot.publicBrowse.status === 'failed'}>{snapshot.publicBrowse.status === 'direct' ? t('desktop-public-direct') : snapshot.publicBrowse.status === 'relay' ? t('desktop-public-relay') : snapshot.publicBrowse.status === 'expired' ? t('desktop-public-expired') : snapshot.publicBrowse.status === 'offline' ? t('desktop-public-offline') : t('desktop-public-invalid')}</p>
              {#if snapshot.publicBrowse.description}<p>{snapshot.publicBrowse.description}</p>{/if}
              {#if snapshot.publicBrowse.languages.length}<small>{snapshot.publicBrowse.languages.join(' · ')}</small>{/if}
              <div class="public-concepts">{#each snapshot.publicBrowse.concepts as concept (`${concept.conceptId}:${concept.sourceRevision}`)}<article><small>{concept.conceptType} · {concept.language} · {t('search-revision', { revision: concept.sourceRevision })}</small><h4>{concept.title}</h4><p>{concept.summary}</p>{#if concept.tags.length}<span>{concept.tags.join(' · ')}</span>{/if}</article>{:else}<p class="empty">{snapshot.publicBrowse.status === 'failed' ? t('desktop-public-invalid-content') : t('desktop-public-empty')}</p>{/each}</div>
              {#if snapshot.publicBrowse.nextCursor}<button class="secondary" onclick={loadMorePublicConcepts} disabled={publicBrowseRequestId !== null}>{publicBrowseRequestId ? t('desktop-loading') : t('search-public-browse-more')}</button>{/if}
            </section>
          {/if}
        {/if}
        {#if actionMessage}<p class="action-message" aria-live="polite">{actionMessage}</p>{/if}
      </div>
    </section>
  </main>
</div>
{/if}
{#if closeChoiceRequired}
  <div class="modal-backdrop" role="presentation">
    <div class="close-dialog" role="dialog" aria-modal="true" aria-labelledby="close-title">
      <p class="section-label">{t('desktop-close-eyebrow')}</p><h2 id="close-title">{t('close-dialog-title')}</h2>
      <p>{t('desktop-hide-services')}</p>
      <div><button class="primary" onclick={() => applyCloseChoice('hide')}>{t('desktop-hide-tray')}</button><button class="danger" onclick={() => applyCloseChoice('quit')}>{t('desktop-quit')}</button><button class="secondary" onclick={() => applyCloseChoice('cancel')}>{t('action-cancel')}</button></div>
    </div>
  </div>
{/if}
