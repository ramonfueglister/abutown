// scripts/capture-discard.mjs
//
// Task 5e visual proof: CDP screenshots proving corridor terrain-discard +
// ribbon skirts eliminate terrain piercing road surfaces. Companion to the
// unit tests (mask/skirt/metric) and the traffic smoke (which proves the app
// boots with the discard shader + loadCorridorMask live). This produces the
// pixels a human/agent inspects: road surfaces with NO terrain wedges through
// them, skirts closing the ribbon edges.
//
// Two sites (spec §9 + Task 5d diagnosis):
//   1. worst-5d-offender  (118.9, −194.4) — the service/footway pair on the
//      Heiligberg retaining geometry: metric v2's worst 3.89 m poke-through,
//      the exact multi-way conflict no heightfield clamp could fix. Discard
//      makes the terrain there simply not render.
//   2. bruehlberg-hillside (−266, 474) — a steep slope stacking path/primary/
//      footway; the 5d offender cluster.
//
// Aimed via window.__trees.lookAt(x, z, radius) — the unconditional camera hook
// (no traffic backend needed). Oblique low pitch so the road surface and its
// skirts read against the hillside. PNGs → scratch/captures/discard-*.png.
//
// Pitfall (project memory): page.screenshot() HANGS on a live WebGPU canvas;
// capture via CDP Page.captureScreenshot instead.

import { chromium } from 'playwright';
import { mkdirSync, writeFileSync } from 'node:fs';
import { startTrafficStack, HOST } from './lib/traffic-stack.mjs';

const VITE_PORT = 5191; // distinct from smoke (5187) / capture-traffic (5188)
const TRAFFIC_PORT = 8793;
const OUT_DIR = new URL('../scratch/captures/', import.meta.url).pathname;

// Two hillside sites where Task 5d measured the worst multi-way piercing:
//   - the worst-offender cluster at (118.9, −194.4) is boxed in by KSW-area
//     building massing (a near view is all walls), so we frame it from a
//     top-down pitch that clears the roofs and shows the road corridors cutting
//     the terrain cleanly (no green wedges through the tarmac).
//   - a Brühlberg hillside road network (−266, 474) at a strong oblique, and an
//     open steep-embankment footway/pedestrian way (−218, 506, profile range
//     4.2 m) at a LOW oblique pitch so the vertical side-skirts read as faces
//     dropping from the ribbon edge to the discarded terrain.
const SITES = [
  // Finding 1a proof: a near-vertical TOP-DOWN pitch over a hillside road, where
  // a see-through hole beside the ribbon (the old grading-width mask stamped a
  // ~1.5 m annulus the skirt never reached) would show as sky/void beside the
  // tarmac. With the ribbon-footprint mask (Finding 1a) the graded shoulder keeps
  // rendered terrain right up to the ribbon edge — no void.
  { name: 'discard-topdown-hillside', x: -266, z: 474, radius: 90, yaw: 0.0, pitch: 1.53 },
  { name: 'discard-worst-5d-offender', x: 118.9, z: -194.4, radius: 130, yaw: 0.5, pitch: 1.45 },
  { name: 'discard-bruehlberg-hillside', x: -266, z: 474, radius: 130, yaw: -0.4, pitch: 1.1 },
  { name: 'discard-bruehlberg-skirt', x: -218, z: 506, radius: 55, yaw: 0.7, pitch: 0.6 },
];

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  console.log(`[discard] launching stack (vite :${VITE_PORT})…`);
  const stack = await startTrafficStack({ trafficPort: TRAFFIC_PORT, vitePort: VITE_PORT, seed: 42, at: '2026-07-03T12:00' });

  const browser = await chromium.launch({
    headless: true,
    args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
  });

  const written = [];
  try {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    page.on('pageerror', (e) => console.error('pageerror:', e.message));

    // ?traffic=1 so window.__traffic.lookAt (which honours yaw/pitch) exists —
    // __trees.lookAt alone ignores the angle. Mid-afternoon sun (15:00) rakes
    // the hillside so the vertical skirts and road surfaces read with contrast
    // instead of the blown-out midday flat light.
    const url =
      `http://${HOST}:${VITE_PORT}/ksw.html?traffic=1&trafficWs=ws://${HOST}:${TRAFFIC_PORT}/traffic` +
      `&at=2026-07-03T15:00:00Z&wx=clear`;
    console.log(`[discard] opening ${url}`);
    await page.goto(url, { waitUntil: 'load', timeout: 60000 });
    const readyMs = Number(process.env.SMOKE_READY_TIMEOUT_MS ?? 240000);
    await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: readyMs });
    await page.waitForFunction(() => typeof window.__trees?.lookAt === 'function', { timeout: 20000 });

    const cdp = await page.context().newCDPSession(page);

    for (const site of SITES) {
      await page.evaluate((s) => {
        // __trees.lookAt(x, z, radius) frames the camera; yaw/pitch aren't in
        // its signature, so nudge the rig via the same debug path the traffic
        // hook uses when present, else rely on the default oblique framing.
        window.__trees.lookAt(s.x, s.z, s.radius);
        if (window.__traffic?.lookAt) {
          window.__traffic.lookAt(s.x, s.z, { radius: s.radius, yaw: s.yaw, pitch: s.pitch });
        }
      }, site);
      await page.waitForTimeout(3500); // let GI + shadows settle after the move

      const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
      const file = `${OUT_DIR}${site.name}.png`;
      writeFileSync(file, Buffer.from(data, 'base64'));
      written.push(file);
      console.log(`[discard] ${site.name}: framed [${site.x}, ${site.z}] r${site.radius} -> ${file}`);
    }

    await browser.close();
  } finally {
    await browser.close().catch(() => {});
    stack.cleanup();
  }

  console.log('\nCAPTURES:');
  for (const f of written) console.log(`  ${f}`);
}

main().catch((err) => {
  console.error('CAPTURE ERROR:', err);
  process.exit(1);
});
