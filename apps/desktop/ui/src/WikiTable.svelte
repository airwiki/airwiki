<script lang="ts">
  import BookOpen from '@lucide/svelte/icons/book-open';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ShimmerText from './components/ShimmerText.svelte';
  import Spinner from './components/Spinner.svelte';
  import type { ApplicationAccessSummary, PeerSummary, WikiSummary, WikiScanSummary } from './api';
  import type { MessageArgs } from './i18n';
  import { applicationCanAccessWiki, wikiExternalAccessBlocked, wikiHasApplicationAccess, wikiHasLanAccess, wikiHasPublicAccess, wikiIsPrivate } from './wikiAccess';
  import { wikiRequiresAttention } from './wikiHealth';

  export let wikis: WikiSummary[];
  export let scans: WikiScanSummary[];
  export let sourceIssueCounts: Record<string, number>;
  export let applications: ApplicationAccessSummary[];
  export let peers: PeerSummary[];
  export let t: (id: string, args?: MessageArgs) => string;
  export let onopen: (wikiId: string) => void;
  export let oncreate: () => void;

  function scanState(wikiId: string) {
    return scans.find((scan) => scan.wikiId === wikiId)?.state ?? null;
  }

  function accessSummary(wiki: WikiSummary): string {
    if (wikiIsPrivate(wiki, peers)) return t('desktop-wiki-private');
    if (wikiHasPublicAccess(wiki)) return t('desktop-journey-public-on-internet');
    return t('desktop-journey-not-public');
  }

  function accessDetail(wiki: WikiSummary): string {
    if (wikiExternalAccessBlocked(wiki)) return t('desktop-wiki-no-external-access');
    const accessibleChannels = [
      wikiHasLanAccess(wiki, peers) ? t('desktop-share-nearby') : '',
      wikiHasPublicAccess(wiki) ? t('desktop-share-public') : ''
    ].filter(Boolean);
    if (accessibleChannels.length) return accessibleChannels.join(' · ');
    const configuredChannels = [
      wiki.peerShareable ? `${t('desktop-share-nearby')}: ${t('desktop-compact-exposure-lan-enabled')}` : '',
      wiki.internetPublic ? t('desktop-journey-public-enabled') : ''
    ].filter(Boolean);
    return configuredChannels.length
      ? configuredChannels.join(' · ')
      : t('desktop-wiki-no-external-access');
  }

  function originLabel(wiki: WikiSummary): string {
    if (wiki.origin === 'importedOkf') return t('desktop-wiki-origin-imported');
    if (wiki.memoryKind === 'project') return t('desktop-wiki-origin-project-memory');
    if (wiki.origin === 'aiMemory') return t('desktop-wiki-origin-personal-memory');
    return wiki.indexingMode === 'continuous'
      ? t('desktop-wiki-origin-folder-continuous')
      : t('desktop-wiki-origin-folder-manual');
  }

  function rowRequiresAttention(wiki: WikiSummary): boolean {
    return wikiRequiresAttention(wiki) || (sourceIssueCounts[wiki.id] ?? 0) > 0;
  }

  function publicIsAdvertised(wiki: WikiSummary): boolean {
    return wiki.internetPublic && wiki.publicAnnouncement.status === 'advertised';
  }

  function lanStatus(wiki: WikiSummary): string {
    if (wikiExternalAccessBlocked(wiki) && wiki.peerShareable) return t('desktop-compact-exposure-unavailable');
    return t(wiki.peerShareable ? 'desktop-compact-exposure-lan-enabled' : 'desktop-compact-exposure-off');
  }

  function internetStatus(wiki: WikiSummary): string {
    if (wikiExternalAccessBlocked(wiki) && wiki.internetPublic) return t('desktop-compact-exposure-unavailable');
    if (publicIsAdvertised(wiki)) return t('desktop-compact-exposure-public');
    if (wiki.internetPublic && wiki.publicAnnouncement.status === 'expired') return t('desktop-compact-exposure-expired');
    if (wiki.internetPublic) return t('desktop-compact-exposure-enabled-offline');
    return t('desktop-compact-exposure-off');
  }

  function statusLabel(wiki: WikiSummary): string {
    if (scanState(wiki.id)) return t('desktop-wiki-updating-status');
    if (wiki.memoryKind === 'project' && wiki.projectMemoryHealth !== 'active') return t('desktop-home-attention');
    if (wiki.failedCount > 0 || wiki.maintenanceRequired) return t('status-needs-attention');
    if (wiki.needsReviewCount > 0) return t('desktop-wiki-pending-status');
    if (wiki.excludedCount > 0 && wiki.publishedCount === 0) return t('desktop-wiki-excluded-status');
    if (wiki.documentCount > 0 && wiki.publishedCount === 0) return t('desktop-wiki-indexing-check-status');
    if (wiki.documentCount === 0 && wiki.publishedCount === 0) return t('desktop-wiki-empty-status');
    if (rowRequiresAttention(wiki)) return t('status-needs-attention');
    return t('desktop-wiki-ready-status');
  }

  function statusDetail(wiki: WikiSummary): string {
    if (scanState(wiki.id)) return t('desktop-wiki-row-checking');
    if (wiki.memoryKind === 'project' && wiki.projectMemoryHealth !== 'active') {
      return t('desktop-wiki-row-project-blocked');
    }
    if (wiki.failedCount > 0 || (sourceIssueCounts[wiki.id] ?? 0) > 0) return t('desktop-wiki-row-failed-count', { count: Math.max(wiki.failedCount, sourceIssueCounts[wiki.id] ?? 0) });
    if (wiki.maintenanceRequired) return t('desktop-wiki-maintenance-required');
    if (wiki.needsReviewCount > 0) return t('desktop-wiki-row-review-count', { count: wiki.needsReviewCount });
    if (wiki.excludedCount > 0 && wiki.publishedCount === 0) return t('desktop-wiki-row-excluded-count', { count: wiki.excludedCount });
    if (wiki.documentCount > 0 && wiki.publishedCount === 0) return t('desktop-wiki-row-indexing-check');
    if (wiki.documentCount === 0 && wiki.publishedCount === 0) return t('desktop-wiki-row-empty');
    if (wiki.staleConceptCount > 0 || wiki.outdatedVerificationCount > 0 || wiki.metadataWarningCount > 0) {
      return t('desktop-okf-status-summary', {
        stale: wiki.staleConceptCount,
        outdated: wiki.outdatedVerificationCount,
        warnings: wiki.metadataWarningCount
      });
    }
    if (wiki.okfCompatibility.kind !== 'declaredV02') return t(`desktop-okf-compatibility-${wiki.okfCompatibility.kind}`);
    return t('desktop-wiki-row-ready');
  }

  function statusTone(wiki: WikiSummary): 'working' | 'attention' | 'ready' | 'neutral' {
    if (scanState(wiki.id)) return 'working';
    if (rowRequiresAttention(wiki)) return 'attention';
    if (wiki.publishedCount > 0) return 'ready';
    return 'neutral';
  }

  function rowLabel(wiki: WikiSummary): string {
    const identity = `${wiki.name} ${t('desktop-wiki-review-progress', { reviewed: wiki.publishedCount, total: wiki.publishedCount + wiki.needsReviewCount + wiki.excludedCount, pending: wiki.needsReviewCount, excluded: wiki.excludedCount })}`;
    return [
      identity,
      accessSummary(wiki),
      accessDetail(wiki),
      wikiHasApplicationAccess(wiki, applications)
        ? t('desktop-compact-ai-access-count', { count: applications.filter((application) => applicationCanAccessWiki(application, wiki)).length })
        : t('desktop-compact-ai-no-wiki-access'),
      statusLabel(wiki),
      statusDetail(wiki),
      originLabel(wiki),
    ].join(' · ');
  }
