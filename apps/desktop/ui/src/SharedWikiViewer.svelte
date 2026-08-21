<script lang="ts">
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import BookOpen from '@lucide/svelte/icons/book-open';
  import FileText from '@lucide/svelte/icons/file-text';
  import LockKeyhole from '@lucide/svelte/icons/lock-keyhole';
  import { tick } from 'svelte';
  import type { NearbyBrowseSummary, PublicBrowseSummary, PublicConceptSummaryDto } from './api';
  import { focusChoiceWithoutScroll } from './focus';
  import type { MessageArgs } from './i18n';

  type SharedWikiSource = 'nearby' | 'public';

  export let source: SharedWikiSource;
  export let sourceName: string;
  export let browse: NearbyBrowseSummary | PublicBrowseSummary | null;
  export let loading: boolean;
  export let initialConceptId: string | null = null;
  export let t: (id: string, args?: MessageArgs) => string;
  export let metadata: (concept: PublicConceptSummaryDto) => string;
  export let onback: () => void;
  export let onmore: () => void;
  export let onblock: ((publisherId: string) => void) | null = null;

  let selectedConceptId: string | null = null;
  let headingElement: HTMLHeadingElement | null = null;
  let renderedWikiIdentity: string | null = null;

  $: synchronizeWikiSelection(browse ? browseIdentity(browse) : null);

  $: selectedConcept = browse?.concepts.find((concept) => concept.conceptId === selectedConceptId)
    ?? browse?.concepts.find((concept) => concept.conceptId === initialConceptId)
    ?? (initialConceptId === null ? browse?.concepts[0] : null)
    ?? null;
  $: requestedConceptUnavailable = Boolean(
    browse
    && initialConceptId
    && !browse.concepts.some((concept) => concept.conceptId === initialConceptId)
  );
  $: if (!loading && browse && !unavailable()) {
    const wikiKey = browseIdentity(browse);
    void tick().then(() => {
      if (!headingElement || headingElement.dataset.focusedWiki === wikiKey) return;
      headingElement.dataset.focusedWiki = wikiKey;
      headingElement.focus({ preventScroll: true });
    });
  }

  function browseIdentity(value: NearbyBrowseSummary | PublicBrowseSummary): string {
    const ownerId = source === 'nearby'
      ? (value as NearbyBrowseSummary).peerId
      : (value as PublicBrowseSummary).publisherId;
    return [source, ownerId ?? '', value.wikiId ?? '', value.wikiName ?? ''].join(':');
  }

  function synchronizeWikiSelection(wikiIdentity: string | null) {
    if (wikiIdentity === renderedWikiIdentity) return;
    renderedWikiIdentity = wikiIdentity;
    selectedConceptId = null;
  }

  function unavailable(): boolean {
    if (!browse) return false;
    return source === 'nearby'
      ? browse.status === 'unavailable'
      : browse.status === 'expired' || browse.status === 'offline' || browse.status === 'failed';
  }

  function statusLabel(): string {
    if (!browse) return '';
    if (source === 'nearby') {
      return browse.status === 'available'
        ? t('desktop-shared-nearby-authenticated')
        : t('desktop-shared-nearby-unavailable');
    }
    const labels: Record<PublicBrowseSummary['status'], string> = {
      direct: 'desktop-public-direct',
      relay: 'desktop-public-relay',
      expired: 'desktop-public-expired',
      offline: 'desktop-public-offline',
      failed: 'desktop-public-invalid-content'
    };
    return t(labels[(browse as PublicBrowseSummary).status]);
  }

  function publisherId(): string | null {
    return source === 'public' ? (browse as PublicBrowseSummary | null)?.publisherId ?? null : null;
  }

  function blockCurrentPublisher() {
    const currentPublisherId = publisherId();
    if (currentPublisherId) onblock?.(currentPublisherId);
  }
</script>

