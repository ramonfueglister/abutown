// Capture the four SOTA-trees polish scenes from ksw.html for the Slice-3
// screenshot loop (Task 6). Frames each spot via window.__traffic.lookAt after
// __LOOK_READY (same pattern as scratch/frame-spot.mjs / capture-traffic.mjs).
//
// Usage:
//   node scripts/capture-trees.mjs [sceneName ...]   (default: all scenes)
// Env overrides:
//   VITE_PORT (default 5188), BASE_URL (default http://127.0.0.1:<port>),
//   OUT_DIR (default scratch/tree-polish), ITER (suffix, default "0")
import { mkdirSync } from 'node:fs';
import { chromium } from 'playwright';

const PORT = process.env.VITE_PORT ?? '5188';
const BASE = process.env.BASE_URL ?? `http://127.0.0.1:${PORT}`;
const OUT_DIR = process.env.OUT_DIR ?? 'scratch/tree-polish';
const ITER = process.env.ITER ?? '0';

// Scenes (world coords, +x east, +z south). Radii per task-6 brief.
const SCENES = {
  // establishing wide shot over the park/quarter — trees + roads + buildings
  establishing: { x: -708, z: 700, r: 600 },
  // street avenue — silhouette variance, no trees on carriageway/in facades
  allee: { x: -651, z: 665, r: 80 },
  // forest edge — conifer/broadleaf mix per leaf_type, size spread
  // spot picked from data/winterthur/nature.json: densest mixed conic/slender +
  // broadleaf cells sit around x -1150..-1400, z 650..750
  waldrand: { x: -1275, z: 700, r: 110 },
  // handoff ring — 150 m near-LOD boundary crosses the frame, so full-geometry
  // and impostor trees are visible side by side. Framed at the forest belt:
  // the city spots have almost no trees beyond the 165 m full-detail band.
  handoff: { x: -1275, z: 700, r: 220 },
};

const wanted = process.argv.slice(2);
const names = wanted.length > 0 ? wanted : Object.keys(SCENES);
for (const n of names) {
  if (!SCENES[n]) {
    console.error(`unknown scene "${n}" — available: ${Object.keys(SCENES).join(', ')}`);
    process.exit(1);
  }
}

mkdirSync(OUT_DIR, { recursive: true });

const browser = await chromium.launch({
  headless: true,
  args: [
    '--headless=new',
    '--use-angle=metal',
    '--enable-features=Vulkan,WebGPU',
    '--enable-unsafe-webgpu',
  ],
});
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
page.on('pageerror', (e) => console.log('pageerror:', e.message));
await page.goto(
  `${BASE}/ksw.html?traffic=1&trafficWs=ws://127.0.0.1:9/traffic&at=2026-07-03T10:00:00Z&wx=clear`,
  { waitUntil: 'load', timeout: 30000 },
);
await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 180000 });
await page.waitForFunction(() => typeof window.__traffic?.lookAt === 'function', {
  timeout: 20000,
});

for (const name of names) {
  const s = SCENES[name];
  await page.evaluate((a) => window.__traffic.lookAt(a.x, a.z, { radius: a.r }), s);
  await page.waitForTimeout(3000); // settle: LOD/impostor swap + shadows + HMR dust
  const out = `${OUT_DIR}/${name}-${ITER}.png`;
  await page.screenshot({ path: out });
  console.log('OK ->', out);
}

await browser.close();
