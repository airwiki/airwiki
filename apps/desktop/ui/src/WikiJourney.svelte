<script lang="ts">
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import Bot from '@lucide/svelte/icons/bot';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Share2 from '@lucide/svelte/icons/share-2';
  import { applicationClientFor, type AiClientIdentity } from './aiClientIdentity';
  import type { ApplicationAccessSummary, IntegrationClient, IntegrationSummary, WikiScanStatus, WikiSummary } from './api';
  import Spinner from './components/Spinner.svelte';
  import { focusChoiceWithoutScroll } from './focus';
  import type { MessageArgs } from './i18n';
  import { applicationCanAccessWiki, wikiExternalAccessBlocked, wikiProjectMemoryBlocked } from './wikiAccess';

  type Tone = 'ready' | 'working' | 'attention' | 'neutral' | 'public';
  type KnowledgeAction = 'review' | 'details' | 'repair';
  type AiDestination = {
    key: string;
    client: AiClientIdentity;
    name: string;
    status: string;
    tone: Tone;
  };

  export let wiki: WikiSummary;
  export let scanState: WikiScanStatus | null;
  export let reanalyzing = false;
  export let sourceIssueCount: number;
  export let peerAccessCount: number;
  export let repairAvailable: boolean;
  export let integrations: IntegrationSummary[];
  export let applications: ApplicationAccessSummary[];
  export let integrationsBusy: boolean;
  export let t: (id: string, args?: MessageArgs) => string;
  export let onreview: () => void;
  export let ondetails: () => void;
  export let onrepair: () => void;
  export let onaccess: () => void;
  export let onapps: () => void;

  $: aiDestinations = buildAiDestinations(wiki, applications, integrations, integrationsBusy);

  function projectBlocked(): boolean {
    return wikiProjectMemoryBlocked(wiki);
  }

  function knowledgeTone(): Tone {
    if (scanState || reanalyzing) return 'working';
    if (projectBlocked() || wiki.failedCount > 0 || wiki.maintenanceRequired || wiki.restrictions.length > 0 || sourceIssueCount > 0 || wiki.needsReviewCount > 0) return 'attention';
    return wiki.publishedCount > 0 ? 'ready' : 'neutral';
  }

  function knowledgeTitle(): string {
    if (scanState) return t('desktop-journey-knowledge-checking');
    if (reanalyzing) return t('desktop-journey-knowledge-reanalyzing');
    if (projectBlocked()) return t('desktop-journey-knowledge-project-blocked');
    if (wiki.restrictions.length > 0) return t('desktop-journey-knowledge-restricted');
    if (wiki.failedCount > 0 || sourceIssueCount > 0) return t('desktop-journey-knowledge-errors', { count: Math.max(wiki.failedCount, sourceIssueCount) });
    if (wiki.maintenanceRequired) return t('desktop-journey-knowledge-maintenance');
    if (wiki.needsReviewCount > 0) return t('desktop-journey-knowledge-review', { count: wiki.needsReviewCount });
    if (wiki.publishedCount > 0) return t('desktop-journey-knowledge-ready', { count: wiki.publishedCount });
    if (wiki.excludedCount > 0) return t('desktop-journey-knowledge-excluded', { count: wiki.excludedCount });
    return t('desktop-journey-knowledge-empty');
  }

  function knowledgeAction(): KnowledgeAction {
    if (wiki.maintenanceRequired && repairAvailable) return 'repair';
    if (wiki.needsReviewCount > 0 && wiki.restrictions.length === 0) return 'review';
    return 'details';
  }

  function runKnowledgeAction(event: MouseEvent) {
    focusChoiceWithoutScroll(event);
    const action = knowledgeAction();
    if (action === 'review') onreview();
    else if (action === 'repair') onrepair();
    else ondetails();
  }

  function knowledgeActionLabel(): string {
    const action = knowledgeAction();
    if (action === 'review') return t('desktop-attention-reviews-action');
    if (action === 'repair') return t('knowledge-repair-review-action');
    if (wiki.failedCount > 0 || sourceIssueCount > 0) return t('desktop-attention-files-action');
    return t('desktop-details');
  }

  function publicIsAdvertised(): boolean {
    return !wikiExternalAccessBlocked(wiki)
      && wiki.internetPublic
      && wiki.publicAnnouncement.status === 'advertised';
  }

  function lanStatus(): string {
    if (wikiExternalAccessBlocked(wiki) && wiki.peerShareable) return t('desktop-compact-exposure-unavailable');
    if (!wiki.peerShareable) return t('desktop-compact-exposure-off');
    if (peerAccessCount > 0) return t('desktop-compact-exposure-lan-count', { count: peerAccessCount });
    return t('desktop-compact-exposure-lan-enabled');
  }

  function internetStatus(): string {
    if (wikiExternalAccessBlocked(wiki) && wiki.internetPublic) return t('desktop-compact-exposure-unavailable');
    if (publicIsAdvertised()) return t('desktop-compact-exposure-public');
    if (wiki.internetPublic && wiki.publicAnnouncement.status === 'expired') return t('desktop-compact-exposure-expired');
    if (wiki.internetPublic) return t('desktop-compact-exposure-enabled-offline');
    return t('desktop-compact-exposure-off');
  }

  function internetTone(): Tone {
    if (publicIsAdvertised()) return 'public';
    if (wikiExternalAccessBlocked(wiki) && wiki.internetPublic) return 'attention';
    return wiki.internetPublic ? 'attention' : 'neutral';
  }

  function integrationName(client: IntegrationClient): string {
    if (client === 'chatGptDesktop') return 'ChatGPT';
    if (client === 'claudeDesktop') return 'Claude Desktop';
    if (client === 'claudeCode') return 'Claude Code';
    if (client === 'geminiCli') return 'Gemini CLI';
    return t('integrations-generic-mcp');
  }

  function integrationTone(integration: IntegrationSummary, currentWiki: WikiSummary, busy: boolean, accessAllowed: boolean): Tone {
    if (busy || integration.status === 'awaitingClientApproval' || integration.status === 'updateAvailable') return 'working';
    if (integration.status === 'conflict' || integration.status === 'error') return 'attention';
    if (integration.status === 'configured' && (wikiExternalAccessBlocked(currentWiki) || !accessAllowed)) return 'attention';
    if (integration.status === 'configured' && accessAllowed) return 'ready';
    return 'neutral';
  }

  function integrationStatus(integration: IntegrationSummary, currentWiki: WikiSummary, busy: boolean, accessAllowed: boolean): string {
    if (busy) return t('status-working');
    if (integration.status === 'configured') {
      if (wikiExternalAccessBlocked(currentWiki) || !accessAllowed) return t('desktop-compact-ai-client-blocked');
      return integration.activityRecent ? t('desktop-journey-verified') : t('desktop-compact-ai-client-access');
    }
    const statusKey = integration.status === 'awaitingClientApproval'
      ? 'awaiting-approval'
      : integration.status === 'updateAvailable'
        ? 'update-available'
        : integration.status === 'notInstalled'
          ? 'not-installed'
          : integration.status;
    return t(`integration-status-${statusKey}`);
  }

  function buildAiDestinations(
    currentWiki: WikiSummary,
    currentApplications: ApplicationAccessSummary[],
    currentIntegrations: IntegrationSummary[],
    busy: boolean
  ): AiDestination[] {
    const destinations: AiDestination[] = [];
    const representedClients: AiClientIdentity[] = [];

    for (const application of currentApplications) {
      if (!application.active) continue;
      const client = applicationClientFor(application);
      const accessAllowed = applicationCanAccessWiki(application, currentWiki);
      destinations.push({
        key: `application:${application.appId}`,
        client,
        name: application.displayName,
        status: t(accessAllowed ? 'desktop-compact-ai-client-access' : 'desktop-compact-ai-client-blocked'),
        tone: accessAllowed ? 'ready' : 'attention'
      });
      if (client !== 'genericMcp') representedClients.push(client);
    }

    for (const integration of currentIntegrations) {
      if (integration.status === 'notInstalled' || integration.status === 'unsupported' || representedClients.includes(integration.client)) continue;
      destinations.push({
        key: `integration:${integration.client}`,
        client: integration.client,
        name: integrationName(integration.client),
        status: integrationStatus(integration, currentWiki, busy, false),
        tone: integrationTone(integration, currentWiki, busy, false)
      });
    }
    return destinations;
  }

  function aiSummary(): string {
    if (integrationsBusy) return t('desktop-compact-ai-connecting');
    if (wikiExternalAccessBlocked(wiki) && aiDestinations.length > 0) return t('desktop-compact-ai-blocked');
    const accessible = aiDestinations.filter((destination) => destination.tone === 'ready').length;
    if (accessible > 0) return t('desktop-compact-ai-access-count', { count: accessible });
    if (aiDestinations.some((destination) => destination.tone === 'attention')) return t('desktop-compact-ai-attention');
    if (integrations.some((integration) => integration.status === 'configured') && !wiki.allowExternalAi) return t('desktop-compact-ai-no-wiki-access');
    if (aiDestinations.length > 0) return t('desktop-compact-ai-available');
    return t('desktop-compact-ai-none');
  }