</script>

<div class="wiki-library-shelf">
  <div class="wiki-table-list" role="list" aria-label={t('desktop-wiki-list-title')}>
    {#each wikis as wiki (wiki.id)}
      {@const scanning = scanState(wiki.id) !== null}
      <div class="wiki-row-item" role="listitem">
        <button class={`wiki-row status-${statusTone(wiki)}`} aria-busy={scanning} aria-label={rowLabel(wiki)} onclick={() => onopen(wiki.id)}>
          <span class="wiki-name">
            <span class="wiki-icon"><BookOpen size={17} aria-hidden="true" /></span>
            <span><strong>{wiki.name}</strong><small>{originLabel(wiki)}</small></span>
          </span>

          <span class="wiki-row-knowledge" aria-hidden="true">
            <small class="wiki-row-kicker">{t('desktop-wiki-column-content')}</small>
            <span class="wiki-row-summary">
              <strong>{t('desktop-wiki-review-progress', { reviewed: wiki.publishedCount, total: wiki.publishedCount + wiki.needsReviewCount + wiki.excludedCount, pending: wiki.needsReviewCount, excluded: wiki.excludedCount })}</strong>
              <small>{t('desktop-wiki-detected-count', { count: wiki.documentCount })}</small>
            </span>
          </span>

          <span class="wiki-row-exposure" aria-hidden="true">
            <small class="wiki-row-kicker">{t('desktop-compact-exposure-label')}</small>
            <span class="wiki-row-exposure-text">
              <span class="active">{t('desktop-compact-exposure-local')}</span>
              <span class:active={wiki.peerShareable && !wikiExternalAccessBlocked(wiki)} class:attention={wiki.peerShareable && wikiExternalAccessBlocked(wiki)}>{t('desktop-compact-exposure-lan')} {lanStatus(wiki)}</span>
              <span class:active={publicIsAdvertised(wiki) && !wikiExternalAccessBlocked(wiki)} class:attention={wiki.internetPublic && (!publicIsAdvertised(wiki) || wikiExternalAccessBlocked(wiki))}>{t('desktop-compact-exposure-internet')} {internetStatus(wiki)}</span>
            </span>
          </span>

          <span class={`wiki-row-status ${statusTone(wiki)}`}>
            <span>{#if scanning}<Spinner size="small" /><ShimmerText text={statusLabel(wiki)} />{:else}<i class="wiki-status-signal" aria-hidden="true"></i><strong>{statusLabel(wiki)}</strong>{/if}</span>
            <small title={statusDetail(wiki)}>{statusDetail(wiki)}</small>
          </span>

          <span class="wiki-row-open"><ChevronRight size={16} aria-hidden="true" /></span>
        </button>
      </div>
    {:else}
      <div class="table-empty"><BookOpen size={28} aria-hidden="true" /><strong>{t('desktop-wiki-empty-title')}</strong><p>{t('desktop-wiki-empty-body')}</p><button class="primary" onclick={oncreate}>{t('desktop-wiki-empty-action')}</button></div>
    {/each}
  </div>
</div>
