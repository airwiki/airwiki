<script lang="ts">
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import BookOpen from '@lucide/svelte/icons/book-open';
  import Bot from '@lucide/svelte/icons/bot';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Globe2 from '@lucide/svelte/icons/globe-2';
  import Laptop from '@lucide/svelte/icons/laptop';
  import RadioTower from '@lucide/svelte/icons/radio-tower';
  import Share2 from '@lucide/svelte/icons/share-2';
  import { applicationClientFor, type AiClientIdentity } from './aiClientIdentity';
  import type { ApplicationAccessSummary, IntegrationClient, IntegrationSummary, WikiScanStatus, WikiSummary } from './api';
  import AiClientIcon from './components/identity/AiClientIcon.svelte';
  import Spinner from './components/Spinner.svelte';
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

  function knowledgeStatus(): string {
    const tone = knowledgeTone();
    if (tone === 'working') return t('status-working');
    if (tone === 'attention') return t('status-needs-attention');
    if (tone === 'ready') return t('desktop-journey-searchable');
    return t('desktop-journey-not-ready');
  }

  function knowledgeAction(): KnowledgeAction {
    if (wiki.maintenanceRequired && repairAvailable) return 'repair';
    if (wiki.needsReviewCount > 0 && wiki.restrictions.length === 0) return 'review';
    return 'details';
  }

  function runKnowledgeAction() {
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

<section class="wiki-journey-compact" aria-label={t('desktop-journey-compact-label', { wiki: wiki.name })}>
  <button
    class="journey-compact-identity"
    aria-label={`${knowledgeTitle()}. ${knowledgeActionLabel()}`}
    title={knowledgeActionLabel()}
    onclick={runKnowledgeAction}
  >
    <span class={`journey-compact-icon ${knowledgeTone()}`} aria-hidden="true">
      {#if knowledgeTone() === 'working'}<Spinner size="small" />{:else if knowledgeTone() === 'attention'}<AlertTriangle size={15} />{:else}<BookOpen size={15} />{/if}
    </span>
    <span class="journey-compact-identity-copy">
      <strong>{wiki.name}</strong>
      <small><em class={knowledgeTone()}>{knowledgeStatus()}</em><span aria-hidden="true"> · </span>{t('desktop-wiki-review-progress', { reviewed: wiki.publishedCount, total: wiki.publishedCount + wiki.needsReviewCount + wiki.excludedCount, pending: wiki.needsReviewCount, excluded: wiki.excludedCount })}</small>
    </span>
    <ChevronRight size={14} aria-hidden="true" />
  </button>

  <div class="journey-compact-exposure" aria-label={t('desktop-compact-exposure-label')}>
    <small>{t('desktop-compact-exposure-label')}</small>
    <ol class="exposure-route">
      <li class="ready" aria-label={`${t('desktop-compact-exposure-local')}: ${t('desktop-compact-exposure-active')}`}>
        <span class="exposure-node" aria-hidden="true"><Laptop size={12} /></span>
        <strong>{t('desktop-compact-exposure-local')}</strong>
        <em>{t('desktop-compact-exposure-active')}</em>
      </li>
      <li class:ready={wiki.peerShareable && !wikiExternalAccessBlocked(wiki)} class:attention={wiki.peerShareable && wikiExternalAccessBlocked(wiki)} class:neutral={!wiki.peerShareable} aria-label={`${t('desktop-compact-exposure-lan')}: ${lanStatus()}`}>
        <span class="exposure-node" aria-hidden="true"><RadioTower size={12} /></span>
        <strong>{t('desktop-compact-exposure-lan')}</strong>
        <em>{lanStatus()}</em>
      </li>
      <li class={internetTone()} aria-label={`${t('desktop-compact-exposure-internet')}: ${internetStatus()}`}>
        <span class="exposure-node" aria-hidden="true"><Globe2 size={12} /></span>
        <strong>{t('desktop-compact-exposure-internet')}</strong>
        <em>{internetStatus()}</em>
      </li>
    </ol>
  </div>

  <button class="journey-compact-ai" aria-label={`${t('desktop-compact-ai-manage')}. ${aiSummary()}`} title={t('desktop-compact-ai-manage')} onclick={onapps}>
    <span class="journey-compact-ai-icons" aria-hidden="true">
        {#each aiDestinations.slice(0, 3) as destination (destination.key)}
        <span class={`compact-ai-client ${destination.tone}`} title={`${destination.name}: ${destination.status}`}>
          <AiClientIcon client={destination.client} label={destination.name} size={25} decorative />
          <span class="compact-ai-client-dot"></span>
        </span>
      {:else}
        <span class="compact-ai-empty"><Bot size={16} /></span>
      {/each}
      {#if aiDestinations.length > 3}<span class="compact-ai-overflow">+{aiDestinations.length - 3}</span>{/if}
    </span>
    <span class="journey-compact-ai-copy"><small>{t('desktop-status-ai-apps')}</small><strong>{aiSummary()}</strong></span>
    <span class="sr-only">{#each aiDestinations as destination (destination.key)}{destination.name}: {destination.status}. {/each}</span>
  </button>

  {#if wiki.restrictions.length === 0}
    <button class="secondary journey-compact-share" onclick={onaccess}><Share2 size={15} aria-hidden="true" />{t('desktop-share-action')}</button>
  {/if}
</section>
