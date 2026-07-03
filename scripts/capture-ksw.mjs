// KSW capture harness: spawn vite dev, open /ksw.html headless, wait for
// first rendered frame, screenshot to artifacts/ksw/<name>.png.
// Usage: node scripts/capture-ksw.mjs [name] [daystate] [cam] [extraQuery]
//   daystate: morning | dusk | night — mapped to a deterministic ?at=/?wx=
//             pair (real sun/weather sim, not the old preset system)
//   cam: overview | er | ops
//   extraQuery: extra URL params appended verbatim, e.g. "interior=1&shell=0"
//               (S3-interim, T17 — additive query passthrough for judging)

import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';
import { mkdirSync } from 'node:fs';

const HOST = '127.0.0.1';
const PORT = 5186;
const NAME = process.argv[2] ?? 'overview-morning';
const DAYSTATE = process.argv[3] ?? 'morning';
const CAM = process.argv[4] ?? 'overview';
const EXTRA = process.argv[5] ?? '';
const OUT = `artifacts/ksw/${NAME}.png`;

// Deterministic (at, wx) per named day-state — replaces the old ?preset=.
// These are UTC instants tuned for Winterthur's latitude (47.5°N) against
// the keyframe anchors used elsewhere (night < -6 deg, golden -6..+4 deg,
// day > 25 deg elevation). Measured via src/diorama/environment/solar.ts
// sunState().elevDeg on 2026-07-03:
//   morning 04:04Z -> elevDeg ~= +3.72 (golden, rising)
//   dusk    19:03Z -> elevDeg ~= +2.55 (golden, descending; matches the
//                     env capture matrix's dusk sample)
//   night   23:00Z -> elevDeg ~= -18.81 (night, unchanged)
const DAYSTATES = {
  morning: { at: '2026-07-03T04:04:00Z', wx: 'clear' },
  dusk: { at: '2026-07-03T19:03:00Z', wx: 'clear' },
  night: { at: '2026-07-03T23:00:00Z', wx: 'clear' },
};
const { at: AT, wx: WX } = DAYSTATES[DAYSTATE] ?? DAYSTATES.morning;

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
    const extraQuery = EXTRA ? `&${EXTRA}` : '';
    await page.goto(`http://${HOST}:${PORT}/ksw.html?at=${AT}&wx=${WX}&cam=${CAM}${extraQuery}`, { waitUntil: 'load', timeout: 20000 });
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
