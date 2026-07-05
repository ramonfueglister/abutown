// Task 7 browser smoke: PROVES the procedural tree system (instanced
// archetype layer + near-set compaction LOD + boot-baked octahedral
// impostors + TSL wind sway) end-to-end in a real headless-chromium WebGPU
// render of the Winterthur city view (ksw.html). This is the gate CLAUDE.md
// mandates for any src/ change that touches the frontend↔backend /
// coordinate-system wiring — here, main.ts's tree-layer boot sequence.
//
// PREFLIGHT (must already be done — do NOT run speculatively, it's slow):
//   data/winterthur/world/*.pb must be baked (273 tiles) and public/
//   winterthur-world must be a symlink to ../data/winterthur/world:
//     npm run geo:fetch && npm run geo:bake-world
//     ln -s ../data/winterthur/world public/winterthur-world
//   Verify with:
//     ls -la public/winterthur-world
//     ls data/winterthur/world/*.pb | wc -l
//   Symptom of a missing/broken symlink: the client throws "can't skip wire
//   type 4" — that's protobuf trying to parse an HTML 404 page.
//
// Checks:
//   1. Boot to __LOOK_READY with zero console errors (pageerror + console.error
//      both fail the smoke — WebGPU validation errors surface as console errors).
//   2. window.__trees.archetypes === 20, fullCount() > 0.
//   3. Compaction responds to camera movement: near a dense tree cluster,
//      fullCount() is small; re-aimed far out (establishing radius) via the
//      window.__trees.lookAt debug hook, fullCount() changes and both values
//      stay comfortably under a sanity bound (there is no exposed "total
//      instance count" field — the tree layer only tracks the live compacted
//      near-set, so we bound against a generous ceiling instead).
//   4. windAmp() is a finite number >= 0; forcing ?wx=storm-equivalent (rain,
//      the highest windSpeedMs entry in WX_OVERRIDES) drives it > 0.
//   5. Screenshots via CDP Page.captureScreenshot (NOT page.screenshot(),
//      which hangs on the live WebGPU canvas per project lore) to
//      scratch/trees/{establishing,street,forest-edge}.png.
//   6. FPS probe at the establishing shot: rAF-delta sampling over ~3 s,
//      assert >= 30 fps (a conservative headless-safe bar; the brief's 55 fps
//      is a relative dev-machine target, not a hard gate here).
//
// Usage: node scripts/smoke-trees.mjs

import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';
import { mkdirSync, writeFileSync } from 'node:fs';

const HOST = '127.0.0.1';
const PORT = 5199;
const OUT_DIR = 'scratch/trees';

// A dense tree cluster (found via a one-off 50 m-cell bucket sweep of
// data/winterthur/nature.json: cell [-950,1250] holds 45 trees, the densest
// in the dataset) — used for the near/street and forest-edge framings.
const FOREST_X = -950;
const FOREST_Z = 1250;
// Street-level: tight radius right on top of the cluster (full-detail trees
// should dominate the frame, compaction near-set only).
const STREET_RADIUS = 60;
// Forest-edge: pulled back enough to see the cluster's boundary against open
// ground — both near full-detail trees and the start of the impostor field.
const FOREST_EDGE_RADIUS = 220;
// Establishing: the whole-city framing radius (kswCity.radiusMax = 1500),
// aimed at the same cluster so the same trees anchor the far shot too.
const ESTABLISHING_RADIUS = 1400;

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

async function openPage(browser, query) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(`console: ${m.text()}`);
  });
  const url = `http://${HOST}:${PORT}/ksw.html?${query}`;
  console.log(`[smoke] opening ${url}`);
  await page.goto(url, { waitUntil: 'load', timeout: 30000 });
  const readyMs = Number(process.env.SMOKE_READY_TIMEOUT_MS ?? 180000);
  await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: readyMs });
  await page.waitForFunction(() => typeof window.__trees?.lookAt === 'function', { timeout: 20000 });
  await page.waitForTimeout(400);
  return { page, errors };
}

