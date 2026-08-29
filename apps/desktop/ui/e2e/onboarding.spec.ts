import { $, $$, browser, expect } from '@wdio/globals';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';

const runVisualMatrix = process.env.AIRWIKI_E2E_VISUAL !== '0';

function required<T>(value: T | undefined, label: string): T {
  if (value === undefined) throw new Error(`missing ${label}`);
  return value;
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`${label} is not an object`);
  }
  return value as Record<string, unknown>;
}

function stringField(value: Record<string, unknown>, field: string): string {
  const result = value[field];
  if (typeof result !== 'string' || result.length === 0) throw new Error(`missing ${field}`);
  return result;
}

class McpStdioClient {
  private static readonly protocolMetadata = {
    'io.modelcontextprotocol/protocolVersion': '2026-07-28',
    'io.modelcontextprotocol/clientInfo': { name: 'airwiki-e2e-agent', version: '1' },
    'io.modelcontextprotocol/clientCapabilities': {}
  };
  private readonly child: ChildProcessWithoutNullStreams;
  private readonly pending = new Map<number, {
    resolve: (value: Record<string, unknown>) => void;
    reject: (error: Error) => void;
    timeout: NodeJS.Timeout;
  }>();
  private nextId = 1;
  private stderr = '';
  private closing = false;

  constructor(command: string, args: string[]) {
    this.child = spawn(command, args, {
      env: process.env,
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true
    });
    createInterface({ input: this.child.stdout }).on('line', (line) => this.receive(line));
    this.child.stderr.setEncoding('utf8');
    this.child.stderr.on('data', (chunk: string) => {
      if (this.stderr.length < 4096) this.stderr += chunk.slice(0, 4096 - this.stderr.length);
    });
    this.child.once('error', (error) => this.failPending(error));
    this.child.once('exit', (code) => {
      if (!this.closing) this.failPending(new Error(`MCP bridge exited unexpectedly (${code ?? 'signal'})`));
    });
  }

  async discover(): Promise<void> {
    const result = await this.request('server/discover', {});
    const supportedVersions = result.supportedVersions;
    if (!Array.isArray(supportedVersions) || !supportedVersions.includes('2026-07-28')) {
      throw new Error('MCP server/discover did not advertise protocol 2026-07-28');
    }
  }

  request(method: string, params: Record<string, unknown>): Promise<Record<string, unknown>> {
    const id = this.nextId;
    this.nextId += 1;
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`MCP request ${method} timed out`));
      }, 15_000);
      this.pending.set(id, { resolve, reject, timeout });
      this.write({
        jsonrpc: '2.0',
        id,
        method,
        params: { ...params, _meta: McpStdioClient.protocolMetadata }
      });
    });
  }

  async callTool(name: string, arguments_: Record<string, unknown>): Promise<Record<string, unknown>> {
    const result = await this.request('tools/call', { name, arguments: arguments_ });
    const structuredContent = record(result.structuredContent, `${name} structured content`);
    if (result.isError === true) {
      const code = stringField(structuredContent, 'code');
      const message = stringField(structuredContent, 'message');
      throw new Error(`${code}: ${message}`);
    }
    return structuredContent;
  }

  async close(): Promise<void> {
    this.closing = true;
    this.child.stdin.end();
    const code = await Promise.race<number | null>([
      new Promise((resolveExit) => this.child.once('exit', resolveExit)),
      new Promise((_, rejectTimeout) => setTimeout(
        () => rejectTimeout(new Error('MCP bridge did not stop after stdin closed')),
        5_000
      ))
    ]).catch((error) => {
      this.child.kill();
      throw error;
    });
    if (code !== 0) {
      throw new Error(`MCP bridge failed (${code ?? 'signal'}): ${this.stderr.trim() || 'no diagnostics'}`);
    }
  }

  private write(message: Record<string, unknown>): void {
    if (!this.child.stdin.write(`${JSON.stringify(message)}\n`)) {
      this.child.stdin.once('drain', () => undefined);
    }
  }

  private receive(line: string): void {
    let message: Record<string, unknown>;
    try {
      message = record(JSON.parse(line), 'MCP response');
    } catch (error) {
      this.failPending(error instanceof Error ? error : new Error('invalid MCP response'));
      return;
    }
    if (typeof message.id !== 'number') return;
    const pending = this.pending.get(message.id);
    if (!pending) return;
    this.pending.delete(message.id);
    clearTimeout(pending.timeout);
    if (message.error !== undefined) {
      const error = record(message.error, 'MCP error');
      pending.reject(new Error(typeof error.message === 'string' ? error.message : 'MCP request failed'));
      return;
    }
    try {
      pending.resolve(record(message.result, 'MCP result'));
    } catch (error) {
      pending.reject(error instanceof Error ? error : new Error('invalid MCP result'));
    }
  }

  private failPending(error: Error): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(error);
    }
    this.pending.clear();
  }
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
  const samples: number[] = [];
  for (let sample = 0; sample < 20; sample += 1) {
    const duration = await browser.executeAsync((done) => {
      const leavingSettings = document.querySelector('.settings-top-bar') !== null;
      const selector = leavingSettings ? '.settings-back' : '.system-status-button';
      const expectedRoute = leavingSettings ? 'library' : 'settings';
      const button = document.querySelector<HTMLButtonElement>(selector);
      if (!button) {
        done(null);
        return;
      }
      const startMark = 'airwiki-navigation-start';
      const endMark = 'airwiki-navigation-painted';
      const measureName = 'airwiki-navigation-paint';
      performance.clearMarks(startMark);
      performance.clearMarks(endMark);
      performance.clearMeasures(measureName);
      performance.mark(startMark);
      const deadline = performance.now() + 2_000;
      let completed = false;
      let routePoll = 0;
      let paintFallback = 0;
      let watchdog = 0;
      const complete = (value: number | null) => {
        if (completed) return;
        completed = true;
        window.clearInterval(routePoll);
        window.clearTimeout(paintFallback);
        window.clearTimeout(watchdog);
        done(value);
      };
      const recordPaint = () => {
        if (completed) return;
        performance.mark(endMark);
        const measurement = performance.measure(measureName, startMark, endMark);
        complete(measurement.duration);
      };
      const waitForPaint = () => {
        const page = document.querySelector<HTMLElement>('.route-page');
        const bounds = page?.getBoundingClientRect();
        const style = page ? getComputedStyle(page) : null;
        if (page?.dataset.route === expectedRoute
          && bounds
          && bounds.width > 0
          && bounds.height > 0
          && style?.visibility === 'visible') {
          window.clearInterval(routePoll);
          requestAnimationFrame(recordPaint);
          // WebKit can suspend animation frames when its test window is
          // occluded. One frame-length fallback keeps the measurement bounded.
          paintFallback = window.setTimeout(recordPaint, 16);
          return;
        }
        if (performance.now() >= deadline) {
          complete(null);
          return;
        }
      };
      watchdog = window.setTimeout(() => complete(null), 2_100);
      button.click();
      routePoll = window.setInterval(waitForPaint, 5);
      waitForPaint();
    });
    if (typeof duration !== 'number') throw new Error(`navigation paint sample ${sample} returned an invalid timestamp`);
    samples.push(duration);
  }
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
  const expected = required(['library', 'settings'][index], `destination ${index}`);
  const current = await browser.execute(() => document.querySelector<HTMLElement>('.route-page')?.dataset.route ?? null);
  if (current !== expected) {
    await $(expected === 'library' ? '.settings-back' : '.system-status-button').click();
  }
  await browser.waitUntil(
    () => browser.execute((route) => {
      const page = document.querySelector<HTMLElement>('.route-page');
      if (!page || page.dataset.route !== route) return false;
      const style = getComputedStyle(page);
      const bounds = page.getBoundingClientRect();
      return Number.parseFloat(style.opacity) >= 0.99
        && style.visibility === 'visible'
        && bounds.width > 0
        && bounds.height > 0;
    }, expected),
    { timeout: 10_000, timeoutMsg: `destination ${expected} did not become visibly interactive` }
  );
  await browser.waitUntil(
    () => browser.execute(() => document.querySelector<HTMLElement>('.drive-page')?.scrollTop === 0),
    { timeout: 10_000, timeoutMsg: `destination ${expected} did not start at the top` }
  );
  const persistentChrome = await browser.execute((route) => Array.from(
    document.querySelectorAll<HTMLElement>(route === 'library'
      ? '.top-brand, .global-search, .top-actions'
      : '.settings-top-bar')
  ).map((element) => {
    const bounds = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return {
      visible: style.display !== 'none' && style.visibility === 'visible' && Number.parseFloat(style.opacity) > 0,
      width: bounds.width,
      left: bounds.left,
      right: bounds.right,
      viewportWidth: document.documentElement.clientWidth
    };
  }), expected);
  expect(persistentChrome).toHaveLength(expected === 'library' ? 3 : 1);
  expect(persistentChrome.every((item) => (
    item.visible && item.width > 0 && item.left >= 0 && item.right <= item.viewportWidth
  ))).toBe(true);
  expect(await browser.execute((route) => route === 'library'
    ? document.querySelector('.settings-top-bar') === null
    : document.querySelector('.top-bar, .global-search, .system-status-button') === null, expected)).toBe(true);
}

