<script lang="ts">
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Info from '@lucide/svelte/icons/info';
  import type { SearchHitSummary, WikiSearchResultSummary, HostPlatform } from '../api';
  import type { MessageArgs } from '../i18n';
  import WikiIcon from './WikiIcon.svelte';
  import DeviceIdentity from './identity/DeviceIdentity.svelte';

  export let result: WikiSearchResultSummary;
  export let resultKey: string;
  export let wikiName: string;
  export let ownerName: string;
  export let scopeLabel: string;
  export let platform: HostPlatform | null;
  export let platformLabel: string;
  export let disabled = false;
  export let t: (id: string, args?: MessageArgs) => string;
  export let assuranceLabel: (hit: SearchHitSummary) => string | null;
  export let onopen: () => void;
  export let onhit: (hit: SearchHitSummary) => void;

  $: warnings = [
    result.source.kind === 'local' && result.source.health !== 'ready' ? t(`desktop-search-health-${result.source.health}`) : null,
    result.source.kind === 'nearby' && !result.source.accessGranted ? t('desktop-search-access-unavailable') : null,
    result.source.kind === 'nearby' && !result.source.available ? t('desktop-search-offline') : null,
    result.okfCompatibility?.kind === 'futureRestricted' ? t('desktop-okf-compatibility-futureRestricted') : null
  ].filter((warning): warning is string => warning !== null);
</script>

<article class="wiki-search-group" aria-label={wikiName} data-search-result={resultKey}>
  <header class="search-group-heading">
    <button class="search-group-wiki" onclick={onopen} {disabled} aria-label={`${t('desktop-open-wiki')}: ${wikiName}`} title={wikiName}><WikiIcon size={16} /><span>{wikiName}</span></button>
    <div class="search-result-origin">{#if result.source.kind !== 'local'}<span>{scopeLabel}</span>{/if}<DeviceIdentity name={ownerName} {platform} {platformLabel} source={result.source.kind === 'public' ? 'public' : 'device'} compact /></div>
    <span class="search-match-count">{t('desktop-search-match-count', { count: result.totalMatches })}</span>
  </header>
  {#if warnings.length > 0}<div class="search-group-warning" role="status"><AlertTriangle size={15} aria-hidden="true" /><span>{warnings.join(' · ')}</span></div>{/if}
  <div class="wiki-search-matches">
    {#each result.matches as hit (hit.conceptId)}
      <section class="search-match">
        <h2><button data-search-concept={hit.conceptId} onclick={() => onhit(hit)} {disabled}><span>{hit.title}</span><ChevronRight size={18} aria-hidden="true" /></button></h2>
        <p class="search-match-snippet">{hit.snippet}</p>
        <div class="citation-row">
          {#if hit.headingOrPage && hit.headingOrPage !== hit.title}<span>{hit.headingOrPage}</span>{/if}
          <span>{t('search-revision', { revision: hit.sourceRevision })}</span>
          {#if assuranceLabel(hit)}<span>{assuranceLabel(hit)}</span>{/if}
        </div>
      </section>
    {/each}
  </div>
  <details class="wiki-search-details">
    <summary><Info size={13} aria-hidden="true" />{t('reader-wiki-details')}</summary>
    <div class="search-group-details">
      {#if result.description}<p>{result.description}</p>{/if}
      <ul>
        {#if result.source.kind === 'local'}<li>{t(result.source.private ? 'desktop-wiki-private' : 'desktop-wiki-shared')}</li><li>{t(`desktop-search-health-${result.source.health}`)}</li>{/if}
        {#if result.source.kind === 'nearby'}<li>{t(result.source.accessGranted ? 'desktop-search-access-granted' : 'desktop-search-access-unavailable')}</li><li>{t(result.source.available ? 'desktop-search-available' : 'desktop-search-offline')}</li>{/if}
        {#if result.source.kind === 'public' && result.languages.length > 0}<li>{result.languages.join(', ')}</li>{/if}
        {#if result.conceptCount !== null}<li>{t('desktop-search-concept-count', { count: result.conceptCount })}</li>{/if}
        <li>{result.okfCompatibility ? t(`desktop-okf-compatibility-${result.okfCompatibility.kind}`) : t('desktop-public-format-unavailable')}</li>
      </ul>
    </div>
  </details>
</article>

<style>
  .wiki-search-group { min-width: 0; padding: 18px 0 20px; border-top: 1px solid var(--line); }
  .search-group-heading { display: flex; align-items: center; flex-wrap: wrap; justify-content: flex-start; gap: 8px 18px; padding: 0; border: 0; color: var(--muted); font: 400 12px/1.5 var(--font-ui); }
  .search-group-wiki { display: inline-flex; align-items: center; gap: 8px; min-width: 0; max-width: min(42%, 380px); min-height: 30px; padding: 3px 0; color: var(--strong); background: transparent; border: 0; font: 600 12px/1.5 var(--font-ui); text-align: left; cursor: pointer; }
  .search-group-wiki span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .search-group-wiki:hover { color: var(--cyan); }
  .search-group-wiki :global(svg) { flex: none; }
  .search-result-origin { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 12px; min-width: 0; }
  .search-match-count { margin-left: auto; font-variant-numeric: tabular-nums; }
  .search-group-warning { display: flex; align-items: center; gap: 8px; margin: 12px 0; color: var(--amber); font: 400 12px/1.5 var(--font-ui); }
  .search-group-warning :global(svg) { flex: none; }
  .wiki-search-matches { display: grid; gap: 20px; padding-top: 12px; }
  .search-match { min-width: 0; }
  .search-match h2 { max-width: none; margin: 0; }
  .search-match h2 button { display: inline-flex; align-items: center; gap: 12px; min-height: 32px; max-width: 100%; padding: 0; color: var(--strong); background: transparent; border: 0; font: 600 22px/1.3 var(--font-display); letter-spacing: -.025em; text-align: left; cursor: pointer; }
  .search-match h2 button span { overflow-wrap: anywhere; }
  .search-match h2 button :global(svg) { flex: none; color: var(--muted); }
  .search-match h2 button:hover { color: var(--cyan); }
  .search-match-snippet { max-width: 76ch; margin: 7px 0 10px; color: var(--body-copy); font: 400 16px/1.6 var(--font-reading); overflow-wrap: anywhere; white-space: pre-wrap; }
  .citation-row { display: flex; align-items: baseline; justify-content: flex-start; flex-wrap: wrap; gap: 4px 14px; margin: 0; color: var(--muted); font: 400 12px/1.5 var(--font-ui); overflow-wrap: anywhere; }
  .citation-row span { font: inherit; }
  .wiki-search-details { margin-top: 12px; font: 400 12px/1.5 var(--font-ui); }
  .wiki-search-details summary { display: flex; align-items: center; gap: 6px; width: fit-content; min-height: 28px; color: var(--muted); cursor: pointer; list-style: none; }
  .wiki-search-details summary::-webkit-details-marker { display: none; }
  .wiki-search-details summary:hover, .wiki-search-details[open] summary { color: var(--strong); }
  .search-group-details { max-width: 76ch; margin-top: 8px; padding-left: 14px; border-left: 2px solid var(--line); color: var(--muted); overflow-wrap: anywhere; }
  .search-group-details p { margin: 0 0 8px; }
  .search-group-details ul { display: flex; flex-wrap: wrap; gap: 6px 18px; margin: 0; padding: 0; list-style: none; }
  @media (max-width: 1100px) { .search-group-wiki { max-width: 100%; } .search-match-count { margin-left: 0; } }
</style>
