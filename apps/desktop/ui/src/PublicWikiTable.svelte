<script lang="ts">
  import BookOpen from '@lucide/svelte/icons/book-open';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import type { PublicCatalogWikiSummary } from './api';
  import type { MessageArgs } from './i18n';

  export let wikis: PublicCatalogWikiSummary[];
  export let t: (id: string, args?: MessageArgs) => string;
  export let onopen: (wiki: PublicCatalogWikiSummary) => void;

  function metadata(wiki: PublicCatalogWikiSummary): string[] {
    const items = [t('desktop-public-wiki-concept-count', { count: wiki.conceptCount })];
    if (wiki.languages.length > 0) items.push(wiki.languages.join(', '));
    if (wiki.okfCompatibility) items.push(t(`desktop-okf-compatibility-${wiki.okfCompatibility.kind}`));
    return items;
  }

  function rowLabel(wiki: PublicCatalogWikiSummary): string {
    return [
      wiki.name,
      wiki.description,
      ...metadata(wiki),
      t('desktop-public-network'),
      t('desktop-open-wiki')
    ].filter(Boolean).join(' · ');
  }
</script>

<div class="public-wiki-list" role="list" aria-label={t('desktop-public-wiki-list-title')}>
  {#each wikis as wiki (`${wiki.publisherId}:${wiki.wikiId}`)}
    <div role="listitem">
      <button class="public-wiki-row" aria-label={rowLabel(wiki)} onclick={() => onopen(wiki)}>
        <span class="public-wiki-identity">
          <span class="wiki-icon"><BookOpen size={17} aria-hidden="true" /></span>
          <span>
            <strong>{wiki.name}</strong>
            <small>{wiki.description || t('desktop-public-wiki-no-description')}</small>
          </span>
        </span>
        <span class="public-wiki-metadata" aria-hidden="true">
          {#each metadata(wiki) as item, index (index)}<span>{item}</span>{/each}
        </span>
        <ChevronRight size={16} aria-hidden="true" />
      </button>
    </div>
  {/each}
</div>
