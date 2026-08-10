<script lang="ts">
  import BookOpen from '@lucide/svelte/icons/book-open';
  import Home from '@lucide/svelte/icons/house';
  import Plus from '@lucide/svelte/icons/plus';
  import Settings2 from '@lucide/svelte/icons/settings-2';
  import Share2 from '@lucide/svelte/icons/share-2';

  type Destination = 'home' | 'wikis' | 'shared' | 'search' | 'system';

  export let destination: Destination;
  export let status: string;
  export let platform: 'macOs' | 'windows';
  export let t: (id: string) => string;
  export let onselect: (destination: Destination) => void;
  export let oncreate: () => void;

  const destinations = [
    { id: 'home', labelId: 'desktop-nav-home', icon: Home, shortcut: '1' },
    { id: 'wikis', labelId: 'desktop-nav-wikis', icon: BookOpen, shortcut: '2' },
    { id: 'shared', labelId: 'desktop-nav-shared', icon: Share2, shortcut: '3' }
  ] as const;
</script>

<aside class="rail" aria-label={t('nav-group-knowledge')}>
  <div class="brand"><span class="brand-mark" aria-hidden="true">A</span><span>AirWiki</span></div>
  <button class="new-wiki-button" onclick={oncreate}><Plus size={18} aria-hidden="true" />{t('desktop-new-wiki')}</button>
  <nav>
    {#each destinations as item (item.id)}
      <button class:active={destination === item.id} aria-current={destination === item.id ? 'page' : undefined} onclick={() => onselect(item.id)}>
        <item.icon size={18} strokeWidth={1.8} aria-hidden="true" />
        <span>{t(item.labelId)}</span>
        <kbd aria-hidden="true">{platform === 'macOs' ? '⌘' : 'Ctrl+'}{item.shortcut}</kbd>
      </button>
    {/each}
  </nav>
  <div class="rail-footer">
    <button class:active={destination === 'system'} onclick={() => onselect('system')}><Settings2 size={18} aria-hidden="true" /><span>{t('desktop-nav-system')}</span></button>
    <div class="device-state"><span class="status-dot" aria-hidden="true"></span><div><strong>{t('nav-device-status')}</strong><small>{status}</small></div></div>
  </div>
</aside>
