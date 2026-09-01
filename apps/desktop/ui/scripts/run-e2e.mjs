import { spawn, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const executable = process.platform === 'win32' ? 'airwiki.exe' : 'airwiki';
const debugTarget = process.platform === 'win32'
  ? '../../../target/x86_64-pc-windows-msvc/debug'
  : '../../../target/debug';
const appBinaryPath = resolve(uiRoot, debugTarget, executable);
const testRoot = realpathSync(mkdtempSync(join(tmpdir(), 'airwiki-e2e-')));
const expectedPrefix = realpathSync(tmpdir()) + sep;
if (!resolve(testRoot).startsWith(expectedPrefix)) throw new Error('unsafe E2E data root');
const sourceFixture = join(testRoot, 'fixtures', 'source');
const okfFixture = join(testRoot, 'fixtures', 'okf-v02');
const projectFixture = join(testRoot, 'fixtures', 'project-memory');
const freshnessDeadline = new Date();
freshnessDeadline.setUTCFullYear(freshnessDeadline.getUTCFullYear() + 1);
const staleAfter = freshnessDeadline.toISOString().slice(0, 10);

async function availableLoopbackPort(purpose) {
  const server = createServer();
  await new Promise((resolveListen, rejectListen) => {
    server.once('error', rejectListen);
    server.listen(0, '127.0.0.1', resolveListen);
  });
  const address = server.address();
  if (!address || typeof address === 'string') {
    server.close();
    throw new Error(`the E2E runner could not reserve a loopback ${purpose} port`);
  }
  await new Promise((resolveClose, rejectClose) => {
    server.close((error) => error ? rejectClose(error) : resolveClose());
  });
  return address.port;
}

const [e2eMcpPort, e2eWebDriverPort] = await Promise.all([
  availableLoopbackPort('MCP'),
  availableLoopbackPort('WebDriver')
]);
const webDriverUrl = `http://127.0.0.1:${e2eWebDriverPort}`;
mkdirSync(sourceFixture, { recursive: true });
mkdirSync(join(okfFixture, 'architecture'), { recursive: true });
mkdirSync(projectFixture, { recursive: true });
writeFileSync(join(sourceFixture, 'synthetic-source.md'), [
  '# Synthetic source',
  '',
  'This document exists only for the isolated desktop E2E journey.',
  ''
].join('\n'));
writeFileSync(join(okfFixture, 'index.md'), [
  '---',
  'okf_version: "0.2"',
  '---',
  '# Synthetic OKF bundle',
  '',
  '- [Architecture decision](architecture/decision.md)',
  ''
].join('\n'));
writeFileSync(join(okfFixture, 'architecture', 'index.md'), [
  '# Architecture',
  '',
  '- [Decision](decision.md)',
  '- [Verified reference](verified.md)',
  ''
].join('\n'));
writeFileSync(join(okfFixture, 'architecture', 'decision.md'), [
  '---',
  'type: Decision',
  'title: Synthetic architecture decision',
  'status: stable',
  'x-e2e-extension:',
  '  preserved: true',
  '---',
  '# Synthetic architecture decision',
  '',
  'The E2E runner preserves this portable OKF concept.',
  ''
].join('\n'));
writeFileSync(join(okfFixture, 'architecture', 'verified.md'), [
  '---',
  'type: Reference',
  'title: Verified architecture reference',
  'generated:',
  '  by: process:e2e',
  '  at: 2026-08-13T09:00:00Z',
  'verified:',
  '  by: human:e2e',
  '  at: 2026-08-13T09:01:00Z',
  `stale_after: ${staleAfter}`,
  'status: stable',
  'sources:',
  '  - id: synthetic-fixture',
  '    resource: urn:airwiki:e2e',
  '---',
  '# Verified architecture reference',
  '',
  'This concept proves that assurance changes atomically with the selected page.',
  ''
].join('\n'));

async function waitForWebDriver(child) {
  const deadline = Date.now() + 30_000;
  let lastError;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`AirWiki exited before WebDriver became ready (${child.exitCode})`);
    }
    try {
      const response = await fetch(`${webDriverUrl}/status`);
      if (response.ok) {
        const sessionId = await createWebDriverSession('readiness');
        try {
          await deleteWebDriverSession(sessionId, 'readiness');
        } catch (error) {
          const detail = error instanceof Error ? `: ${error.message}` : '';
          throw new Error(
            `AirWiki WebDriver readiness session was created but could not be closed; refusing to retry${detail}`,
            { cause: error }
          );
        }
        return;
      }
    } catch (error) {
      if (error instanceof Error && error.message.startsWith(
        'AirWiki WebDriver readiness session was created but could not be closed'
      )) {
        throw error;
      }
      lastError = error;
      // Startup races are expected until the local server can attach to a window.
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
  }
  const detail = lastError instanceof Error ? ` (${lastError.message})` : '';
  const appState = child.exitCode === null
    ? 'AirWiki is still running'
    : `AirWiki exited with ${child.exitCode}`;
  throw new Error(
    `AirWiki WebDriver did not become ready with main window within 30 seconds; ${appState}${detail}`
  );
}