async function waitForVisualPaint(route: 'library' | 'settings'): Promise<void> {
  if (route === 'settings') {
    await browser.waitUntil(
      () => browser.execute(() => document.querySelectorAll('.settings-page select').length >= 3),
      { timeout: 10_000, timeoutMsg: 'general settings did not reach their complete DOM state' }
    );
  }
  const painted = await browser.executeAsync((done) => {
    let completed = false;
    const finish = () => {
      if (completed) return;
      completed = true;
      done(true);
    };
    document.body.getBoundingClientRect();
    const fallback = window.setTimeout(finish, 250);
    requestAnimationFrame(() => requestAnimationFrame(() => {
      window.clearTimeout(fallback);
      finish();
    }));
  });
  expect(painted).toBe(true);
}

async function configureVisualPreferences(locale: 'en' | 'es', theme: 'light' | 'dark'): Promise<void> {
  await navigateToDestination(1);
  await $('a[href="#settings/general"]').click();
  await $('.settings-page').waitForDisplayed();
  await selectValue('.device-preferences-form select', 0, locale);
  await selectValue('.device-preferences-form select', 1, theme);
  await $('.settings-form-actions button.primary').click();
  await browser.waitUntil(async () => (
    await $('html').getAttribute('lang') === (locale === 'es' ? 'es' : 'en-US')
    && await $('html').getAttribute('data-theme') === theme
  ), { timeout: 10_000, timeoutMsg: `visual preferences ${locale}/${theme} were not applied` });
}

async function openAiAppsSettings(): Promise<void> {
  const route = await browser.execute(() => document.querySelector<HTMLElement>('.route-page')?.dataset.route ?? null);
  if (route !== 'settings') await $('.system-status-button').click();
  await $('a[href="#settings/apps"]').click();
  await browser.waitUntil(
    () => browser.execute(() => window.location.hash === '#settings/apps'
      && document.querySelector('.integration-list') !== null),
    { timeout: 30_000, timeoutMsg: 'AI apps settings did not become ready' }
  );
}

async function returnToLibrary(): Promise<void> {
  const route = await browser.execute(() => document.querySelector<HTMLElement>('.route-page')?.dataset.route ?? null);
  if (route === 'settings') await $('.settings-back').click();
  await browser.waitUntil(
    () => browser.execute(() => document.querySelector<HTMLElement>('.route-page')?.dataset.route === 'library'),
    { timeout: 10_000, timeoutMsg: 'Library did not restore after leaving Settings' }
  );
  await $('.top-brand').click();
}

async function assertVisualMatrix(): Promise<void> {
  const viewports = [
    { width: 1180, height: 760 },
    { width: 1440, height: 900 }
  ];
  const routes = ['library', 'settings'] as const;
  for (const locale of ['en', 'es'] as const) {
    for (const theme of ['light', 'dark'] as const) {
      await configureVisualPreferences(locale, theme);
      for (const viewport of viewports) {
        await setCssViewport(viewport.width, viewport.height);
        for (let index = 0; index < routes.length; index += 1) {
          await navigateToDestination(index);
          await browser.execute(() => {
            if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
            document.querySelector('.action-message')?.remove();
            const style = document.createElement('style');
            style.id = 'visual-capture-styles';
            style.textContent = `
              .secondary:hover:not(:disabled) {
                background: transparent !important;
                border-color: var(--line) !important;
              }
              .system-status-button:hover { color: var(--muted) !important; background: transparent !important; }
              .select-control select:hover:not(:disabled),
              .select-control select:focus-visible {
                border-color: var(--control-border, var(--line)) !important;
                box-shadow: inset 0 1px 1px #0000000d !important;
              }
            `;
            document.head.append(style);
          });
          await waitForVisualPaint(routes[index]);
          const result = await browser.checkScreen(`${locale}-${theme}-${routes[index]}`);
          await browser.execute(() => document.querySelector('#visual-capture-styles')?.remove());
          const mismatch = typeof result === 'number' ? result : result.misMatchPercentage;
          expect(mismatch).toBeLessThanOrEqual(0.1);
        }
      }
    }
  }
}

