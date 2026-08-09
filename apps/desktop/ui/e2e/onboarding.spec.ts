import { $, $$, browser, expect } from '@wdio/globals';

function required<T>(value: T | undefined, label: string): T {
  if (value === undefined) throw new Error(`missing ${label}`);
  return value;
}

async function selectValue(selector: string, index: number, value: string): Promise<void> {
  const changed = await browser.execute((selectSelector, selectIndex, nextValue) => {
    const element = document.querySelectorAll<HTMLSelectElement>(selectSelector).item(selectIndex);
    if (!element) return false;
    element.value = nextValue;
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
    return element.value === nextValue;
  }, selector, index, value);
  expect(changed).toBe(true);
}

describe('AirWiki real IPC journey', () => {
  it('persists onboarding and explicit appearance preferences', async () => {
    await browser.waitUntil(
      () => browser.execute(() => Boolean(document.querySelector('main.onboarding:not(.startup)'))),
      { timeout: 30_000, timeoutMsg: 'the runtime did not reach interactive onboarding' }
    );
    const onboarding = await $('main.onboarding:not(.startup)');
    await expect(onboarding).toBeDisplayed();

    const onboardingSelects = await $$('main.onboarding:not(.startup) select');
    const language = required(onboardingSelects[0], 'language preference');
    const localNetwork = required(onboardingSelects[1], 'local network preference');
    const closeBehavior = required(onboardingSelects[2], 'close behavior preference');
    await selectValue('main.onboarding:not(.startup) select', 0, 'en');
    await selectValue('main.onboarding:not(.startup) select', 1, 'disabled');
    await selectValue('main.onboarding:not(.startup) select', 2, 'ask');
    await expect(language).toHaveValue('en');
    await expect(localNetwork).toHaveValue('disabled');
    await expect(closeBehavior).toHaveValue('ask');
    const finishOnboarding = await $('main.onboarding:not(.startup) button.onboarding-action');
    await expect(finishOnboarding).toBeEnabled();
    await finishOnboarding.click();

    const shell = await $('.shell');
    try {
      await shell.waitForDisplayed({ timeout: 30_000 });
    } catch (error) {
      const message = await $('.action-message');
      const detail = await message.isExisting() ? await message.getText() : 'no UI error';
      throw new Error(`onboarding did not complete: ${detail}`, { cause: error });
    }
    for (const destination of ['Library', 'Review', 'Search', 'System']) {
      await expect($(`button*=${destination}`)).toBeDisplayed();
    }

    await $('button*=System').click();
    await $('#system-preferences').waitForDisplayed();
    const preferenceSelects = await $$('#system-preferences select');
    const appearance = required(preferenceSelects[1], 'appearance preference');
    await selectValue('#system-preferences select', 1, 'dark');
    await expect(appearance).toHaveValue('dark');
    await $('#system-preferences button.primary').click();
    await browser.waitUntil(async () => (
      await $('html').getAttribute('data-theme') === 'dark'
    ), { timeout: 10_000, timeoutMsg: 'the persisted theme was not applied' });

    const route = await browser.getUrl();
    expect(route.endsWith('#system/preferences')).toBe(true);
    const layout = await browser.execute(() => ({
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth
    }));
    expect(layout.scrollWidth).toBeLessThanOrEqual(layout.clientWidth);
  });
});
