<script lang="ts">
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import type { AppSnapshot, EnrichmentDraft, ReviewSummary } from '../api';
  import type { MessageArgs } from '../i18n';
  import Spinner from './Spinner.svelte';
  import TextField from './controls/TextField.svelte';

  export let review: ReviewSummary;
  export let draft: EnrichmentDraft;
  export let evidence: AppSnapshot['reviewEvidence'];
  export let evidenceReady: boolean;
  export let evidenceLoading: boolean;
  export let evidenceLoadingMore: boolean;
  export let evidenceIssue: string;
  export let evidenceCanRetry: boolean;
  export let readOnly: boolean;
  export let updating: boolean;
  export let stale: boolean;
  export let canReload: boolean;
  export let busy: boolean;
  export let error: boolean;
  export let dirty: boolean;
  export let completed: number;
  export let remaining: number;
  export let t: (id: string, args?: MessageArgs) => string;
  export let onedit: (value: EnrichmentDraft) => void;
  export let onback: () => void;
  export let onreload: () => void;
  export let onretry: () => void;
  export let onmore: () => void;
  export let ondecide: (decision: 'approve' | 'reject') => void;
  export let scrollRegion: HTMLElement | null = null;
  let panel: 'evidence' | 'proposal' = 'proposal';
</script>

