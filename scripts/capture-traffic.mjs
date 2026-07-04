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

import { chromium } from 'playwright';
import { mkdirSync, writeFileSync } from 'node:fs';
import { startTrafficStack, HOST } from './lib/traffic-stack.mjs';

const TRAFFIC_PORT = 8790;
const VITE_PORT = 5188; // distinct from the smoke's 5187 so both can run
const SEED = 42;
const OUT_DIR = new URL('../scratch/traffic-captures/', import.meta.url).pathname;

// Each shot first aims the AOI subscription at a dense cell (found via
// probe-density.mjs), then RE-FRAMES onto the centroid of the actually-present
// cars with a radius that fits their spread — because vehicles scatter across
// the full 3×3 AOI (~384 m), a fixed tight framing usually misses them. A high
// aerial pitch reads which side of the road each car is on. Auto-framing makes
// the shot robust to the fleet's live distribution.
// A ~0.95 rad pitch is oblique enough to keep the camera clear of building
// roofs (a steeper near-top-down framing can clip inside a tall block when the
// car centroid lands next to one) while still reading which side of the road
// each car is on.
const PITCH = 0.95;
const SHOTS = [
  // 1. bahnhof — the required station landmark (meta.json). Traffic is genuinely
  //    light here (~2 veh in the AOI), so this frames the station in context
  //    rather than a busy road. Wide radius keeps the camera clear of the roofs.
  { name: 'bahnhof', aoi: [-338, 734], radius: 320, yaw: -0.6, pitch: 1.02, autoFrame: false },
  // 2. dense central spine (cell 81, ~17 veh).
  { name: 'central-spine', aoi: [26, 249], yaw: -0.2, pitch: PITCH, autoFrame: true },
  // 3. eastern corridor (cell 96, ~14 veh) — an open junction away from the
  //    tall central blocks.
  { name: 'east-corridor', aoi: [136, 417], yaw: 0.3, pitch: PITCH, autoFrame: true },
];

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  console.log(`[capture] launching stack (traffic :${TRAFFIC_PORT} seed=${SEED}, vite :${VITE_PORT})…`);
  const stack = await startTrafficStack({ trafficPort: TRAFFIC_PORT, vitePort: VITE_PORT, seed: SEED });

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
    await page.goto(url, { waitUntil: 'load', timeout: 30000 });
    await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 40000 });
    await page.waitForFunction(() => typeof window.__traffic?.lookAt === 'function', { timeout: 20000 });

    // Let the morning-peak fleet build up before framing.
    await page.waitForTimeout(12000);

    const cdp = await page.context().newCDPSession(page);

    for (const shot of SHOTS) {
      // 1. Point the AOI subscription at the dense cell and let it populate
      //    (keyframes replace cell membership within ≤5 s).
      await page.evaluate(
        (s) => window.__traffic.lookAt(s.aoi[0], s.aoi[1], { radius: 260, yaw: s.yaw, pitch: s.pitch }),
        shot,
      );
      await page.waitForTimeout(5000);

      // 2. Compute the framing. For autoFrame shots, aim at the centroid of the
      //    present cars and size the radius to their spread (with sane bounds).
      let target = [...shot.aoi];
      let radius = shot.radius ?? 120;
      if (shot.autoFrame) {
        const s = await page.evaluate(() => window.__traffic.sample());
        if (s.length > 0) {
          const cx = s.reduce((a, v) => a + v.x, 0) / s.length;
          const cz = s.reduce((a, v) => a + v.z, 0) / s.length;
          // spread = max distance of a car from the centroid (95th pct-ish via
          // sort to ignore a lone straggler blowing up the radius).
          const dists = s.map((v) => Math.hypot(v.x - cx, v.z - cz)).sort((a, b) => a - b);
          const spread = dists[Math.floor(dists.length * 0.85)] ?? 60;
          target = [cx, cz];
          radius = Math.max(70, Math.min(160, spread * 1.6));
        }
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
