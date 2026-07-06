// Task 7 (M3) browser smoke: PROVES camera-driven tile streaming end-to-end
// in a real headless-chromium WebGPU render of the Winterthur city view
// (ksw.html, NO ?traffic — the streamer is independent of the traffic stack).
// This is the gate CLAUDE.md mandates for frontend-wiring changes: the M3
// streamer crosses camera ↔ tile ↔ chunk coordinate systems, exactly the
// class of bug unit tests missed in Phase 7a.
//
// PREFLIGHT (must already be done — do NOT run speculatively, it's slow):
//   data/winterthur/world/tiles/{L0,L1,L2}/*.pb must be baked (273 tiles)
//   and public/winterthur-world must symlink to ../data/winterthur/world:
//     npm run geo:fetch && npm run geo:bake-world
//     ln -s ../data/winterthur/world public/winterthur-world
//   Symptom of a missing/broken symlink: "can't skip wire type 4" (protobuf
//   parsing an HTML 404 page).
//
// Assertions (each PASS/FAIL logged, exit != 0 on any FAIL):
//   1. Boot: __LOOK_READY < 30 s (vite cold-transform excluded via a warmup
//      page load — the 30 s budget measures APP boot, not esbuild's first
//      compile of the module graph); 0 < __stream.live() < 80 (maxLive).
//   2. Fly-through center(0,0) → (4000,0) → (1500,3000) → back to (0,0),
//      each via __trees.lookAt(x, z, 600) (moves the rig → camera position →
//      the animate loop's ~2 Hz movement throttle re-runs streamer.update).
//      After each leg, poll up to 20 s until __stream.live()/disposed()
//      stabilize: the live-tile SET (__stream.liveKeys()) CHANGES on every
//      leg — the raw count can coincidentally stay equal across regions with
//      similar tile coverage while tiles churn underneath, so the assertion
//      is on the key set (the brief's "Live-Tile-Menge ÄNDERT sich") — and
//      after the return leg __stream.disposed() > 0 (tiles left behind on
//      the way were unloaded).
//   3. Zero pageerrors AND __stream.failed() === 0 at the end (no tile
//      fetch failed permanently, no unhandled exception anywhere).
//   4. FPS probe (rAF deltas over 5 s) parked at the city-edge leg
//      (4000, 0) >= 85. Before measuring, `pgrep -x Chromium | wc -l` is
//      logged (Lesson #136: orphaned Chromium processes steal GPU time and
//      poison FPS numbers — a warning is logged, the measurement still runs).
//   5. Screenshots via CDP Page.captureScreenshot (NOT page.screenshot(),
//      which hangs on the live WebGPU canvas per project lore):
//        scratch/streaming/edge.png         at (4000, 0),    r=600
//        scratch/streaming/forest.png       at (1500, 3000), r=600
//        scratch/streaming/forest-dense.png at (984, -3481), r=600
//        scratch/streaming/horizon.png      at (0, 0),       r=3000
//      The images are judged manually in the task report (building massing
//      in the midfield on horizon; terrain+buildings+trees on edge;
//      conifer/broadleaf mix on forest).
//
// Usage: node scripts/smoke-streaming.mjs
//   Env: SMOKE_VITE_PORT (default 5199), SMOKE_READY_TIMEOUT_MS (warmup only)

import { chromium } from 'playwright';
import { spawn, execSync } from 'node:child_process';
import net from 'node:net';
import { mkdirSync, writeFileSync } from 'node:fs';

const HOST = '127.0.0.1';
const PORT = Number(process.env.SMOKE_VITE_PORT ?? 5199);
const OUT_DIR = 'scratch/streaming';
const QUERY = 'at=2026-07-03T11:00:00Z&wx=clear&cam=overview';