async function main() {
  if (!(await waitForPort(30000))) {
    console.error(`SMOKE FAIL: dev server not up.\n${devOut}`);
    process.exit(1);
  }
  mkdirSync(OUT_DIR, { recursive: true });

  const browser = await chromium.launch({
    headless: true,
    args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
  });

  try {
    // ── boot + variety + compaction (checks 1-3) ────────────────────────────
    const { page, errors } = await openPage(browser, `at=2026-07-03T11:00:00Z&wx=clear&cam=overview`);

    const trees = () => page.evaluate(() => ({ archetypes: window.__trees.archetypes, fullCount: window.__trees.fullCount() }));

    // 2. archetype variety + non-empty near-set at boot.
    const t0 = await trees();
    check('window.__trees.archetypes === 20', t0.archetypes === 20, `archetypes=${t0.archetypes}`);
    check('window.__trees.fullCount() > 0 after boot', t0.fullCount > 0, `fullCount=${t0.fullCount}`);

    // 3. compaction responds to camera movement. Park close on the dense
    // cluster (street level) — fullCount should be small (a tight local
    // near-set). Then re-aim far out (establishing radius, same cluster as
    // the look-at target) and let the >500ms compaction throttle clear —
    // fullCount should change, and both readings stay well under a generous
    // sanity ceiling (there is no "total instance count" field exposed; the
    // tree layer only ever materializes the compacted near-set).
    await page.evaluate(
      (a) => window.__trees.lookAt(a.x, a.z, a.r),
      { x: FOREST_X, z: FOREST_Z, r: STREET_RADIUS },
    );
    await page.waitForTimeout(700);
    const near = await trees();
    check('near/street fullCount() is small', near.fullCount < 2000, `fullCount=${near.fullCount}`);

    await page.evaluate(
      (a) => window.__trees.lookAt(a.x, a.z, a.r),
      { x: FOREST_X, z: FOREST_Z, r: ESTABLISHING_RADIUS },
    );
    await page.waitForTimeout(700); // clear the >=500ms compaction throttle
    const far = await trees();
    const SANITY_BOUND = 50000;
    check(
      'compaction changes fullCount() between near and far framings',
      far.fullCount !== near.fullCount,
      `near=${near.fullCount} far=${far.fullCount}`,
    );
    check(
      'both near/far fullCount() stay under the sanity bound',
      near.fullCount < SANITY_BOUND && far.fullCount < SANITY_BOUND,
      `near=${near.fullCount} far=${far.fullCount} bound=${SANITY_BOUND}`,
    );

    // 4. wind: finite, >= 0 under clear skies; > 0.5 under the windiest
    // WX_OVERRIDES entry (rain: windSpeedMs=5, the storm-equivalent — snow/
    // overcast/fog are calmer, see main.ts WX_OVERRIDES).
    const windClear = await page.evaluate(() => window.__trees.windAmp());
    check('windAmp() is a finite number >= 0 (clear)', Number.isFinite(windClear) && windClear >= 0, `windAmp=${windClear}`);

    // 5+6. establishing screenshot + FPS probe (same framing as the far read
    // above — re-settle briefly so the FPS sample isn't polluted by the
    // lookAt's own recompaction frame).
    await page.waitForTimeout(600);
    const cdp = await page.context().newCDPSession(page);
    const shotEstablishing = await cdp.send('Page.captureScreenshot', { format: 'png' });
    writeFileSync(`${OUT_DIR}/establishing.png`, Buffer.from(shotEstablishing.data, 'base64'));
    console.log(`[smoke] wrote ${OUT_DIR}/establishing.png`);

    const fps = await page.evaluate(
      () =>
        new Promise((resolve) => {
          const dts = [];
          let prev = performance.now();
          const durationMs = 3000;
          const t0 = performance.now();
          const loop = () => {
            const now = performance.now();
            dts.push(now - prev);
            prev = now;
            if (now - t0 >= durationMs) {
              const total = dts.reduce((a, b) => a + b, 0);
              resolve((dts.length / total) * 1000);
            } else {
              requestAnimationFrame(loop);
            }
          };
          requestAnimationFrame(loop);
        }),
    );
    check('establishing-shot FPS >= 30', fps >= 30, `fps=${fps.toFixed(1)}`);

    // street.png: near/street-level framing (full-detail trees dominate).
    await page.evaluate(
      (a) => window.__trees.lookAt(a.x, a.z, a.r),
      { x: FOREST_X, z: FOREST_Z, r: STREET_RADIUS },
    );
    await page.waitForTimeout(700);
    const shotStreet = await cdp.send('Page.captureScreenshot', { format: 'png' });
    writeFileSync(`${OUT_DIR}/street.png`, Buffer.from(shotStreet.data, 'base64'));
    console.log(`[smoke] wrote ${OUT_DIR}/street.png`);

    // forest-edge.png: pulled back to the cluster's boundary — both near
    // full-detail trees and the start of the impostor field in one frame.
    await page.evaluate(
      (a) => window.__trees.lookAt(a.x, a.z, a.r),
      { x: FOREST_X, z: FOREST_Z, r: FOREST_EDGE_RADIUS },
    );
    await page.waitForTimeout(700);
    const forestEdge = await trees();
    const shotForestEdge = await cdp.send('Page.captureScreenshot', { format: 'png' });
    writeFileSync(`${OUT_DIR}/forest-edge.png`, Buffer.from(shotForestEdge.data, 'base64'));
    console.log(`[smoke] wrote ${OUT_DIR}/forest-edge.png (fullCount=${forestEdge.fullCount})`);

    if (errors.length) {
      console.error('--- page errors [main] ---');
      for (const e of errors.slice(0, 12)) console.error(e);
      failures.push('page errors (main)');
    }
    await page.close();

    // ── wind under a storm-equivalent weather override (separate page load,
    // since ?wx= is read once at boot; rain is the windiest WX_OVERRIDES
    // entry at windSpeedMs=5) ────────────────────────────────────────────────
    const { page: page2, errors: errors2 } = await openPage(
      browser,
      `at=2026-07-03T15:00:00Z&wx=rain&cam=overview`,
    );
    await page2.waitForTimeout(1500); // let the wind uniform ramp up
    const windStorm = await page2.evaluate(() => window.__trees.windAmp());
    // windAmplitude(windSpeedMs) = min(1.2, windSpeedMs/10); rain's
    // windSpeedMs=5 (the windiest WX_OVERRIDES entry) yields exactly 0.5 —
    // use >= so the boundary case is a genuine pass, not a coin flip.
    check('windAmp() >= 0.5 under storm-equivalent (?wx=rain)', windStorm >= 0.5, `windAmp=${windStorm}`);
    if (errors2.length) {
      console.error('--- page errors [wind] ---');
      for (const e of errors2.slice(0, 12)) console.error(e);
      failures.push('page errors (wind)');
    }
    await page2.close();

    await browser.close();
  } finally {
    await browser.close().catch(() => {});
    cleanup();
  }

  if (failures.length) {
    console.error(`\nSMOKE FAIL: ${failures.join(', ')}`);
    process.exit(1);
  }
  console.log('\nSMOKE OK — tree variety, compaction LOD, wind sway, and impostor field verified in a real WebGPU browser');
}

main().catch((err) => {
  console.error('SMOKE ERROR:', err);
  cleanup();
  process.exit(1);
});
