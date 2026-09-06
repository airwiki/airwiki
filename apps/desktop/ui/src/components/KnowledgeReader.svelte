<script lang="ts">
  import Info from '@lucide/svelte/icons/info';
  import Library from '@lucide/svelte/icons/library';
  import X from '@lucide/svelte/icons/x';
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import { tick, type Snippet } from 'svelte';
  import type { KnowledgeBlock, KnowledgeSourceSummary } from '../api';
  import type { MessageArgs } from '../i18n';

  export let title: string;
  export let blocks: KnowledgeBlock[];
  export let truncated = false;
  export let sources: KnowledgeSourceSummary[] | null = null;
  export let showSources = true;
  export let status: Snippet | null = null;
  export let warnings: Snippet | null = null;
  export let actions: Snippet | null = null;
  export let hasActions = false;
  export let related: Snippet | null = null;
  export let hasRelated = false;
  export let details: Snippet;
  export let t: (id: string, args?: MessageArgs) => string;

  let viewportWidth = 1024;
  let panel: 'sources' | 'details' | null = null;
  let dialog: HTMLDialogElement;
  let inspector: HTMLElement | null = null;
  let trigger: HTMLButtonElement | null = null;
  const contextId = `reader-${crypto.randomUUID()}`;
  $: wide = viewportWidth >= 1360;
  $: synchronizeDialog(panel, wide);

  async function openPanel(next: 'sources' | 'details', event: MouseEvent) {
    if (panel === next) {
      closePanel();
      return;
    }
    trigger = event.currentTarget as HTMLButtonElement;
    panel = next;
    await tick();
    if (wide) inspector?.focus({ preventScroll: true });
    else dialog?.querySelector<HTMLButtonElement>('button')?.focus();
  }

  function closePanel() {
    panel = null;
    if (dialog?.open) dialog.close();
    trigger?.focus({ preventScroll: true });
  }

  async function synchronizeDialog(currentPanel: typeof panel, isWide: boolean) {
    await tick();
    if (currentPanel !== panel || isWide !== wide || !dialog) return;
    if (currentPanel && !isWide && !dialog.open) dialog.showModal();
    else if ((!currentPanel || isWide) && dialog.open) {
      dialog.close();
      if (currentPanel && isWide) inspector?.focus({ preventScroll: true });
    }
  }

  function closeOnEscape(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    closePanel();
  }
</script>

<svelte:window bind:innerWidth={viewportWidth} onkeydown={(event) => {
  if (panel && event.target instanceof Node && (inspector?.contains(event.target) || dialog?.contains(event.target))) closeOnEscape(event);
}} />

