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
  logLevel: 'warn',
  waitforTimeout: 10_000,
  connectionRetryTimeout: 30_000,
  connectionRetryCount: 1,
  mochaOpts: { ui: 'bdd', timeout: 60_000 }
};
