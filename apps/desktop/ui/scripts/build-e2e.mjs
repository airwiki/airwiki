import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const desktopRoot = resolve(uiRoot, '..');
const tauriCli = fileURLToPath(import.meta.resolve('@tauri-apps/cli'));
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
