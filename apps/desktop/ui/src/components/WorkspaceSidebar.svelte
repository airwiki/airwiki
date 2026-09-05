<script lang="ts">
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import Library from '@lucide/svelte/icons/library';
  import Inbox from '@lucide/svelte/icons/inbox';
  import Plus from '@lucide/svelte/icons/plus';
  import type { Snippet } from 'svelte';
  import type { AppSnapshot, WikiSummary } from '../api';
  import type { MessageArgs } from '../i18n';
  import SystemStatusButton from '../SystemStatusButton.svelte';
  import WikiIcon from './WikiIcon.svelte';

  export let snapshot: AppSnapshot;
  export let wikis: WikiSummary[];
  export let destination: 'library' | 'review' | 'settings';
  export let wikiId: string | null;
  export let contextLabel: string | null = null;
  export let contextKey: string;
  export let context: Snippet;
  export let t: (id: string, args?: MessageArgs) => string;
  export let onlibrary: () => void;
  export let onreview: () => void;
  export let onsettings: () => void;
  export let onwiki: (id: string) => void;
  export let oncreate: () => void;
  export let newWikiMenuOpen: boolean;

  let choosingWiki = false;
  const navigationContext: { key: string | null } = { key: null };
  $: if (contextKey !== navigationContext.key) {
    navigationContext.key = contextKey;
    choosingWiki = false;
  }
  $: reviewCount = snapshot.reviews.filter((review) => !review.excluded).length;

  function chooseWiki(id: string) {
    choosingWiki = false;
    onwiki(id);
  }
</script>

<aside class="workspace-sidebar" aria-label={t('desktop-sidebar-navigation')}>
  <nav class="workspace-destinations" aria-label={t('desktop-sidebar-navigation')}>
    <button class:active={destination === 'library' && !contextLabel} aria-current={destination === 'library' && !contextLabel ? 'page' : undefined} onclick={onlibrary}><Library size={17} aria-hidden="true" /><span>{t('desktop-library-title')}</span></button>
    <button class:active={destination === 'review'} aria-current={destination === 'review' ? 'page' : undefined} onclick={onreview}><Inbox size={17} aria-hidden="true" /><span>{t('desktop-review-queue-title')}</span>{#if reviewCount > 0}<small>{reviewCount}</small>{/if}</button>
  </nav>
  <div class="sidebar-context">
    {#if destination === 'settings'}
      {@render context()}
    {:else}
      <button class="wiki-picker" aria-expanded={choosingWiki || !contextLabel} onclick={() => { choosingWiki = !choosingWiki; }} disabled={!contextLabel}>
        <WikiIcon size={20} /><strong>{contextLabel ?? t('desktop-sidebar-wikis')}</strong>{#if contextLabel}<ChevronDown size={15} aria-hidden="true" />{/if}
      </button>
      {#if choosingWiki || !contextLabel}
        <nav class="sidebar-wikis" aria-label={t('desktop-sidebar-wikis')}>
          {#each wikis as wiki (wiki.id)}
            <button class:active={wiki.id === wikiId} aria-current={wiki.id === wikiId ? 'page' : undefined} onclick={() => chooseWiki(wiki.id)} title={wiki.name}><WikiIcon size={16} /><span>{wiki.name}</span>{#if wiki.needsReviewCount > 0}<small>{wiki.needsReviewCount}</small>{/if}</button>
          {/each}
        </nav>
      {:else}
        {@render context()}
      {/if}
    {/if}
  </div>
  <footer>
    <button class="new-wiki-command" aria-label={t('desktop-new-wiki')} aria-haspopup="dialog" aria-expanded={newWikiMenuOpen} onclick={oncreate}><Plus size={17} aria-hidden="true" /><span>{t('desktop-new-wiki')}</span></button>
    <SystemStatusButton {snapshot} {t} onclick={onsettings} />
  </footer>
</aside>

<style>
  .workspace-sidebar { display: flex; flex-direction: column; gap: 20px; height: 100%; min-height: 0; padding: 16px 12px 12px; font-family: var(--font-ui); }
  nav { display: grid; align-content: start; gap: 3px; }
  button { display: flex; align-items: center; gap: 10px; width: 100%; min-height: 36px; padding: 8px; color: var(--muted); background: transparent; border: 1px solid transparent; border-radius: var(--control-radius); font-size: var(--font-size-ui); text-align: left; cursor: pointer; }
  button:hover, button.active { color: var(--strong); background: var(--nav-active); }
  button.active { box-shadow: inset 2px 0 0 var(--cyan); }
  button > span { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  button > :global(svg) { flex: none; }
  small { flex: none; font-size: 11px; font-variant-numeric: tabular-nums; }
  .sidebar-context { display: flex; flex: 1; flex-direction: column; min-height: 0; }
  .wiki-picker { flex: none; margin-bottom: 8px; color: var(--strong); }
  .wiki-picker strong { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .wiki-picker:disabled { opacity: 1; cursor: default; }
  .sidebar-wikis { min-height: 0; overflow: auto; padding: 4px; margin: -4px; scrollbar-gutter: stable; }
  footer { display: grid; gap: 3px; padding-top: 8px; border-top: 1px solid var(--line); }
  @media (forced-colors: active) { button.active { border-color: Highlight; } }
</style>
