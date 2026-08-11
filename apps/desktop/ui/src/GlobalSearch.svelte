<script lang="ts">
  import Search from '@lucide/svelte/icons/search';

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
  <label class="sr-only" for="global-search">{t('desktop-search-question')}</label>
  <input id="global-search" aria-describedby={!ready ? 'global-search-readiness' : undefined} value={question} oninput={(event) => onquestion(event.currentTarget.value)} onfocus={onopen} maxlength="4096" placeholder={ready ? t('desktop-search-placeholder') : t('desktop-search-preparing-placeholder')} />
  {#if !ready}<span id="global-search-readiness" class="sr-only">{t('desktop-search-preparing-body')}</span>{/if}
  <label class="search-scope"><input type="checkbox" checked={includePublic} onchange={(event) => onpublic(event.currentTarget.checked)} />{t('desktop-search-public-short')}</label>
  <kbd aria-hidden="true">{platform === 'macOs' ? '⌘K' : 'Ctrl+K'}</kbd>
  <button type="submit" aria-label={ready ? t('desktop-search-evidence') : t('desktop-search-preparing-title')} title={!ready ? t('desktop-search-preparing-title') : undefined} disabled={busy || !ready || !question.trim()}><Search size={17} aria-hidden="true" /></button>
</form>