// Fly-through legs (world coords, meters). Center is the manifest origin
// region; (4000, 0) is city edge (outside the boot near-ring). (1500, 3000)
// is the brief's mandated third stop — NOTE: in the bake this is farmland
// (its L2 tile 8_9 holds only 209 trees), so forest.png shows open terrain.
// forest-dense adds the densest mixed-forest L2 tile (8_4: 11'059 trees,
// 72% broadleaf / 28% conifer, decoded from the bake) to actually prove the
// Nadel/Laub mix renders through the streaming path.
const LEGS = [
  { name: 'edge', x: 4000, z: 0, r: 600 },
  { name: 'forest', x: 1500, z: 3000, r: 600 },
  { name: 'forest-dense', x: 984, z: -3481, r: 600 },
  { name: 'return', x: 0, z: 0, r: 600 },
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

async function openPage(browser) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(`console: ${m.text()}`);
  });
  const url = `http://${HOST}:${PORT}/ksw.html?${QUERY}`;
  console.log(`[smoke] opening ${url}`);
  const t0 = Date.now();
  await page.goto(url, { waitUntil: 'load', timeout: 30000 });
  return { page, errors, t0 };
}

/** Poll __stream.{live,disposed} at 500 ms until both are unchanged across 3
 * consecutive reads (streamer settled: queue drained, unloads applied) or
 * 20 s elapsed. Returns the last reading. */
async function settleStream(page) {
  let last = null;
  let stableReads = 0;
  const t0 = Date.now();
  while (Date.now() - t0 < 20000) {
    await page.waitForTimeout(500);
    const cur = await page.evaluate(() => ({
      live: window.__stream.live(),
      keys: window.__stream.liveKeys().sort(),
      disposed: window.__stream.disposed(),
      failed: window.__stream.failed(),
    }));
    if (last && cur.live === last.live && cur.disposed === last.disposed) {
      stableReads++;
      if (stableReads >= 2) return cur; // 3 identical consecutive reads
    } else {
      stableReads = 0;
    }
    last = cur;
  }
  console.log('[smoke] WARN: stream did not fully stabilize within 20 s, using last reading');
  return last;
}

