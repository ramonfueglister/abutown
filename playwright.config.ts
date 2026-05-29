import { defineConfig, devices } from '@playwright/test';

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
      command: 'export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-backend/target}"; export CORS_ALLOWED_ORIGINS="http://127.0.0.1:5173"; scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server --bin e2e_server && exec "$CARGO_TARGET_DIR/debug/e2e_server"',
      url: 'http://127.0.0.1:8080/health',
      reuseExistingServer: false,
      timeout: 120_000,
    },
    {
      command: 'node node_modules/vite/bin/vite.js preview --host 127.0.0.1 --port 5173',
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
