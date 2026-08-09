import { join } from 'node:path';

export const config: WebdriverIO.Config = {
  runner: 'local',
  specs: ['./e2e/**/*.spec.ts'],
  maxInstances: 1,
  hostname: '127.0.0.1',
  port: 4445,
  path: '/',
  capabilities: [{}],
  framework: 'mocha',
  reporters: ['spec'],
  services: [[
    'visual',
    {
      baselineFolder: join(process.cwd(), 'e2e', 'baselines', process.platform),
      screenshotPath: join(process.cwd(), '.artifacts', 'visual'),
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
