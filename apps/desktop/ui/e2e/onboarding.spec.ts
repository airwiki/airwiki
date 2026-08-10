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

async function measureNavigationPaintP95(): Promise<number> {
  const result = await browser.executeAsync((done) => {
    const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('.top-brand, .top-actions .icon-button'));
    if (buttons.length !== 2) {
      done([]);
      return;
    }
    const durations: number[] = [];
    let sample = 0;
    const measure = () => {
      const button = buttons[sample % buttons.length];
      if (!button) {
        done([]);
        return;
      }
      const started = performance.now();
      button.click();
      requestAnimationFrame(() => requestAnimationFrame(() => {
        durations.push(performance.now() - started);
        sample += 1;
        if (sample === 20) done(durations);
        else measure();
      }));
    };
    measure();
  });
  if (!Array.isArray(result) || !result.every((sample) => typeof sample === 'number')) {
    throw new Error('navigation paint measurement returned an invalid sample set');
  }
  const samples: number[] = result;
  expect(samples).toHaveLength(20);
  const ordered = [...samples].sort((left, right) => left - right);
  return required(ordered[Math.ceil(ordered.length * 0.95) - 1], 'navigation p95');
}

async function setCssViewport(width: number, height: number): Promise<void> {
  const ratio = await browser.execute(() => window.devicePixelRatio || 1);
  let physicalWidth = Math.ceil(width * ratio);
  const physicalHeight = Math.ceil(height * ratio);
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await browser.setWindowSize(physicalWidth, physicalHeight);
    const clientWidth = await browser.execute(() => document.documentElement.clientWidth);
    if (clientWidth >= width) return;
    physicalWidth += Math.ceil((width - clientWidth) * ratio);
  }
  throw new Error(`could not reach the ${width}x${height} CSS viewport`);
}

async function navigateToDestination(index: number): Promise<void> {
  const navigation = await $$('.top-brand, .top-actions .icon-button');
  await required(navigation[index], `destination ${index}`).click();
  const expected = ['wikis', 'system'][index];
  await browser.waitUntil(
    () => browser.execute((route) => document.querySelector('.route-page')?.getAttribute('data-route') === route, expected),
    { timeout: 10_000, timeoutMsg: `destination ${expected} did not become interactive` }
  );
}

async function configureVisualPreferences(locale: 'en' | 'es', theme: 'light' | 'dark'): Promise<void> {
  await navigateToDestination(1);
  await $('#system-preferences').waitForDisplayed();
  await selectValue('#system-preferences select', 0, locale);
  await selectValue('#system-preferences select', 1, theme);
  await $('#system-preferences button.primary').click();
  await browser.waitUntil(async () => (
    await $('html').getAttribute('lang') === (locale === 'es' ? 'es' : 'en-US')
    && await $('html').getAttribute('data-theme') === theme
  ), { timeout: 10_000, timeoutMsg: `visual preferences ${locale}/${theme} were not applied` });
}

async function assertVisualMatrix(): Promise<void> {
  const viewports = [
    { width: 1020, height: 640 },
    { width: 1180, height: 760 },
    { width: 1440, height: 900 }
  ];
  const routes = ['wikis', 'system'] as const;
  for (const locale of ['en', 'es'] as const) {
    for (const theme of ['light', 'dark'] as const) {
      await configureVisualPreferences(locale, theme);
      for (const viewport of viewports) {
        await setCssViewport(viewport.width, viewport.height);
        for (let index = 0; index < routes.length; index += 1) {
          await navigateToDestination(index);
          const result = await browser.checkScreen(`${locale}-${theme}-${routes[index]}`);
          const mismatch = typeof result === 'number' ? result : result.misMatchPercentage;
          expect(mismatch).toBeLessThanOrEqual(0.1);
        }
      }
    }
  }
}

