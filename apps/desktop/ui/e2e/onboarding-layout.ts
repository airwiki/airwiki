import { $, browser, expect } from '@wdio/globals';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

export async function assertOnboardingLayout(): Promise<void> {
  const onboarding = await $('main.onboarding:not(.startup)');
  for (const locale of ['es', 'en']) {
    let step = Number(await onboarding.getAttribute('data-step'));
    while (step > 0) {
      await $('.onboarding-back').click();
      await expect(onboarding).toHaveAttribute('data-step', String(--step));
    }
    await browser.execute((locale) => {
      const language = document.querySelector<HTMLSelectElement>('main.onboarding select');
      if (!language) throw new Error('onboarding language is missing');
      language.value = locale;
      language.dispatchEvent(new Event('change', { bubbles: true }));
    }, locale);
    while (await $('.onboarding-next').isExisting()) {
      await $('.onboarding-next').click();
      await expect(onboarding).toHaveAttribute('data-step', String(++step));
    }
    for (const target of [{ width: 1024, height: 720 }, { width: 1180, height: 740 }]) {
      for (let attempt = 0; attempt < 3; attempt++) {
        const frame = await browser.getWindowSize();
        const content = await browser.execute(() => ({ width: innerWidth, height: innerHeight }));
        if (content.width === target.width && content.height === target.height) break;
        const ratio = await browser.execute(() => devicePixelRatio);
        await browser.setWindowSize(
          frame.width + Math.round((target.width - content.width) * ratio),
          frame.height + Math.round((target.height - content.height) * ratio)
        );
      }
      expect(await browser.execute(() => ({ width: innerWidth, height: innerHeight }))).toEqual(target);
      // Capture the settled page, not the first transparent frame of its entry animation.
      await browser.waitUntil(async () => browser.execute(() => {
        const page = document.querySelector('.onboarding-page');
        return !!page && document.fonts.status === 'loaded'
          && getComputedStyle(page).opacity === '1'
          && page.getAnimations().every((animation) => animation.playState === 'finished');
      }), { timeout: 3_000, timeoutMsg: 'onboarding page did not finish painting' });
      const layout = await browser.execute(() => {
        const footer = document.querySelector('.onboarding-actions')?.getBoundingClientRect();
        const stage = document.querySelector('.onboarding-stage');
        if (!footer || !stage) return { actionsVisible: false, contentReachable: false };
        stage.scrollTo({ top: stage.scrollHeight, behavior: 'instant' });
        const end = stage.querySelector('.onboarding-page > :last-child')?.getBoundingClientRect();
        return {
          actionsVisible: footer.top >= 0 && footer.bottom <= innerHeight,
          contentReachable: !!end && end.bottom <= stage.getBoundingClientRect().bottom + 1,
        };
      });
      expect(layout).toEqual({ actionsVisible: true, contentReachable: true });
      const directory = join(process.cwd(), '.artifacts', 'onboarding');
      mkdirSync(directory, { recursive: true });
      await browser.saveScreenshot(join(directory, `${locale}-${target.width}x${target.height}.png`));
    }
  }
}
