<script lang="ts">
  import { onMount } from 'svelte';
  import type { Core, ElementDefinition } from 'cytoscape';
  import type { KnowledgeBundleSummary, KnowledgePageInput } from './api';

  export let bundle: KnowledgeBundleSummary;
  export let onselect: (page: KnowledgePageInput) => void;

  let container: HTMLDivElement;
  let graph: Core | null = null;
  let loadFailed = false;

  function pageKey(page: KnowledgePageInput): string {
    if (page.kind === 'concept') return `concept:${page.id}`;
    return page.kind;
  }

  function pageTitle(page: KnowledgePageInput): string {
    if (page.kind === 'index') return 'Índice';
    if (page.kind === 'log') return 'Historial';
    return bundle.concepts.find((concept) => concept.page.kind === 'concept' && concept.page.id === page.id)?.title ?? 'Concepto';
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
          id: `link:${index}`,
          source: pageKey(link.source),
          target: pageKey(link.target),
          label: link.label
        }
      }));
    return [...nodes, ...edges];
  }

  onMount(() => {
    let disposed = false;
    void import('cytoscape').then(({ default: cytoscape }) => {
      if (disposed) return;
      graph = cytoscape({
        container,
        elements: graphElements(),
        minZoom: 0.45,
        maxZoom: 2.2,
        wheelSensitivity: 0.18,
        style: [
          { selector: 'node', style: { 'background-color': '#163f54', 'border-color': '#61d6e8', 'border-width': 1.5, color: '#edf7f8', label: 'data(label)', 'font-family': 'Atkinson, sans-serif', 'font-size': '11px', 'text-max-width': '120px', 'text-wrap': 'ellipsis', 'text-valign': 'bottom', 'text-margin-y': 8, height: 22, width: 22 } },
          { selector: 'node[kind = "index"]', style: { 'background-color': '#61d6e8', 'border-width': 0, height: 30, width: 30 } },
          { selector: 'node[kind = "log"]', style: { 'background-color': '#9b8cff', 'border-color': '#9b8cff' } },
          { selector: 'edge', style: { 'curve-style': 'bezier', 'line-color': '#355b70', 'target-arrow-color': '#61d6e8', 'target-arrow-shape': 'triangle', width: 1 } },
          { selector: 'node:selected', style: { 'border-color': '#61c995', 'border-width': 3, 'overlay-opacity': 0 } }
        ],
        layout: { name: 'breadthfirst', roots: ['#index'], directed: true, padding: 34, spacingFactor: 1.25, animate: false }
      });
      graph.on('tap', 'node', (event) => {
        const page = event.target.data('page') as KnowledgePageInput | undefined;
        if (page) onselect(page);
      });
    }).catch(() => { loadFailed = true; });

    return () => {
      disposed = true;
      graph?.destroy();
      graph = null;
    };
  });
</script>

<div class="graph-shell">
  <div class="graph-heading">
    <div><p>Relaciones verificadas</p><h3>{bundle.collectionName}</h3></div>
    <small>{bundle.concepts.length + 2} páginas · {bundle.links.length} enlaces internos</small>
  </div>
  {#if loadFailed}
    <p class="graph-error" role="status">El mapa no pudo cargarse. La navegación por páginas sigue disponible.</p>
  {:else}
    <div class="graph-canvas" bind:this={container} role="img" aria-label={`Mapa de relaciones de ${bundle.collectionName}`}></div>
  {/if}
  <div class="graph-index" aria-label="Páginas del mapa">
    <button onclick={() => onselect({ kind: 'index' })}>Índice</button>
    <button onclick={() => onselect({ kind: 'log' })}>Historial</button>
    {#each bundle.concepts as concept}
      <button onclick={() => onselect(concept.page)}>{concept.title}</button>
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
  .graph-canvas { min-height: 340px; background: #081824; border-bottom: 1px solid var(--line); }
  .graph-index { display: flex; gap: 7px; padding-top: 14px; overflow-x: auto; }
  .graph-index button { flex: 0 0 auto; padding: 7px 10px; color: var(--muted); background: transparent; border: 1px solid var(--line); border-radius: 999px; cursor: pointer; }
  .graph-index button:hover, .graph-index button:focus-visible { color: inherit; border-color: var(--cyan); }
  .graph-error { padding: 20px; color: var(--amber); border-left: 2px solid var(--amber); }
  @media (prefers-color-scheme: light) { .graph-canvas { background: #f4f8f9; } }
</style>
