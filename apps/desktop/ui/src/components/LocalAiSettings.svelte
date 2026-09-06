<script lang="ts">
  import type { AppSnapshot } from '../api';
  import type { MessageArgs } from '../i18n';
  import type { SystemStatusItem } from '../systemStatus';
  import SelectField from './controls/SelectField.svelte';
  import Spinner from './Spinner.svelte';

  export let model: AppSnapshot['model'];
  export let installation: AppSnapshot['modelInstall'];
  export let status: SystemStatusItem | undefined;
  export let ready: boolean;
  export let restartPending: boolean;
  export let canInstall: boolean;
  export let busy: boolean;
  export let installLabel: string;
  export let t: (id: string, args?: MessageArgs) => string;
  export let formatBytes: (bytes: number) => string;
  export let onprofile: (profile: string) => void;
  export let onprepare: () => void;
  export let oncancel: () => void;
  export let onlicense: () => void;
</script>

<section class="local-ai-settings" aria-labelledby="local-ai-settings-title">
  <header class="local-ai-heading">
    <h2 id="local-ai-settings-title">{t('settings-local-ai-title')}</h2>
    <span class={`local-ai-status ${status?.tone ?? 'off'}`} role="status">{status?.detail ?? t('desktop-model-needs-setup-short')}</span>
  </header>
  <p class="local-ai-introduction">{t(ready ? 'settings-local-ai-ready-body' : 'settings-local-ai-body')}</p>
  <div class="local-model-control">
    <div class="local-model-summary">
      <span>{t('settings-model-selected')}</span>
      <strong>{model?.displayName ?? t('settings-model-none')}</strong>
      {#if !ready && !restartPending && !installation && model}<small>{t(model.installed ? 'settings-local-model-installed' : 'settings-local-model-download-needed')}</small>{/if}
      {#if model}
        <div class="local-model-metadata">
          {#if !model.installed && model.downloadBytes > 0}<span>{t('settings-model-download-size', { size: formatBytes(model.downloadBytes) })}</span>{/if}
          {#if !model.installed && model.requiredFreeBytes > 0}<span>{t('settings-model-space-needed', { size: formatBytes(model.requiredFreeBytes) })}</span>{/if}
          {#if model.license}<span>{t('models-license', { name: model.license })}</span>{/if}
        </div>
      {/if}
    </div>
    <SelectField label={t('settings-model-selector')} value={model?.profile ?? 'automatic'} onchange={onprofile} options={[
      { value: 'automatic', label: t('settings-model-profile-automatic') },
      { value: 'efficient', label: t('settings-model-profile-efficient') },
      { value: 'quality', label: t('settings-model-profile-quality') }
    ]} disabled={busy || installation !== null} />
  </div>
  {#if model?.degraded}<p class="local-ai-warning">{t('settings-model-compatible-fallback')}</p>{/if}
  {#if model?.fitsAvailableDisk === false}<p class="local-ai-warning" role="alert">{t('settings-model-insufficient-space')}</p>{/if}
  {#if !canInstall && !ready}<p class="local-ai-warning">{t('onboarding-model-unavailable-body')}</p>{/if}
  {#if (model?.issues.length ?? 0) > 0 && !ready && !installation}<p class="local-ai-warning" role="alert">{t('settings-local-ai-attention')}</p>{/if}
  {#if restartPending}<p class="local-ai-warning">{t('settings-local-ai-restart-body')}</p>{/if}

  {#if installation}
    <div class="model-install-state">
      {#if installation.status === 'downloading' && installation.totalBytes > 0}
        <progress aria-label={installLabel} max={installation.totalBytes} value={installation.downloaded}></progress>
        <div class="model-install-copy" role="status" aria-live="polite"><strong>{installLabel}</strong><small>{t('models-install-progress', { downloaded: formatBytes(installation.downloaded), total: formatBytes(installation.totalBytes) })}</small></div>
      {:else}
        <div class="model-install-wait" role="status" aria-live="polite"><Spinner size="small" /><div><strong>{installLabel}</strong><small>{t(installation.status === 'queued' ? 'models-install-queued-detail' : 'models-install-phase-detail')}</small></div></div>
      {/if}
      <button class="secondary" onclick={oncancel} disabled={busy}>{t(installation.status === 'queued' ? 'models-cancel-request' : 'action-cancel')}</button>
    </div>
  {:else}
    <div class="local-model-actions">
      {#if !ready && !restartPending}<button class="primary" onclick={onprepare} disabled={busy || !canInstall || model?.fitsAvailableDisk === false}>{t('models-install')}</button>{/if}
      {#if model?.licenseUrl}<button class="text-action" onclick={onlicense}>{t('models-license-open')}</button>{/if}
    </div>
  {/if}

  <details class="local-ai-explanation">
    <summary>{t('settings-local-ai-explanation')}</summary>
    <p>{t('settings-model-selector-help')} {t('settings-model-profile-explanation')}</p>
    <dl>
      <div><dt>{t('settings-local-ai-drafts-title')}</dt><dd>{t('settings-local-ai-drafts-body')}</dd></div>
      <div><dt>{t('settings-local-ai-search-title')}</dt><dd>{t('settings-local-ai-search-body')}</dd></div>
      <div><dt>{t('settings-local-ai-control-title')}</dt><dd>{t('settings-local-ai-control-body')}</dd></div>
    </dl>
  </details>
</section>

<style>
  .local-ai-heading { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 8px 20px; padding: 0; border: 0; }
  h2 { margin: 0; }
  .local-ai-status { color: var(--muted); font: 500 12px/1.5 var(--font-ui); }
  .local-ai-status.ready { color: var(--verify); }
  .local-ai-status.warning { color: var(--amber); }
  .local-ai-status.failed { color: var(--coral); }
  .local-ai-status.working { color: var(--cyan); }
  .local-ai-introduction { margin: 10px 0 24px; max-width: 68ch; }
  .local-model-control { display: grid; grid-template-columns: minmax(0, 1fr) minmax(180px, .75fr); gap: 24px; align-items: start; }
  .local-model-summary { display: grid; gap: 7px; min-width: 0; }
  .local-model-summary > span, .local-model-summary > small { color: var(--muted); font: 400 12px/1.45 var(--font-ui); }
  .local-model-summary > strong { font: 600 19px/1.35 var(--font-display); overflow-wrap: anywhere; }
  .local-model-metadata { display: flex; flex-wrap: wrap; gap: 5px 18px; color: var(--muted); font: 400 12px/1.5 var(--font-ui); }
  .local-model-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 16px; margin-top: 20px; }
  .local-ai-warning { margin: 14px 0 0; color: var(--amber); font: 400 13px/1.55 var(--font-ui); }
  .local-ai-explanation { margin-top: 22px; color: var(--muted); font: 400 13px/1.55 var(--font-ui); }
  .local-ai-explanation summary { width: fit-content; max-width: 100%; cursor: pointer; }
  .local-ai-explanation p { max-width: 68ch; margin: 16px 0; }
  .local-ai-explanation dl { display: grid; gap: 14px; margin: 16px 0 0; }
  .local-ai-explanation dl > div { display: grid; grid-template-columns: 150px minmax(0, 1fr); gap: 16px; }
  .local-ai-explanation dt { color: var(--strong); font-weight: 500; }
  .local-ai-explanation dd { margin: 0; }
  .model-install-state { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: center; gap: 12px 20px; margin-top: 20px; padding: 18px 0; border-block: 1px solid var(--line); }
  .model-install-state progress { grid-column: 1 / -1; height: 6px; margin: 0; }
  .model-install-copy strong, .model-install-copy small, .model-install-wait strong, .model-install-wait small { display: block; }
  .model-install-copy small, .model-install-wait small { margin-top: 4px; color: var(--muted); font: 400 12px/1.5 var(--font-ui); }
  .model-install-wait { display: flex; align-items: center; gap: 12px; min-width: 0; }
  @media (max-width: 1100px) {
    .local-model-control { grid-template-columns: minmax(0, 1fr); gap: 18px; }
    .local-ai-explanation dl > div { grid-template-columns: minmax(0, 1fr); gap: 3px; }
  }
</style>