async function createFolderWiki(): Promise<void> {
  await $('button*=New wiki').click();
  const sourceDialog = await $('#new-wiki-source-title');
  await sourceDialog.waitForDisplayed();
  await $('button*=From a folder').click();
  await $('#create-wiki-title').waitForDisplayed();
  const name = await $('.create-wiki-dialog input:not([type="checkbox"])');
  await name.setValue('E2E folder wiki');
  await expect($('.create-wiki-dialog input[role="switch"]')).toBeSelected();
  await $('button*=Create wiki').click();
  await $('button*=AirWiki').click();
  try {
    await browser.waitUntil(
      () => browser.execute(() => Array.from(document.querySelectorAll('.wiki-row'))
        .some((row) => row.textContent?.includes('E2E folder wiki') === true)),
      { timeout: 10_000, timeoutMsg: 'folder wiki did not appear after the real IPC command' }
    );
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      actionMessage: document.querySelector('.action-message')?.textContent?.trim() ?? null,
      dialogOpen: document.querySelector('#create-wiki-title') !== null,
      wikiCount: document.querySelectorAll('.wiki-row').length,
    }));
    throw new Error(`folder wiki creation failed: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
  const row = await $('.wiki-row*=E2E folder wiki');
  await expect(row).toHaveText(expect.stringContaining('automatic updates'));
}

async function importOkfWiki(): Promise<void> {
  const captureTheme = process.env.AIRWIKI_E2E_JOURNEY_THEME === 'dark' ? 'dark' : 'light';
  await $('button*=New wiki').click();
  await $('#new-wiki-source-title').waitForDisplayed();
  await $('button*=Import OKF folder').click();
  await $('#import-okf-title').waitForDisplayed();
  await expect($('.create-wiki-dialog')).toHaveText(expect.stringContaining('OKF v0.2'));
  await expect($('.create-wiki-dialog')).toHaveText(expect.stringContaining('2'));
  const name = await $('.create-wiki-dialog input:not([type="checkbox"])');
  await name.setValue('E2E imported wiki');
  await $('button*=Import wiki').click();
  await $('button*=AirWiki').click();
  await browser.waitUntil(
    () => browser.execute(() => Array.from(document.querySelectorAll('.wiki-row'))
      .some((row) => row.textContent?.includes('E2E imported wiki') === true)),
    { timeout: 10_000, timeoutMsg: 'imported OKF wiki did not appear after the real IPC command' }
  );
  const row = await $('.wiki-row*=E2E imported wiki');
  await expect(row).toHaveText(expect.stringContaining('Imported OKF'));
  await row.click();
  await browser.waitUntil(
    async () => (await $('.file-list')).isExisting(),
    { timeout: 10_000, timeoutMsg: 'imported OKF hierarchy did not load' }
  );
  await setCssViewport(1180, 760);
  const statusBar = await $('.wiki-journey-compact');
  await expect(statusBar).toHaveText(expect.stringContaining('Searchable'));
  await expect(statusBar).toHaveText(expect.stringContaining('Local'));
  await expect(statusBar).toHaveText(expect.stringContaining('LAN'));
  await expect(statusBar).toHaveText(expect.stringContaining('Internet'));
  await expect(statusBar).toHaveText(expect.stringContaining('AI apps'));
  await expect(statusBar).toHaveText(expect.stringContaining('Share'));
  expect(await $$('.wiki-journey')).toHaveLength(0);
  expect(await $$('.exposure-route li')).toHaveLength(3);
  const statusBarLayout = await browser.execute(() => {
    const status = document.querySelector<HTMLElement>('.wiki-journey-compact');
    const controls = Array.from(status?.querySelectorAll<HTMLElement>('button') ?? []);
    const page = document.querySelector<HTMLElement>('.wiki-detail-body');
    const statusRect = status?.getBoundingClientRect();
    return {
      statusLeft: statusRect?.left ?? -1,
      statusRight: statusRect?.right ?? Number.POSITIVE_INFINITY,
      pageLeft: page?.getBoundingClientRect().left ?? 0,
      pageRight: page?.getBoundingClientRect().right ?? 0,
      essentialTextVisible: [
        status?.querySelector<HTMLElement>('.journey-compact-identity-copy small'),
        status?.querySelector<HTMLElement>('.journey-compact-ai-copy strong')
      ].every((label) => label instanceof HTMLElement && label.scrollWidth <= label.clientWidth + 1),
      controlsOperable: controls.every((control) => {
        const bounds = control.getBoundingClientRect();
        return getComputedStyle(control).visibility === 'visible' && bounds.width >= 24 && bounds.height >= 24;
      })
    };
  });
  expect(statusBarLayout.statusLeft).toBeGreaterThanOrEqual(statusBarLayout.pageLeft);
  expect(statusBarLayout.statusRight).toBeLessThanOrEqual(statusBarLayout.pageRight);
  expect(statusBarLayout.essentialTextVisible).toBe(true);
  expect(statusBarLayout.controlsOperable).toBe(true);
  if (process.env.AIRWIKI_E2E_CAPTURE_JOURNEY === '1') {
    await browser.saveScreenshot(join(process.cwd(), '.artifacts', 'visual', `wiki-access-bar-review-${captureTheme}.png`));
  }
  await expect($('.file-list')).toHaveText(expect.stringContaining('architecture/decision.md'));
  await expect($('.file-list')).toHaveText(expect.stringContaining('architecture/verified.md'));

  await $('.file-list').$('button*=Synthetic architecture decision').click();
  await browser.waitUntil(
    () => browser.execute(() => document.querySelector('.concept-assurance')?.textContent?.includes('Unverified') === true),
    { timeout: 10_000, timeoutMsg: 'unverified concept assurance did not load' }
  );
  await expect($('.concept-assurance')).toHaveText(expect.stringContaining('Decision'));

  await $('.file-list').$('button*=Verified architecture reference').click();
  await browser.waitUntil(
    () => browser.execute(() => document.querySelector('.concept-assurance')?.textContent?.includes('Human-reviewed') === true),
    { timeout: 10_000, timeoutMsg: 'verified concept assurance did not replace the previous page atomically' }
  );
  const workspaceLayout = await browser.execute(() => {
    const page = document.querySelector<HTMLElement>('.drive-page');
    const topBar = document.querySelector<HTMLElement>('.top-bar');
    const heading = document.querySelector<HTMLElement>('.wiki-route > .wiki-heading');
    const detail = document.querySelector<HTMLElement>('.wiki-detail-body');
    const browserPanel = document.querySelector<HTMLElement>('.wiki-detail-body > .file-browser');
    const list = browserPanel?.querySelector<HTMLElement>('.file-list');
    const preview = browserPanel?.querySelector<HTMLElement>('.file-preview');
    const sticky = document.querySelector<HTMLElement>('.wiki-content-sticky');
    const pageRect = page?.getBoundingClientRect();
    if (page) page.scrollTop = 10_000;
    const topBarRect = topBar?.getBoundingClientRect();
    const headingRect = heading?.getBoundingClientRect();
    const stickyRect = sticky?.getBoundingClientRect();
    return {
      pageScrollTop: page?.scrollTop ?? -1,
      pageOverflowY: page ? getComputedStyle(page).overflowY : '',
      detailOverflowY: detail ? getComputedStyle(detail).overflowY : '',
      listOverflowY: list ? getComputedStyle(list).overflowY : '',
      previewOverflowY: preview ? getComputedStyle(preview).overflowY : '',
      headingTop: headingRect?.top ?? -1,
      pageTop: pageRect?.top ?? -1,
      topBarTop: topBarRect?.top ?? -1,
      topBarBottom: topBarRect?.bottom ?? -1,
      stickyTop: stickyRect?.top ?? Number.POSITIVE_INFINITY,
      stickyPosition: sticky ? getComputedStyle(sticky).position : '',
      browserHeight: browserPanel?.getBoundingClientRect().height ?? 0
    };
  });
  expect(workspaceLayout.pageScrollTop).toBeGreaterThan(0);
  expect(workspaceLayout.pageOverflowY).toBe('auto');
  expect(workspaceLayout.detailOverflowY).toBe('visible');
  expect(workspaceLayout.listOverflowY).toBe('visible');
  expect(workspaceLayout.previewOverflowY).toBe('visible');
  expect(workspaceLayout.headingTop).toBeLessThan(workspaceLayout.pageTop);
  expect(workspaceLayout.topBarTop).toBeGreaterThanOrEqual(0);
  expect(workspaceLayout.pageTop).toBeGreaterThanOrEqual(workspaceLayout.topBarBottom);
  expect(workspaceLayout.stickyPosition).toBe('sticky');
  expect(workspaceLayout.stickyTop).toBeGreaterThanOrEqual(workspaceLayout.topBarBottom);
  expect(workspaceLayout.stickyTop).toBeLessThanOrEqual(workspaceLayout.topBarBottom + 2);
  expect(workspaceLayout.browserHeight).toBeGreaterThan(0);
  const contentToolbarGeometry = await browser.execute(() => {
    const sticky = document.querySelector<HTMLElement>('.wiki-content-sticky');
    const summary = sticky?.querySelector<HTMLElement>('.wiki-journey-compact');
    const actions = sticky?.querySelector<HTMLElement>('.content-tabs-actions');
    return {
      stickyHeight: sticky?.getBoundingClientRect().height ?? 0,
      summaryWidth: summary?.getBoundingClientRect().width ?? 0,
      actionsWidth: actions?.getBoundingClientRect().width ?? 0
    };
  });
  await $('.content-filters').$('button*=Drafts').click();
  await browser.waitUntil(
    () => browser.execute(() => document.querySelector('.view-switch') === null),
    { timeout: 5_000, timeoutMsg: 'all-content view controls remained visible in Drafts' }
  );
  const draftToolbarGeometry = await browser.execute(() => {
    const sticky = document.querySelector<HTMLElement>('.wiki-content-sticky');
    const summary = sticky?.querySelector<HTMLElement>('.wiki-journey-compact');
    const actions = sticky?.querySelector<HTMLElement>('.content-tabs-actions');
    return {
      stickyHeight: sticky?.getBoundingClientRect().height ?? 0,
      summaryWidth: summary?.getBoundingClientRect().width ?? 0,
      actionsWidth: actions?.getBoundingClientRect().width ?? 0
    };
  });
  expect(Math.abs(draftToolbarGeometry.stickyHeight - contentToolbarGeometry.stickyHeight)).toBeLessThan(1);
  expect(Math.abs(draftToolbarGeometry.summaryWidth - contentToolbarGeometry.summaryWidth)).toBeLessThan(1);
  expect(Math.abs(draftToolbarGeometry.actionsWidth - contentToolbarGeometry.actionsWidth)).toBeLessThan(1);
  await $('.content-filters').$('button*=All').click();
  await browser.waitUntil(
    () => browser.execute(() => document.querySelector('.view-switch') !== null),
    { timeout: 5_000, timeoutMsg: 'all-content view controls did not return after leaving Drafts' }
  );
  if (process.env.AIRWIKI_E2E_CAPTURE_JOURNEY === '1') {
    await browser.saveScreenshot(join(process.cwd(), '.artifacts', 'visual', `wiki-content-scroll-review-${captureTheme}.png`));
    await setCssViewport(1440, 900);
    await browser.execute(() => {
      const page = document.querySelector<HTMLElement>('.drive-page');
      if (page) page.scrollTop = 10_000;
    });
    await waitForVisualPaint('library');
    await browser.saveScreenshot(join(process.cwd(), '.artifacts', 'visual', `wiki-content-scroll-wide-${captureTheme}.png`));
    await setCssViewport(1180, 760);
  }
  const assurance = await $('.concept-assurance');
  await expect(assurance).toHaveText(expect.stringContaining('Reference'));
  await expect(assurance).toHaveText(expect.stringContaining('Current'));
  await expect(assurance).toHaveText(expect.stringContaining('process:e2e'));
  await expect(assurance).not.toHaveText(expect.stringContaining('Unverified'));
}

async function genericMcpArticle() {
  for (const article of await $$('.integration-list article')) {
    if ((await article.getText()).includes('Generic MCP client')) return article;
  }
  throw new Error('generic MCP article is missing');
}

async function clickGenericMcpAction(label: 'Connect' | 'Disconnect'): Promise<void> {
  try {
    await browser.waitUntil(
      () => browser.execute((expectedLabel) => {
        const article = Array.from(document.querySelectorAll('.integration-list article'))
          .find((candidate) => candidate.textContent?.includes('Generic MCP client') === true);
        const button = article
          ? Array.from(article.querySelectorAll<HTMLButtonElement>('button'))
            .find((candidate) => candidate.textContent?.trim() === expectedLabel)
          : undefined;
        if (!button || button.disabled) return false;
        button.click();
        return true;
      }, label),
      { timeout: 30_000, timeoutMsg: `the generic MCP ${label} action did not become available` }
    );
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      actionMessage: document.querySelector('.action-message')?.textContent?.trim() ?? null,
      integrations: Array.from(document.querySelectorAll('.integration-list article'))
        .map((article) => ({
          text: article.textContent?.trim().slice(0, 160) ?? '',
          buttons: Array.from(article.querySelectorAll<HTMLButtonElement>('button'))
            .map((button) => ({ text: button.textContent?.trim() ?? '', disabled: button.disabled }))
        })),
      route: window.location.hash
    }));
    throw new Error(`generic MCP ${label} failed: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
}