<section class="review-workspace" aria-label={t('review-workspace-label')}>
  <div class="review-content" bind:this={scrollRegion}>
  <div class="review-content-inner">
  <header class="review-workspace-heading">
    <div class="review-toolbar">
    <button class="text-action" onclick={onback} disabled={busy}><ArrowLeft size={16} aria-hidden="true" />{t('review-back-queue')}</button>
    <p class="review-progress">{t('review-session-progress', { completed, remaining })}</p>
    </div>
    <p class="section-label">{review.wikiName} · {t(review.excluded ? 'desktop-review-state-excluded' : 'desktop-review-state-draft')}</p>
    <h1 tabindex="-1">{review.draft.title}</h1>
    <p class="review-source-name">{review.sourceName} · {t('review-revision', { revision: review.sourceRevision })}</p>
  </header>
  <p class="review-introduction">{t(review.excluded ? 'review-excluded-body' : 'review-draft-body')}</p>
  {#if stale && !busy}<div class="review-notice" role="alert"><AlertTriangle size={18} aria-hidden="true" /><div><strong>{t('review-changed-title')}</strong><p>{t('review-changed-body')}</p>{#if canReload}<button class="text-action" onclick={onreload}>{t('review-open-current')}</button>{/if}</div></div>{/if}
  {#if error}<p class="review-notice" role="alert"><AlertTriangle size={18} aria-hidden="true" />{t('review-decision-failed')}</p>{/if}
  {#if updating}<p class="review-notice" role="status"><Spinner size="small" />{t('desktop-journey-knowledge-reanalyzing')}</p>{/if}
  {#if readOnly}<p class="review-notice" role="status"><AlertTriangle size={18} aria-hidden="true" />{t('review-okf-read-only')}</p>{/if}
  <div class="review-view-switch" role="group" aria-label={t('review-comparison-view')}>
    <button aria-pressed={panel === 'evidence'} aria-controls="review-evidence" onclick={() => panel = 'evidence'}>{t('desktop-evidence')}</button>
    <button aria-pressed={panel === 'proposal'} aria-controls="review-proposal" onclick={() => panel = 'proposal'}>{t('desktop-proposal')}{#if dirty}<span> · {t('review-edited')}</span>{/if}</button>
  </div>
  <div class="review-columns" data-panel={panel}>
    <section id="review-evidence" aria-labelledby="review-evidence-title">
      <h2 id="review-evidence-title">{t('desktop-evidence')}</h2>
      {#if evidenceLoading}
        <p class="review-local-progress" role="status"><Spinner size="small" />{t('review-evidence-loading')}</p>
      {:else if evidenceReady && evidence}
        <div class="review-excerpts">{#each evidence.excerpts as excerpt (excerpt.ordinal)}<blockquote><p class="review-excerpt-label">{excerpt.headingOrPage}</p><p>{excerpt.text}</p>{#if excerpt.truncated}<small>{t('review-excerpt-truncated')}</small>{/if}</blockquote>{/each}</div>
        {#if evidenceLoadingMore}<p class="review-local-progress" role="status"><Spinner size="small" />{t('review-evidence-loading')}</p>{:else if evidence.nextOrdinal != null}<button class="text-action" onclick={onmore} disabled={busy}>{t('desktop-load-more')}</button>{/if}
      {:else}<p class="evidence-warning">{evidenceIssue}</p>{/if}
      {#if evidenceCanRetry && !evidenceLoading}<button class="text-action" onclick={onretry} disabled={busy}>{t('review-evidence-retry')}</button>{/if}
    </section>
    <section id="review-proposal" aria-labelledby="review-proposal-title">
      <h2 id="review-proposal-title">{t('desktop-proposal')}</h2>
      <TextField label={t('review-edit-title')} value={draft.title} oninput={(title) => onedit({ ...draft, title })} maxlength={200} disabled={readOnly || updating || busy || stale} />
      <TextField label={t('review-edit-summary')} value={draft.summary} oninput={(summary) => onedit({ ...draft, summary })} maxlength={2000} rows={12} multiline disabled={readOnly || updating || busy || stale} />
      {#if dirty}<p class="review-edit-state">{t('review-edits-unsaved')}</p>{/if}
    </section>
  </div>
  </div>
  </div>
  <footer class="review-actions">
    <div class="review-actions-inner">
    <p class="review-publish-scope">{t('review-publish-scope')}</p>
    <div class="review-secondary-actions"><button class="secondary" onclick={onback} disabled={busy}>{t('review-later')}</button>{#if !review.excluded}<button class="secondary" onclick={() => ondecide('reject')} disabled={busy || updating || stale || !evidenceReady || evidenceLoadingMore}>{t('review-exclude')}</button>{/if}</div>
    <button class="primary" onclick={() => ondecide('approve')} disabled={busy || readOnly || updating || stale || !evidenceReady || evidenceLoadingMore}>{#if busy}<Spinner size="small" /><span role="status">{t('review-decision-saving')}</span>{:else}{t('review-approve-next')}{/if}</button>
    </div>
  </footer>
</section>

<style>
  .review-workspace { display: grid; grid-template-rows: minmax(0, 1fr) auto; flex: 1; min-height: 0; min-width: 0; }
  .review-content { min-height: 0; overflow: auto; overscroll-behavior: contain; padding: 20px 38px 24px; }
  .review-content-inner { container-type: inline-size; max-width: 1240px; margin: 0 auto; }
  .review-workspace-heading { display: block; padding: 0; border: 0; }
  .review-toolbar { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px 20px; margin-bottom: 12px; }
  .review-toolbar > button { display: inline-flex; align-items: center; gap: 6px; padding-inline: 0; }
  .section-label { margin-bottom: 4px; }
  h1 { margin: 4px 0; font: 650 clamp(24px, 2.4vw, 28px)/1.18 var(--font-display); overflow-wrap: anywhere; }
  .review-source-name, .review-progress { margin: 0; color: var(--muted); font: 400 12px/1.6 var(--font-ui); overflow-wrap: anywhere; }
  .review-source-name { margin-top: 6px; }
  .review-introduction { margin: 12px 0 16px; max-width: 84ch; }
  .review-notice { display: flex; align-items: flex-start; gap: 10px; padding: 14px 0; color: var(--amber); font: 400 13px/1.5 var(--font-ui); }
  .review-notice :global(svg) { flex-shrink: 0; }
  .review-notice p { margin: 4px 0 8px; }
  .review-columns { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 32px; border-top: 1px solid var(--line); }
  .review-columns > section { min-width: 0; padding: 18px 0; }
  h2 { margin: 0 0 12px; font: 600 17px/1.4 var(--font-display); }
  .review-excerpts blockquote { margin: 0 0 22px; padding: 0 0 0 16px; border-left: 2px solid var(--line); color: var(--strong); font: 400 16px/1.65 var(--font-reading); overflow-wrap: anywhere; }
  .review-excerpts p { margin: 0; white-space: pre-wrap; }
  .review-excerpts .review-excerpt-label, .review-excerpts small, .review-edit-state { margin: 0 0 6px; color: var(--muted); font: 400 12px/1.5 var(--font-ui); }
  .review-local-progress { display: flex; align-items: center; gap: 8px; font: 400 13px/1.5 var(--font-ui); }
  .review-view-switch { display: none; gap: 8px; margin: 0 0 16px; }
  .review-view-switch button { padding: 6px 12px; border: 1px solid transparent; border-radius: var(--control-radius); background: transparent; color: var(--muted); font: 500 13px var(--font-ui); cursor: pointer; }
  .review-view-switch button[aria-pressed='true'] { border-color: var(--line); background: var(--surface-raised); color: var(--strong); }
  .review-columns :global(.control-field + .control-field) { margin-top: 18px; }
  .review-publish-scope { flex-basis: 100%; margin: 0; color: var(--muted); font: 400 12px/1.5 var(--font-ui); }
  .review-edit-state { margin-top: 12px; }
  .review-actions { padding: 16px 38px 20px; border-top: 1px solid var(--line); background: var(--slate); }
  .review-actions-inner { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 12px; max-width: 1240px; margin: 0 auto; }
  @media (max-width: 1100px) {
    .review-content { padding: 20px 16px 24px; }
    .review-actions { padding: 14px 16px 16px; }
  }
  @container (max-width: 780px) {
    .review-view-switch { display: flex; flex-wrap: wrap; margin-bottom: 8px; }
    .review-columns { grid-template-columns: minmax(0, 1fr); }
    .review-columns > section { padding-top: 12px; }
    /* The active tab already names the panel; retain its heading for assistive navigation. */
    .review-columns h2 { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .review-columns :global(.control-field + .control-field) { margin-top: 12px; }
    .review-columns[data-panel='proposal'] #review-evidence, .review-columns[data-panel='evidence'] #review-proposal { display: none; }
  }
</style>
