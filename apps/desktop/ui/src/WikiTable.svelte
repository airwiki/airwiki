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
    <button class="wiki-row" role="row" onclick={() => onopen(wiki.id)}>
      <span class="wiki-name" role="cell"><span class="wiki-icon"><BookOpen size={18} aria-hidden="true" /></span><strong>{wiki.name}</strong></span>
      <span role="cell">{t('desktop-wiki-content-count', { published: wiki.publishedCount, pending: wiki.needsReviewCount })}</span>
      <span role="cell">{accessLabel(wiki)}</span>
      <span role="cell" class:attention={wiki.failedCount > 0 || wiki.maintenanceRequired || wiki.needsReviewCount > 0}>{scanState(wiki.id) ? t('status-working') : wiki.failedCount > 0 || wiki.maintenanceRequired ? t('status-needs-attention') : wiki.needsReviewCount > 0 ? t('desktop-wiki-pending-status') : t('status-ready')}</span>
      <ChevronRight size={17} aria-hidden="true" />
    </button>
  {:else}
    <div class="table-empty"><BookOpen size={28} aria-hidden="true" /><strong>{t('desktop-wiki-empty-title')}</strong><p>{t('desktop-wiki-empty-body')}</p></div>
  {/each}
</div>
