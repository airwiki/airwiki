<script lang="ts">
  import Search from '@lucide/svelte/icons/search';

  export let question: string;
  export let includePublic: boolean;
  export let busy: boolean;
  export let platform: 'macOs' | 'windows';
  export let t: (id: string) => string;
  export let onquestion: (value: string) => void;
  export let onpublic: (value: boolean) => void;
  export let onsearch: () => void;
  export let onopen: () => void;
</script>

<form class="global-search" role="search" onsubmit={(event) => { event.preventDefault(); onsearch(); }}>
  <Search size={18} aria-hidden="true" />
  <label class="sr-only" for="global-search">{t('desktop-search-question')}</label>
  <input id="global-search" value={question} oninput={(event) => onquestion(event.currentTarget.value)} onfocus={onopen} maxlength="4096" placeholder={t('desktop-search-placeholder')} />
  <label class="search-scope"><input type="checkbox" checked={includePublic} onchange={(event) => onpublic(event.currentTarget.checked)} />{t('desktop-search-public-short')}</label>
  <kbd aria-hidden="true">{platform === 'macOs' ? '⌘K' : 'Ctrl+K'}</kbd>
  <button type="submit" aria-label={t('desktop-search-evidence')} disabled={busy || !question.trim()}><Search size={17} aria-hidden="true" /></button>
</form>
