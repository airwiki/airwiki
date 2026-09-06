import { $, browser, expect } from '@wdio/globals';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { captureVisual, configureVisualPreferences, runVisualMatrix, setCssViewport, visualViewports } from './visual.js';

async function assertReviewLayout(): Promise<void> {
  const layout = await browser.execute(() => {
    const action = document.querySelector('.review-actions .primary')?.getBoundingClientRect();
    const footer = document.querySelector('.review-actions')?.getBoundingClientRect();
    const workspace = document.querySelector('.drive-page')?.getBoundingClientRect();
    const content = document.querySelector('.review-content')?.getBoundingClientRect();
    return {
      overflow: document.documentElement.scrollWidth > innerWidth,
      approvalVisible: !!action && action.top >= 0 && action.bottom <= innerHeight,
      footerFillsWidth: !!footer && !!workspace && Math.abs(footer.left - workspace.left) < 1 && Math.abs(footer.right - workspace.right) < 1,
      footerAtBottom: !!footer && !!workspace && Math.abs(footer.bottom - workspace.bottom) < 1,
      contentReserved: !!content && !!footer && content.bottom <= footer.top,
    };
  });
  expect(layout).toEqual({ overflow: false, approvalVisible: true, footerFillsWidth: true, footerAtBottom: true, contentReserved: true });
  await browser.execute(() => {
    const content = document.querySelector('.review-content');
    content?.scrollTo({ top: content.scrollHeight, behavior: 'instant' });
  });
  expect(await browser.execute(() => {
    const editor = document.querySelector('#review-proposal textarea')?.getBoundingClientRect();
    const content = document.querySelector('.review-content')?.getBoundingClientRect();
    return !!editor && !!content && editor.bottom <= content.bottom && editor.bottom > content.top;
  })).toBe(true);
}

async function assertReviewVisualMatrix(): Promise<void> {
  for (const locale of ['en', 'es'] as const) {
    for (const theme of ['light', 'dark'] as const) {
      await configureVisualPreferences(locale, theme);
      await $('.settings-back').click();
      await expect($('.review-workspace h1')).toHaveText('Review maintenance');
      await $('.review-actions .primary').waitForEnabled();
      for (const viewport of visualViewports) {
        await setCssViewport(viewport.width, viewport.height);
        await browser.execute(() => document.querySelector('.review-content')?.scrollTo({ top: 0, behavior: 'instant' }));
        const compact = await $('.review-view-switch').isDisplayed();
        await captureVisual(`${locale}-${theme}-review-${compact ? 'proposal' : 'comparison'}`);
        if (compact) {
          await $('.review-view-switch [aria-controls="review-evidence"]').click();
          await expect($('#review-evidence')).toBeDisplayed();
          await captureVisual(`${locale}-${theme}-review-evidence`);
          await $('.review-view-switch [aria-controls="review-proposal"]').click();
        }
        await assertReviewLayout();
      }
    }
  }
  await configureVisualPreferences('en', 'light');
  await $('.settings-back').click();
  await setCssViewport(1440, 900);
  await $('.review-actions .primary').waitForEnabled();
}

async function selectValue(selector: string, value: string): Promise<void> {
  expect(await browser.execute((selector, value) => {
    const field = document.querySelector<HTMLSelectElement>(selector);
    if (!field) return false;
    field.value = value;
    field.dispatchEvent(new Event('change', { bubbles: true }));
    return field.value === value;
  }, selector, value)).toBe(true);
}

async function requestQuitIntent(): Promise<void> {
  // Deliver the same application event as native menu/tray/window requests.
  // Installed-platform checks separately exercise those OS entry points.
  await browser.execute(async () => {
    const runtime = (window as unknown as {
      __TAURI_INTERNALS__: { invoke(command: string, args: object): Promise<void> };
    }).__TAURI_INTERNALS__;
    await runtime.invoke('plugin:event|emit', { event: 'quit-requested', payload: null });
  });
}

