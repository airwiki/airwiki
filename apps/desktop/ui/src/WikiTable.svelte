<script lang="ts">
  import BookOpen from '@lucide/svelte/icons/book-open';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
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
    if (wiki.okfCompatibility.kind === 'futureRestricted') return t('desktop-okf-local-only');
    if (wiki.outdatedVerificationCount > 0) return t('desktop-assurance-outdated');
    if (wiki.staleConceptCount > 0) return t('desktop-freshness-stale');
    if (wiki.metadataWarningCount > 0) return t('desktop-okf-metadata-warning');
    return t(`desktop-trust-${wiki.trustSummary}`);
  }

  function rowLabel(wiki: WikiSummary): string {
    const identity = `${wiki.name} ${t('desktop-wiki-content-count', { published: wiki.publishedCount, pending: wiki.needsReviewCount })}`;
    return [
      identity,
      accessLabel(wiki),
      trustLabel(wiki),
      originLabel(wiki),
    ].join(' · ');
  }
</script>

<div class="wiki-table" role="table" aria-label={t('desktop-nav-wikis')}>
  <div class="wiki-table-head" role="row">
    <span role="columnheader">{t('desktop-wiki-column-name')}</span>
    <span role="columnheader">{t('desktop-wiki-column-content')}</span>
    <span role="columnheader">{t('desktop-wiki-column-access')}</span>
    <span role="columnheader">{t('desktop-wiki-column-status')}</span>
    <span aria-hidden="true"></span>
  </div>
  {#each wikis as wiki (wiki.id)}
    <button class="wiki-row" role="row" aria-label={rowLabel(wiki)} onclick={() => onopen(wiki.id)}>
      <span class="wiki-name" role="cell"><span class="wiki-icon"><BookOpen size={18} aria-hidden="true" /></span><span><strong>{wiki.name}</strong><small>{originLabel(wiki)}</small></span></span>
      <span role="cell">{t('desktop-wiki-content-count', { published: wiki.publishedCount, pending: wiki.needsReviewCount })}</span>
      <span role="cell">{accessLabel(wiki)}</span>
      <span role="cell" class:attention={wiki.failedCount > 0 || wiki.maintenanceRequired || wiki.needsReviewCount > 0 || wiki.staleConceptCount > 0 || wiki.outdatedVerificationCount > 0 || wiki.metadataWarningCount > 0 || wiki.okfCompatibility.kind === 'futureRestricted'}>{scanState(wiki.id) ? t('status-working') : wiki.failedCount > 0 || wiki.maintenanceRequired ? t('status-needs-attention') : wiki.needsReviewCount > 0 ? t('desktop-wiki-pending-status') : trustLabel(wiki)}</span>
      <ChevronRight size={17} aria-hidden="true" />
    </button>
  {:else}
    <div class="table-empty"><BookOpen size={28} aria-hidden="true" /><strong>{t('desktop-wiki-empty-title')}</strong><p>{t('desktop-wiki-empty-body')}</p></div>
  {/each}
</div>
