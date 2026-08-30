<script lang="ts">
  import { onMount } from 'svelte';
  import type { Core, ElementDefinition } from 'cytoscape';
  import type { RemoteWikiGraphLinkSummary, RemoteWikiPageDescriptorSummary, RemoteWikiPageInput } from './api';
  import LoadingState from './components/LoadingState.svelte';
  import { focusChoiceWithoutScroll } from './focus';

  export let wikiName: string;
  export let pages: RemoteWikiPageDescriptorSummary[];
  export let links: RemoteWikiGraphLinkSummary[];
  export let onselect: (page: RemoteWikiPageInput) => void;
  export let graphLabel: string;
  export let errorLabel: string;
  export let loadingLabel: string;
  export let pagesLabel: string;
  export let countsLabel: string;

  let container: HTMLDivElement;
  let graph: Core | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let layoutReady = false;
  let graphReady = false;
  let loadFailed = false;

  function pageKey(page: RemoteWikiPageInput): string {
    return page.kind === 'concept' ? `concept:${page.conceptId}` : page.kind;
  }

  function pageTitle(page: RemoteWikiPageInput): string {
    return pages.find((descriptor) => pageKey(descriptor.page) === pageKey(page))?.title ?? pageKey(page);
  }

  function visibleLinks() {
    const nodeIds = new Set(pages.map((descriptor) => pageKey(descriptor.page)));
    return links.filter((link) => nodeIds.has(pageKey(link.source)) && nodeIds.has(pageKey(link.target)));
  }

  function graphElements(): ElementDefinition[] {
    const nodes = pages.map((descriptor) => ({
      data: {
        id: pageKey(descriptor.page),
        label: descriptor.title,
        page: descriptor.page,
        kind: descriptor.page.kind
      }
    }));
    const edges = visibleLinks()
      .map((link, index) => ({
        data: {
          id: `link:${pageKey(link.source)}:${pageKey(link.target)}:${index}`,
          source: pageKey(link.source),
          target: pageKey(link.target),
          label: link.label
        }
      }));
    return [...nodes, ...edges];
  }

  function graphRoots(): string[] {
    const targets = new Set(links.map((link) => pageKey(link.target)));
    const roots = pages.map((descriptor) => pageKey(descriptor.page))
      .filter((nodeId) => !targets.has(nodeId));
    return roots.length > 0 ? roots : pages.slice(0, 1).map((descriptor) => pageKey(descriptor.page));
  }

  function selectPage(page: RemoteWikiPageInput) {
    graph?.elements().unselect();
    graph?.edges().removeClass('related');
    const node = graph?.getElementById(pageKey(page));
    node?.select();
    node?.connectedEdges().addClass('related');
    onselect(page);
  }

  function renderGraph() {
    if (!graph || container.clientWidth === 0 || container.clientHeight === 0) return;
    graph.resize();
    if (!layoutReady) {
      graph.layout({
        name: 'breadthfirst',
        roots: graphRoots(),
        directed: true,
        padding: 42,
        spacingFactor: 1.35,
        avoidOverlap: true,
        nodeDimensionsIncludeLabels: true,
        animate: false,
        transform: (_node, position) => ({ x: position.y, y: position.x })
      }).run();
      layoutReady = true;
    }
    graph.fit(undefined, 24);
    graphReady = true;
  }

  onMount(() => {
    let disposed = false;
    void import('cytoscape').then(({ default: cytoscape }) => {
      if (disposed) return;
      const palette = getComputedStyle(container);
      const strong = palette.getPropertyValue('--strong').trim() || '#dce7ec';
      const muted = palette.getPropertyValue('--muted').trim() || '#93a7b4';
      const cyan = palette.getPropertyValue('--cyan').trim() || '#59cfe1';
      const violet = palette.getPropertyValue('--violet').trim() || '#9585ff';
      const verify = palette.getPropertyValue('--verify').trim() || '#58c78d';
      const surface = palette.getPropertyValue('--slate').trim() || '#121b25';
      graph = cytoscape({
        container,
        elements: graphElements(),
        minZoom: 0.45,
        maxZoom: 2.2,
        style: [
          { selector: 'node', style: { 'background-color': surface, 'border-color': cyan, 'border-width': 1.5, color: strong, label: 'data(label)', 'font-family': 'Atkinson, sans-serif', 'font-size': '11px', 'text-background-color': surface, 'text-background-opacity': 0.92, 'text-background-padding': '2px', 'text-max-width': '120px', 'text-wrap': 'ellipsis', 'text-valign': 'bottom', 'text-margin-y': 8, height: 22, width: 22 } },
          { selector: 'node[kind = "index"]', style: { 'background-color': cyan, 'border-width': 0, height: 30, width: 30 } },
          { selector: 'node[kind = "log"]', style: { 'background-color': violet, 'border-color': violet } },
          { selector: 'edge', style: { 'curve-style': 'bezier', 'line-color': muted, 'line-opacity': 0.58, 'target-arrow-color': muted, 'target-arrow-shape': 'triangle', 'arrow-scale': 1, 'source-endpoint': 'outside-to-node', 'target-endpoint': 'outside-to-node', width: 1.7, 'z-index': 1 } },
          { selector: 'edge.related', style: { 'line-color': cyan, 'line-opacity': 1, 'target-arrow-color': cyan, width: 2.2 } },
          { selector: 'node:selected', style: { 'border-color': verify, 'border-width': 3, 'overlay-opacity': 0 } }
        ],
        layout: { name: 'preset', fit: false }
      });
      resizeObserver = new ResizeObserver(renderGraph);
      resizeObserver.observe(container);
      requestAnimationFrame(renderGraph);
      graph.on('tap', 'node', (event) => {
        const page = event.target.data('page') as RemoteWikiPageInput | undefined;
        if (page) selectPage(page);
      });
    }).catch(() => { loadFailed = true; });

    return () => {
      disposed = true;
      resizeObserver?.disconnect();
      graph?.destroy();
      graph = null;
      layoutReady = false;
      graphReady = false;
    };
  });
