<script lang="ts">
  import Search from '@lucide/svelte/icons/search';
  import Checkbox from './components/controls/Checkbox.svelte';
  import TextField from './components/controls/TextField.svelte';

  export let question: string;
  export let includePublic: boolean;
  export let busy: boolean;
  export let ready: boolean;
  export let platform: 'macOs' | 'windows';
  export let t: (id: string) => string;
  export let onquestion: (value: string) => void;
  export let onpublic: (value: boolean) => void;
  export let onsearch: () => void;
  export let onopen: () => void;
</script>

<form class:preparing={!ready} class="global-search" role="search" onsubmit={(event) => { event.preventDefault(); if (ready) onsearch(); }}>
  <Search size={18} aria-hidden="true" />
  <TextField id="global-search" label={t('desktop-search-question')} value={question} oninput={onquestion} onfocus={onopen} maxlength={4096} describedby={!ready ? 'global-search-readiness' : undefined} placeholder={ready ? t('desktop-search-placeholder') : t('desktop-search-preparing-placeholder')} variant="search" />
  {#if !ready}<span id="global-search-readiness" class="sr-only">{t('desktop-search-preparing-body')}</span>{/if}
  <div class="search-scope"><Checkbox label={t('desktop-search-public-short')} checked={includePublic} onchange={onpublic} compact /></div>
  <kbd aria-hidden="true">{platform === 'macOs' ? '⌘K' : 'Ctrl+K'}</kbd>
  <button type="submit" aria-label={ready ? t('desktop-search-evidence') : t('desktop-search-preparing-title')} title={!ready ? t('desktop-search-preparing-title') : undefined} disabled={busy || !ready || !question.trim()}><Search size={17} aria-hidden="true" /></button>
</form>
