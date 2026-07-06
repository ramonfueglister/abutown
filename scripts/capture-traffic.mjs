// scripts/capture-traffic.mjs
//
// Task 10 visual-verification harness: CDP screenshots of the live traffic sim
// at three Winterthur locations, for human/agent inspection (cars on roads, on
// the right, queues at signals). Companion to scripts/smoke-traffic.mjs — the
// smoke proves the wire numerically; this produces the pixels.
//
// Known pitfall (project memory): page.screenshot() HANGS on a live WebGPU
// canvas. We therefore capture via CDP Page.captureScreenshot, which does not.
//
// Each shot aims the camera (and the AOI subscription, which follows rig.target)
// via the ?traffic debug hook window.__traffic.lookAt(x, z, radius) — deterministic
// framing with no synthetic mouse input. PNGs are written to scratch/ (gitignored).
//
// Locations (no roundabout exists in this bake — the net has 40 signals, 0
// roundabouts — so we substitute a second signal corridor):
//   1. bahnhof            — the station forecourt (meta.json landmark)
//   2. signal-corridor    — signal node 724 [8, 161] on the dense central spine
//   3. dense-corridor     — [8, 289], the busiest AOI cell (probe-density.mjs)
//
// Task 13 addition: a ZOOMED-OUT far-LOD shot (flow-a1-zoomout.png) framing a
// long stretch of the A1 mainline from high up, where per-vehicle CellFrames
// don't reach (the 3×3 AOI covers only 384 m around the camera target) and
// the impostor flow layer (Task 12) must carry the traffic impression.
// CAPTURE_FLOW_ONLY=1 skips the slow whole-net sweep + cluster shots and
// takes just this one.

import { chromium } from 'playwright';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { startTrafficStack, HOST } from './lib/traffic-stack.mjs';

const TRAFFIC_PORT = Number(process.env.SMOKE_TRAFFIC_PORT ?? 8790);
const VITE_PORT = 5188; // distinct from the smoke's 5187 so both can run
const SEED = 42;
const OUT_DIR = new URL('../scratch/traffic-captures/', import.meta.url).pathname;

// The traffic net's world extent, from the baked lanes — used to lay down a
// coarse grid of AOI aim points that sweeps the client's subscription across
// the whole plate so we can find the globally busiest corridors.
const NET = JSON.parse(readFileSync(new URL('../data/winterthur/trafficnet.json', import.meta.url), 'utf8'));
const NET_BOUNDS = (() => {
  let minX = Infinity, minZ = Infinity, maxX = -Infinity, maxZ = -Infinity;
  for (const l of NET.lanes)
    for (const [x, z] of l.pts) {
      if (x < minX) minX = x;
      if (z < minZ) minZ = z;
      if (x > maxX) maxX = x;
      if (z > maxZ) maxZ = z;
    }
  return { minX, minZ, maxX, maxZ };
})();

/** Sweep the camera (and thus the AOI subscription) across a coarse grid over
 * the whole net, reading the app's OWN dead-reckoned vehicle table at each stop
 * and accumulating unique vehicle world positions. This finds the globally
 * busiest corridors using the client's correct decode (units, ghost-heal) — the
 * app's live AOI only ever holds the cells around the camera, so one look isn't
 * enough. */
async function sampleWholeNet(page) {
  const { minX, minZ, maxX, maxZ } = NET_BOUNDS;
  const STEP = 300; // AOI sweep stride (m); radius 320 overlaps neighbours.
  const seen = new Map(); // id -> {x,z} (latest position wins)
  for (let z = minZ + 150; z <= maxZ; z += STEP) {
    for (let x = minX + 150; x <= maxX; x += STEP) {
      await page.evaluate((a) => window.__traffic.lookAt(a[0], a[1], { radius: 320 }), [x, z]);
      await page.waitForTimeout(700);
      const cars = await page.evaluate(() => window.__traffic.sample());
      for (const c of cars) seen.set(c.id, { x: c.x, z: c.z });
    }
  }
  return [...seen.values()];
}

// Post-#119 the traffic plate is draped on real DEM terrain and hardcoded AOI
// coords drift off the live fleet's densest corridors, so instead of fixed
// coordinates each shot is aimed at a DENSE CLUSTER discovered from the client's
// own vehicle table (window.__traffic.sample()) — the same idea the smoke uses
// to find a busy corridor. We bin every present car into 128 m cells, pick the
// three busiest well-separated cells, then tightly frame the cars in each so
// they read clearly rather than as specks in a wide overview.
//
// A ~1.0 rad pitch is oblique enough to keep the camera clear of building roofs
// while still reading which side of the road each car drives on.
const PITCH = 1.05;
const CELL = 128; // clustering bin size (m), matches the AOI cell grid.