</script>

<div class="graph-shell">
  <div class="graph-heading">
    <h3>{wikiName}</h3>
    <small>{countsLabel}</small>
  </div>
  {#if loadFailed}
    <p class="graph-error" role="status">{errorLabel}</p>
  {:else}
    <div class="graph-stage">
      <div class="graph-canvas" bind:this={container} aria-hidden="true"></div>
      {#if !graphReady}<div class="graph-loading"><LoadingState label={loadingLabel} compact /></div>{/if}
    </div>
  {/if}
  {#if !loadFailed}
    <div class="sr-only">
      <p>{graphLabel}. {countsLabel}</p>
      <ul>
        {#each visibleLinks() as link (pageKey(link.source) + pageKey(link.target) + link.label)}
          <li>{pageTitle(link.source)} → {pageTitle(link.target)}</li>
        {/each}
      </ul>
    </div>
  {/if}
  <div class="graph-index" role="group" aria-label={pagesLabel}>
    {#each pages as descriptor (pageKey(descriptor.page))}
      <button onmousedown={focusChoiceWithoutScroll} onclick={() => selectPage(descriptor.page)}>{descriptor.title}</button>
    {/each}
  </div>
</div>

<style>
  .graph-shell { display: grid; grid-template-rows: auto minmax(300px, 1fr) auto; min-height: 500px; padding: 20px; background: var(--slate); border: 1px solid var(--line); border-radius: var(--panel-radius); }
  .graph-heading { display: flex; align-items: end; justify-content: space-between; gap: 20px; padding-bottom: 16px; border-bottom: 1px solid var(--line); }
  .graph-heading h3 { margin: 0; font: 500 22px 'Space Grotesk', sans-serif; }
  .graph-heading small { color: var(--muted); }
  .graph-stage { display: grid; min-width: 0; min-height: 340px; border-bottom: 1px solid var(--line); }
  .graph-canvas { position: relative; grid-area: 1 / 1; min-width: 0; min-height: 340px; background: var(--graph-canvas); }
  .graph-loading { z-index: 1; display: grid; grid-area: 1 / 1; place-items: center; background: var(--graph-canvas); pointer-events: none; }
  .graph-index { display: flex; flex-wrap: wrap; gap: 7px; padding-top: 14px; }
  .graph-index button { flex: 1 1 220px; min-width: 0; padding: 7px 10px; overflow-wrap: anywhere; color: var(--muted); background: transparent; border: 1px solid var(--line); border-radius: var(--control-radius); text-align: left; cursor: pointer; }
  .graph-index button:hover, .graph-index button:focus-visible { color: inherit; border-color: var(--cyan); }
  .graph-error { padding: 20px; color: var(--amber); border-left: 2px solid var(--amber); }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
</style>
