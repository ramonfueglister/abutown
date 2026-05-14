import { defineConfig, devices } from '@playwright/test';
import { createHash } from 'node:crypto';

const e2ePort = Number(process.env.PLAYWRIGHT_PORT) || 17_000 + (
  createHash('sha1').update(process.cwd()).digest().readUInt32BE(0) % 1000
);

export default defineConfig({
  testDir: 'tests/e2e',
  timeout: 30_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL: `http://127.0.0.1:${e2ePort}`,
    trace: 'retain-on-failure',
  },
  webServer: {
    command: `npm run build && npm run preview -- --port ${e2ePort} --strictPort`,
    url: `http://127.0.0.1:${e2ePort}`,
    reuseExistingServer: false,
    timeout: 60_000,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
