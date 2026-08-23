<script lang="ts">
  import BookOpen from '@lucide/svelte/icons/book-open';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ShimmerText from './components/ShimmerText.svelte';
  import Spinner from './components/Spinner.svelte';
  import type { WikiSummary, WikiScanSummary } from './api';
  import type { MessageArgs } from './i18n';

  export let wikis: WikiSummary[];
  export let scans: WikiScanSummary[];
  export let t: (id: string, args?: MessageArgs) => string;
  export let onopen: (wikiId: string) => void;

  function scanState(wikiId: string) {
    return scans.find((scan) => scan.wikiId === wikiId)?.state ?? null;
  }

  function accessLabel(wiki: WikiSummary): string {
    const channels = [wiki.peerShareable ? t('desktop-share-nearby') : '', wiki.allowExternalAi ? t('desktop-share-ai-apps') : '', wiki.internetPublic ? t('desktop-share-public') : ''].filter(Boolean);
    return channels.length ? channels.join(' · ') : t('desktop-wiki-private');
  }

  function originLabel(wiki: WikiSummary): string {
    if (wiki.origin === 'importedOkf') return t('desktop-wiki-origin-imported');
    if (wiki.origin === 'aiMemory') return t('desktop-wiki-origin-memory');
    return wiki.indexingMode === 'continuous'
      ? t('desktop-wiki-origin-folder-continuous')
      : t('desktop-wiki-origin-folder-manual');
  }

  function trustLabel(wiki: WikiSummary): string {
    if (wiki.okfCompatibility.kind === 'legacyV01') return t('desktop-okf-compatibility-legacyV01');
    if (wiki.okfCompatibility.kind === 'futureRestricted') return t('desktop-okf-local-only');
    if (wiki.outdatedVerificationCount > 0) return t('desktop-assurance-outdated');
    if (wiki.staleConceptCount > 0) return t('desktop-freshness-stale');
    if (wiki.metadataWarningCount > 0) return t('desktop-okf-metadata-warning');
    return t(`desktop-trust-${wiki.trustSummary}`);
  }

  function statusLabel(wiki: WikiSummary): string {
    if (scanState(wiki.id)) return t('status-working');
    if (wiki.failedCount > 0 || wiki.maintenanceRequired) return t('status-needs-attention');
    if (wiki.needsReviewCount > 0) return t('desktop-wiki-pending-status');
    return trustLabel(wiki);
  }

  function rowLabel(wiki: WikiSummary): string {
    const identity = `${wiki.name} ${t('desktop-wiki-content-count', { published: wiki.publishedCount, pending: wiki.needsReviewCount })}`;
    return [
      identity,
      accessLabel(wiki),
      statusLabel(wiki),
      originLabel(wiki),
    ].join(' · ');
  }
</script>

<div class="wiki-table">
  <div class="wiki-table-head" aria-hidden="true">
    <span>{t('desktop-wiki-column-name')}</span>
    <span>{t('desktop-wiki-column-content')}</span>
    <span>{t('desktop-wiki-column-access')}</span>
    <span>{t('desktop-wiki-column-status')}</span>
    <span aria-hidden="true"></span>
  </div>
  <div class="wiki-table-list" role="list" aria-label={t('desktop-nav-wikis')}>
    {#each wikis as wiki (wiki.id)}
      {@const scanning = scanState(wiki.id) !== null}
      <div class="wiki-row-item" role="listitem">
        <button class="wiki-row" aria-busy={scanning} aria-label={rowLabel(wiki)} onclick={() => onopen(wiki.id)}>
          <span class="wiki-name"><span class="wiki-icon"><BookOpen size={18} aria-hidden="true" /></span><span><strong>{wiki.name}</strong><small>{originLabel(wiki)}</small></span></span>
          <span>{t('desktop-wiki-content-count', { published: wiki.publishedCount, pending: wiki.needsReviewCount })}</span>
          <span>{accessLabel(wiki)}</span>
          <span class:attention={!scanning && (wiki.failedCount > 0 || wiki.maintenanceRequired || wiki.needsReviewCount > 0 || wiki.staleConceptCount > 0 || wiki.outdatedVerificationCount > 0 || wiki.metadataWarningCount > 0 || wiki.okfCompatibility.kind === 'legacyV01' || wiki.okfCompatibility.kind === 'futureRestricted')} class:working={scanning} class="wiki-status">{#if scanning}<Spinner size="small" /><ShimmerText text={statusLabel(wiki)} />{:else}{statusLabel(wiki)}{/if}</span>
          <ChevronRight size={17} aria-hidden="true" />
        </button>
      </div>
    {:else}
      <div class="table-empty"><BookOpen size={28} aria-hidden="true" /><strong>{t('desktop-wiki-empty-title')}</strong><p>{t('desktop-wiki-empty-body')}</p></div>
    {/each}
  </div>
</div>
