<script lang="ts">
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import ArrowRight from '@lucide/svelte/icons/arrow-right';
  import Check from '@lucide/svelte/icons/check';
  import type { AppSnapshot, CloseBehavior, LanPreference, LocalePreference } from './api';
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
          <label class="choice-field"><span>{t('settings-language')}</span><select bind:value={locale}><option value="system">{t('language-system')}</option><option value="es">{t('language-spanish')}</option><option value="en">{t('language-english')}</option></select></label>
        {:else if step === 1}
          <p class="eyebrow">{t('onboarding-privacy-title')}</p>
          <h1>{t('onboarding-lan-title')}</h1>
          <p class="lede">{t('onboarding-lan-body')}</p>
          <label class="choice-field"><span>{t('desktop-lan')}</span><select bind:value={lanPreference}><option value="disabled">{t('onboarding-lan-disable')}</option><option value="enabled">{t('onboarding-lan-enable')}</option></select></label>
          <p class="privacy-note">{t('onboarding-privacy-local')}</p>
        {:else if step === 2}
          <p class="eyebrow">{t('desktop-sign-in')}</p>
          <h1>{t('onboarding-background-title')}</h1>
          <p class="lede">{t('onboarding-background-body')}</p>
          <label class="choice-field"><span>{t('desktop-close')}</span><select bind:value={closeBehavior}><option value="ask">{t('close-dialog-title')}</option><option value="hide_to_tray">{t('close-dialog-background')}</option><option value="quit">{t('tray-quit')}</option></select></label>
        {:else if step === 3 && hasModelStep && snapshot.model}
          <p class="eyebrow">{t('component-local-ai')}</p>
          <h1>{t('onboarding-model-title')}</h1>
          <p class="lede">{snapshot.model.displayName ?? t('onboarding-model-recommended')} · {(snapshot.model.downloadBytes / 1073741824).toFixed(1)} GiB</p>
          <label class="license-choice"><input type="checkbox" bind:checked={modelLicensesConfirmed} /><span><strong>{t('models-accept-licenses')}</strong><small>{snapshot.model.license ?? t('models-license')}</small></span></label>
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
