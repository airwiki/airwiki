<script lang="ts">
  import BookOpen from '@lucide/svelte/icons/book-open';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ShimmerText from './components/ShimmerText.svelte';
  import Spinner from './components/Spinner.svelte';
  import type { WikiSummary, WikiScanSummary } from './api';
  import type { MessageArgs } from './i18n';
  import { wikiRequiresAttention } from './wikiHealth';

  export let wikis: WikiSummary[];
  export let scans: WikiScanSummary[];
  export let t: (id: string, args?: MessageArgs) => string;
  export let onopen: (wikiId: string) => void;
  export let oncreate: () => void;

  function scanState(wikiId: string) {
    return scans.find((scan) => scan.wikiId === wikiId)?.state ?? null;
  }

  function accessLabel(wiki: WikiSummary): string {
    const channels = [wiki.peerShareable ? t('desktop-share-nearby') : '', wiki.allowExternalAi ? t('desktop-share-ai-apps') : '', wiki.internetPublic ? t('desktop-share-public') : ''].filter(Boolean);
    return channels.length ? channels.join(' · ') : t('desktop-wiki-private');
  }

  function originLabel(wiki: WikiSummary): string {
    if (wiki.origin === 'importedOkf') return t('desktop-wiki-origin-imported');
    if (wiki.memoryKind === 'project') return t('desktop-wiki-origin-project-memory');
    if (wiki.origin === 'aiMemory') return t('desktop-wiki-origin-personal-memory');
    return wiki.indexingMode === 'continuous'
      ? t('desktop-wiki-origin-folder-continuous')
      : t('desktop-wiki-origin-folder-manual');
  }

  function trustLabel(wiki: WikiSummary): string {
    if (wiki.okfCompatibility.kind === 'legacyV01') return t('desktop-okf-compatibility-legacyV01');
    if (wiki.okfCompatibility.kind === 'futureRestricted') return t('desktop-okf-local-only');
    if (wiki.outdatedVerificationCount > 0) return t('desktop-assurance-outdated');
    if (wiki.staleConceptCount > 0) return t('desktop-freshness-stale');
    if (wiki.metadataWarningCount > 0) return t('desktop-okf-metadata-warning');
    return t(`desktop-trust-${wiki.trustSummary}`);
  }

  function statusLabel(wiki: WikiSummary): string {
    if (scanState(wiki.id)) return t('status-working');
    if (wiki.memoryKind === 'project' && wiki.projectMemoryHealth !== 'active') {
      return t(`desktop-project-memory-health-${wiki.projectMemoryHealth ?? 'invalid'}`);
    }
    if (wiki.failedCount > 0 || wiki.maintenanceRequired) return t('status-needs-attention');
    if (wiki.needsReviewCount > 0) return t('desktop-wiki-pending-status');
    return trustLabel(wiki);
  }

  function statusDetail(wiki: WikiSummary): string {
    if (scanState(wiki.id)) return t('desktop-wiki-row-checking');
    if (wiki.memoryKind === 'project' && wiki.projectMemoryHealth !== 'active') {
      return t('desktop-wiki-row-project-blocked');
    }
    if (wiki.failedCount > 0) return t('desktop-wiki-row-failed-count', { count: wiki.failedCount });
    if (wiki.maintenanceRequired) return t('desktop-wiki-maintenance-required');
    if (wiki.needsReviewCount > 0) return t('desktop-wiki-row-review-count', { count: wiki.needsReviewCount });
    if (wiki.staleConceptCount > 0 || wiki.outdatedVerificationCount > 0 || wiki.metadataWarningCount > 0) {
      return t('desktop-okf-status-summary', {
        stale: wiki.staleConceptCount,
        outdated: wiki.outdatedVerificationCount,
        warnings: wiki.metadataWarningCount
      });
    }
    return t(`desktop-okf-compatibility-${wiki.okfCompatibility.kind}`);
  }

  function folioTone(wiki: WikiSummary): 'folder' | 'project' | 'personal' | 'imported' {
    if (wiki.memoryKind === 'project') return 'project';
    if (wiki.origin === 'aiMemory') return 'personal';
    if (wiki.origin === 'importedOkf') return 'imported';
    return 'folder';
  }

  function rowLabel(wiki: WikiSummary): string {
    const identity = `${wiki.name} ${t('desktop-wiki-content-count', { published: wiki.publishedCount, pending: wiki.needsReviewCount })}`;
    return [
      identity,
      accessLabel(wiki),
      statusLabel(wiki),
      statusDetail(wiki),
      originLabel(wiki),
    ].join(' · ');
  }
</script>

<div class="wiki-table">
  {#if wikis.length > 0}
    <div class="wiki-table-head" aria-hidden="true">
      <span>{t('desktop-wiki-column-name')}</span>
      <span>{t('desktop-wiki-column-content')}</span>
      <span>{t('desktop-wiki-column-access')}</span>
      <span>{t('desktop-wiki-column-status')}</span>
      <span aria-hidden="true"></span>
    </div>
  {/if}
  <div class="wiki-table-list" role="list" aria-label={t('desktop-library-title')}>
    {#each wikis as wiki (wiki.id)}
      {@const scanning = scanState(wiki.id) !== null}
      <div class="wiki-row-item" role="listitem">
        <button class={`wiki-row folio-${folioTone(wiki)}`} aria-busy={scanning} aria-label={rowLabel(wiki)} onclick={() => onopen(wiki.id)}>
          <span class="wiki-name"><span class="wiki-icon"><BookOpen size={18} aria-hidden="true" /></span><span><strong>{wiki.name}</strong><small>{originLabel(wiki)}</small></span></span>
          <span class="wiki-data-cell"><span>{t('desktop-wiki-content-count', { published: wiki.publishedCount, pending: wiki.needsReviewCount })}</span><small>{t('desktop-wiki-source-count', { count: wiki.documentCount })}</small></span>
          <span>{accessLabel(wiki)}</span>
          <span class:attention={!scanning && wikiRequiresAttention(wiki)} class:working={scanning} class="wiki-status wiki-data-cell"><span>{#if scanning}<Spinner size="small" /><ShimmerText text={statusLabel(wiki)} />{:else}<i class="wiki-status-signal" aria-hidden="true"></i>{statusLabel(wiki)}{/if}</span><small>{statusDetail(wiki)}</small></span>
          <ChevronRight size={17} aria-hidden="true" />
        </button>
      </div>
    {:else}
      <div class="table-empty"><BookOpen size={28} aria-hidden="true" /><strong>{t('desktop-wiki-empty-title')}</strong><p>{t('desktop-wiki-empty-body')}</p><button class="primary" onclick={oncreate}>{t('desktop-wiki-empty-action')}</button></div>
    {/each}
  </div>
</div>
