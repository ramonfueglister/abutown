// scripts/smoke-cs-cars.mjs
//
// Task 6 browser smoke: PROVES the Cities-Skylines-style car layer renders in a
// real headless-chromium WebGPU frame of the Winterthur city view
// (ksw.html?traffic=1). Tasks 1-5 gave the layer 6 CS variants (body+glass
// InstancedMeshes per variant, one global wheel InstancedMesh with spinning +
// steering wheels). This is the frontend gate CLAUDE.md mandates for any
// src/ ↔ backend/ change.
//
// It launches the full stack (traffic binary + vite — see lib/traffic-stack),
// opens the city view, frames a busy corridor, and asserts:
//   (a) vehicles stream in         (window.__traffic.count() > 0)
//   (b) CS meshes draw             (≥1 variant has instances, wheels = 4×bodies)
//   (c) variant diversity          (a busy view shows ≥3 of the 6 silhouettes)
//   (d) WHEELS ACTUALLY TURN       (a sampled wheel matrix's rotation block
//                                   [elements 5,6,9,10] changes over 1 s while
//                                   traffic flows — sampled across ~10 slots so
//                                   a single red-light standstill can't fail it)
// then writes scratch/cs-cars-smoke.png AFTER the corridor framing so the
// picture actually shows cars, and prints the per-variant histogram.
//
// WebGPU headless flags + the window.__LOOK_READY gate follow smoke-traffic.mjs.

