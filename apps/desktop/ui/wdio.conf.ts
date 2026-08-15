import { rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const supportedBaselinePlatforms = new Set(['darwin', 'win32']);
if (!supportedBaselinePlatforms.has(process.platform)) {
  throw new Error(`visual baselines are unsupported on ${process.platform}`);
}

const uiRoot = dirname(fileURLToPath(import.meta.url));
const baselineFolder = join(uiRoot, 'e2e', 'baselines', process.platform);
if (process.env.UPDATE_VISUAL_BASELINES === '1') {
  rmSync(baselineFolder, { recursive: true, force: true });
}

export const config: WebdriverIO.Config = {
  runner: 'local',
  specs: ['./e2e/**/*.spec.ts'],
  maxInstances: 1,
  hostname: '127.0.0.1',
  port: 4445,
  path: '/',
  capabilities: [{ 'wdio:enforceWebDriverClassic': true }],
  framework: 'mocha',
  reporters: ['spec'],
  services: [[
    'visual',
    {
      baselineFolder,
      screenshotPath: join(uiRoot, '.artifacts', 'visual'),
      formatImageName: '{tag}-{width}x{height}',
      autoSaveBaseline: process.env.UPDATE_VISUAL_BASELINES === '1',
      alwaysSaveActualImage: false,
      clearRuntimeFolder: true,
      disableBlinkingCursor: true,
      disableCSSAnimation: true,
      enableLegacyScreenshotMethod: true,
      hideScrollBars: true,
      waitForFontsLoaded: true
    }
  ]],
  logLevel: 'warn',
  waitforTimeout: 10_000,
  connectionRetryTimeout: 30_000,
  connectionRetryCount: 1,
  mochaOpts: { ui: 'bdd', timeout: 300_000 }
};