async function exerciseProjectMemory(client: McpStdioClient): Promise<void> {
  const projectRoot = process.env.AIRWIKI_E2E_PROJECT_FOLDER;
  if (!projectRoot) throw new Error('missing AIRWIKI_E2E_PROJECT_FOLDER');
  const projectName = 'E2E portable project';
  const initialized = await client.callTool('initialize_airwiki_project', {
    project_root: projectRoot,
    name: projectName
  });
  expect(initialized.status).toBe('awaiting_confirmation');

  await browser.waitUntil(
    () => browser.execute(() => document.querySelector('#project-memory-requests-title') !== null),
    { timeout: 10_000, timeoutMsg: 'the project-memory confirmation did not reach the UI' }
  );
  const request = await $('[aria-labelledby="project-memory-requests-title"] .computation-list article');
  await expect(request).toHaveText(expect.stringContaining('E2E portable project'));
  await request.$('button*=Approve').click();
  await browser.waitUntil(
    () => browser.execute(() => document.querySelector('#project-memory-requests-title') === null),
    { timeout: 15_000, timeoutMsg: 'approved project-memory request remained pending' }
  );
  await returnToLibrary();
  await browser.waitUntil(
    () => browser.execute((name) => Array.from(document.querySelectorAll('.wiki-row'))
      .some((row) => row.textContent?.includes(name) === true), projectName),
    { timeout: 15_000, timeoutMsg: 'approved project memory did not become active in Library' }
  );

  const opened = await client.callTool('open_airwiki_project', { project_root: projectRoot });
  expect(opened.status).toBe('ready');
  const wikiId = stringField(opened, 'wiki_id');
  const created = await client.callTool('write_airwiki_memory', {
    wiki_id: wikiId,
    concept_id: null,
    expected_fingerprint: null,
    title: 'Portable project decision',
    description: 'Synthetic project-memory E2E fixture',
    concept_type: 'Decision',
    tags: ['e2e', 'project'],
    body_markdown: '# Portable project decision\n\nKeep this synthetic conclusion with the project.\n'
  });
  const conceptId = stringField(created, 'conceptId');
  const searched = await client.callTool('search_airwiki_memory', {
    wiki_id: wikiId,
    query: 'synthetic conclusion',
    limit: 10
  });
  if (!Array.isArray(searched.matches) || searched.matches.length !== 1) {
    throw new Error('project-memory search did not return the written concept');
  }
  expect(stringField(record(searched.matches[0], 'project-memory match'), 'conceptId')).toBe(conceptId);
  const read = await client.callTool('get_airwiki_memory', {
    wiki_id: wikiId,
    concept_id: conceptId
  });
  if (!Array.isArray(read.concepts) || read.concepts.length !== 1) {
    throw new Error('project-memory read did not return the written concept');
  }
  expect(record(read.concepts[0], 'project-memory concept').bodyMarkdown)
    .toBe('# Portable project decision\n\nKeep this synthetic conclusion with the project.');

  const row = await $(`.wiki-row*=${projectName}`);
  await row.click();
  await $('.wiki-content-sticky').$('button*=Details').click();
  await expect($('.project-memory-details')).toBeDisplayed();
  await $('.project-memory-details').$('button*=Detach').click();
  await browser.waitUntil(
    () => browser.execute((name) => !Array.from(document.querySelectorAll('.wiki-row'))
      .some((candidate) => candidate.textContent?.includes(name) === true), projectName),
    { timeout: 10_000, timeoutMsg: 'detached project memory remained in the app' }
  );
  expect(existsSync(join(projectRoot, '.airwiki', 'project.yaml'))).toBe(true);
}