<section class="shared-wiki-viewer" aria-busy={loading}>
  {#if loading}
    <button class="shared-wiki-back" onclick={onback}>
      <ArrowLeft size={16} aria-hidden="true" />
      {t('desktop-shared-back-results')}
    </button>
    <div class="shared-wiki-state" role="status">
      <span class="status-dot working" aria-hidden="true"></span>
      <div><strong>{t('desktop-shared-loading-title')}</strong><p>{t('desktop-shared-loading-body')}</p></div>
    </div>
  {:else if browse && unavailable()}
    <button class="shared-wiki-back" onclick={onback}>
      <ArrowLeft size={16} aria-hidden="true" />
      {t('desktop-shared-back-results')}
    </button>
    <div class="shared-wiki-state warning" role="alert">
      <AlertTriangle size={20} aria-hidden="true" />
      <div><strong>{statusLabel()}</strong><p>{t('desktop-shared-unavailable-body')}</p></div>
    </div>
  {:else if browse}
    <header class="page-heading wiki-heading shared-wiki-heading">
      <div>
        <nav class="breadcrumb" aria-label={t('desktop-page-search-title')}>
          <button onclick={onback}>{t('desktop-shared-back-results')}</button>
          <span aria-hidden="true">/</span>
          <span>{browse.wikiName ?? t('desktop-public-origin-missing')}</span>
        </nav>
        <h1 bind:this={headingElement} tabindex="-1">{browse.wikiName ?? t('desktop-public-origin-missing')}</h1>
        {#if source === 'public' && (browse as PublicBrowseSummary).description}<p>{(browse as PublicBrowseSummary).description}</p>{/if}
      </div>
    </header>

    <section class="wiki-access-strip shared-wiki-access" aria-label={t('desktop-shared-access-title')}>
      <LockKeyhole size={17} aria-hidden="true" />
      <div>
        <span>{t('desktop-shared-read-only')}</span>
        <span>{sourceName}</span>
      </div>
      <small>{statusLabel()}</small>
      {#if publisherId() && onblock}<button class="text-action" onclick={blockCurrentPublisher}>{t('search-public-block-publisher')}</button>{/if}
    </section>

    <div class="content-tabs-bar shared-content-tabs">
      <div class="content-tabs" aria-label={t('desktop-wiki-sections')}>
        <span class="content-tab-label active">{t('desktop-wiki-content-tab')}<span>{browse.concepts.length}</span></span>
      </div>
      <span class="shared-format">{browse.okfCompatibility ? t(`desktop-okf-compatibility-${browse.okfCompatibility.kind}`) : t('desktop-public-format-unavailable')}</span>
    </div>

    <div class="file-browser shared-file-browser">
      <aside class="file-list" aria-label={t('knowledge-pages')}>
        {#each browse.concepts as concept (`${concept.conceptId}:${concept.sourceRevision}`)}
          <button class:active={selectedConcept?.conceptId === concept.conceptId} aria-current={selectedConcept?.conceptId === concept.conceptId ? 'page' : undefined} onmousedown={focusChoiceWithoutScroll} onclick={() => selectedConceptId = concept.conceptId}>
            <FileText size={17} aria-hidden="true" />
            <span><strong>{concept.title}</strong><small>{concept.conceptType} · {concept.language}</small></span>
          </button>
        {:else}
          <div class="shared-file-empty"><BookOpen size={20} aria-hidden="true" /><span>{t('desktop-shared-empty-title')}</span></div>
        {/each}
        {#if browse.appendFailed}
          <div class="shared-more-warning" role="status">
            <AlertTriangle size={15} aria-hidden="true" />
            <span>{t('desktop-shared-more-failed')}</span>
          </div>
        {/if}
        {#if browse.nextCursor}<button class="shared-wiki-more" onclick={onmore}>{t('search-public-browse-more')}</button>{/if}
      </aside>
      <section class="file-preview shared-file-preview">
        {#if requestedConceptUnavailable}
          <div class="table-empty shared-target-unavailable" role="alert">
            <AlertTriangle size={20} aria-hidden="true" />
            <div><strong>{t('desktop-shared-target-unavailable-title')}</strong><p>{t('desktop-shared-target-unavailable-body')}</p></div>
          </div>
        {:else if selectedConcept}
          <header><p class="section-label">{t('desktop-shared-summary-label')}</p><h2>{selectedConcept.title}</h2></header>
          <aside class="concept-assurance shared-concept-assurance" aria-label={t('desktop-concept-assurance-title')}>
            <div><span>{t('desktop-concept-type')}</span><strong>{selectedConcept.conceptType}</strong></div>
            <div><span>{t('desktop-concept-trust')}</span><strong>{metadata(selectedConcept)}</strong></div>
            <div><span>{t('desktop-shared-source')}</span><strong>{sourceName}</strong></div>
          </aside>
          {#if selectedConcept.description}<p class="shared-concept-description">{selectedConcept.description}</p>{/if}
          <div class="knowledge-body"><p>{selectedConcept.summary}</p></div>
          {#if selectedConcept.tags.length > 0}<div class="shared-tags" aria-label={t('desktop-shared-tags')}>{#each selectedConcept.tags as tag (tag)}<span>{tag}</span>{/each}</div>{/if}
          <p class="shared-summary-note"><LockKeyhole size={14} aria-hidden="true" />{t('desktop-shared-summary-note')}</p>
        {:else}
          <div class="table-empty"><strong>{t('desktop-shared-empty-title')}</strong><p>{t('desktop-shared-empty-body')}</p></div>
        {/if}
      </section>
    </div>
  {:else}
    <button class="shared-wiki-back" onclick={onback}>
      <ArrowLeft size={16} aria-hidden="true" />
      {t('desktop-shared-back-results')}
    </button>
    <div class="shared-wiki-state warning" role="alert">
      <AlertTriangle size={20} aria-hidden="true" />
      <div><strong>{t('desktop-public-invalid-content')}</strong><p>{t('desktop-shared-unavailable-body')}</p></div>
    </div>
  {/if}
</section>
