<script lang="ts">
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import ArrowRight from '@lucide/svelte/icons/arrow-right';
  import Check from '@lucide/svelte/icons/check';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import { tick } from 'svelte';
  import type { AppSnapshot, FolderSelection, LocalePreference } from './api';
  import Checkbox from './components/controls/Checkbox.svelte';
  import SelectField from './components/controls/SelectField.svelte';
  import Switch from './components/controls/Switch.svelte';
  import TextField from './components/controls/TextField.svelte';
  import { message } from './i18n';

  export let snapshot: AppSnapshot;
  export let locale: LocalePreference;
  export let modelLicensesConfirmed: boolean;
  export let actionBusy: boolean;
  export let actionMessage: string;
  export let onpickfolder: () => Promise<FolderSelection | null>;
  export let oncreatewiki: (name: string, token: string, continuous: boolean) => Promise<void>;
  export let onprepare: () => void;
  export let onopenmodelsettings: () => void | Promise<void>;
  export let onfinish: () => void;

  const existingFolderWiki = snapshot.wikis.find((wiki) => wiki.origin === 'folder');
  let step = 0;
  let stepIndexes: number[];
  let heading: HTMLHeadingElement | null = null;
  let folderSelection: FolderSelection | null = null;
  let folderName = existingFolderWiki?.name ?? '';
  let continuousIndexing = true;
  let folderBusy = false;
  let folderCreated = existingFolderWiki !== undefined;
  let folderSkipped = false;
  let folderError = '';
  $: hasModelStep = Boolean(snapshot.model && !snapshot.model.active);
  $: modelPreparing = snapshot.modelInstall !== null;
  $: modelUnavailable = !snapshot.model?.active && snapshot.hardware?.canInstall === false;
  $: modelSetupNeeded = !snapshot.model?.active && !modelUnavailable;
  $: reviewStep = hasModelStep ? 3 : 2;
  $: totalSteps = reviewStep + 1;
  $: stepIndexes = Array.from({ length: totalSteps }, (_, index) => index);

  function t(id: string, args?: Record<string, string>): string {
    return message(locale, id, args);
  }

  async function moveTo(nextStep: number) {
    step = nextStep;
    await tick();
    heading?.focus({ preventScroll: true });
  }

  function next() {
    if (step < reviewStep) void moveTo(step + 1);
  }

  function back() {
    if (step > 0) void moveTo(step - 1);
  }

  async function chooseFolder() {
    folderBusy = true;
    folderError = '';
    try {
      folderSelection = await onpickfolder();
      if (folderSelection) {
        folderName = folderSelection.displayName || t('onboarding-default-folder-name');
        folderSkipped = false;
      }
    } catch {
      folderError = t('error-collection');
    } finally {
      folderBusy = false;
    }
  }

  async function createFolderWiki() {
    if (!folderSelection || !folderName.trim()) return;
    folderBusy = true;
    folderError = '';
    try {
      await oncreatewiki(folderName.trim(), folderSelection.token, continuousIndexing);
      folderCreated = true;
      folderSelection = null;
    } catch {
      folderError = t('error-collection');
    } finally {
      folderBusy = false;
    }
  }

  function skipFolder() {
    folderSelection = null;
    folderSkipped = true;
    folderError = '';
  }

  function modelInstallLabel(): string {
    const labels: Record<string, string> = {
      queued: 'models-install-queued',
      downloading: 'models-install-downloading',
      verifying: 'models-install-verifying',
      extracting: 'models-install-extracting',
      activating: 'models-install-activating'
    };
    return t(labels[snapshot.modelInstall?.status ?? ''] ?? 'models-install-activating');
  }
</script>

