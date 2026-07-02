// KSW capture harness: spawn vite dev, open /ksw.html headless, wait for
// first rendered frame, screenshot to artifacts/ksw/<name>.png.
// Usage: node scripts/capture-ksw.mjs [name] [preset] [cam]
//   preset: morning | dusk | night     cam: overview | er | ops

import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';
import { mkdirSync } from 'node:fs';

const HOST = '127.0.0.1';
const PORT = 5186;
const NAME = process.argv[2] ?? 'overview-morning';
const PRESET = process.argv[3] ?? 'morning';
const CAM = process.argv[4] ?? 'overview';
const OUT = `artifacts/ksw/${NAME}.png`;

function portOpen(host, port) {
  return new Promise((resolve) => {
    const s = net.createConnection({ host, port }, () => {
      s.end();
      resolve(true);
    });
    s.on('error', () => resolve(false));
    s.setTimeout(1000, () => {
      s.destroy();
      resolve(false);
    });
  });
}

async function waitForPort(timeoutMs) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await portOpen(HOST, PORT)) return true;
    await new Promise((r) => setTimeout(r, 200));
  }
  return false;
}

const dev = spawn('npm', ['run', 'dev', '--', '--port', '5186', '--strictPort'], {
  cwd: new URL('..', import.meta.url).pathname,
  env: { ...process.env },
  stdio: ['ignore', 'pipe', 'pipe'],
  detached: true,
});
let devOut = '';
dev.stdout.on('data', (d) => (devOut += d.toString()));
dev.stderr.on('data', (d) => (devOut += d.toString()));

let cleaned = false;
function cleanup() {
  if (cleaned) return;
  cleaned = true;
  if (dev.pid) {
    try {
      process.kill(-dev.pid, 'SIGKILL');
    } catch {}
  }
  try {
    dev.kill('SIGKILL');
  } catch {}
}
process.on('exit', cleanup);

let failed = false;
function fail(msg) {
  console.error(`CAPTURE FAIL: ${msg}`);
  failed = true;
}

try {
  if (!(await waitForPort(30000))) {
    fail(`dev server not up.\n${devOut}`);
  } else {
    mkdirSync('artifacts/ksw', { recursive: true });
    const browser = await chromium.launch({
      headless: true,
      args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
    });
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const errors = [];
    page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
    page.on('console', (m) => {
      if (m.type() === 'error') errors.push(`console: ${m.text()}`);
    });
    await page.goto(`http://${HOST}:${PORT}/ksw.html?preset=${PRESET}&cam=${CAM}`, { waitUntil: 'load', timeout: 20000 });
    try {
      await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 30000 });
      await page.waitForTimeout(900);
      const backend = await page.evaluate(() => window.__LOOK_BACKEND);
      await page.screenshot({ path: OUT });
      console.log(`CAPTURE OK -> ${OUT} (backend: ${backend})`);
    } catch (e) {
      fail(`scene never became ready: ${e}`);
      await page.screenshot({ path: `artifacts/ksw/${NAME}-broken.png` }).catch(() => {});
    }
    if (errors.length) {
      console.error('--- page errors ---');
      for (const e of errors.slice(0, 12)) console.error(e);
      failed = true;
    }
    await browser.close();
  }
} finally {
  cleanup();
}
process.exit(failed ? 1 : 0);
