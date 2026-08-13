<script lang="ts">
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import ArrowRight from '@lucide/svelte/icons/arrow-right';
  import Check from '@lucide/svelte/icons/check';
  import type { AppSnapshot, CloseBehavior, LanPreference, LocalePreference } from './api';
  import Checkbox from './components/controls/Checkbox.svelte';
  import SelectField from './components/controls/SelectField.svelte';
  import { message } from './i18n';

  export let snapshot: AppSnapshot;
  export let locale: LocalePreference;
  export let lanPreference: LanPreference;
  export let closeBehavior: CloseBehavior;
  export let modelLicensesConfirmed: boolean;
  export let actionBusy: boolean;
  export let actionMessage: string;
  export let onprepare: () => void;
  export let onfinish: () => void;

  let step = 0;
  let stepIndexes: number[];
  $: hasModelStep = Boolean(snapshot.model && !snapshot.model.active);
  $: reviewStep = hasModelStep ? 4 : 3;
  $: totalSteps = reviewStep + 1;
  $: stepIndexes = Array.from({ length: totalSteps }, (_, index) => index);
  function t(id: string): string {
    return message(locale, id);
  }

  function next() {
    if (step < reviewStep) step += 1;
  }

  function back() {
    if (step > 0) step -= 1;
  }
</script>

<main class="onboarding" data-step={step}>
  <div class="onboarding-chrome">
    <div class="onboarding-mark" aria-hidden="true">A</div>
    <div class="step-meter" aria-label={t('onboarding-progress-title')}>
      {#each stepIndexes as index (index)}
        <i class:complete={index < step} class:current={index === step}></i>
      {/each}
    </div>
    <span>{step + 1} / {totalSteps}</span>
  </div>

  <div class="onboarding-stage">
    {#key step}
      <section class="onboarding-page">
        {#if step === 0}
          <p class="eyebrow">{t('onboarding-welcome-title')}</p>
          <h1>{t('settings-language')}</h1>
          <p class="lede">{t('settings-subtitle')}</p>
          <div class="choice-field"><SelectField label={t('settings-language')} bind:value={locale} options={[{ value: 'system', label: t('language-system') }, { value: 'es', label: t('language-spanish') }, { value: 'en', label: t('language-english') }]} /></div>
        {:else if step === 1}
          <p class="eyebrow">{t('onboarding-privacy-title')}</p>
          <h1>{t('onboarding-lan-title')}</h1>
          <p class="lede">{t('onboarding-lan-body')}</p>
          <div class="choice-field"><SelectField label={t('desktop-lan')} bind:value={lanPreference} options={[{ value: 'disabled', label: t('onboarding-lan-disable') }, { value: 'enabled', label: t('onboarding-lan-enable') }]} /></div>
          <p class="privacy-note">{t('onboarding-privacy-local')}</p>
        {:else if step === 2}
          <p class="eyebrow">{t('desktop-sign-in')}</p>
          <h1>{t('onboarding-background-title')}</h1>
          <p class="lede">{t('onboarding-background-body')}</p>
          <div class="choice-field"><SelectField label={t('desktop-close')} bind:value={closeBehavior} options={[{ value: 'ask', label: t('close-dialog-title') }, { value: 'hide_to_tray', label: t('close-dialog-background') }, { value: 'quit', label: t('tray-quit') }]} /></div>
        {:else if step === 3 && hasModelStep && snapshot.model}
          <p class="eyebrow">{t('component-local-ai')}</p>
          <h1>{t('onboarding-model-title')}</h1>
          <p class="lede">{snapshot.model.displayName ?? t('onboarding-model-recommended')} · {(snapshot.model.downloadBytes / 1073741824).toFixed(1)} GiB</p>
          <div class="license-choice"><Checkbox label={t('models-accept-licenses')} description={snapshot.model.license ?? t('models-license')} bind:checked={modelLicensesConfirmed} /></div>
          <button class="secondary onboarding-model" onclick={onprepare} disabled={actionBusy || (!modelLicensesConfirmed && !snapshot.model.licenseAccepted) || !snapshot.model.fitsAvailableDisk}>{t('primary-button-prepare')}</button>
        {:else}
          <p class="eyebrow">{t('onboarding-review-title')}</p>
          <h1>{t('onboarding-finish')}</h1>
          <p class="lede">{t('onboarding-welcome-body')}</p>
          <dl class="onboarding-summary">
            <div><dt>{t('settings-language')}</dt><dd><Check size={16} />{locale === 'system' ? t('language-system') : locale === 'es' ? t('language-spanish') : t('language-english')}</dd></div>
            <div><dt>{t('desktop-lan')}</dt><dd><Check size={16} />{lanPreference === 'enabled' ? t('desktop-enabled') : t('desktop-disabled')}</dd></div>
            <div><dt>{t('desktop-close')}</dt><dd><Check size={16} />{closeBehavior === 'ask' ? t('desktop-ask') : closeBehavior === 'hide_to_tray' ? t('desktop-hide-tray') : t('desktop-quit')}</dd></div>
          </dl>
        {/if}
      </section>
    {/key}
  </div>

  <footer class="onboarding-actions">
    <button class="text-action onboarding-back" onclick={back} disabled={step === 0}><ArrowLeft size={16} />{t('onboarding-back')}</button>
    {#if step < reviewStep}
      <button class="primary onboarding-next" onclick={next} disabled={step === 1 && lanPreference === 'undecided'}>{t('onboarding-next')}<ArrowRight size={16} /></button>
    {:else}
      <button class="primary onboarding-action" onclick={onfinish} disabled={actionBusy || lanPreference === 'undecided'}>{t('onboarding-finish')}<Check size={16} /></button>
    {/if}
  </footer>
  {#if actionMessage}<p class="action-message" aria-live="polite">{actionMessage}</p>{/if}
</main>
