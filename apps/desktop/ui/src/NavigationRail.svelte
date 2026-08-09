<script lang="ts">
  import BookOpen from '@lucide/svelte/icons/book-open';
  import Share2 from '@lucide/svelte/icons/share-2';
  import Settings2 from '@lucide/svelte/icons/settings-2';

  type Destination = 'library' | 'review' | 'search' | 'share' | 'system';

  export let destination: Destination;
  export let status: string;
  export let platform: 'macOs' | 'windows';
  export let t: (id: string) => string;
  export let onselect: (destination: Destination) => void;

  const destinations = [
    { id: 'library', labelId: 'desktop-nav-library', icon: BookOpen, shortcut: '1' },
    { id: 'share', labelId: 'desktop-nav-share', icon: Share2, shortcut: '2' },
    { id: 'system', labelId: 'desktop-nav-system', icon: Settings2, shortcut: '3' }
  ] as const;
</script>

<aside class="rail" aria-label={t('nav-group-knowledge')}>
  <div class="brand"><span class="brand-mark" aria-hidden="true">A</span><span>AirWiki</span></div>
  <nav>
    {#each destinations as item (item.id)}
      <button class:active={destination === item.id} aria-current={destination === item.id ? 'page' : undefined} onclick={() => onselect(item.id)}>
        <item.icon size={18} strokeWidth={1.8} aria-hidden="true" />
        <span>{t(item.labelId)}</span>
        <kbd aria-hidden="true">{platform === 'macOs' ? '⌘' : 'Ctrl+'}{item.shortcut}</kbd>
      </button>
    {/each}
  </nav>
  <div class="device-state">
    <span class="status-dot" aria-hidden="true"></span>
    <div><strong>{t('nav-device-status')}</strong><small>{status}</small></div>
  </div>
</aside>