{#snippet context()}
  <header class="inspector-heading">
    <h2 id={`${contextId}-title`}>{t(panel === 'sources' ? 'reader-concept-sources' : 'reader-page-details')}</h2>
    <button class="inspector-close" onclick={closePanel} aria-label={t('action-close')}><X size={18} aria-hidden="true" /></button>
  </header>
  {#if panel === 'sources'}
    {#if sources === null}
      <p class="inspector-note">{t('reader-sources-unavailable')}</p>
    {:else if sources.length === 0}
      <p class="inspector-note">{t('reader-sources-empty')}</p>
    {:else}
      <p class="inspector-note">{t('reader-sources-scope')}</p>
      <ul class="reader-sources">
        {#each sources as source, index (index)}
          <li>
            <h3>{source.title ?? source.id ?? t('desktop-concept-source-unnamed')}</h3>
            <dl>
              {#if source.resource}<div><dt>{t('reader-source-resource')}</dt><dd>{source.resource}</dd></div>{/if}
              {#if source.author}<div><dt>{t('reader-source-author')}</dt><dd>{source.author}</dd></div>{/if}
              {#if source.lastModified}<div><dt>{t('reader-source-date')}</dt><dd>{source.lastModified}</dd></div>{/if}
            </dl>
          </li>
        {/each}
      </ul>
    {/if}
  {:else}
    {@render details()}
  {/if}
{/snippet}

<div class="reading-workspace" class:with-inspector={wide && panel !== null}>
  <article class="file-preview reader-article">
    <header class="reader-heading">
      <div class="reader-tools">
        {#if showSources}<button aria-expanded={panel === 'sources'} aria-controls={contextId} onclick={(event) => openPanel('sources', event)}><Library size={15} aria-hidden="true" />{t('reader-sources')}{#if sources !== null}<span>{sources.length}</span>{/if}</button>{/if}
        <button aria-expanded={panel === 'details'} aria-controls={contextId} onclick={(event) => openPanel('details', event)}><Info size={15} aria-hidden="true" />{t('desktop-details')}</button>
      </div>
      <h1 tabindex="-1">{title}</h1>
      {#if status}<div class="concept-reading-status">{@render status()}</div>{/if}
      {#if actions && hasActions}<div class="reader-page-actions">{@render actions()}</div>{/if}
    </header>
    {#if warnings}{@render warnings()}{/if}
    {#if truncated}<p class="evidence-warning"><AlertTriangle size={15} aria-hidden="true" />{t('knowledge-page-truncated')}</p>{/if}
    <div class="knowledge-blocks">
      {#each blocks as block, index (index)}
        {#if block.kind === 'heading'}{#if index !== 0 || block.text.trim() !== title.trim()}<svelte:element this={block.level > 2 ? 'h3' : 'h2'}>{block.text}</svelte:element>{/if}
        {:else if block.kind === 'paragraph'}<p>{block.text}</p>
        {:else if block.kind === 'listItem'}<div class="safe-list-item"><span>{block.ordered ? '—' : '•'}</span><p>{block.text}</p></div>
        {:else if block.kind === 'code'}<pre><code>{block.text}</code></pre>
        {:else if block.kind === 'quote'}<blockquote>{block.text}</blockquote>
        {:else}<hr />{/if}
      {/each}
    </div>
    {#if related && hasRelated}<footer class="reader-related">{@render related()}</footer>{/if}
  </article>
  {#if wide && panel}
    <aside class="reader-inspector" id={contextId} aria-labelledby={`${contextId}-title`} tabindex="-1" bind:this={inspector}>
      {@render context()}
    </aside>
  {/if}
</div>
<dialog class="reader-dialog" id={wide && panel ? undefined : contextId} aria-labelledby={`${contextId}-title`} bind:this={dialog} onclose={() => { if (!wide && panel) closePanel(); }}>
  {#if !wide && panel}{@render context()}{/if}
</dialog>

<style>
  .reading-workspace { display: grid; grid-template-columns: minmax(0, 1fr); align-items: start; min-width: 0; }
  .reading-workspace.with-inspector { grid-template-columns: minmax(0, 1fr) 280px; gap: var(--space-6); }
  .reader-article { width: 100%; max-width: calc(var(--reading-width) + 64px); margin-inline: auto; padding: 32px; }
  .reader-article > .reader-heading { display: flex; flex-direction: column; align-items: stretch; gap: 14px; padding: 0 0 20px; border: 0; }
  .reader-heading h1 { margin: 0; font: 600 clamp(27px, 2.6vw, 36px)/1.2 var(--font-display); letter-spacing: -.04em; overflow-wrap: anywhere; }
  .reader-tools { display: flex; justify-content: flex-end; gap: 4px; margin: -12px -8px 8px; font-family: var(--font-ui); }
  .reader-tools button { display: inline-flex; align-items: center; gap: 6px; min-height: 30px; padding: 5px 8px; color: var(--muted); background: transparent; border: 1px solid transparent; border-radius: var(--control-radius); font: 500 12px var(--font-ui); cursor: pointer; }
  .reader-tools button:hover, .reader-tools button[aria-expanded="true"] { color: var(--strong); background: var(--surface-raised); }
  .reader-tools span { font-variant-numeric: tabular-nums; }
  .concept-reading-status { margin: 0; }
  .reader-page-actions { display: flex; gap: 8px; }
  .knowledge-blocks { max-width: var(--reading-width); }
  .knowledge-blocks :global(h2) { margin: 28px 0 12px; font: 600 21px/1.35 var(--font-display); letter-spacing: -.02em; }
  .knowledge-blocks :global(h3) { margin: 24px 0 12px; font: 600 18px/1.4 var(--font-display); }
  .reader-related { margin-top: 36px; padding-top: 20px; border-top: 1px solid var(--line); }
  .reader-inspector { position: sticky; top: 76px; min-width: 0; max-height: calc(100vh - 160px); overflow: auto; padding: 20px 0 20px 20px; border-left: 1px solid var(--line); font-family: var(--font-ui); }
  .reader-dialog { width: min(440px, calc(100vw - 48px)); max-height: calc(100vh - 64px); box-sizing: border-box; margin: auto; padding: 24px; color: var(--strong); background: var(--slate); border: 1px solid var(--line); border-radius: var(--panel-radius); font-family: var(--font-ui); }
  .reader-dialog::backdrop { background: var(--overlay); }
  .inspector-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 20px; padding: 0; border: 0; }
  .inspector-heading h2 { margin: 0; font: 600 16px/1.3 var(--font-ui); }
  .inspector-close { display: grid; flex: none; place-items: center; width: 30px; height: 30px; padding: 0; color: var(--muted); background: transparent; border: 1px solid transparent; border-radius: var(--control-radius); cursor: pointer; }
  .inspector-close:hover { background: var(--surface-raised); color: var(--strong); }
  .inspector-note { color: var(--muted); font-size: 12px; line-height: 1.5; }
  .reader-sources { display: grid; gap: 20px; padding: 0; margin: 24px 0 0; list-style: none; }
  .reader-sources li + li { padding-top: 20px; border-top: 1px solid var(--line); }
  .reader-sources h3 { margin: 0 0 12px; font: 600 13px/1.5 var(--font-ui); overflow-wrap: anywhere; }
  dl { display: grid; gap: 10px; margin: 0; font-size: 12px; line-height: 1.4; }
  dt { margin-bottom: 3px; color: var(--muted); }
  dd { margin: 0; overflow-wrap: anywhere; white-space: pre-wrap; }
  @media (max-width: 1100px) { .reader-article { padding: 24px 16px; } }
  @media (forced-colors: active) { .reader-tools button[aria-expanded="true"] { border-color: Highlight; } }
</style>