</script>

<section class="wiki-journey-compact wiki-context-actions" aria-label={t('desktop-journey-compact-label', { wiki: wiki.name })}>
  {#if knowledgeTone() !== 'ready'}
    <button class={`journey-notice ${knowledgeTone()}`} aria-label={`${knowledgeTitle()}. ${knowledgeActionLabel()}`} title={knowledgeActionLabel()} onclick={runKnowledgeAction}>
      {#if knowledgeTone() === 'working'}<Spinner size="small" />{:else}<AlertTriangle size={15} aria-hidden="true" />{/if}
      <span>{knowledgeTitle()}</span><ChevronRight size={14} aria-hidden="true" />
    </button>
  {/if}
  <div class="wiki-permission-actions">
    <button class="journey-compact-share" aria-label={t('desktop-share-action')} aria-describedby={`share-state-${wiki.id}`} onclick={(event) => { focusChoiceWithoutScroll(event); onaccess(); }} disabled={wiki.restrictions.length > 0}>
      <Share2 size={15} aria-hidden="true" /><span>{t('desktop-share-action')}</span>
      <small class:attention={internetTone() === 'attention' || (wiki.peerShareable && wikiExternalAccessBlocked(wiki))}>{wiki.internetPublic ? internetStatus() : wiki.peerShareable ? `LAN · ${lanStatus()}` : t('reader-access-private')}</small>
    </button>
    <span class="sr-only" id={`share-state-${wiki.id}`}><span aria-label={`${t('desktop-compact-exposure-lan')}: ${lanStatus()}`}>LAN: {lanStatus()}.</span> <span aria-label={`${t('desktop-compact-exposure-internet')}: ${internetStatus()}`}>Internet: {internetStatus()}.</span></span>
    <button class="journey-compact-ai" aria-label={`${t('desktop-compact-ai-manage')}. ${aiSummary()}`} aria-describedby={`ai-state-${wiki.id}`} onclick={(event) => { focusChoiceWithoutScroll(event); onapps(); }}>
      <Bot size={15} aria-hidden="true" /><span>{t('desktop-status-ai-apps')}</span><small>{aiSummary()}</small>
      <span class="sr-only" id={`ai-state-${wiki.id}`}>{#each aiDestinations as destination (destination.key)}{destination.name}: {destination.status}. {/each}</span>
    </button>
  </div>
</section>

<style>
  .wiki-context-actions { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 8px 16px; min-width: 0; width: auto; }
  .wiki-context-actions button { display: inline-flex; align-items: center; gap: 7px; min-height: 32px; padding: 6px 8px; color: var(--muted); background: transparent; border: 1px solid transparent; border-radius: var(--control-radius); font: 500 12px/1.35 var(--font-ui); cursor: pointer; }
  .wiki-context-actions button:hover { color: var(--strong); background: var(--surface-raised); }
  .wiki-context-actions button:disabled { cursor: default; opacity: .7; }
  .wiki-context-actions small { font: 400 11px/1.35 var(--font-ui); }
  .wiki-context-actions .journey-notice { margin-right: auto; text-align: left; }
  .wiki-context-actions .attention { color: var(--amber); }
  .wiki-context-actions .working { color: var(--violet); }
  .wiki-permission-actions { display: flex; flex-wrap: wrap; gap: 4px 8px; }
  @media (max-width: 1180px) { .wiki-context-actions .journey-compact-ai small { display: none; } }
</style>
