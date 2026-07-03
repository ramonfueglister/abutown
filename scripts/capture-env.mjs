// Environment capture matrix: spawn vite dev, open the target page headless for
// each [name, query] pair, wait for __LOOK_READY + 1s settle, screenshot to
// <outdir>/<name>.png. Drives the real Winterthur sun/moon + weather via the
// ?at=/?wx= overrides. Replaces the retired preset-based capture-look.mjs.
//
// Usage:
//   node scripts/capture-env.mjs                     # look.html → artifacts/env
//   node scripts/capture-env.mjs --page=/ --out=env-city  # city → artifacts/env-city
//
// The city matrix reuses the 9 environment states and adds three cam=city
// establishing shots (dawn/noon/night) so the whole KSW↔Bahnhof↔ZAG span is
// reviewable at the city framing, not just the overview.

import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';
import { mkdirSync } from 'node:fs';

const HOST = '127.0.0.1';
const PORT = 5175;

const argv = process.argv.slice(2);
const getArg = (name, fallback) => {
  const hit = argv.find((a) => a.startsWith(`--${name}=`));
  return hit ? hit.slice(name.length + 3) : fallback;
};
// --page=/ selects the city (index.html); default is the room prototype.
const rawPage = getArg('page', 'look.html');
const PAGE = rawPage === '/' ? '' : rawPage.replace(/^\//, '');
const IS_CITY = PAGE === '';
const OUTDIR = `artifacts/${getArg('out', 'env')}`;

const BASE_MATRIX = [
  ['dawn', 'at=2026-07-03T04:10:00Z&wx=clear'],
  ['noon', 'at=2026-07-03T11:00:00Z&wx=clear'],
  // 19:03Z ≈ 21:03 local: sun at ~+2.6° descending, shortly before sunset
  // (~19:26Z). This is the real golden-evening moment — the DREDGE
  // "Amber unter Teal" — whereas the old 19:35Z was already past sunset
  // (elev −1.6°) and read as near-night black.
  ['dusk', 'at=2026-07-03T19:03:00Z&wx=clear'],
  // Moon-honest night: at the old 2026-07-03T23:30Z the real Winterthur moon
  // sat at only +16.3° near the horizon (illum 0.86) and drifted to the frame
  // edge — the sky read moonless. 2026-08-27T23:00Z is a real deep-night moment
  // with the moon high and full: measured moonElev +31.1°, illumination 1.00,
  // sunElev −31.9° (via src/diorama/environment/solar moonState/sunState). Same
  // honesty principle as the DAYSTATES comments in capture-ksw.mjs.
  ['night', 'at=2026-08-27T23:00:00Z&wx=clear'],
  ['overcast', 'at=2026-07-03T11:00:00Z&wx=overcast'],
  ['rain', 'at=2026-07-03T15:00:00Z&wx=rain'],
  ['snow', 'at=2026-01-15T11:00:00Z&wx=snow'],
  ['hochnebel', 'at=2026-10-20T09:00:00Z&wx=fog'],
  ['winter-night-1730', 'at=2026-01-15T16:35:00Z&wx=clear'],
];

// City-only: the same dawn/noon/night states re-shot from the high establishing
// framing (cam=city). Look.html has no camera presets, so these are city-only.
const CITY_CAM = [
  ['city-dawn', 'at=2026-07-03T04:10:00Z&wx=clear&cam=city'],
  ['city-noon', 'at=2026-07-03T11:00:00Z&wx=clear&cam=city'],
  // moon-honest (see night above): moonElev +31.1°, illum 1.00, sunElev −31.9°
  ['city-night', 'at=2026-08-27T23:00:00Z&wx=clear&cam=city'],
];

const MATRIX = IS_CITY ? [...BASE_MATRIX, ...CITY_CAM] : BASE_MATRIX;

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
        await page.goto(`http://${HOST}:${PORT}/${PAGE}?${query}`, { waitUntil: 'load', timeout: 20000 });
        await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 25000 });
        await page.waitForTimeout(1000);
        const backend = await page.evaluate(() => window.__LOOK_BACKEND);
        // page.screenshot composites via CDP captureScreenshot — proven on the
        // live WebGPU canvas here (look.html shipped this way). A hard cap keeps
        // a stuck compositor from stalling the whole matrix instead of hanging.
        await page.screenshot({ path: out, timeout: 20000 });
        console.log(`CAPTURE OK -> ${out} (backend: ${backend})`);
      } catch (e) {
        fail(`[${name}] scene never became ready: ${e}`);
        await page.screenshot({ path: `${OUTDIR}/${name}-broken.png`, timeout: 20000 }).catch(() => {});
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
