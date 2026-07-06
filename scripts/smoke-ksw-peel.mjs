// KSW storey-peel smoke: real browser, real wheel events. Verifies the
// Phase A cutaway contract end-to-end (radius -> peel.p mapping):
//   1. boot with ?cam=er (radius 40 == kswPeel.endR, already over the main
//      building): peel exists, storeyCount >= 2, fully open (p === storeyCount)
//   2. wheel OUT to a mid-window radius (~75, between endR=40 and startR=110):
//      0 < p < storeyCount (partially peeled)
//   3. keep wheeling OUT past startR=110: p === 0 again (closed, reversible,
//      no stuck state)
//   4. zero page errors / console errors during the whole run
// Screenshots land in scratch/ (gitignored). Exits non-zero on any violation.
// Usage: node scripts/smoke-ksw-peel.mjs

import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';
import fs from 'node:fs';

const HOST = '127.0.0.1';
const PORT = 5189; // distinct from other worktrees' dev servers (5186-5188 seen in use)
const SCRATCH_DIR = new URL('../scratch/', import.meta.url).pathname;

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

const dev = spawn('npm', ['run', 'dev', '--', '--port', String(PORT), '--strictPort'], {
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

const failures = [];
function check(name, ok, detail) {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  (${detail})` : ''}`);
  if (!ok) failures.push(name);
}

// Wheel repeatedly and poll state() until radius stabilizes near targetRadius
// (zoomTarget lerps toward its target each frame, so a single wheel batch
// doesn't land instantly). Mirrors the template's own "wheel then settle"
// pattern but polls instead of a single fixed wait, since we need to land at
// specific intermediate radii, not just "in" or "out".
async function wheelToRadius(page, state, targetRadius, opts = {}) {
  const { maxSteps = 60, deltaPerStep = 120, settleTolerance = 1.5, pollMs = 100, maxPollMs = 6000 } = opts;
  await page.mouse.move(640, 400);
  let last = await state();
  for (let i = 0; i < maxSteps; i++) {
    const cur = await state();
    const diff = targetRadius - cur.radius;
    if (Math.abs(diff) <= settleTolerance) break;
    const dir = diff > 0 ? 1 : -1; // deltaY > 0 zooms out (radius grows)
    await page.mouse.wheel(0, dir * deltaPerStep);
    await page.waitForTimeout(60);
  }
  // let the eased zoom settle: poll until radius stops moving
  const settleStart = Date.now();
  let prev = (await state()).radius;
  while (Date.now() - settleStart < maxPollMs) {
    await page.waitForTimeout(pollMs);
    const cur = await state();
    if (Math.abs(cur.radius - prev) < 0.05) {
      last = cur;
      break;
    }
    prev = cur.radius;
    last = cur;
  }
  return last;
}

try {
  fs.mkdirSync(SCRATCH_DIR, { recursive: true });

  if (!(await waitForPort(30000))) {
    console.error(`SMOKE FAIL: dev server not up.\n${devOut}`);
    process.exit(1);
  }
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
  await page.goto(`http://${HOST}:${PORT}/ksw.html?at=2026-07-03T09:00:00Z&wx=clear&cam=er`, {
    waitUntil: 'load',
    timeout: 20000,
  });
  await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 30000 });
  await page.waitForTimeout(400);

  const state = () => page.evaluate(() => window.__KSW);
  const EPS = 1e-2;

  // ── 1. boot at ?cam=er (radius 40 == kswPeel.endR): fully open ──────────
  const s0 = await state();
  check('__KSW.peel exists at boot', !!s0.peel, JSON.stringify(s0.peel));
  const storeyCount = s0.peel?.storeyCount ?? 0;
  check('storeyCount >= 2 (real KSW eave has multiple storeys)', storeyCount >= 2, `storeyCount=${storeyCount}`);
  check(
    'boot (cam=er, radius=40) peel is fully open (p === storeyCount)',
    Math.abs(s0.peel.p - storeyCount) <= EPS,
    `p=${s0.peel.p.toFixed(3)} storeyCount=${storeyCount} radius=${s0.radius.toFixed(1)}`,
  );
  await page.screenshot({ path: `${SCRATCH_DIR}peel-open.png` });

  // ── 2. wheel OUT to a mid-window radius (~75, between endR=40, startR=110) ─
  const sMid = await wheelToRadius(page, state, 75);
  const midP = sMid.peel.p;
  check(
    'mid-window radius (~75) gives a partially open peel (0 < p < storeyCount)',
    midP > EPS && midP < storeyCount - EPS,
    `p=${midP.toFixed(3)} radius=${sMid.radius.toFixed(1)} storeyCount=${storeyCount}`,
  );
  await page.screenshot({ path: `${SCRATCH_DIR}peel-mid.png` });

  // ── 3. wheel OUT past startR=110: fully closed again, reversible ────────
  const sClosed = await wheelToRadius(page, state, 130);
  const closedP = sClosed.peel.p;
  check(
    'wheeling out past startR=110 closes the peel again (p === 0, reversible)',
    Math.abs(closedP - 0) <= EPS,
    `p=${closedP.toFixed(3)} radius=${sClosed.radius.toFixed(1)}`,
  );
  await page.screenshot({ path: `${SCRATCH_DIR}peel-closed.png` });

  if (errors.length) {
    console.error('--- page errors ---');
    for (const e of errors.slice(0, 12)) console.error(e);
    failures.push('page errors');
  }
  await browser.close();
} finally {
  cleanup();
}

if (failures.length) {
  console.error(`SMOKE FAIL: ${failures.join(', ')}`);
  process.exit(1);
}
console.log('SMOKE OK — storey peel verified in a real browser');
