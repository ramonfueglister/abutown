import { defineConfig, devices } from '@playwright/test';

const backendUrl = process.env.E2E_BACKEND_URL ?? 'http://127.0.0.1:18080';
const backendBindAddr = backendUrl.replace(/^https?:\/\//, '');

export default defineConfig({
  testDir: 'tests/e2e',
  timeout: 30_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL: 'http://127.0.0.1:5173',
    trace: 'retain-on-failure',
  },
  webServer: [
    {
      command: `ABUTOWN_SERVER_MODE=memory ABUTOWN_BIND_ADDR=${backendBindAddr} cargo run --manifest-path backend/Cargo.toml -p sim-server`,
      url: `${backendUrl}/health`,
      reuseExistingServer: false,
      timeout: 120_000,
    },
    {
      command: 'npm run preview -- --port 5173',
      url: 'http://127.0.0.1:5173',
      reuseExistingServer: false,
      timeout: 60_000,
    },
  ],
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
