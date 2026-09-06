import { $, browser, expect } from '@wdio/globals';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

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
    await expect($('.review-workspace h1')).toHaveText('Review recovery');
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
    await $('.review-workspace-heading').$('button=Back to review queue').click();

    await $('.workspace-destinations').$('button=Library').click();
    await $('.wiki-row').click();
    await $('.file-list').$('button*=Maintenance approved by a person').click();
    await expect($('.file-preview h1')).toHaveText('Maintenance approved by a person');
    await expect($('.file-preview .knowledge-blocks')).toHaveText(expect.stringContaining('Create a local backup'));

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
