// KSW interaction smoke: real browser, real input events. Verifies the
// dynamic camera contract end-to-end:
//   1. wheel up   -> radius shrinks (zoom in) -> roofs fade out
//   2. wheel down -> radius grows (zoom out)  -> roofs fade back in
//   3. left-drag  -> yaw changes (orbit), radius unchanged
// Exits non-zero on any violation. Usage: node scripts/smoke-ksw.mjs

import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';

const HOST = '127.0.0.1';
const PORT = 5186;

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

const failures = [];
function check(name, ok, detail) {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  (${detail})` : ''}`);
  if (!ok) failures.push(name);
}

try {
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
  await page.goto(`http://${HOST}:${PORT}/ksw.html?preset=morning&cam=overview`, {
    waitUntil: 'load',
    timeout: 20000,
  });
  await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 30000 });
  await page.waitForTimeout(400);

  const state = () => page.evaluate(() => window.__KSW);

  const s0 = await state();
  check('overview starts with roofs fully on', s0.roofFade === 1, `fade=${s0.roofFade}`);

  // zoom in: wheel with negative deltaY over the canvas
  await page.mouse.move(640, 400);
  for (let i = 0; i < 22; i++) await page.mouse.wheel(0, -400);
  await page.waitForTimeout(900); // zoom eases toward its target — let it settle
  const s1 = await state();
  check('wheel up zooms in (radius shrinks)', s1.radius < s0.radius - 5, `${s0.radius.toFixed(1)} -> ${s1.radius.toFixed(1)}`);
  check('zooming in fades the roofs out', s1.roofFade < 0.05, `fade=${s1.roofFade.toFixed(3)}`);

  // zoom out again
  for (let i = 0; i < 22; i++) await page.mouse.wheel(0, 400);
  await page.waitForTimeout(900);
  const s2 = await state();
  check('wheel down zooms back out', s2.radius > s1.radius + 5, `${s1.radius.toFixed(1)} -> ${s2.radius.toFixed(1)}`);
  check('zooming out brings the roofs back', s2.roofFade > 0.95, `fade=${s2.roofFade.toFixed(3)}`);

  // left-drag: orbit
  await page.mouse.move(640, 400);
  await page.mouse.down({ button: 'left' });
  await page.mouse.move(880, 430, { steps: 12 });
  await page.mouse.up({ button: 'left' });
  await page.waitForTimeout(250);
  const s3 = await state();
  check('left-drag rotates the camera (yaw changes)', Math.abs(s3.yaw - s2.yaw) > 0.2, `${s2.yaw.toFixed(2)} -> ${s3.yaw.toFixed(2)}`);
  // eased zoom may still be settling by a hair — drag itself must not dolly
  check('drag does not change the zoom radius', Math.abs(s3.radius - s2.radius) < 0.5, `${s2.radius.toFixed(2)} vs ${s3.radius.toFixed(2)}`);

  // moving without the button held must not rotate
  await page.mouse.move(400, 300, { steps: 6 });
  await page.waitForTimeout(150);
  const s4 = await state();
  check('hover without button held does not rotate', Math.abs(s4.yaw - s3.yaw) < 1e-6, `${s3.yaw.toFixed(3)} vs ${s4.yaw.toFixed(3)}`);

  // AoE2 edge scrolling: cursor parked at the right edge pans the target
  const before = await state();
  await page.mouse.move(1278, 400);
  await page.waitForTimeout(1200);
  const after = await state();
  const panned = Math.hypot(after.target[0] - before.target[0], after.target[2] - before.target[2]);
  check('cursor at screen edge pans the camera (AoE2 style)', panned > 3, `moved ${panned.toFixed(1)} units`);
  await page.mouse.move(640, 400);
  await page.waitForTimeout(300);
  const settled = await state();
  await page.waitForTimeout(500);
  const settled2 = await state();
  const drift = Math.hypot(settled2.target[0] - settled.target[0], settled2.target[2] - settled.target[2]);
  check('pan stops when the cursor leaves the edge', drift < 0.05, `drift ${drift.toFixed(3)}`);

  // People actually move through the REAL KSW interior (T19). The crowd total
  // is the generated plan's people count (dynamic — no authored 72 anymore),
  // so assert a healthy floor instead of an exact number. Agent state is
  // CPU-driven: __KSW.agents updates even while the interior meshes are
  // hidden behind the closed dollhouse at the overview framing.
  const a0 = (await state()).agents;
  check('a full crowd is spawned from the generated plan', a0.total >= 60, `total=${a0.total}`);
  await page.waitForTimeout(6000);
  const a1 = (await state()).agents;
  const moved = a0.samples.filter((p, i) => Math.hypot(p[0] - a1.samples[i][0], p[1] - a1.samples[i][1]) > 0.3).length;
  check('people walk around (sampled positions change)', moved >= 2, `${moved}/12 samples moved`);
  check('someone is walking at any given moment', a0.walking + a1.walking > 0, `walking=${a0.walking}->${a1.walking}`);

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
console.log('SMOKE OK — dynamic camera verified in a real browser');