async function main() {
  if (!(await waitForPort(30000))) {
    console.error(`SMOKE FAIL: dev server not up.\n${devOut}`);
    process.exit(1);
  }
  mkdirSync(OUT_DIR, { recursive: true });

  const browser = await chromium.launch({
    headless: true,
    // --disable-frame-rate-limit/--disable-gpu-vsync: without these, headless
    // chromium caps rAF at the 60 Hz vsync, so the A4 fps probe reads ~59.6
    // regardless of actual GPU headroom. The 85 fps gate measures real render
    // throughput, so the cap must come off.
    args: [
      '--enable-unsafe-webgpu',
      '--enable-gpu',
      '--use-angle=metal',
      '--disable-frame-rate-limit',
      '--disable-gpu-vsync',
    ],
  });

  try {
    // ── warmup load: pay vite's cold-transform of the module graph ONCE so
    // the measured boot below reflects app boot, not esbuild compile time.
    {
      const { page: warm } = await openPage(browser);
      const warmMs = Number(process.env.SMOKE_READY_TIMEOUT_MS ?? 180000);
      await warm.waitForFunction(() => window.__LOOK_READY === true, { timeout: warmMs });
      await warm.close();
      console.log('[smoke] warmup load done (vite transform cache primed)');
    }

    // ── assertion 1: boot < 30 s, 0 < live < 80 ─────────────────────────────
    const { page, errors, t0 } = await openPage(browser);
    let bootOk = true;
    try {
      await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 30000 });
    } catch {
      bootOk = false;
    }
    const bootMs = Date.now() - t0;
    check('A1: __LOOK_READY < 30 s', bootOk && bootMs < 30000, `bootMs=${bootMs}`);
    await page.waitForFunction(() => typeof window.__stream?.live === 'function', { timeout: 5000 });
    await page.waitForFunction(() => typeof window.__trees?.lookAt === 'function', { timeout: 5000 });
    const bootStream = await settleStream(page);
    check(
      'A1: 0 < __stream.live() < 80 after boot',
      bootStream.live > 0 && bootStream.live < 80,
      `live=${bootStream.live}`,
    );

    const cdp = await page.context().newCDPSession(page);
    const shoot = async (name) => {
      const shot = await cdp.send('Page.captureScreenshot', { format: 'png' });
      writeFileSync(`${OUT_DIR}/${name}.png`, Buffer.from(shot.data, 'base64'));
      console.log(`[smoke] wrote ${OUT_DIR}/${name}.png`);
    };

    // ── assertion 2: fly-through, the live tile SET changes per leg ─────────
    const setDiff = (a, b) => {
      const bs = new Set(b);
      const as = new Set(a);
      return a.filter((k) => !bs.has(k)).length + b.filter((k) => !as.has(k)).length;
    };
    let prev = bootStream;
    const perLeg = {};
    for (const leg of LEGS) {
      await page.evaluate((a) => window.__trees.lookAt(a.x, a.z, a.r), leg);
      const cur = await settleStream(page);
      perLeg[leg.name] = cur;
      const churn = setDiff(prev.keys, cur.keys);
      check(
        `A2: live tile set changes on leg "${leg.name}" (${leg.x},${leg.z})`,
        churn > 0,
        `live ${prev.live}→${cur.live}, set churn=${churn}, disposed=${cur.disposed}, keys=[${cur.keys.join(' ')}]`,
      );
      prev = cur;

      if (leg.name === 'edge') {
        // ── assertion 5 (edge shot) + assertion 4 (fps at the city edge) ────
        await shoot('edge');
        // Orphan check (Lesson #136): other Chromium processes on the box
        // steal GPU time and poison the FPS number. Warn, measure anyway.
        let chromiumCount = 0;
        try {
          chromiumCount = Number(
            execSync('pgrep -x Chromium | wc -l', { encoding: 'utf8' }).trim(),
          );
        } catch {}
        console.log(`[smoke] pgrep -x Chromium | wc -l → ${chromiumCount}`);
        if (chromiumCount > 1) {
          console.log(
            `[smoke] WARN: ${chromiumCount} Chromium processes running — possible orphans, FPS may read low`,
          );
        }
        const fps = await page.evaluate(
          () =>
            new Promise((resolve) => {
              const dts = [];
              let prevT = performance.now();
              const start = performance.now();
              const loop = () => {
                const now = performance.now();
                dts.push(now - prevT);
                prevT = now;
                if (now - start >= 5000) {
                  const total = dts.reduce((a, b) => a + b, 0);
                  resolve((dts.length / total) * 1000);
                } else {
                  requestAnimationFrame(loop);
                }
              };
              requestAnimationFrame(loop);
            }),
        );
        check('A4: FPS at city edge (4000,0) >= 85', fps >= 85, `fps=${fps.toFixed(1)}`);
      }
      if (leg.name === 'forest') await shoot('forest');
      if (leg.name === 'forest-dense') await shoot('forest-dense');
    }
    check(
      'A2: __stream.disposed() > 0 after return leg',
      perLeg.return.disposed > 0,
      `disposed=${perLeg.return.disposed}`,
    );

    // ── assertion 5 (horizon shot): wide framing over the center ────────────
    await page.evaluate(() => window.__trees.lookAt(0, 0, 3000));
    await settleStream(page);
    await shoot('horizon');

    // ── assertion 3: zero pageerrors, zero permanently failed tiles ─────────
    const failed = await page.evaluate(() => window.__stream.failed());
    check('A3: __stream.failed() === 0', failed === 0, `failed=${failed}`);
    check('A3: zero page errors', errors.length === 0, `count=${errors.length}`);
    if (errors.length) {
      console.error('--- page errors ---');
      for (const e of errors.slice(0, 12)) console.error(e);
    }

    await page.close();
    await browser.close();
  } finally {
    await browser.close().catch(() => {});
    cleanup();
  }

  if (failures.length) {
    console.error(`\nSMOKE FAIL: ${failures.join(', ')}`);
    process.exit(1);
  }
  console.log(
    '\nSMOKE OK — boot ring, camera-driven load/unload, dispose-on-leave, zero fetch failures, and edge-of-city FPS verified in a real WebGPU browser',
  );
}

main().catch((err) => {
  console.error('SMOKE ERROR:', err);
  cleanup();
  process.exit(1);
});
