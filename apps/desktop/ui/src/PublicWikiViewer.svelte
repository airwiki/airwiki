<script lang="ts">
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import BookOpen from '@lucide/svelte/icons/book-open';
  import type { PublicBrowseSummary, PublicConceptSummaryDto } from './api';
  import type { MessageArgs } from './i18n';

  export let browse: PublicBrowseSummary | null;
  export let loading: boolean;
  export let t: (id: string, args?: MessageArgs) => string;
  export let metadata: (concept: PublicConceptSummaryDto) => string;
  export let onback: () => void;
  export let onmore: () => void;
  export let onblock: (publisherId: string) => void;

  function statusLabel(status: PublicBrowseSummary['status']): string {
    const labels: Record<PublicBrowseSummary['status'], string> = {
      direct: 'desktop-public-direct',
      relay: 'desktop-public-relay',
      expired: 'desktop-public-expired',
      offline: 'desktop-public-offline',
      failed: 'desktop-public-invalid-content'
    };
    return t(labels[status]);
  }

  function unavailable(status: PublicBrowseSummary['status']): boolean {
    return status === 'expired' || status === 'offline' || status === 'failed';
  }
</script>

<section class="public-wiki-viewer" aria-live="polite" aria-busy={loading}>
  <button class="public-wiki-back" onclick={onback}>
    <ArrowLeft size={16} aria-hidden="true" />
    {t('desktop-public-back-results')}
  </button>

  {#if loading}
    <div class="public-wiki-state" role="status">
      <span class="status-dot working" aria-hidden="true"></span>
      <div><strong>{t('desktop-public-loading-title')}</strong><p>{t('desktop-public-loading-body')}</p></div>
    </div>
  {:else if browse && unavailable(browse.status)}
    <div class="public-wiki-state warning" role="alert">
      <AlertTriangle size={20} aria-hidden="true" />
      <div><strong>{statusLabel(browse.status)}</strong><p>{t('desktop-public-unavailable-body')}</p></div>
    </div>
  {:else if browse}
    <header class="public-wiki-heading">
      <div>
        <p class="section-label">{t('desktop-public-network')}</p>
        <h2>{browse.wikiName ?? t('desktop-public-origin-missing')}</h2>
        {#if browse.description}<p>{browse.description}</p>{/if}
        <div class="public-wiki-metadata">
          <span>{statusLabel(browse.status)}</span>
          <span>{browse.okfCompatibility ? t(`desktop-okf-compatibility-${browse.okfCompatibility.kind}`) : t('desktop-public-metadata-unavailable')}</span>
        </div>
      </div>
      {#if browse.publisherId}<button class="danger" onclick={() => onblock(browse.publisherId!)}>{t('search-public-block-publisher')}</button>{/if}
    </header>

    <div class="public-concept-list" role="list" aria-label={browse.wikiName ?? t('search-public-browse-title')}>
      {#each browse.concepts as concept (`${concept.conceptId}:${concept.sourceRevision}`)}
        <article role="listitem">
          <span class="public-concept-icon"><BookOpen size={17} aria-hidden="true" /></span>
          <div><small>{concept.conceptType} · {concept.language}</small><h3>{concept.title}</h3><p>{concept.summary}</p><span>{metadata(concept)}</span></div>
        </article>
      {:else}
        <div class="public-wiki-state"><BookOpen size={20} aria-hidden="true" /><strong>{t('desktop-public-empty')}</strong></div>
      {/each}
    </div>
    {#if browse.nextCursor}<button class="secondary public-wiki-more" onclick={onmore}>{t('search-public-browse-more')}</button>{/if}
  {:else}
    <div class="public-wiki-state warning" role="alert">
      <AlertTriangle size={20} aria-hidden="true" />
      <div><strong>{t('desktop-public-invalid-content')}</strong><p>{t('desktop-public-unavailable-body')}</p></div>
    </div>
  {/if}
</section>
