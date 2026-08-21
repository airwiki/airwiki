import { $, $$, browser, expect } from '@wdio/globals';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
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

  async initialize(): Promise<void> {
    await this.request('initialize', {
      protocolVersion: '2026-07-28',
      capabilities: {},
      clientInfo: { name: 'airwiki-e2e-agent', version: '1' }
    });
    this.notify('notifications/initialized', {});
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
      this.write({ jsonrpc: '2.0', id, method, params });
    });
  }

  notify(method: string, params: Record<string, unknown>): void {
    this.write({ jsonrpc: '2.0', method, params });
  }

  async callTool(name: string, arguments_: Record<string, unknown>): Promise<Record<string, unknown>> {
    const result = await this.request('tools/call', { name, arguments: arguments_ });
    return record(result.structuredContent, `${name} structured content`);
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
}

async function waitForVisualPaint(route: 'wikis' | 'system'): Promise<void> {
  if (route === 'system') {
    await browser.waitUntil(
      () => browser.execute(() => document.querySelectorAll('#system-preferences select').length === 4),
      { timeout: 10_000, timeoutMsg: 'system preferences did not reach their complete DOM state' }
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
          await browser.execute(() => {
            if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
            document.querySelector('.action-message')?.remove();
            const style = document.createElement('style');
            style.id = 'visual-capture-styles';
            style.textContent = `
              .system-status-bar button:hover { color: var(--muted) !important; background: transparent !important; }
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

async function exerciseGenericMcpMemory(): Promise<void> {
  const expectedTools = [
    'search_airwiki',
    'list_airwiki_memories',
    'create_airwiki_memory',
    'get_airwiki_memory',
    'write_airwiki_memory',
    'deprecate_airwiki_memory',
    'request_airwiki_computation',
    'get_airwiki_computation_run'
  ];
  await $('button[aria-label^="AI apps:"]').click();
  await $('.connections-drawer').waitForDisplayed();
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
    await client.initialize();
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
    await $('.connections-drawer .icon-button').click();
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
      expect(error instanceof Error ? error.message : '').toContain('invalid');
    }
    expect(staleRejected).toBe(true);

    const afterStale = await client.callTool('get_airwiki_memory', { wiki_id: wikiId });
    if (!Array.isArray(afterStale.concepts)) throw new Error('memory read did not return concepts');
    const persisted = record(afterStale.concepts[0], 'persisted concept');
    expect(persisted.title).toBe('Portable agent memory updated');
    expect(persisted.fingerprint).toBe(updatedFingerprint);
    await $('.file-list').$('button*=Portable agent memory updated').click();
    await expect($('.concept-assurance')).toHaveText(expect.stringContaining('stable'));

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
    await expect($('.concept-assurance')).toHaveText(expect.stringContaining('deprecated'));

    await $('button[aria-label^="AI apps:"]').click();
    await $('.connections-drawer').waitForDisplayed();
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
      expect(error instanceof Error ? error.message : '').toContain('revoked');
    }
    expect(revoked).toBe(true);
  } finally {
    await client.close();
  }

  const disconnectedClient = new McpStdioClient(command, rawArgs);
  try {
    await disconnectedClient.initialize();
    const listedTools = await disconnectedClient.request('tools/list', {});
    const tools = listedTools.tools;
    if (!Array.isArray(tools)) throw new Error('disconnected MCP tools/list did not return tools');
    expect(tools.map((tool) => stringField(record(tool, 'MCP tool'), 'name'))).toEqual(['search_airwiki']);
  } finally {
    await disconnectedClient.close();
  }
  await $('.connections-drawer button[aria-label="Close"]').click();
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
      let dimensions: {
        clientWidth: number;
        scrollWidth: number;
        statusHeight: number;
        statusBottom: number;
        viewportHeight: number;
      } | undefined;
      for (let attempt = 0; attempt < 3; attempt += 1) {
        await browser.setWindowSize(physicalWidth, physicalHeight);
        dimensions = await browser.execute(() => {
          const statusBar = document.querySelector<HTMLElement>('.system-status-bar');
          const statusRect = statusBar?.getBoundingClientRect();
          return {
            clientWidth: document.documentElement.clientWidth,
            scrollWidth: document.documentElement.scrollWidth,
            statusHeight: statusRect?.height ?? 0,
            statusBottom: statusRect?.bottom ?? Number.POSITIVE_INFINITY,
            viewportHeight: window.innerHeight
          };
        });
        if (dimensions.clientWidth >= viewport.width) break;
        physicalWidth += Math.ceil((viewport.width - dimensions.clientWidth) * devicePixelRatio);
      }
      dimensions = required(dimensions, 'responsive viewport dimensions');
      expect(dimensions.clientWidth).toBeGreaterThanOrEqual(viewport.width);
      expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
      expect(dimensions.statusHeight).toBeGreaterThan(0);
      expect(dimensions.statusBottom).toBeLessThanOrEqual(dimensions.viewportHeight);
    }

    await $('button[aria-label="Settings"]').click();
    await $('#system-preferences').waitForDisplayed();
    const systemShell = await browser.execute(() => {
      const main = document.querySelector<HTMLElement>('.drive-page');
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
    expect(await browser.execute(() => document.querySelector<HTMLElement>('.drive-page')?.scrollTop ?? -1)).toBe(0);

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

    await browser.execute(() => { window.location.hash = 'system/connectivity'; });
    await $('.connections-drawer').waitForDisplayed();
    await $('.connection-advanced > summary').click();
    const desktopControlLayout = await browser.execute(() => {
      const indicator = document.querySelector<HTMLElement>('.check-indicator');
      const disclosure = document.querySelector<HTMLElement>('.connection-advanced > summary');
      const statusGrid = document.querySelector<HTMLElement>('.connection-advanced > dl');
      const firstStatus = statusGrid?.querySelector<HTMLElement>('dd');
      const drawer = document.querySelector<HTMLElement>('.connections-drawer');
      const integration = document.querySelector<HTMLElement>('.integration-item');
      return {
        checkboxWidth: indicator?.getBoundingClientRect().width ?? 0,
        checkboxHeight: indicator?.getBoundingClientRect().height ?? 0,
        disclosureDisplay: disclosure ? getComputedStyle(disclosure).display : '',
        statusDisplay: statusGrid ? getComputedStyle(statusGrid).display : '',
        statusColumns: statusGrid ? getComputedStyle(statusGrid).gridTemplateColumns.split(' ').length : 0,
        statusMarginLeft: firstStatus ? getComputedStyle(firstStatus).marginLeft : '',
        drawerClientWidth: drawer?.clientWidth ?? 0,
        drawerScrollWidth: drawer?.scrollWidth ?? Number.POSITIVE_INFINITY,
        integrationColumns: integration ? getComputedStyle(integration).gridTemplateColumns.split(' ').length : 0
      };
    });
    expect(desktopControlLayout.checkboxWidth).toBe(16);
    expect(desktopControlLayout.checkboxHeight).toBe(16);
    expect(desktopControlLayout.disclosureDisplay).toBe('flex');
    expect(desktopControlLayout.statusDisplay).toBe('grid');
    expect(desktopControlLayout.statusColumns).toBe(3);
    expect(desktopControlLayout.statusMarginLeft).toBe('0px');
    expect(desktopControlLayout.drawerScrollWidth).toBeLessThanOrEqual(desktopControlLayout.drawerClientWidth);
    expect(desktopControlLayout.integrationColumns).toBe(2);
    await $('.connections-drawer button[aria-label="Close"]').click();

    if (runVisualMatrix) await assertVisualMatrix();
    await configureVisualPreferences('en', 'light');
    await navigateToDestination(0);
    await exerciseGenericMcpMemory();
    await createFolderWiki();
    await importOkfWiki();
    await $('button*=AirWiki').click();
    await browser.waitUntil(
      () => browser.execute(() => {
        const page = document.querySelector<HTMLElement>('.route-page');
        if (!page || page.dataset.route !== 'wikis') return false;
        const style = getComputedStyle(page);
        return Number.parseFloat(style.opacity) >= 0.99 && style.visibility === 'visible';
      }),
      { timeout: 10_000, timeoutMsg: 'visible wiki list did not restore after the OKF journey' }
    );
  });
});
