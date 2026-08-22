<script lang="ts">
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import BookOpen from '@lucide/svelte/icons/book-open';
  import FileText from '@lucide/svelte/icons/file-text';
  import History from '@lucide/svelte/icons/history';
  import List from '@lucide/svelte/icons/list';
  import LockKeyhole from '@lucide/svelte/icons/lock-keyhole';
  import Network from '@lucide/svelte/icons/network';
  import { tick } from 'svelte';
  import type {
    NearbyBrowseSummary,
    PublicBrowseSummary,
    PublicConceptSummaryDto,
    HostPlatform,
    RemoteWikiPageDescriptorSummary,
    RemoteWikiPageInput
  } from './api';
  import LoadingState from './components/LoadingState.svelte';
  import DeviceIdentity from './components/identity/DeviceIdentity.svelte';
  import { focusChoiceWithoutScroll } from './focus';
  import type { MessageArgs } from './i18n';
  import RemoteWikiGraph from './RemoteWikiGraph.svelte';

  type SharedWikiSource = 'nearby' | 'public';

  export let source: SharedWikiSource;
  export let sourceName: string;
  export let sourcePlatform: HostPlatform | null = null;
  export let sourceLabel: string | null = null;
  export let browse: NearbyBrowseSummary | PublicBrowseSummary | null;
  export let loading: boolean;
  export let structureLoading: boolean;
  export let pageLoading: boolean;
  export let initialConceptId: string | null = null;
  export let t: (id: string, args?: MessageArgs) => string;
  export let metadata: (concept: PublicConceptSummaryDto) => string;
  export let onback: () => void;
  export let onopenpage: (page: RemoteWikiPageInput, expectedFingerprint: string) => void;
  export let onblock: ((publisherId: string) => void) | null = null;

  let selectedPage: RemoteWikiPageInput | null = null;
  let viewMode: 'list' | 'graph' = 'list';
  let headingElement: HTMLHeadingElement | null = null;
  let renderedWikiIdentity: string | null = null;

  $: synchronizeWikiSelection(browse ? browseIdentity(browse) : null);
  $: descriptors = browse
    ? [...browse.reservedPages, ...browse.documents].sort((left, right) => left.logicalPath.localeCompare(right.logicalPath))
    : [];
  $: selectedDescriptor = descriptors.find((descriptor) => samePage(descriptor.page, selectedPage))
    ?? descriptorForInitialConcept(descriptors)
    ?? descriptors[0]
    ?? null;
  $: selectedConceptId = selectedPage?.kind === 'concept'
    ? selectedPage.conceptId
    : selectedDescriptor?.page.kind === 'concept'
      ? selectedDescriptor.page.conceptId
      : initialConceptId;
  $: selectedConcept = browse?.concepts.find((concept) => concept.conceptId === selectedConceptId)
    ?? (selectedConceptId === null ? browse?.concepts[0] : null)
    ?? null;
  $: selectedDocument = browse?.page && selectedDescriptor && samePage(browse.page.descriptor.page, selectedDescriptor.page)
    ? browse.page
    : null;
  $: requestedConceptUnavailable = Boolean(
    browse
    && browse.workspaceSupported
    && !structureLoading
    && initialConceptId
    && !browse.documents.some((descriptor) => descriptor.page.kind === 'concept' && descriptor.page.conceptId === initialConceptId)
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
    selectedPage = initialConceptId ? { kind: 'concept', conceptId: initialConceptId } : null;
    viewMode = 'list';
  }

  function descriptorForInitialConcept(
    available: RemoteWikiPageDescriptorSummary[]
  ): RemoteWikiPageDescriptorSummary | null {
    if (!initialConceptId) return null;
    return available.find((descriptor) => descriptor.page.kind === 'concept' && descriptor.page.conceptId === initialConceptId) ?? null;
  }

  function pageKey(page: RemoteWikiPageInput | null): string {
    if (!page) return '';
    return page.kind === 'concept' ? `concept:${page.conceptId}` : page.kind;
  }

  function samePage(left: RemoteWikiPageInput, right: RemoteWikiPageInput | null): boolean {
    return pageKey(left) === pageKey(right);
  }

  function pageTitle(descriptor: RemoteWikiPageDescriptorSummary): string {
    if (descriptor.page.kind === 'index') return t('knowledge-index-title');
    if (descriptor.page.kind === 'log') return t('knowledge-recovery-history');
    return descriptor.title;
  }

  function selectPage(descriptor: RemoteWikiPageDescriptorSummary) {
    selectedPage = descriptor.page;
    onopenpage(descriptor.page, descriptor.fingerprint);
  }

  function selectGraphPage(page: RemoteWikiPageInput) {
    const descriptor = descriptors.find((candidate) => samePage(candidate.page, page));
    if (descriptor) selectPage(descriptor);
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

<section class="shared-wiki-viewer" aria-busy={loading || structureLoading || pageLoading}>
  {#if loading}
    <button class="shared-wiki-back" onclick={onback}>
      <ArrowLeft size={16} aria-hidden="true" />
      {t('desktop-shared-back-results')}
    </button>
    <div class="shared-wiki-state">
      <LoadingState label={t('desktop-shared-loading-title')} detail={t('desktop-shared-loading-body')} />
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
      <div class="shared-access-copy"><span>{t('desktop-shared-read-only')}</span><DeviceIdentity name={sourceName} platform={sourcePlatform} platformLabel={sourceLabel ?? sourceName} source={source === 'public' ? 'public' : 'device'} compact /></div>
      <small>{statusLabel()}</small>
      {#if publisherId() && onblock}<button class="text-action" onclick={blockCurrentPublisher}>{t('search-public-block-publisher')}</button>{/if}
    </section>

    <div class="content-tabs-bar shared-content-tabs">
      <div class="content-tabs" aria-label={t('desktop-wiki-sections')}>
        <span class="content-tab-label active">{t('desktop-wiki-content-tab')}<span>{browse.documents.length}</span></span>
      </div>
      <span class="shared-format">{browse.okfCompatibility ? t(`desktop-okf-compatibility-${browse.okfCompatibility.kind}`) : t('desktop-public-format-unavailable')}</span>
    </div>

    {#if browse.workspaceSupported}
      <div class="wiki-toolbar shared-wiki-toolbar">
        {#if structureLoading}<LoadingState label={t('desktop-shared-loading-structure')} compact />{/if}
        <div class="view-switch" aria-label={t('desktop-wiki-view')}>
          <button class:active={viewMode === 'list'} aria-pressed={viewMode === 'list'} onclick={() => viewMode = 'list'}><List size={15} aria-hidden="true" />{t('desktop-view-list')}</button>
          <button class:active={viewMode === 'graph'} aria-pressed={viewMode === 'graph'} onclick={() => viewMode = 'graph'}><Network size={15} aria-hidden="true" />{t('desktop-view-graph')}</button>
        </div>
      </div>

      {#if viewMode === 'graph'}
        <section class="graph-view shared-graph-view">
          {#key `${browse.wikiId}:${browse.documents.length}:${browse.links.length}`}
            <RemoteWikiGraph
              wikiName={browse.wikiName ?? t('desktop-public-origin-missing')}
              pages={descriptors}
              links={browse.links}
              onselect={selectGraphPage}
              graphLabel={t('desktop-graph-map-label', { wiki: browse.wikiName ?? t('desktop-public-origin-missing') })}
              errorLabel={t('desktop-graph-error')}
              loadingLabel={t('desktop-graph-loading')}
              pagesLabel={t('desktop-graph-pages-label')}
              countsLabel={t('knowledge-graph-counts', { nodes: descriptors.length, links: browse.links.length })}
            />
          {/key}
        </section>
      {:else}
        <div class="file-browser shared-file-browser">
          <aside class="file-list" aria-label={t('knowledge-pages')}>
            {#each descriptors as descriptor (pageKey(descriptor.page))}
              <button class:active={selectedDescriptor && samePage(selectedDescriptor.page, descriptor.page)} aria-current={selectedDescriptor && samePage(selectedDescriptor.page, descriptor.page) ? 'page' : undefined} onmousedown={focusChoiceWithoutScroll} onclick={() => selectPage(descriptor)} disabled={pageLoading}>
                {#if descriptor.page.kind === 'index'}<BookOpen size={17} aria-hidden="true" />{:else if descriptor.page.kind === 'log'}<History size={17} aria-hidden="true" />{:else}<FileText size={17} aria-hidden="true" />{/if}
                <span><strong>{pageTitle(descriptor)}</strong><small>{descriptor.logicalPath}</small></span>
              </button>
            {:else}
              <div class="shared-file-empty"><BookOpen size={20} aria-hidden="true" /><span>{t('desktop-shared-empty-title')}</span></div>
            {/each}
            {#if structureLoading}<LoadingState label={t('desktop-shared-loading-structure')} compact />{/if}
            {#if browse.appendFailed}<div class="shared-more-warning" role="status"><AlertTriangle size={15} aria-hidden="true" /><span>{t('desktop-shared-structure-failed')}</span></div>{/if}
          </aside>
          <section class="file-preview shared-file-preview" aria-live="polite">
            {#if pageLoading}
              <LoadingState label={t('desktop-shared-loading-page')} detail={selectedDescriptor?.logicalPath ?? null} />
            {:else if requestedConceptUnavailable}
              <div class="table-empty shared-target-unavailable" role="alert"><AlertTriangle size={20} aria-hidden="true" /><div><strong>{t('desktop-shared-target-unavailable-title')}</strong><p>{t('desktop-shared-target-unavailable-body')}</p></div></div>
            {:else if selectedDocument}
              <header><p class="section-label">{selectedDocument.descriptor.logicalPath}</p><h2>{pageTitle(selectedDocument.descriptor)}</h2></header>
              {#if selectedConcept}
                <aside class="concept-assurance shared-concept-assurance" aria-label={t('desktop-concept-assurance-title')}>
                  <div><span>{t('desktop-concept-type')}</span><strong>{selectedConcept.conceptType}</strong></div>
                  <div><span>{t('desktop-concept-trust')}</span><strong>{metadata(selectedConcept)}</strong></div>
                  <div class="shared-source-assurance"><span>{t('desktop-shared-source')}</span><DeviceIdentity name={sourceName} platform={sourcePlatform} platformLabel={sourceLabel ?? sourceName} source={source === 'public' ? 'public' : 'device'} compact /></div>
                </aside>
              {/if}
              <div class="knowledge-blocks">
                {#each selectedDocument.blocks as block, blockIndex (blockIndex)}
                  {#if block.kind === 'heading'}<h3 class:minor={block.level > 2}>{block.text}</h3>{:else if block.kind === 'paragraph'}<p>{block.text}</p>{:else if block.kind === 'listItem'}<div class="safe-list-item"><span>{block.ordered ? '—' : '•'}</span><p>{block.text}</p></div>{:else if block.kind === 'code'}<pre><code>{block.text}</code></pre>{:else if block.kind === 'quote'}<blockquote>{block.text}</blockquote>{:else}<hr />{/if}
                {/each}
              </div>
              {#if selectedDocument.metadata.length > 0}
                <details class="advanced-disclosure shared-metadata"><summary>{t('desktop-shared-published-metadata')}</summary><dl>{#each selectedDocument.metadata as entry, metadataIndex (`${metadataIndex}:${entry[0]}`)}<div><dt>{entry[0]}</dt><dd>{entry[1]}</dd></div>{/each}</dl></details>
              {/if}
            {:else if selectedDescriptor}
              <div class="file-empty"><BookOpen size={28} aria-hidden="true" /><h2>{t('knowledge-select-page')}</h2><p>{t('desktop-shared-open-page-body')}</p></div>
            {:else}
              <div class="table-empty"><strong>{t('desktop-shared-empty-title')}</strong><p>{t('desktop-shared-empty-body')}</p></div>
            {/if}
          </section>
        </div>
      {/if}
    {:else}
      <div class="shared-legacy" role="status"><AlertTriangle size={18} aria-hidden="true" /><div><strong>{t('desktop-shared-legacy-title')}</strong><p>{t('desktop-shared-legacy-body')}</p></div></div>
      <div class="file-browser shared-file-browser legacy">
        <aside class="file-list" aria-label={t('knowledge-pages')}>
          {#each browse.concepts as concept (`${concept.conceptId}:${concept.sourceRevision}`)}
            <button class:active={selectedConcept?.conceptId === concept.conceptId} onmousedown={focusChoiceWithoutScroll} onclick={() => selectedPage = { kind: 'concept', conceptId: concept.conceptId }}><FileText size={17} aria-hidden="true" /><span><strong>{concept.title}</strong><small>{concept.conceptType} · {concept.language}</small></span></button>
          {/each}
        </aside>
        <section class="file-preview shared-file-preview">
          {#if selectedConcept}<header><p class="section-label">{t('desktop-shared-summary-label')}</p><h2>{selectedConcept.title}</h2></header><p>{selectedConcept.summary}</p>{:else}<div class="file-empty"><BookOpen size={28} aria-hidden="true" /><h2>{t('knowledge-select-page')}</h2></div>{/if}
        </section>
      </div>
    {/if}
  {:else}
    <button class="shared-wiki-back" onclick={onback}><ArrowLeft size={16} aria-hidden="true" />{t('desktop-shared-back-results')}</button>
    <div class="shared-wiki-state warning" role="alert"><AlertTriangle size={20} aria-hidden="true" /><div><strong>{t('desktop-public-invalid-content')}</strong><p>{t('desktop-shared-unavailable-body')}</p></div></div>
  {/if}
</section>