import { chromium } from 'playwright';
import { existsSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { startTrafficStack, HOST } from './lib/traffic-stack.mjs';

const REPO_ROOT = fileURLToPath(new URL('..', import.meta.url));
const TRAFFIC_PORT = Number(process.env.SMOKE_TRAFFIC_PORT ?? 8791);
const VITE_PORT = Number(process.env.SMOKE_VITE_PORT ?? 5188);
const SEED = 42;

/** Fresh-worktree preflight: startTrafficStack spawns the release binary
 * DIRECTLY (unlike startWorldStack, it never builds it), and the app import
 * of src/proto/*_pb.ts fails at vite dev-server request time, not at stack
 * boot — both failure modes surface only as an opaque __LOOK_READY timeout
 * tens of seconds later. Fail fast with the exact fix commands instead. */
function preflight() {
  const missing = [];
  const trafficBin = path.join(REPO_ROOT, 'backend/target/release/winterthur-traffic');
  if (!existsSync(trafficBin)) {
    missing.push(
      `traffic binary missing: ${trafficBin}\n` +
        `  fix: scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml --release -p winterthur-traffic`,
    );
  }
  const protoFiles = [
    'src/proto/world_pb.ts',
    'src/proto/live_pb.ts',
    'src/proto/traffic_pb.ts',
    'src/proto/abutown_pb.ts',
  ];
  const missingProto = protoFiles.filter((f) => !existsSync(path.join(REPO_ROOT, f)));
  if (missingProto.length) {
    missing.push(`generated proto files missing: ${missingProto.join(', ')}\n  fix: npm run generate:proto`);
  }
  if (missing.length) {
    console.error('[smoke] PREFLIGHT FAILED — this smoke would otherwise die with an opaque timeout:\n');
    for (const m of missing) console.error(`  - ${m}\n`);
    process.exit(1);
  }
}

// Morning rush, full YYYY-MM-DDTHH:MM form so day_kind is pinned to a workday
// regardless of the run day (2026-07-03 is a Friday) — same anchor
// smoke-traffic.mjs uses so we frame the same reproducible busy scene.
const AT_RUSH = '2026-07-03T07:30';

// Candidate aim points on the Gemeinde census net's busiest central corridors
// at workday 07:30 — identical list to smoke-traffic.mjs. We SCAN these and
// PARK on the busiest, because the AOI is a fixed 3×3 cell block around the
// camera target and instantaneous per-cell density wanders as platoons move.
const AIMS = [
  { x: -1100, z: 1550 },
  { x: -855, z: 1850 },
  { x: -1330, z: 1440 },
  { x: -485, z: 365 },
];
const CAM_RADIUS = 300;

const SCREENSHOT = path.join(REPO_ROOT, 'scratch/cs-cars-smoke.png');

async function main() {
  const failures = [];
  const check = (name, ok, detail) => {
    console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  (${detail})` : ''}`);
    if (!ok) failures.push(name);
  };

  preflight();

  console.log(
    `[smoke] launching stack (traffic :${TRAFFIC_PORT} seed=${SEED} at=${AT_RUSH}, vite :${VITE_PORT})…`,
  );
  const stack = await startTrafficStack({
    trafficPort: TRAFFIC_PORT,
    vitePort: VITE_PORT,
    seed: SEED,
    at: AT_RUSH,
  });

  const browser = await chromium.launch({
    headless: true,
    args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
  });

  const errors = [];
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
    page.on('console', (m) => {
      if (m.type() === 'error') errors.push(`console: ${m.text()}`);
    });

    const url =
      `http://${HOST}:${VITE_PORT}/ksw.html` +
      `?traffic=1&trafficWs=ws://${HOST}:${TRAFFIC_PORT}/traffic` +
      `&cam=bahnhof&at=2026-07-03T08:00:00Z&wx=clear`;
    console.log(`[smoke] opening ${url}`);
    await page.goto(url, { waitUntil: 'load', timeout: 60000 });

    const readyMs = Number(process.env.SMOKE_READY_TIMEOUT_MS ?? 180000);
    try {
      await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: readyMs });
    } catch (e) {
      console.error(`[smoke] __LOOK_READY not set after ${readyMs}ms; page errors so far:`);
      for (const line of errors) console.error(`  ${line}`);
      throw e;
    }
    await page.waitForFunction(() => typeof window.__traffic?.lookAt === 'function', {
      timeout: 20000,
    });

    // (a) vehicles stream in.
    await page.waitForFunction(() => window.__traffic && window.__traffic.count() > 0, null, {
      timeout: 60000,
    });
    const count0 = await page.evaluate(() => window.__traffic.count());
    check('(a) vehicles stream in', count0 > 0, `count()=${count0}`);

    // Scan the candidate corridors and PARK on the busiest one, so the
    // per-variant view + wheel sampling below happen on a dense scene.
    let best = { aim: AIMS[0], n: -1 };
    for (const aim of AIMS) {
      await page.evaluate(
        (a) => window.__traffic.lookAt(a.x, a.z, { radius: a.radius }),
        { ...aim, radius: CAM_RADIUS },
      );
      await page.waitForTimeout(4500);
      const n = await page.evaluate(() => window.__traffic.count());
      if (n > best.n) best = { aim, n };
    }
    console.log(`[smoke] parking on busiest aim [${best.aim.x}, ${best.aim.z}] (${best.n} veh)`);
    await page.evaluate(
      (a) => window.__traffic.lookAt(a.x, a.z, { radius: a.radius }),
      { ...best.aim, radius: CAM_RADIUS },
    );
    // Let the AOI re-populate and the car layer build its instances.
    await page.waitForTimeout(3000);

    // (b) CS meshes draw: ≥1 variant mesh has instances, wheels = 4 × bodies.
    const counts = await page.evaluate(() => window.__traffic.cars());
    const wheels = await page.evaluate(() => window.__traffic.wheels());
    const bodies = counts.reduce((a, b) => a + b, 0);
    console.log(`[smoke] per-variant body histogram: ${JSON.stringify(counts)}`);
    console.log(`[smoke] total bodies=${bodies}, wheels=${wheels}`);
    check('(b) CS car bodies drawn', bodies > 0, `histogram ${JSON.stringify(counts)}`);
    check('(b) wheels = 4 × bodies', wheels === 4 * bodies, `wheels=${wheels}, 4×bodies=${4 * bodies}`);

    // (c) variant diversity: a busy view shows ≥3 of the 6 silhouettes.
    const liveVariants = counts.filter((c) => c > 0).length;
    check('(c) ≥3 variants live', liveVariants >= 3, `${liveVariants} variants: ${JSON.stringify(counts)}`);

    // (d) WHEELS ACTUALLY TURN. Sample ~10 wheel slots (0,4,8,…,40 clamped to
    // wheels()-1), one matrix each, then again 1 s later, and require AT LEAST
    // ONE slot's rotation block (elements 5,6,9,10) to change by > 0.01. A full
    // simultaneous standstill of 10 sampled cars for 1 s does not happen at rush
    // demand on a through-corridor, so a single red-light park can't fail this.
    const slots = [];
    for (let s = 0; s <= 40; s += 4) slots.push(Math.min(s, Math.max(0, wheels - 1)));
    const uniqSlots = [...new Set(slots)];
    // Diversity guard: with few wheels, clamping collapses most/all of the 11
    // candidate slots to the same index — degrading "sample ~10 wheels" to
    // "check one wheel twice," which would pass vacuously even if only ONE
    // wheel in the whole scene happens to be turning. Require real spread.
    const minDiverse = Math.max(2, Math.min(5, Math.floor(wheels / 4)));
    check(
      '(d) wheel-sample diversity',
      uniqSlots.length >= minDiverse,
      `${uniqSlots.length} distinct slots of ${wheels} wheels (need ≥${minDiverse})`,
    );
    const sampleSlots = (ss) =>
      page.evaluate((arr) => arr.map((i) => window.__traffic.wheelMatrix(i)), ss);

    const before = await sampleSlots(uniqSlots);
    await page.waitForTimeout(1000);
    const after = await sampleSlots(uniqSlots);

    let maxDelta = 0;
    const perSlot = [];
    for (let k = 0; k < uniqSlots.length; k++) {
      const a = before[k];
      const b = after[k];
      if (!a || !b) {
        perSlot.push(`slot${uniqSlots[k]}:null`);
        continue;
      }
      const d = [5, 6, 9, 10].reduce((s, e) => s + Math.abs(a[e] - b[e]), 0);
      if (d > maxDelta) maxDelta = d;
      perSlot.push(`slot${uniqSlots[k]}:Δ${d.toFixed(3)}`);
    }
    console.log(`[smoke] wheel-rotation deltas: ${perSlot.join('  ')}`);
    check('(d) wheels rotate (≥1 slot rotation block changes)', maxDelta > 0.01, `max Δ=${maxDelta.toFixed(3)}`);

    // Screenshot for the visual CS-look gate. At the density radius (300 m) the
    // cars are sub-pixel; drop to a close ground-level radius + low pitch so the
    // silhouettes, glass and wheels are actually resolvable in the picture. We
    // sample the pose of a live vehicle and aim the tight frame at it so a car
    // is guaranteed in-shot (the AOI subscription is target-driven, so the aim
    // point stays inside the subscribed cells → the car keeps rendering).
    const aimPose = await page.evaluate((best) => {
      const s = window.__traffic.sample();
      if (!s.length) return null;
      // Prefer the sampled vehicle nearest best.aim — sample()[0] may be far
      // from the corridor that just passed the assertions, framing an empty
      // patch of road instead of the busy scene the checks above verified.
      let nearest = s[0];
      let bestD2 = Infinity;
      for (const v of s) {
        const d2 = (v.x - best.x) ** 2 + (v.z - best.z) ** 2;
        if (d2 < bestD2) {
          bestD2 = d2;
          nearest = v;
        }
      }
      return { x: nearest.x, z: nearest.z };
    }, best.aim);
    const shotAim = aimPose ?? best.aim;
    await page.evaluate(
      (a) => window.__traffic.lookAt(a.x, a.z, { radius: a.radius, pitch: a.pitch }),
      { ...shotAim, radius: 55, pitch: 0.22 },
    );
    // Let the AOI re-centre on the tight aim and cars settle in-frame.
    await page.waitForTimeout(2500);
    const shotBodies = (await page.evaluate(() => window.__traffic.cars())).reduce((a, b) => a + b, 0);
    console.log(`[smoke] close-up frame at [${shotAim.x.toFixed?.(0) ?? shotAim.x}, ${shotAim.z.toFixed?.(0) ?? shotAim.z}] r=55 — ${shotBodies} bodies in AOI`);
    mkdirSync(path.dirname(SCREENSHOT), { recursive: true });
    await page.screenshot({ path: SCREENSHOT });
    console.log(`[smoke] screenshot written: ${SCREENSHOT}`);

    if (errors.length) {
      console.error('--- page/console errors ---');
      for (const e of errors.slice(0, 12)) console.error(e);
      failures.push('page errors');
    }
  } finally {
    await browser.close().catch(() => {});
    stack.cleanup();
  }

  if (failures.length) {
    console.error(`\nSMOKE FAIL: ${failures.join(', ')}`);
    process.exit(1);
  }
  console.log(
    '\nSMOKE OK — CS cars draw (all-variant histogram printed above), wheels = 4×bodies, ≥3 silhouettes live, and wheels rotate while traffic flows (verified in a real WebGPU browser)',
  );
}

main().catch((err) => {
  console.error('SMOKE ERROR:', err);
  process.exit(1);
});
