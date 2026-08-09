import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const executable = process.platform === 'win32' ? 'airwiki.exe' : 'airwiki';
const appBinaryPath = resolve(uiRoot, '../../../target/debug', executable);
const testRoot = mkdtempSync(join(tmpdir(), 'airwiki-e2e-'));
const expectedPrefix = resolve(tmpdir()) + sep;
if (!resolve(testRoot).startsWith(expectedPrefix)) throw new Error('unsafe E2E data root');

async function waitForWebDriver(child) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`AirWiki exited before WebDriver became ready (${child.exitCode})`);
    }
    try {
      const response = await fetch('http://127.0.0.1:4445/status');
      if (response.ok) return;
    } catch {
      // Startup races are expected until the local server binds.
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
  }
  throw new Error('AirWiki WebDriver did not become ready within 30 seconds');
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

const app = spawn(appBinaryPath, [], {
  env: {
    ...process.env,
    AIRWIKI_E2E_DATA_ROOT: testRoot,
    TAURI_WEBDRIVER_PORT: '4445'
  },
  stdio: 'inherit'
});

try {
  await waitForWebDriver(app);
  const wdioEntry = fileURLToPath(import.meta.resolve('@wdio/cli'));
  const wdioCli = resolve(dirname(wdioEntry), '..', 'bin', 'wdio.js');
  const result = spawnSync(process.execPath, [wdioCli, 'run', 'wdio.conf.ts'], {
    cwd: uiRoot,
    env: { ...process.env, AIRWIKI_E2E_DATA_ROOT: testRoot },
    stdio: 'inherit'
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exitCode = result.status ?? 1;
} finally {
  await stopApp(app);
  rmSync(testRoot, { recursive: true, force: true });
}
