// Environment capture matrix: spawn vite dev, open /look.html headless for each
// [name, query] pair, wait for __LOOK_READY + 1s settle, screenshot to
// artifacts/env/<name>.png. Drives the real Winterthur sun/moon + weather via
// the ?at=/?wx= overrides. Replaces the retired preset-based capture-look.mjs.
// Usage: node scripts/capture-env.mjs

import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';
import { mkdirSync } from 'node:fs';

const HOST = '127.0.0.1';
const PORT = 5175;
const OUTDIR = 'artifacts/env';

const MATRIX = [
  ['dawn', 'at=2026-07-03T04:10:00Z&wx=clear'],
  ['noon', 'at=2026-07-03T11:00:00Z&wx=clear'],
  ['dusk', 'at=2026-07-03T19:35:00Z&wx=clear'],
  ['night', 'at=2026-07-03T23:30:00Z&wx=clear'],
  ['overcast', 'at=2026-07-03T11:00:00Z&wx=overcast'],
  ['rain', 'at=2026-07-03T15:00:00Z&wx=rain'],
  ['snow', 'at=2026-01-15T11:00:00Z&wx=snow'],
  ['hochnebel', 'at=2026-10-20T09:00:00Z&wx=fog'],
  ['winter-night-1730', 'at=2026-01-15T16:35:00Z&wx=clear'],
];

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

const dev = spawn('npm', ['run', 'dev'], {
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
    mkdirSync(OUTDIR, { recursive: true });
    const browser = await chromium.launch({
      headless: true,
      args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
    });
    for (const [name, query] of MATRIX) {
      const out = `${OUTDIR}/${name}.png`;
      const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
      const errors = [];
      page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
      page.on('console', (m) => {
        if (m.type() === 'error') errors.push(`console: ${m.text()}`);
      });
      try {
        await page.goto(`http://${HOST}:${PORT}/look.html?${query}`, { waitUntil: 'load', timeout: 20000 });
        await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 25000 });
        await page.waitForTimeout(1000);
        const backend = await page.evaluate(() => window.__LOOK_BACKEND);
        await page.screenshot({ path: out });
        console.log(`CAPTURE OK -> ${out} (backend: ${backend})`);
      } catch (e) {
        fail(`[${name}] scene never became ready: ${e}`);
        await page.screenshot({ path: `${OUTDIR}/${name}-broken.png` }).catch(() => {});
      }
      if (errors.length) {
        console.error(`--- page errors [${name}] ---`);
        for (const e of errors.slice(0, 12)) console.error(e);
        failed = true;
      }
      await page.close();
    }
    await browser.close();
  }
} finally {
  cleanup();
}
process.exit(failed ? 1 : 0);