async function exerciseGenericMcpMemory(): Promise<void> {
  const expectedTools = [
    'search_airwiki',
    'list_airwiki_memories',
    'create_airwiki_memory',
    'initialize_airwiki_project',
    'open_airwiki_project',
    'search_airwiki_memory',
    'get_airwiki_memory',
    'write_airwiki_memory',
    'deprecate_airwiki_memory',
    'request_airwiki_computation',
    'get_airwiki_computation_run'
  ];
  await openAiAppsSettings();
  try {
    await browser.waitUntil(
      () => browser.execute(() => Array.from(document.querySelectorAll('.integration-list article'))
        .some((article) => article.textContent?.includes('Generic MCP client') === true)),
      { timeout: 30_000, timeoutMsg: 'the generic MCP integration was not discovered' }
    );
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      actionMessage: document.querySelector('.action-message')?.textContent?.trim() ?? null,
      integrations: Array.from(document.querySelectorAll('.integration-list article'))
        .map((article) => article.textContent?.trim().slice(0, 120) ?? ''),
      route: window.location.hash
    }));
    throw new Error(`generic MCP discovery failed: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
  await clickGenericMcpAction('Connect');
  let article: WebdriverIO.Element;
  try {
    await browser.waitUntil(
      () => browser.execute(() => Array.from(document.querySelectorAll('.integration-list article'))
        .some((candidate) => candidate.textContent?.includes('Generic MCP client') === true
          && candidate.querySelector('.mcp-setup pre') !== null)),
      { timeout: 30_000, timeoutMsg: 'the generic MCP bridge was not provisioned by the real UI action' }
    );
  } catch (error) {
    article = await genericMcpArticle();
    throw new Error(`generic MCP provisioning failed: ${await article.getText()}`, { cause: error });
  }

  article = await genericMcpArticle();
  await expect(article).toHaveText(expect.stringContaining('Local connection'));
  await expect(article).toHaveText(expect.stringContaining('Assisted memory'));
  await expect(article).toHaveText(expect.stringContaining('Included with the connection'));
  const setupText = await article.$('.mcp-setup pre').getText();
  const setup = record(JSON.parse(setupText), 'generic MCP setup');
  const command = stringField(setup, 'command');
  const rawArgs = setup.args;
  if (!Array.isArray(rawArgs) || !rawArgs.every((argument) => typeof argument === 'string')) {
    throw new Error('generic MCP setup args are invalid');
  }

  const memoryName = 'E2E agent memory';
  const client = new McpStdioClient(command, rawArgs);
  try {
    await client.discover();
    const listedTools = await client.request('tools/list', {});
    const tools = listedTools.tools;
    if (!Array.isArray(tools)) throw new Error('MCP tools/list did not return tools');
    const toolNames = tools.map((tool) => stringField(record(tool, 'MCP tool'), 'name'));
    expect([...toolNames].sort()).toEqual([...expectedTools].sort());
    expect(toolNames.some((name) => /delete|share|verify/i.test(name))).toBe(false);
    for (const tool of tools) {
      const schema = record(record(tool, 'MCP tool').inputSchema, 'MCP input schema');
      expect(schema.type).toBe('object');
    }

    await exerciseProjectMemory(client);

    const created = await client.callTool('create_airwiki_memory', { name: memoryName });
    const wikiId = stringField(created, 'wikiId');
    await browser.waitUntil(
      () => browser.execute((name) => Array.from(document.querySelectorAll('.wiki-row'))
        .some((row) => row.textContent?.includes(name) === true), memoryName),
      { timeout: 10_000, timeoutMsg: 'the MCP-created wiki did not refresh in the live UI' }
    );

    const first = await client.callTool('write_airwiki_memory', {
      wiki_id: wikiId,
      concept_id: null,
      expected_fingerprint: null,
      title: 'Portable agent memory',
      description: 'Synthetic cross-platform MCP fixture',
      concept_type: 'Decision',
      tags: ['e2e', 'agent'],
      body_markdown: '# Portable agent memory\n\nInitial synthetic decision.\n'
    });
    const conceptId = stringField(first, 'conceptId');
    const firstFingerprint = stringField(first, 'fingerprint');
    expect(first.status).toBe('stable');
    await returnToLibrary();
    await $(`.wiki-row*=${memoryName}`).click();
    await expect($('.file-list')).toHaveText(expect.stringContaining('Portable agent memory'));

    const updated = await client.callTool('write_airwiki_memory', {
      wiki_id: wikiId,
      concept_id: conceptId,
      expected_fingerprint: firstFingerprint,
      title: 'Portable agent memory updated',
      description: 'Synthetic cross-platform MCP fixture',
      concept_type: 'Decision',
      tags: ['e2e', 'agent'],
      body_markdown: '# Portable agent memory updated\n\nUpdated synthetic decision.\n'
    });
    const updatedFingerprint = stringField(updated, 'fingerprint');
    expect(updatedFingerprint).not.toBe(firstFingerprint);
    await browser.waitUntil(
      () => browser.execute(() => Array.from(document.querySelectorAll('.file-list strong'))
        .some((title) => title.textContent?.trim() === 'Portable agent memory updated')),
      { timeout: 10_000, timeoutMsg: 'the open AI-memory wiki did not refresh after an MCP write' }
    );

    let staleRejected = false;
    try {
      await client.callTool('write_airwiki_memory', {
        wiki_id: wikiId,
        concept_id: conceptId,
        expected_fingerprint: firstFingerprint,
        title: 'Stale update must not persist',
        description: '',
        concept_type: 'Decision',
        tags: [],
        body_markdown: '# Stale update\n'
      });
    } catch (error) {
      staleRejected = true;
      expect(error instanceof Error ? error.message : '').toContain('conflict');
    }
    expect(staleRejected).toBe(true);

    const afterStale = await client.callTool('get_airwiki_memory', { wiki_id: wikiId });
    if (!Array.isArray(afterStale.concepts)) throw new Error('memory read did not return concepts');
    const persisted = record(afterStale.concepts[0], 'persisted concept');
    expect(persisted.title).toBe('Portable agent memory updated');
    expect(persisted.fingerprint).toBe(updatedFingerprint);
    expect(persisted.bodyMarkdown).toBeNull();
    expect(afterStale.nextCursor).toBeNull();

    const targetedRead = await client.callTool('get_airwiki_memory', {
      wiki_id: wikiId,
      concept_id: conceptId
    });
    if (!Array.isArray(targetedRead.concepts) || targetedRead.concepts.length !== 1) {
      throw new Error('targeted memory read did not return exactly one concept');
    }
    const readable = record(targetedRead.concepts[0], 'targeted memory concept');
    expect(readable.fingerprint).toBe(updatedFingerprint);
    expect(readable.bodyMarkdown).toBe('# Portable agent memory updated\n\nUpdated synthetic decision.');
    expect(targetedRead.nextCursor).toBeNull();
    await $('.file-list').$('button*=Portable agent memory updated').click();
    await expect($('.concept-assurance')).toHaveText(expect.stringContaining('Reviewed'));

    const deprecated = await client.callTool('deprecate_airwiki_memory', {
      wiki_id: wikiId,
      concept_id: conceptId,
      expected_fingerprint: updatedFingerprint
    });
    expect(deprecated.status).toBe('deprecated');
    await browser.waitUntil(
      () => browser.execute(() => document.querySelector('.concept-assurance') === null),
      { timeout: 10_000, timeoutMsg: 'the open AI-memory page was not invalidated after deprecation' }
    );
    await $('.file-list').$('button*=Portable agent memory updated').click();
    await expect($('.concept-assurance')).toHaveText(expect.stringContaining('Deprecated'));

    await openAiAppsSettings();
    await clickGenericMcpAction('Disconnect');
    await browser.waitUntil(
      () => browser.execute(() => Array.from(document.querySelectorAll('.integration-list article'))
        .some((candidate) => candidate.textContent?.includes('Generic MCP client') === true
          && candidate.textContent.includes('Available')
          && candidate.querySelector('.mcp-setup') === null)),
      { timeout: 10_000, timeoutMsg: 'the generic MCP capability was not revoked by the UI' }
    );
    let revoked = false;
    try {
      await client.callTool('list_airwiki_memories', {});
    } catch (error) {
      revoked = true;
      expect(error instanceof Error ? error.message : '').toContain('authorization_required');
    }
    expect(revoked).toBe(true);
  } finally {
    await client.close();
  }

  const disconnectedClient = new McpStdioClient(command, rawArgs);
  try {
    await disconnectedClient.discover();
    const listedTools = await disconnectedClient.request('tools/list', {});
    const tools = listedTools.tools;
    if (!Array.isArray(tools)) throw new Error('disconnected MCP tools/list did not return tools');
    expect(tools.map((tool) => stringField(record(tool, 'MCP tool'), 'name'))).toEqual(['search_airwiki']);
  } finally {
    await disconnectedClient.close();
  }
  await returnToLibrary();
}

describe('AirWiki real IPC journey', () => {
  it('persists onboarding and explicit appearance preferences', async () => {
    try {
      await browser.waitUntil(
        () => browser.execute(() => Boolean(document.querySelector('main.onboarding:not(.startup)'))),
        { timeout: 30_000, timeoutMsg: 'the runtime did not reach interactive onboarding' }
      );
    } catch (error) {
      const diagnostic = await browser.execute(() => ({
        bodyText: document.body.textContent?.trim().slice(0, 160) ?? '',
        mainClass: document.querySelector('main')?.className ?? null,
        readyState: document.readyState,
      }));
      throw new Error(`interactive onboarding was unavailable: ${JSON.stringify(diagnostic)}`, { cause: error });
    }
    const onboarding = await $('main.onboarding:not(.startup)');
    await expect(onboarding).toBeDisplayed();

    await selectValue('main.onboarding:not(.startup) select', 0, 'en');
    const language = required((await $$('main.onboarding:not(.startup) select'))[0], 'language preference');
    await expect(language).toHaveValue('en');
    await $('button.onboarding-next').click();

    // WebKit WebDriver misreports this visible text button as hidden; the
    // component test covers ordinary pointer activation.
    const skippedFolder = await browser.execute(() => {
      const button = document.querySelector<HTMLButtonElement>('.onboarding-folder-choice button.text-action');
      button?.click();
      return button?.textContent?.trim() ?? null;
    });
    expect(skippedFolder).toBe('Continue without a folder');
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
    await expect($('.system-status-button')).toBeDisplayed();
    expect(await $('.system-status-button').getAttribute('aria-label')).toContain('Settings');
    expect(await $$('.status-segment')).toHaveLength(3);
    expect(await $('.system-status-bar').isExisting()).toBe(false);
    expect(await measureNavigationPaintP95()).toBeLessThanOrEqual(100);

    const globalSearch = await $('#global-search');
    await globalSearch.click();
    expect(await browser.execute(() => document.activeElement?.id)).toBe('global-search');
    await globalSearch.setValue('focus regression');
    await expect(globalSearch).toHaveValue('focus regression');
    await globalSearch.clearValue();

    const devicePixelRatio = await browser.execute(() => window.devicePixelRatio || 1);
    for (const viewport of [
      { width: 1180, height: 760 },
      { width: 1440, height: 900 }
    ]) {
      let physicalWidth = Math.ceil(viewport.width * devicePixelRatio);
      const physicalHeight = Math.ceil(viewport.height * devicePixelRatio);
      let dimensions: {
        clientWidth: number;
        scrollWidth: number;
        statusWidth: number;
        statusHeight: number;
        headerBottom: number;
        viewportHeight: number;
      } | undefined;
      for (let attempt = 0; attempt < 3; attempt += 1) {
        await browser.setWindowSize(physicalWidth, physicalHeight);
        dimensions = await browser.execute(() => {
          const statusButton = document.querySelector<HTMLElement>('.system-status-button');
          const statusRect = statusButton?.getBoundingClientRect();
          const headerRect = document.querySelector<HTMLElement>('.top-bar')?.getBoundingClientRect();
          return {
            clientWidth: document.documentElement.clientWidth,
            scrollWidth: document.documentElement.scrollWidth,
            statusWidth: statusRect?.width ?? 0,
            statusHeight: statusRect?.height ?? 0,
            headerBottom: headerRect?.bottom ?? Number.POSITIVE_INFINITY,
            viewportHeight: window.innerHeight
          };
        });
        if (dimensions.clientWidth >= viewport.width) break;
        physicalWidth += Math.ceil((viewport.width - dimensions.clientWidth) * devicePixelRatio);
      }
      dimensions = required(dimensions, 'responsive viewport dimensions');
      expect(dimensions.clientWidth).toBeGreaterThanOrEqual(viewport.width);
      expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
      expect(dimensions.statusWidth).toBe(44);
      expect(dimensions.statusHeight).toBe(44);
      expect(dimensions.headerBottom).toBeLessThanOrEqual(dimensions.viewportHeight);
    }

    await $('.system-status-button').click();
    await $('.settings-layout').waitForDisplayed();
    const settingsShell = await browser.execute(() => {
      const main = document.querySelector<HTMLElement>('.drive-page');
      const topBar = document.querySelector<HTMLElement>('.settings-top-bar');
      return {
        documentScrollTop: document.scrollingElement?.scrollTop ?? -1,
        mainScrollTop: main?.scrollTop ?? -1,
        topBarTop: topBar?.getBoundingClientRect().top ?? -1,
        ordinaryHeaderPresent: document.querySelector('.top-bar, .global-search, .system-status-button') !== null,
        sidebarPresent: document.querySelector('.settings-sidebar') !== null
      };
    });
    expect(settingsShell.documentScrollTop).toBe(0);
    expect(settingsShell.mainScrollTop).toBe(0);
    expect(settingsShell.topBarTop).toBe(0);
    expect(settingsShell.ordinaryHeaderPresent).toBe(false);
    expect(settingsShell.sidebarPresent).toBe(true);
    expect((await browser.getUrl()).endsWith('#settings/general')).toBe(true);

    await browser.execute(() => { window.location.hash = 'system/preferences'; });
    await browser.waitUntil(
      async () => (await browser.getUrl()).endsWith('#settings/general'),
      { timeout: 10_000, timeoutMsg: 'legacy General route did not canonicalize' }
    );
    const preferenceSelects = await $$('.device-preferences-form select');
    const appearance = required(preferenceSelects[1], 'appearance preference');
    await selectValue('.device-preferences-form select', 1, 'dark');
    await expect(appearance).toHaveValue('dark');
    await $('.settings-form-actions button.primary').click();
    await browser.waitUntil(async () => (
      await $('html').getAttribute('data-theme') === 'dark'
    ), { timeout: 10_000, timeoutMsg: 'the persisted theme was not applied' });

    const route = await browser.getUrl();
    expect(route.endsWith('#settings/general')).toBe(true);
    const layout = await browser.execute(() => ({
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth
    }));
    expect(layout.scrollWidth).toBeLessThanOrEqual(layout.clientWidth);

    await browser.execute(() => { window.location.hash = 'system/connectivity'; });
    await browser.waitUntil(
      async () => (await browser.getUrl()).endsWith('#settings/connections'),
      { timeout: 10_000, timeoutMsg: 'legacy Connections route did not canonicalize' }
    );
    await $('.private-network-section').waitForDisplayed();
    await $('.connection-advanced > summary').click();
    const desktopControlLayout = await browser.execute(() => {
      const disclosure = document.querySelector<HTMLElement>('.connection-advanced > summary');
      const statusGrid = document.querySelector<HTMLElement>('.connection-advanced > dl');
      const firstStatus = statusGrid?.querySelector<HTMLElement>('dd');
      const settings = document.querySelector<HTMLElement>('.settings-layout');
      return {
        disclosureDisplay: disclosure ? getComputedStyle(disclosure).display : '',
        statusDisplay: statusGrid ? getComputedStyle(statusGrid).display : '',
        statusColumns: statusGrid ? getComputedStyle(statusGrid).gridTemplateColumns.split(' ').length : 0,
        statusMarginLeft: firstStatus ? getComputedStyle(firstStatus).marginLeft : '',
        settingsClientWidth: settings?.clientWidth ?? 0,
        settingsScrollWidth: settings?.scrollWidth ?? Number.POSITIVE_INFINITY
      };
    });
    expect(desktopControlLayout.disclosureDisplay).toBe('flex');
    expect(desktopControlLayout.statusDisplay).toBe('grid');
    expect(desktopControlLayout.statusColumns).toBe(3);
    expect(desktopControlLayout.statusMarginLeft).toBe('0px');
    expect(desktopControlLayout.settingsScrollWidth).toBeLessThanOrEqual(desktopControlLayout.settingsClientWidth);

    await $('a[href="#settings/apps"]').click();
    await $('.integration-list').waitForDisplayed();
    const integrationColumns = await browser.execute(() => {
      const integration = document.querySelector<HTMLElement>('.integration-item');
      return integration ? getComputedStyle(integration).gridTemplateColumns.split(' ').length : 0;
    });
    expect(integrationColumns).toBe(2);
    await returnToLibrary();

    if (runVisualMatrix) await assertVisualMatrix();
    await configureVisualPreferences('en', process.env.AIRWIKI_E2E_JOURNEY_THEME === 'dark' ? 'dark' : 'light');
    await navigateToDestination(0);
    await exerciseGenericMcpMemory();
    await createFolderWiki();
    await importOkfWiki();
    await $('button*=AirWiki').click();
    await browser.waitUntil(
      () => browser.execute(() => {
        const page = document.querySelector<HTMLElement>('.route-page');
        if (!page || page.dataset.route !== 'library') return false;
        const style = getComputedStyle(page);
        return Number.parseFloat(style.opacity) >= 0.99 && style.visibility === 'visible';
      }),
      { timeout: 10_000, timeoutMsg: 'visible wiki list did not restore after the OKF journey' }
    );
    await expect($('h1=Your Wikis')).toBeDisplayed();
    await expect($('.library-filter-group')).toHaveText(expect.stringContaining('All'));
    await expect($('.library-filter-group')).toHaveText(expect.stringContaining('Needs attention'));
    await expect($('.library-filter-group')).toHaveText(expect.stringContaining('Only you'));
    await expect($('.library-filter-group')).toHaveText(expect.stringContaining('Shared'));
    await expect($('.library-scope-tabs')).toHaveText(expect.stringContaining('On this device'));
    await expect($('.library-scope-tabs')).toHaveText(expect.stringContaining('Public'));
    expect(await $$('.wiki-row')).toHaveLength(3);
    expect(await $$('.wiki-table-head')).toHaveLength(0);
    const libraryRows = await browser.execute(() => Array.from(document.querySelectorAll<HTMLElement>('.wiki-row')).map((row) => {
      const shelf = row.closest<HTMLElement>('.wiki-library-shelf');
      const icon = row.querySelector<HTMLElement>('.wiki-icon');
      return {
        height: row.getBoundingClientRect().height,
        summaryParts: row.querySelectorAll('.wiki-row-summary > *').length,
        exposureItems: row.querySelectorAll('.wiki-row-exposure-text > span').length,
        hasOpenLabel: row.querySelector('.wiki-row-open')?.textContent?.includes('Open Wiki') === true,
        shelfRadius: shelf ? Number.parseFloat(getComputedStyle(shelf).borderTopLeftRadius) : null,
        iconShadow: icon ? getComputedStyle(icon).boxShadow : null,
      };
    }));
    for (const row of libraryRows) {
      expect(row.height).toBeGreaterThanOrEqual(68);
      expect(row.height).toBeLessThanOrEqual(78);
      expect(row.summaryParts).toBe(2);
      expect(row.exposureItems).toBe(3);
      expect(row.hasOpenLabel).toBe(false);
      expect(row.shelfRadius).not.toBeNull();
      expect(row.shelfRadius ?? Number.POSITIVE_INFINITY).toBeLessThanOrEqual(6);
      expect(row.iconShadow).toBe('none');
    }
    await $('.library-scope-tabs').$('button*=Public').click();
    await browser.waitUntil(
      () => browser.execute(() => window.location.hash === '#library/public'
        && document.querySelector('.public-library-state') !== null),
      { timeout: 10_000, timeoutMsg: 'explicit public catalog exploration did not settle' }
    );
    await expect($('h1=Explore public Wikis')).toBeDisplayed();
    await expect($('.public-library-state')).toHaveText(expect.stringContaining('No public index is configured'));
    expect(await $$('.public-wiki-row')).toHaveLength(0);
    await $('.library-scope-tabs').$('button*=On this device').click();
    await browser.waitUntil(
      () => browser.execute(() => document.querySelectorAll('.wiki-row').length === 3),
      { timeout: 10_000, timeoutMsg: 'local Wiki rows did not return after public exploration' }
    );
    if (process.env.AIRWIKI_E2E_CAPTURE_LIBRARY === '1') {
      const theme = process.env.AIRWIKI_E2E_JOURNEY_THEME === 'dark' ? 'dark' : 'light';
      await browser.saveScreenshot(join(process.cwd(), '.artifacts', 'visual', `wiki-list-review-${theme}.png`));
      await setCssViewport(1020, 728);
      await browser.saveScreenshot(join(process.cwd(), '.artifacts', 'visual', `wiki-list-review-${theme}-narrow.png`));
    }
  });
});