describe('AirWiki real IPC journey', () => {
  it('persists onboarding and explicit appearance preferences', async () => {
    await browser.waitUntil(
      () => browser.execute(() => Boolean(document.querySelector('main.onboarding:not(.startup)'))),
      { timeout: 30_000, timeoutMsg: 'the runtime did not reach interactive onboarding' }
    );
    const onboarding = await $('main.onboarding:not(.startup)');
    await expect(onboarding).toBeDisplayed();

    await selectValue('main.onboarding:not(.startup) select', 0, 'en');
    const language = required((await $$('main.onboarding:not(.startup) select'))[0], 'language preference');
    await expect(language).toHaveValue('en');
    await $('button.onboarding-next').click();

    const localNetwork = required((await $$('main.onboarding:not(.startup) select'))[0], 'local network preference');
    await selectValue('main.onboarding:not(.startup) select', 0, 'disabled');
    await expect(localNetwork).toHaveValue('disabled');
    await $('button.onboarding-next').click();

    const closeBehavior = required((await $$('main.onboarding:not(.startup) select'))[0], 'close behavior preference');
    await selectValue('main.onboarding:not(.startup) select', 0, 'ask');
    await expect(closeBehavior).toHaveValue('ask');
    await $('button.onboarding-next').click();
    while (await $('button.onboarding-next').isExisting()) await $('button.onboarding-next').click();
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
    await expect($('button*=AirWiki')).toBeDisplayed();
    await expect($('button*=New wiki')).toBeDisplayed();
    await expect($('button[aria-label="Settings"]')).toBeDisplayed();
    await expect($('.system-status-bar')).toBeDisplayed();
    expect(await measureNavigationPaintP95()).toBeLessThanOrEqual(100);

    const devicePixelRatio = await browser.execute(() => window.devicePixelRatio || 1);
    for (const viewport of [
      { width: 1020, height: 640 },
      { width: 1180, height: 760 },
      { width: 1440, height: 900 }
    ]) {
      let physicalWidth = Math.ceil(viewport.width * devicePixelRatio);
      const physicalHeight = Math.ceil(viewport.height * devicePixelRatio);
      let dimensions: { clientWidth: number; scrollWidth: number } | undefined;
      for (let attempt = 0; attempt < 3; attempt += 1) {
        await browser.setWindowSize(physicalWidth, physicalHeight);
        dimensions = await browser.execute(() => ({
          clientWidth: document.documentElement.clientWidth,
          scrollWidth: document.documentElement.scrollWidth
        }));
        if (dimensions.clientWidth >= viewport.width) break;
        physicalWidth += Math.ceil((viewport.width - dimensions.clientWidth) * devicePixelRatio);
      }
      dimensions = required(dimensions, 'responsive viewport dimensions');
      expect(dimensions.clientWidth).toBeGreaterThanOrEqual(viewport.width);
      expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
    }

    await $('button[aria-label="Settings"]').click();
    await $('#system-preferences').waitForDisplayed();
    const systemShell = await browser.execute(() => {
      const main = document.querySelector<HTMLElement>('.shell > main');
      const topBar = document.querySelector<HTMLElement>('.top-bar');
      const statusBar = document.querySelector<HTMLElement>('.system-status-bar');
      return {
        documentScrollTop: document.scrollingElement?.scrollTop ?? -1,
        mainScrollTop: main?.scrollTop ?? -1,
        topBarTop: topBar?.getBoundingClientRect().top ?? -1,
        statusVisible: statusBar ? statusBar.getBoundingClientRect().height > 0 : false,
        sidebarPresent: document.querySelector('.rail') !== null
      };
    });
    expect(systemShell.documentScrollTop).toBe(0);
    expect(systemShell.mainScrollTop).toBe(0);
    expect(systemShell.topBarTop).toBe(0);
    expect(systemShell.statusVisible).toBe(true);
    expect(systemShell.sidebarPresent).toBe(false);

    await $('a[href="#system/updates"]').click();
    await $('#system-updates').waitForDisplayed();
    expect(await $('#system-preferences').isExisting()).toBe(false);
    expect(await browser.execute(() => document.querySelector<HTMLElement>('.shell > main')?.scrollTop ?? -1)).toBe(0);

    await $('a[href="#system/preferences"]').click();
    await $('#system-preferences').waitForDisplayed();
    expect(await $('#system-updates').isExisting()).toBe(false);
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

    await assertVisualMatrix();
  });
});