describe('AirWiki review with real storage and IPC', () => {
  it('retains edits, confirms publication, excludes a draft and reopens its current evidence', async () => {
    await $('main.onboarding:not(.startup)').waitForDisplayed({ timeout: 30_000 });
    await selectValue('main.onboarding select', 'en');
    await $('.onboarding-next').click();
    await expect($('.onboarding-folder-success')).toHaveText(expect.stringContaining('Synthetic review workspace'));
    while (await $('.onboarding-next').isExisting()) await $('.onboarding-next').click();
    await $('.onboarding-action').click();
    await $('.shell').waitForDisplayed({ timeout: 30_000 });

    await $('.workspace-destinations').$('button*=To review').click();
    await $('.review-queue').$('button*=Review maintenance').click();
    await expect($('.review-workspace h1')).toHaveText('Review maintenance');
    await $('.review-actions .primary').waitForEnabled();
    await expect($('#review-evidence')).toHaveText(expect.stringContaining('Create a local backup and verify its checksum before beginning.'));
    if (runVisualMatrix) await assertReviewVisualMatrix();
    else {
      for (const viewport of visualViewports) {
        await setCssViewport(viewport.width, viewport.height);
        await assertReviewLayout();
      }
      await setCssViewport(1440, 900);
    }
    await $('#review-proposal input').setValue('Maintenance approved by a person');
    await requestQuitIntent();
    await $('#review-discard-title').waitForDisplayed();
    await $('.close-dialog').$('button=Keep reviewing').click();
    await expect($('#review-proposal input')).toHaveValue('Maintenance approved by a person');
    const fixtureRoot = process.env.AIRWIKI_E2E_DATA_ROOT;
    if (!fixtureRoot) throw new Error('missing isolated review fixture root');
    const source = join(fixtureRoot, 'data', 'synthetic-review-source', 'maintenance.md');
    const original = await readFile(source);
    try {
      await writeFile(source, 'Synthetic source changed after the evidence was loaded.');
      await $('.review-actions .primary').click();
      await $('.review-notice[role="alert"]').waitForDisplayed();
      await expect($('.review-notice[role="alert"]')).toHaveText(expect.stringContaining('The decision could not be confirmed. Your edits are kept'));
      await expect($('.review-workspace h1')).toHaveText('Review maintenance');
      await expect($('#review-proposal input')).toHaveValue('Maintenance approved by a person');
      await expect($('.review-progress')).toHaveText('Confirmed decisions: 0 · Pending: 2');
    } finally {
      await writeFile(source, original);
    }
    await $('.review-actions .primary').click();
    // Confirmation replaces the keyed workspace. Read the current heading on
    // each attempt instead of retaining an element from the previous draft.
    await browser.waitUntil(
      () => browser.execute(() => document.querySelector('.review-workspace h1')?.textContent === 'Review recovery'),
      { timeout: 10_000, timeoutMsg: 'Confirmed approval did not open the next draft' }
    );
    await $('.review-actions .primary').waitForEnabled();
    await expect($('.review-progress')).toHaveText('Confirmed decisions: 1 · Pending: 1');
    await expect($('#review-evidence')).toHaveText(expect.stringContaining('restore the backup and check the recovered state.'));
    await $('.review-actions').$('button=Exclude from this Wiki').click();
    await $('h1=To review').waitForDisplayed();
    await expect($('.page-heading')).toHaveText(expect.stringContaining('Confirmed decisions: 2 · Pending: 0'));
    await $('.review-excluded-list summary').click();
    await $('.review-excluded-list').$('button*=Review recovery').click();
    await expect($('.review-workspace h1')).toHaveText('Review recovery');
    await $('.review-actions .primary').waitForEnabled();
    expect(await $('.review-actions').$('button=Exclude from this Wiki').isExisting()).toBe(false);
    await $('.review-actions .primary').click();
    await $('h1=To review').waitForDisplayed();
    await expect($('.page-heading')).toHaveText(expect.stringContaining('Confirmed decisions: 1 · Pending: 0'));

    await $('.workspace-destinations').$('button=Library').click();
    await $('.wiki-row').click();
    await $('.file-list').$('button*=Maintenance approved by a person').click();
    await expect($('.file-preview h1')).toHaveText('Maintenance approved by a person');
    await expect($('.file-preview .knowledge-blocks')).toHaveText(expect.stringContaining('Create a local backup'));

    for (const trigger of ['.wiki-context-details', '.journey-compact-share', '.journey-compact-ai']) {
      await $(trigger).click();
      await $('.side-drawer[role="dialog"]').waitForDisplayed();
      await browser.keys('Escape');
      await browser.waitUntil(
        () => browser.execute((selector) => document.activeElement === document.querySelector(selector), trigger),
        { timeout: 5_000, timeoutMsg: `Closing the Wiki panel did not return focus to ${trigger}` }
      );
    }

    await $('.system-status-button').click();
    await $('a[href="#settings/general"]').click();
    await selectValue('.device-preferences-form .control-field:nth-of-type(3) select', 'quit');
    await $('.settings-form-actions .primary').click();
    await browser.waitUntil(async () => !(await $('.settings-form-actions .primary').isEnabled()));
    await selectValue('.device-preferences-form .control-field:nth-of-type(2) select', 'dark');
    await requestQuitIntent();
    await $('#settings-discard-title').waitForDisplayed();
    await $('.close-dialog').$('button=Continue editing').click();
    await expect($('.device-preferences-form .control-field:nth-of-type(2) select')).toHaveValue('dark');
    await $('.settings-form-actions .secondary').click();
    await $('.settings-back').click();
    await expect($('.file-preview h1')).toHaveText('Maintenance approved by a person');
  });
});