/** Bin cars into CELL-sized cells, return the top-N densest cells (as centroids
 * with counts), greedily skipping cells within `minSep` of an already-picked
 * one so the three shots don't all land on the same junction. */
function densestClusters(cars, n, minSep) {
  const bins = new Map();
  for (const c of cars) {
    const key = `${Math.floor(c.x / CELL)},${Math.floor(c.z / CELL)}`;
    let b = bins.get(key);
    if (!b) bins.set(key, (b = { n: 0, sx: 0, sz: 0 }));
    b.n++;
    b.sx += c.x;
    b.sz += c.z;
  }
  const ranked = [...bins.values()]
    .map((b) => ({ n: b.n, x: b.sx / b.n, z: b.sz / b.n }))
    .sort((a, b) => b.n - a.n);
  const picked = [];
  for (const cell of ranked) {
    if (picked.length >= n) break;
    if (picked.some((p) => Math.hypot(p.x - cell.x, p.z - cell.z) < minSep)) continue;
    picked.push(cell);
  }
  return picked;
}

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  console.log(`[capture] launching stack (traffic :${TRAFFIC_PORT} seed=${SEED}, vite :${VITE_PORT})…`);
  // Pin the backend to the workday morning rush (same anchor as the smoke's
  // scenario 1) so the fleet — including the authored A1 through traffic the
  // Task 13 flow shot frames — is reproducibly dense regardless of the real
  // run time.
  const stack = await startTrafficStack({ trafficPort: TRAFFIC_PORT, vitePort: VITE_PORT, seed: SEED, at: '2026-07-03T07:30' });

  const browser = await chromium.launch({
    headless: true,
    args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
  });

  const written = [];
  try {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    page.on('pageerror', (e) => console.error('pageerror:', e.message));

    const url =
      `http://${HOST}:${VITE_PORT}/ksw.html` +
      `?traffic=1&trafficWs=ws://${HOST}:${TRAFFIC_PORT}/traffic` +
      `&cam=bahnhof&at=2026-07-03T08:00:00Z&wx=clear`;
    console.log(`[capture] opening ${url}`);
    await page.goto(url, { waitUntil: 'load', timeout: 60000 });
    // Post-#119 a cold dev load streams the 77 MB world pyramid; env-tunable.
    const readyMs = Number(process.env.SMOKE_READY_TIMEOUT_MS ?? 180000);
    await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: readyMs });
    await page.waitForFunction(() => typeof window.__traffic?.lookAt === 'function', { timeout: 20000 });

    // Let the morning-peak fleet build up before framing.
    await page.waitForTimeout(12000);

    const cdp = await page.context().newCDPSession(page);

    // ── Task 13: zoomed-out far-LOD flow shot over the A1 mainline ─────────
    {
      // The A1 mainline point closest to the city centre (~1.9 km out, from a
      // one-off motorway-lane scan: speedMs > 27) — inside the well-baked
      // world pyramid area (the far east A1 segment sits on blurry far-LOD
      // landcover with no road ribbons, unusable for a visual check).
      const A1 = { x: -897, z: -1677 };
      // r=650: far past the 3×3 AOI (384 m) so most of the framed motorway
      // renders from the flow layer, but close enough that a 4.5 m impostor
      // is still ~6 px and below the distance haze that washes out r>1500.
      // Near-top-down pitch reads the carriageway streams unambiguously.
      await page.evaluate(
        (a) => window.__traffic.lookAt(a.x, a.z, { radius: 650, yaw: -0.4, pitch: 1.4 }),
        A1,
      );
      // Give the warm-started A1 through traffic time to spread along the
      // mainline + the 2 s flow channel a few publish cycles.
      await page.waitForTimeout(25000);
      const flowCount = await page.evaluate(() => window.__traffic?.flowCount?.() ?? 0);
      const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
      const file = `${OUT_DIR}flow-a1-zoomout.png`;
      writeFileSync(file, Buffer.from(data, 'base64'));
      written.push({ file, count: flowCount, target: [A1.x, A1.z], radius: 650 });
      console.log(`[capture] flow-a1-zoomout: ${flowCount} impostors drawn -> ${file}`);
    }

    if (process.env.CAPTURE_FLOW_ONLY === '1') {
      await browser.close();
      console.log('\nCAPTURES:');
      for (const w of written) console.log(`  ${w.file}  (${w.count} impostors/veh, target [${w.target}])`);
      return;
    }

    // Sweep the whole net to find the globally busiest corridors to frame.
    // A modest separation (150 m) keeps the three shots distinct while still
    // letting all three land on genuinely busy stretches of the central spine —
    // forcing wider geographic spread drove shots into sparse village corridors
    // with a single car, which read worse than three views of live traffic.
    const allCars = await sampleWholeNet(page);
    const clusters = densestClusters(allCars, 3, 150);
    console.log(`[capture] ${allCars.length} cars sampled across the net; ${clusters.length} dense clusters:`, clusters.map((c) => `[${c.x.toFixed(0)},${c.z.toFixed(0)}]×${c.n}`).join(' '));

    // Fixed yaws so the three shots read from distinct angles. A moderate pitch
    // (PITCH) is oblique enough to read the scene without the disorienting
    // near-top-down tilt a steeper angle gives at a tight radius.
    const YAWS = [-0.5, 0.3, 1.1];
    const shots = clusters.map((c, i) => ({ name: `cluster-${i + 1}`, aoi: [c.x, c.z], yaw: YAWS[i % YAWS.length], pitch: PITCH }));

    for (const shot of shots) {
      // 1. Point the AOI subscription at the cluster and let it populate
      //    (keyframes replace cell membership within ≤5 s).
      await page.evaluate(
        (s) => window.__traffic.lookAt(s.aoi[0], s.aoi[1], { radius: 260, yaw: s.yaw, pitch: s.pitch }),
        shot,
      );
      await page.waitForTimeout(5000);

      // 2. Re-frame onto the DENSEST local knot of cars in this AOI so the frame
      //    centres on vehicles-on-road, not empty tarmac or a building roof. We
      //    aim at the car with the most neighbours within 60 m (a real on-road
      //    point) and size the radius to hold its ~8 nearest neighbours.
      let target = [...shot.aoi];
      let radius = 90;
      const s = await page.evaluate(() => window.__traffic.sample());
      const near = s.filter((v) => Math.hypot(v.x - shot.aoi[0], v.z - shot.aoi[1]) < 200);
      const pool = near.length >= 3 ? near : s;
      if (pool.length > 0) {
        let best = pool[0];
        let bestN = -1;
        for (const c of pool) {
          const n = pool.filter((o) => Math.hypot(o.x - c.x, o.z - c.z) < 60).length;
          if (n > bestN) {
            bestN = n;
            best = c;
          }
        }
        target = [best.x, best.z];
        // radius = distance to the 8th-nearest car (clamped), so a handful of
        // cars share the frame at a legible size.
        const dists = pool.map((v) => Math.hypot(v.x - best.x, v.z - best.z)).sort((a, b) => a - b);
        const k = Math.min(dists.length - 1, 7);
        radius = Math.max(60, Math.min(110, (dists[k] ?? 60) * 1.3 + 25));
      }

      // 3. Re-frame (keeps the AOI subscription on the same corridor) and let a
      //    few dead-reckoned frames settle.
      await page.evaluate(
        (a) => window.__traffic.lookAt(a.target[0], a.target[1], { radius: a.radius, yaw: a.yaw, pitch: a.pitch }),
        { target, radius, yaw: shot.yaw, pitch: shot.pitch },
      );
      await page.waitForTimeout(2500);

      const count = await page.evaluate(() => window.__traffic?.count() ?? 0);
      const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
      const file = `${OUT_DIR}${shot.name}.png`;
      writeFileSync(file, Buffer.from(data, 'base64'));
      written.push({ file, count, target: target.map((n) => Math.round(n)), radius: Math.round(radius) });
      console.log(`[capture] ${shot.name}: ${count} veh in AOI, framed [${target.map((n) => n.toFixed(0))}] r${radius.toFixed(0)} -> ${file}`);
    }

    await browser.close();
  } finally {
    await browser.close().catch(() => {});
    stack.cleanup();
  }

  console.log('\nCAPTURES:');
  for (const w of written) console.log(`  ${w.file}  (${w.count} veh, target [${w.target}])`);
}

main().catch((err) => {
  console.error('CAPTURE ERROR:', err);
  process.exit(1);
});
