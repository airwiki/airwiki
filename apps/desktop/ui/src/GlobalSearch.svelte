<script lang="ts">
  import Search from '@lucide/svelte/icons/search';

  export let question: string;
  export let includePublic: boolean;
  export let busy: boolean;
  export let platform: 'macOs' | 'windows';
  export let t: (id: string) => string;
  export let onsearch: () => void;
  export let onopen: () => void;
</script>

<form class="global-search" role="search" onsubmit={(event) => { event.preventDefault(); onsearch(); }}>
  <Search size={17} strokeWidth={1.8} aria-hidden="true" />
  <label class="sr-only" for="global-knowledge-search">{t('desktop-search-question')}</label>
  <input
    id="global-knowledge-search"
    bind:value={question}
    maxlength="4096"
    placeholder={t('desktop-global-search-placeholder')}
    onfocus={onopen}
    required
  />
  <label class="search-scope" title={t('desktop-search-include-public')}>
    <input type="checkbox" bind:checked={includePublic} />
    <span>{t('desktop-search-scope-public')}</span>
  </label>
  <kbd aria-hidden="true">{platform === 'macOs' ? '⌘K' : 'Ctrl+K'}</kbd>
  <button class="search-submit" aria-label={t('desktop-search-evidence')} disabled={busy || !question.trim()}>
    <Search size={16} aria-hidden="true" />
  </button>
</form>
