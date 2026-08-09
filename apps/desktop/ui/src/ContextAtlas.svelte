<script lang="ts">
  import { onMount } from 'svelte';
  import type { Core, ElementDefinition, StylesheetJson } from 'cytoscape';
  import Network from '@lucide/svelte/icons/network';
  import type { AtlasModel, AtlasNode } from './atlas';

  export let model: AtlasModel;
  export let label: string;
  export let emptyLabel: string;
  export let onselect: ((id: string) => void) | undefined = undefined;

  let container: HTMLDivElement;
  let graph: Core | null = null;
  let loadFailed = false;

  function toneColors(): Record<AtlasNode['tone'], string> {
    return {
      neutral: color('--line', '#283a49'),
      active: color('--cyan', '#59cfe1'),
      ai: color('--violet', '#9585ff'),
      verified: color('--verify', '#58c78d'),
      attention: color('--amber', '#d9a24f'),
      error: color('--coral', '#ef766b')
    };
  }

  function graphElements(): ElementDefinition[] {
    const tones = toneColors();
    const depthByNode: Record<string, number> = {};
    for (const node of model.nodes) {
      let depth = 0;
      let current = node.id;
      const visited: string[] = [];
      while (!visited.includes(current) && depth < model.nodes.length) {
        visited.push(current);
        const incoming = model.edges.find((edge) => edge.target === current);
        if (!incoming) break;
        current = incoming.source;
        depth += 1;
      }
      depthByNode[node.id] = depth;
    }
    return [
      ...model.nodes.map((node) => {
        const depth = depthByNode[node.id] ?? 0;
        const peers = model.nodes.filter((candidate) => depthByNode[candidate.id] === depth);
        const peerIndex = peers.findIndex((candidate) => candidate.id === node.id);
        return {
          data: { ...node, color: tones[node.tone] },
          position: { x: peerIndex * 96, y: depth * 88 }
        };
      }),
      ...model.edges.map((edge, index) => ({ data: { id: `edge:${index}`, ...edge } }))
    ];
  }

  function color(name: string, fallback: string): string {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
  }

  function graphStyle(): StylesheetJson {
    const muted = color('--muted', '#93a7b4');
    const line = color('--line', '#283a49');
    return [
      { selector: 'node', style: { 'background-color': 'data(color)', 'border-color': line, 'border-width': 2, label: 'data(label)', 'text-opacity': 0, height: 16, width: 16, 'overlay-opacity': 0 } },
      { selector: 'edge', style: { 'curve-style': 'taxi', 'taxi-direction': 'downward', 'line-color': line, 'target-arrow-color': muted, 'target-arrow-shape': 'triangle', width: 1, 'arrow-scale': .7 } },
      { selector: 'node:selected', style: { 'border-color': color('--strong', '#dce7ec'), 'border-width': 3, height: 20, width: 20 } }
    ];
  }

  function syncGraph() {
    if (!graph) return;
    graph.elements().remove();
    graph.add(graphElements());
    graph.style(graphStyle());
    graph.layout({ name: 'preset', fit: true, padding: 24, animate: false }).run();
    if (model.selectedId) graph.getElementById(model.selectedId).select();
  }

  $: if (graph && model) syncGraph();

  onMount(() => {
    if (!model.nodes.length) return;
    let disposed = false;
    void import('cytoscape').then(({ default: cytoscape }) => {
      if (disposed) return;
      graph = cytoscape({
        container,
        elements: graphElements(),
        style: graphStyle(),
        layout: { name: 'preset', fit: true, padding: 24, animate: false },
        minZoom: .7,
        maxZoom: 1.35,
        userPanningEnabled: false,
        userZoomingEnabled: false,
        boxSelectionEnabled: false
      });
      if (model.selectedId) graph.getElementById(model.selectedId).select();
      graph.on('tap', 'node', (event) => onselect?.(String(event.target.id())));
    }).catch(() => { loadFailed = true; });
    return () => {
      disposed = true;
      graph?.destroy();
      graph = null;
    };
  });
</script>

<aside class="context-atlas" aria-labelledby="atlas-title">
  <header><Network size={15} aria-hidden="true" /><span>{label}</span></header>
  <div class="atlas-copy"><h2 id="atlas-title">{model.title}</h2><p>{model.description}</p></div>
  {#if model.nodes.length && !loadFailed}
    <div class="atlas-canvas" bind:this={container} aria-hidden="true"></div>
  {:else}
    <div class="atlas-empty"><Network size={22} aria-hidden="true" /><p>{emptyLabel}</p></div>
  {/if}
  <ol class="atlas-list" aria-label={model.title}>
    {#each model.nodes as node (node.id)}
      <li class:active={node.id === model.selectedId} data-tone={node.tone}>
        {#if onselect}<button onclick={() => onselect?.(node.id)}><span>{node.label}</span>{#if node.detail}<small>{node.detail}</small>{/if}</button>
        {:else}<div><span>{node.label}</span>{#if node.detail}<small>{node.detail}</small>{/if}</div>{/if}
      </li>
    {/each}
  </ol>
</aside>
