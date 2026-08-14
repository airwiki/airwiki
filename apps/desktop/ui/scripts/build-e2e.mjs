import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const desktopRoot = resolve(uiRoot, '..');
const svelteCheckPackage = fileURLToPath(import.meta.resolve('svelte-check/package.json'));
const svelteCheckCli = resolve(dirname(svelteCheckPackage), 'bin', 'svelte-check');
const vitePackage = fileURLToPath(import.meta.resolve('vite/package.json'));
const viteCli = resolve(dirname(vitePackage), 'bin', 'vite.js');
const tauriEntry = fileURLToPath(import.meta.resolve('@tauri-apps/cli'));
const tauriCli = resolve(dirname(tauriEntry), 'tauri.js');

function runNode(arguments_, cwd) {
  const result = spawnSync(process.execPath, arguments_, { cwd, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

runNode([svelteCheckCli, '--tsconfig', './tsconfig.json'], uiRoot);
runNode([viteCli, 'build', '--mode', 'e2e'], uiRoot);

const bridgeArguments = ['build', '--locked', '-p', 'airwiki-mcp-bridge', '--features', 'e2e'];
if (process.platform === 'win32') bridgeArguments.push('--target', 'x86_64-pc-windows-msvc');
const bridge = spawnSync('cargo', bridgeArguments, { cwd: resolve(desktopRoot, '../..'), stdio: 'inherit' });
if (bridge.error) throw bridge.error;
if (bridge.status !== 0) process.exit(bridge.status ?? 1);

const arguments_ = [
  tauriCli,
  'build',
  '--ci',
  '--debug',
  '--no-bundle',
  '--features',
  'e2e',
  '--config',
  'tauri.e2e.conf.json'
];
if (process.platform === 'win32') {
  arguments_.push('--target', 'x86_64-pc-windows-msvc');
}
const result = spawnSync(process.execPath, arguments_, { cwd: desktopRoot, stdio: 'inherit' });

if (result.error) throw result.error;
if (result.status !== 0) process.exitCode = result.status ?? 1;
