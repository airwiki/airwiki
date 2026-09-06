import { $, browser, expect } from '@wdio/globals';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

export const runVisualMatrix = process.env.AIRWIKI_E2E_VISUAL !== '0'
  || process.env.AIRWIKI_E2E_CAPTURE_MATRIX === '1';
export const visualViewports = [
  { width: 1024, height: 720 },
  { width: 1180, height: 760 },
  { width: 1440, height: 900 }
] as const;

export async function setCssViewport(width: number, height: number): Promise<void> {
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

export async function configureVisualPreferences(locale: 'en' | 'es', theme: 'light' | 'dark'): Promise<void> {
  if (!(await $('.settings-top-bar').isExisting())) await $('.system-status-button').click();
  await $('a[href="#settings/general"]').click();
  await $('.device-preferences-form').waitForExist();
  expect(await browser.execute((locale, theme) => {
    const fields = document.querySelectorAll<HTMLSelectElement>('.device-preferences-form select');
    const language = fields.item(0);
    const appearance = fields.item(1);
    if (!language || !appearance) return false;
    language.value = locale;
    language.dispatchEvent(new Event('change', { bubbles: true }));
    appearance.value = theme;
    appearance.dispatchEvent(new Event('change', { bubbles: true }));
    return true;
  }, locale, theme)).toBe(true);
  const save = await $('.settings-form-actions button.primary');
  if (await save.isEnabled()) await save.click();
  await browser.waitUntil(async () => (
    await $('html').getAttribute('lang') === (locale === 'es' ? 'es' : 'en-US')
    && await $('html').getAttribute('data-theme') === theme
    && !(await $('.settings-form-actions button.primary').isEnabled())
  ), { timeout: 10_000, timeoutMsg: `visual preferences ${locale}/${theme} were not applied` });
  await browser.execute(() => document.querySelector('.drive-page')?.scrollTo({ top: 0, behavior: 'instant' }));
}

export async function captureVisual(tag: string): Promise<void> {
  await browser.executeAsync((done) => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    const style = document.createElement('style');
    style.id = 'visual-capture-styles';
    // Suppress only ephemeral feedback and pointer decoration. Product status,
    // errors, focus handling and permission explanations remain in the captures.
    style.textContent = `
      *, *::before, *::after { transition: none !important; animation: none !important; caret-color: transparent !important; }
      .action-message { visibility: hidden !important; }
      .workspace-sidebar button:hover:not(.active):not(.wiki-picker) { color: var(--muted) !important; background: transparent !important; }
      .secondary:hover:not(:disabled) { background: transparent !important; border-color: var(--line) !important; }
      .system-status-button:hover { color: var(--muted) !important; background: transparent !important; }
      .select-control select:hover:not(:disabled), .select-control select:focus-visible {
        border-color: var(--control-border, var(--line)) !important;
        box-shadow: inset 0 1px 1px #0000000d !important;
      }
    `;
    document.head.append(style);
    void document.fonts.ready.then(() => {
      let completed = false;
      const finish = () => { if (!completed) { completed = true; done(true); } };
      document.body.getBoundingClientRect();
      const fallback = window.setTimeout(finish, 250);
      requestAnimationFrame(() => requestAnimationFrame(() => { window.clearTimeout(fallback); finish(); }));
    });
  });
  try {
    if (process.env.AIRWIKI_E2E_CAPTURE_MATRIX === '1') {
      const directory = join(process.cwd(), '.artifacts', 'visual', 'matrix');
      mkdirSync(directory, { recursive: true });
      const size = await browser.execute(() => `${innerWidth}x${innerHeight}`);
      await browser.saveScreenshot(join(directory, `${tag}-${size}.png`));
    }
    if (process.env.AIRWIKI_E2E_VISUAL !== '0') {
      const result = await browser.checkScreen(tag);
      const mismatch = typeof result === 'number' ? result : result.misMatchPercentage;
      expect(mismatch).toBeLessThanOrEqual(0.1);
    }
  } finally {
    await browser.execute(() => document.querySelector('#visual-capture-styles')?.remove());
  }
}
