<script lang="ts">
  import Search from '@lucide/svelte/icons/search';
  import Spinner from './components/Spinner.svelte';
  import Checkbox from './components/controls/Checkbox.svelte';
  import TextField from './components/controls/TextField.svelte';
  import type { LocalSearchState } from './systemStatus';

  export let question: string;
  export let includePublic: boolean;
  export let busy: boolean;
  export let state: LocalSearchState;
  export let platform: 'macOs' | 'windows';
  export let privateScopeLabel: string;
  export let t: (id: string) => string;
  export let onquestion: (value: string) => void;
  export let oncompositionstart: () => void;
  export let oncompositionend: (value: string) => void;
  export let onpublic: (value: boolean) => void;
  export let onsearch: () => void;
  export let onopen: () => void;
  $: ready = state === 'ready';
  $: unavailableKey = state === 'preparing' ? 'desktop-search-preparing' : 'desktop-search-unavailable';
</script>

<form class:unavailable={!ready} class="global-search" role="search" onsubmit={(event) => { event.preventDefault(); if (ready && !busy) onsearch(); }}>
  <Search size={18} aria-hidden="true" />
  <TextField id="global-search" label={t('desktop-search-question')} value={question} oninput={onquestion} oncompositionstart={oncompositionstart} oncompositionend={oncompositionend} onfocus={onopen} maxlength={4096} describedby={!ready ? 'global-search-readiness' : undefined} placeholder={ready ? t('desktop-search-placeholder') : t(`${unavailableKey}-placeholder`)} variant="search" />
  {#if !ready}<span id="global-search-readiness" class="sr-only">{t(`${unavailableKey}-body`)}</span>{/if}
  <div class="search-scope" role="group" aria-label={t('desktop-search-scope-label')}>
    <span class="search-scope-base">{privateScopeLabel}</span>
    <Checkbox label={t('desktop-search-public-short')} checked={includePublic} onchange={onpublic} compact />
  </div>
  <kbd aria-hidden="true">{platform === 'macOs' ? '⌘K' : 'Ctrl+K'}</kbd>
  <button type="submit" aria-label={busy ? t('search-running') : ready ? t('desktop-search-evidence') : t(`${unavailableKey}-title`)} title={!ready ? t(`${unavailableKey}-title`) : undefined} disabled={busy || !ready || !question.trim()}>{#if busy}<Spinner size="small" />{:else}<Search size={17} aria-hidden="true" />{/if}</button>
</form>