async function stopApp(child) {
  if (child.exitCode !== null) return;
  const exited = new Promise((resolveExit) => child.once('exit', resolveExit));
  child.kill();
  await Promise.race([
    exited,
    new Promise((resolveDelay) => setTimeout(resolveDelay, 5_000))
  ]);
  if (child.exitCode === null) child.kill('SIGKILL');
}

async function describeWebDriverFailure(response, context) {
  let detail = '';
  try {
    const payload = await response.json();
    const error = payload?.value?.error;
    const message = payload?.value?.message;
    if (typeof error === 'string' && typeof message === 'string') {
      detail = `: ${error}: ${message}`;
    } else if (typeof message === 'string') {
      detail = `: ${message}`;
    } else if (typeof error === 'string') {
      detail = `: ${error}`;
    }
  } catch {
    // A non-JSON error response is still reported with its HTTP status.
  }
  return `${context} (${response.status})${detail}`;
}

async function createWebDriverSession(context) {
  const response = await fetch(`${webDriverUrl}/session`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      capabilities: {
        alwaysMatch: { 'wdio:tauriServiceOptions': { windowLabel: 'main' } },
        firstMatch: [{}]
      }
    })
  });
  if (!response.ok) {
    throw new Error(await describeWebDriverFailure(response, `could not create ${context} WebDriver session`));
  }
  const payload = await response.json();
  const sessionId = payload?.value?.sessionId;
  if (typeof sessionId !== 'string' || sessionId.length === 0) {
    throw new Error(`${context} WebDriver session returned no session ID`);
  }
  return sessionId;
}

async function deleteWebDriverSession(sessionId, context) {
  const response = await fetch(`${webDriverUrl}/session/${sessionId}`, {
    method: 'DELETE'
  });
  if (!response.ok) {
    throw new Error(await describeWebDriverFailure(response, `could not close ${context} WebDriver session`));
  }
}

async function requestGracefulShutdown(child) {
  const sessionId = await createWebDriverSession('shutdown');
  try {
    await fetch(`${webDriverUrl}/session/${sessionId}/execute/sync`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        script: "return window.__TAURI_INTERNALS__.invoke('quit_completely')",
        args: []
      })
    });
  } catch {
    // The local server may close before returning the command response.
  }

  if (child.exitCode !== null) return;
  const exitCode = await Promise.race([
    new Promise((resolveExit) => child.once('exit', resolveExit)),
    new Promise((_, rejectTimeout) => setTimeout(
      () => rejectTimeout(new Error('AirWiki did not complete graceful shutdown within 5 seconds')),
      5_000
    ))
  ]);
  if (exitCode !== 0) throw new Error(`AirWiki graceful shutdown returned ${exitCode}`);
}

const app = spawn(appBinaryPath, [], {
  env: {
    ...process.env,
    AIRWIKI_E2E_DATA_ROOT: testRoot,
    AIRWIKI_E2E_CONFIRMATIONS: 'allow',
    AIRWIKI_E2E_WIKI_FOLDER: sourceFixture,
    AIRWIKI_E2E_OKF_FOLDER: okfFixture,
    AIRWIKI_E2E_PROJECT_FOLDER: projectFixture,
    AIRWIKI_E2E_MCP_PORT: String(e2eMcpPort),
    TAURI_WEBDRIVER_PORT: String(e2eWebDriverPort)
  },
  stdio: 'inherit'
});

try {
  await waitForWebDriver(app);
  const wdioEntry = fileURLToPath(import.meta.resolve('@wdio/cli'));
  const wdioCli = resolve(dirname(wdioEntry), '..', 'bin', 'wdio.js');
  const result = spawnSync(process.execPath, [wdioCli, 'run', 'wdio.conf.ts'], {
    cwd: uiRoot,
    env: {
      ...process.env,
      AIRWIKI_E2E_DATA_ROOT: testRoot,
      AIRWIKI_E2E_PROJECT_FOLDER: projectFixture,
      AIRWIKI_E2E_MCP_PORT: String(e2eMcpPort),
      TAURI_WEBDRIVER_PORT: String(e2eWebDriverPort)
    },
    stdio: 'inherit'
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    process.exitCode = result.status ?? 1;
  } else {
    await requestGracefulShutdown(app);
  }
} finally {
  await stopApp(app);
  if (process.env.AIRWIKI_E2E_KEEP_DATA === '1') {
    console.error(`E2E data retained at ${testRoot}`);
  } else {
    rmSync(testRoot, { recursive: true, force: true });
  }
}
