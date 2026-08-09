<script lang="ts">
  import BookOpen from '@lucide/svelte/icons/book-open';
  import CheckCircle2 from '@lucide/svelte/icons/circle-check-big';
  import Files from '@lucide/svelte/icons/files';
  import Share2 from '@lucide/svelte/icons/share-2';
  import Sparkles from '@lucide/svelte/icons/sparkles';
  import type { AppSnapshot } from './api';

  type Destination = 'library' | 'review' | 'share' | 'system';

  export let snapshot: AppSnapshot;
  export let destination: 'library' | 'review' | 'search' | 'share' | 'system';
  export let t: (id: string) => string;
  export let onselect: (destination: Destination, section?: 'models') => void;

  $: documents = snapshot.collections.reduce((total, collection) => total + collection.documentCount, 0);
  $: published = snapshot.collections.reduce((total, collection) => total + collection.publishedCount, 0);
  $: shared = snapshot.collections.filter((collection) => collection.peerShareable || collection.allowExternalAi || collection.internetPublic).length;
</script>

<nav class="wiki-spine" aria-label={t('desktop-wiki-cycle')}>
  <button class:active={destination === 'library' && !snapshot.knowledge} onclick={() => onselect('library')}>
    <Files size={16} aria-hidden="true" />
    <span>{t('desktop-spine-sources')}</span>
    <strong>{documents}</strong>
  </button>
  <i aria-hidden="true"></i>
  <button class="ai-stage" onclick={() => onselect('system', 'models')}>
    <Sparkles size={16} aria-hidden="true" />
    <span>{t('settings-local-ai')}</span>
    <strong>{snapshot.model?.active ? t('desktop-spine-ai-ready') : t('desktop-spine-ai-off')}</strong>
  </button>
  <i aria-hidden="true"></i>
  <button class:active={destination === 'review'} class:attention={snapshot.reviews.length > 0} onclick={() => onselect('review')}>
    <CheckCircle2 size={16} aria-hidden="true" />
    <span>{t('desktop-spine-review')}</span>
    <strong>{snapshot.reviews.length}</strong>
  </button>
  <i aria-hidden="true"></i>
  <button class:active={destination === 'library' && Boolean(snapshot.knowledge)} onclick={() => onselect('library')}>
    <BookOpen size={16} aria-hidden="true" />
    <span>{t('desktop-spine-wiki')}</span>
    <strong>{published}</strong>
  </button>
  <i aria-hidden="true"></i>
  <button class:active={destination === 'share'} onclick={() => onselect('share')}>
    <Share2 size={16} aria-hidden="true" />
    <span>{t('desktop-nav-share')}</span>
    <strong>{shared}</strong>
  </button>
</nav>