<main class="onboarding" data-step={step}>
  <div class="onboarding-chrome">
    <div class="onboarding-mark" aria-hidden="true">A</div>
    <div
      class="step-meter"
      role="progressbar"
      aria-label={t('onboarding-progress-title')}
      aria-valuemin="1"
      aria-valuemax={totalSteps}
      aria-valuenow={step + 1}
      aria-valuetext={`${step + 1} / ${totalSteps}`}
    >
      {#each stepIndexes as index (index)}
        <i class:complete={index < step} class:current={index === step} aria-hidden="true"></i>
      {/each}
    </div>
    <span aria-hidden="true">{step + 1} / {totalSteps}</span>
  </div>

  <div class="onboarding-stage">
    {#key step}
      <section class="onboarding-page" aria-labelledby={`onboarding-step-${step}`}>
        {#if step === 0}
          <p class="eyebrow">{t('onboarding-welcome-title')}</p>
          <h1 id={`onboarding-step-${step}`} tabindex="-1" bind:this={heading}>{t('settings-language')}</h1>
          <p class="lede">{t('onboarding-welcome-body')}</p>
          <div class="choice-field"><SelectField label={t('settings-language')} bind:value={locale} options={[{ value: 'system', label: t('language-system') }, { value: 'es', label: t('language-spanish') }, { value: 'en', label: t('language-english') }]} /></div>
          <p class="privacy-note">{t('onboarding-privacy-local')}</p>
        {:else if step === 1}
          <p class="eyebrow">{t('onboarding-privacy-title')}</p>
          <h1 id={`onboarding-step-${step}`} tabindex="-1" bind:this={heading}>{t('onboarding-collection-title')}</h1>
          <p class="lede">{t('onboarding-collection-body')}</p>
          {#if folderCreated}
            <div class="onboarding-folder-success" role="status"><Check size={18} aria-hidden="true" /><span><strong>{t('onboarding-collection-linked')}</strong><small>{folderName}</small></span></div>
          {:else if folderSelection}
            <div class="onboarding-folder-form">
              <div class="onboarding-folder-selection"><FolderOpen size={18} aria-hidden="true" /><span><strong>{folderSelection.displayName}</strong><small>{t('desktop-folder-privacy')}</small></span></div>
              <TextField label={t('desktop-wiki-name')} bind:value={folderName} maxlength={120} required />
              <Switch label={t('desktop-continuous-indexing')} description={t('desktop-continuous-indexing-body')} bind:checked={continuousIndexing} />
              <div class="row-actions"><button class="text-action" onclick={chooseFolder} disabled={folderBusy}>{t('onboarding-review-choose-folder')}</button><button class="primary" onclick={createFolderWiki} disabled={folderBusy || !folderName.trim()}>{t('desktop-create-wiki')}</button></div>
            </div>
          {:else}
            <div class="onboarding-folder-choice">
              <button class="primary" onclick={chooseFolder} disabled={folderBusy}><FolderOpen size={17} aria-hidden="true" />{t('collections-choose-folder')}</button>
              <button class="text-action" onclick={skipFolder}>{t('onboarding-skip-folder')}</button>
              <small>{t('onboarding-skip-folder-help')}</small>
            </div>
          {/if}
          {#if folderSkipped}<p class="privacy-note" role="status">{t('onboarding-skip-folder-help')}</p>{/if}
          {#if folderError}<p class="onboarding-inline-error" role="alert">{folderError}</p>{/if}
        {:else if step === 2 && hasModelStep && snapshot.model}
          <p class="eyebrow">{t('component-local-ai')}</p>
          <h1 id={`onboarding-step-${step}`} tabindex="-1" bind:this={heading}>{t('onboarding-model-title')}</h1>
          <p class="lede">{t('onboarding-model-body')}</p>
          <p class="onboarding-model-summary"><strong>{snapshot.model.displayName ?? t('onboarding-model-recommended')}</strong><span>{(snapshot.model.downloadBytes / 1073741824).toFixed(1)} GiB</span></p>
          <div class="license-choice"><Checkbox label={t('models-accept-licenses')} description={snapshot.model.license ?? t('models-license')} bind:checked={modelLicensesConfirmed} /></div>
          {#if snapshot.modelInstall}
            <div class="onboarding-model-install" role="status" aria-live="polite">
              <strong>{modelInstallLabel()}</strong>
              {#if snapshot.modelInstall.status === 'downloading' && snapshot.modelInstall.totalBytes > 0}
                <progress aria-label={modelInstallLabel()} max={snapshot.modelInstall.totalBytes} value={snapshot.modelInstall.downloaded}></progress>
                <small>{t('models-install-progress', { downloaded: `${(snapshot.modelInstall.downloaded / 1073741824).toFixed(1)} GiB`, total: `${(snapshot.modelInstall.totalBytes / 1073741824).toFixed(1)} GiB` })}</small>
              {:else}
                <small>{t(snapshot.modelInstall.status === 'queued' ? 'models-install-queued-detail' : 'models-install-phase-detail')}</small>
              {/if}
            </div>
          {:else}
            <div class="row-actions"><button class="secondary onboarding-model" onclick={onprepare} disabled={actionBusy || (!modelLicensesConfirmed && !snapshot.model.licenseAccepted) || !snapshot.model.fitsAvailableDisk}>{t(actionMessage ? 'onboarding-model-retry' : 'primary-button-prepare')}</button><small>{t('onboarding-model-change-later')}</small></div>
          {/if}
          {#if actionMessage}<p class="onboarding-inline-error" role="alert">{actionMessage}</p>{/if}
        {:else}
          <p class="eyebrow">{t('onboarding-review-title')}</p>
          <h1 id={`onboarding-step-${step}`} tabindex="-1" bind:this={heading}>{t('onboarding-complete-title')}</h1>
          <p class="lede">{folderCreated ? t('onboarding-complete-with-wiki') : t('onboarding-complete-without-wiki')}</p>
          <dl class="onboarding-summary">
            <div><dt>{t('settings-language')}</dt><dd><Check size={16} aria-hidden="true" />{locale === 'system' ? t('language-system') : locale === 'es' ? t('language-spanish') : t('language-english')}</dd></div>
            <div><dt>{t('desktop-page-wikis-title')}</dt><dd><Check size={16} aria-hidden="true" />{folderCreated ? folderName : t('onboarding-summary-wiki-later')}</dd></div>
            <div><dt>{t('component-local-ai')}</dt><dd class:needs-action={modelSetupNeeded}><Check size={16} aria-hidden="true" />{snapshot.model?.active ? t('status-ready') : modelUnavailable ? t('onboarding-summary-ai-unavailable') : modelPreparing ? t('onboarding-summary-ai-preparing') : t('onboarding-summary-ai-setup-needed')}</dd></div>
          </dl>
          {#if modelSetupNeeded || modelUnavailable}
            <aside class="onboarding-next-step" aria-labelledby="onboarding-next-step-title">
              <strong id="onboarding-next-step-title">{t(modelUnavailable ? 'onboarding-model-unavailable-title' : 'onboarding-next-step-title')}</strong>
              <p>{t(modelUnavailable ? 'onboarding-model-unavailable-body' : modelPreparing ? 'onboarding-next-step-preparing' : 'onboarding-next-step-search')}</p>
              {#if modelSetupNeeded}<button class="secondary" onclick={onopenmodelsettings} disabled={actionBusy}>{t('onboarding-open-local-ai')}</button>{/if}
            </aside>
          {/if}
          {#if actionMessage}<p class="onboarding-inline-error" role="alert">{actionMessage}</p>{/if}
          <p class="privacy-note">{t('onboarding-complete-body')}</p>
        {/if}
      </section>
    {/key}
  </div>

  <footer class="onboarding-actions">
    <button class="text-action onboarding-back" onclick={back} disabled={step === 0}><ArrowLeft size={16} aria-hidden="true" />{t('onboarding-back')}</button>
    {#if step < reviewStep}
      <button class="primary onboarding-next" onclick={next} disabled={step === 1 && !folderCreated && !folderSkipped}>{t('onboarding-next')}<ArrowRight size={16} aria-hidden="true" /></button>
    {:else}
      <button class="primary onboarding-action" onclick={onfinish} disabled={actionBusy}>{t('onboarding-finish')}<Check size={16} aria-hidden="true" /></button>
    {/if}
  </footer>
</main>
