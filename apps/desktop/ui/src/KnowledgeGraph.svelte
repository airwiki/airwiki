<script lang="ts">
  import { onMount } from 'svelte';
  import type { Core, ElementDefinition } from 'cytoscape';
  import type { KnowledgeBundleSummary, KnowledgePageInput } from './api';
  import { message } from './i18n';
  import type { LocalePreference } from './generated/ui-contract';

  export let bundle: KnowledgeBundleSummary;
  export let onselect: (page: KnowledgePageInput) => void;
  export let locale: LocalePreference;

  let container: HTMLDivElement;
  let graph: Core | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let loadFailed = false;

  const t = (id: string, args?: Record<string, string | number>) => message(locale, id, args);

  function pageKey(page: KnowledgePageInput): string {
    if (page.kind === 'concept') return `concept:${page.id}`;
    return page.kind;
  }

  function pageTitle(page: KnowledgePageInput): string {
    if (page.kind === 'index') return t('knowledge-index-title');
    if (page.kind === 'log') return t('knowledge-recovery-history');
    return bundle.concepts.find((concept) => concept.page.kind === 'concept' && concept.page.id === page.id)?.title ?? t('knowledge-concept-fallback');
  }

  function graphElements(): ElementDefinition[] {
    const pages: KnowledgePageInput[] = [
      { kind: 'index' },
      { kind: 'log' },
      ...bundle.concepts.map((concept) => concept.page)
    ];
    const nodeIds = new Set(pages.map(pageKey));
    const nodes: ElementDefinition[] = pages.map((page) => ({
      data: { id: pageKey(page), label: pageTitle(page), page, kind: page.kind }
    }));
    const edges: ElementDefinition[] = bundle.links
      .filter((link) => nodeIds.has(pageKey(link.source)) && nodeIds.has(pageKey(link.target)))
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
    const targets = new Set(bundle.links.map((link) => pageKey(link.target)));
    const roots = ['index', 'log', ...bundle.concepts.map((concept) => pageKey(concept.page))]
      .filter((nodeId) => !targets.has(nodeId));
    return roots.length > 0 ? roots : ['index'];
  }

  function selectPage(page: KnowledgePageInput) {
    graph?.elements().unselect();
    graph?.edges().removeClass('related');
    const node = graph?.getElementById(pageKey(page));
    node?.select();
    node?.connectedEdges().addClass('related');
    onselect(page);
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
        wheelSensitivity: 0.18,
        style: [
          { selector: 'node', style: { 'background-color': surface, 'border-color': cyan, 'border-width': 1.5, color: strong, label: 'data(label)', 'font-family': 'Atkinson, sans-serif', 'font-size': '11px', 'text-background-color': surface, 'text-background-opacity': 0.92, 'text-background-padding': '2px', 'text-max-width': '120px', 'text-wrap': 'ellipsis', 'text-valign': 'bottom', 'text-margin-y': 8, height: 22, width: 22 } },
          { selector: 'node[kind = "index"]', style: { 'background-color': cyan, 'border-width': 0, height: 30, width: 30 } },
          { selector: 'node[kind = "log"]', style: { 'background-color': violet, 'border-color': violet } },
          { selector: 'edge', style: { 'curve-style': 'bezier', 'line-color': muted, 'line-opacity': 0.58, 'target-arrow-color': muted, 'target-arrow-shape': 'triangle', 'arrow-scale': 1, 'source-endpoint': 'outside-to-node', 'target-endpoint': 'outside-to-node', width: 1.7, 'z-index': 1 } },
          { selector: 'edge.related', style: { 'line-color': cyan, 'line-opacity': 1, 'target-arrow-color': cyan, width: 2.2 } },
          { selector: 'edge.related[label != ""]', style: { label: 'data(label)', color: muted, 'font-family': 'Atkinson, sans-serif', 'font-size': '9px', 'text-background-color': surface, 'text-background-opacity': 0.96, 'text-background-padding': '3px', 'text-margin-y': -9, 'text-rotation': 'autorotate' } },
          { selector: 'node:selected', style: { 'border-color': verify, 'border-width': 3, 'overlay-opacity': 0 } }
        ],
        layout: {
          name: 'breadthfirst',
          roots: graphRoots(),
          directed: true,
          padding: 42,
          spacingFactor: 1.35,
          avoidOverlap: true,
          nodeDimensionsIncludeLabels: true,
          animate: false,
          transform: (_node, position) => ({ x: position.y, y: position.x })
        }
      });
      resizeObserver = new ResizeObserver(() => {
        graph?.resize();
        graph?.fit(undefined, 24);
      });
      resizeObserver.observe(container);
      requestAnimationFrame(() => {
        graph?.resize();
        graph?.fit(undefined, 24);
      });
      graph.on('tap', 'node', (event) => {
        const page = event.target.data('page') as KnowledgePageInput | undefined;
        if (page) selectPage(page);
      });
    }).catch(() => { loadFailed = true; });

    return () => {
      disposed = true;
      resizeObserver?.disconnect();
      resizeObserver = null;
      graph?.destroy();
      graph = null;
    };
  });
</script>

<div class="graph-shell">
  <div class="graph-heading">
    <div><p>{t('desktop-graph-verified')}</p><h3>{bundle.wikiName}</h3></div>
    <small>{t('knowledge-graph-counts', { nodes: bundle.concepts.length + 2, links: bundle.links.length })}</small>
  </div>
  {#if loadFailed}
    <p class="graph-error" role="status">{t('desktop-graph-error')}</p>
  {:else}
    <div class="graph-canvas" bind:this={container} role="img" aria-label={t('desktop-graph-map-label', { collection: bundle.wikiName })}></div>
  {/if}
  <div class="graph-index" aria-label={t('desktop-graph-pages-label')}>
    <button onclick={() => selectPage({ kind: 'index' })}>{t('knowledge-index-title')}</button>
    <button onclick={() => selectPage({ kind: 'log' })}>{t('knowledge-recovery-history')}</button>
    {#each bundle.concepts as concept (concept.title)}
      <button onclick={() => selectPage(concept.page)}>{concept.title}</button>
    {/each}
  </div>
</div>

<style>
  .graph-shell { display: grid; grid-template-rows: auto minmax(300px, 1fr) auto; min-height: 500px; }
  .graph-heading { display: flex; align-items: end; justify-content: space-between; gap: 20px; padding-bottom: 16px; border-bottom: 1px solid var(--line); }
  .graph-heading p, .graph-heading h3 { margin: 0; }
  .graph-heading p { margin-bottom: 5px; color: var(--cyan); font: 600 11px 'Space Grotesk', sans-serif; letter-spacing: .1em; text-transform: uppercase; }
  .graph-heading h3 { font: 500 22px 'Space Grotesk', sans-serif; }
  .graph-heading small { color: var(--muted); }
  .graph-canvas { position: relative; min-height: 340px; background: var(--graph-canvas); border-bottom: 1px solid var(--line); }
  .graph-index { display: flex; flex-wrap: wrap; gap: 7px; padding-top: 14px; }
  .graph-index button { flex: 1 1 220px; min-width: 0; padding: 7px 10px; overflow-wrap: anywhere; color: var(--muted); background: transparent; border: 1px solid var(--line); border-radius: 8px; text-align: left; cursor: pointer; }
  .graph-index button:hover, .graph-index button:focus-visible { color: inherit; border-color: var(--cyan); }
  .graph-error { padding: 20px; color: var(--amber); border-left: 2px solid var(--amber); }
</style>
