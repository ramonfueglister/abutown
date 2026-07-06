// scripts/capture-pr1.mjs
//
// PR 1 visual proof (Task 6 ship): four CDP screenshots demonstrating the
// swiss-roads arc — terrain-graded corridors, road-owned profiles, corridor
// discard + platform (apron/Bankett + terrain-grounded skirts):
//   a. worst pre-arc site — hillside road at (−347, 1073), the classic
//      "versinkende Strasse" before grading; now a graded bench.
//   b. Brühlberg oblique — skirts dropping to terrain (fill slopes/Dämme) and
//      cut banks (Einschnitte) against the steep slope.
//   c. rail corridor embankment — the main-line rail bed south-west of the
//      Bahnhof, 200 m smoothing window carving the classic embankment.
//   d. top-down no-holes — the former see-through-void site; the platform
//      apron fills to the mask edge, no white voids beside narrow ways.
//
// Mirrors scripts/capture-discard.mjs (CDP Page.captureScreenshot — plain
// page.screenshot() HANGS on a live WebGPU canvas). PNGs →
// scratch/captures/pr1-*.png.

import { chromium } from 'playwright';
import { mkdirSync, writeFileSync } from 'node:fs';
import { startTrafficStack, HOST } from './lib/traffic-stack.mjs';

const VITE_PORT = 5192; // distinct from smoke (5187) / discard captures (5191)
const TRAFFIC_PORT = 8794;
const OUT_DIR = new URL('../scratch/captures/', import.meta.url).pathname;

const SITES = [
  { name: 'pr1-worst-prearc-hillside', x: -347, z: 1073, radius: 220, yaw: -0.5, pitch: 1.25 },
  { name: 'pr1-bruehlberg-oblique', x: -266, z: 474, radius: 200, yaw: -0.4, pitch: 1.0 },
  // Highest-relief rail profile in the Gemeinde (13.9 m over one way) — the
  // main line NE of the city where the 200 m smoothing window carves the
  // classic embankment/cut silhouette.
  { name: 'pr1-rail-embankment', x: 2735, z: 4080, radius: 220, yaw: 0.9, pitch: 1.1 },
  { name: 'pr1-topdown-no-holes', x: -266, z: 474, radius: 160, yaw: 0.0, pitch: 1.53 },
];

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  console.log(`[pr1] launching stack (vite :${VITE_PORT})…`);
  const stack = await startTrafficStack({ trafficPort: TRAFFIC_PORT, vitePort: VITE_PORT, seed: 42, at: '2026-07-03T12:00' });

  const browser = await chromium.launch({
    headless: true,
    args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
  });

  const written = [];
  try {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    page.on('pageerror', (e) => console.error('pageerror:', e.message));

    const url =
      `http://${HOST}:${VITE_PORT}/ksw.html?traffic=1&trafficWs=ws://${HOST}:${TRAFFIC_PORT}/traffic` +
      `&at=2026-07-03T15:00:00Z&wx=clear`;
    console.log(`[pr1] opening ${url}`);
    await page.goto(url, { waitUntil: 'load', timeout: 60000 });
    const readyMs = Number(process.env.SMOKE_READY_TIMEOUT_MS ?? 240000);
    await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: readyMs });
    await page.waitForFunction(() => typeof window.__trees?.lookAt === 'function', { timeout: 20000 });

    const cdp = await page.context().newCDPSession(page);

    for (const site of SITES) {
      await page.evaluate((s) => {
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
      console.log(`[pr1] ${site.name}: framed [${site.x}, ${site.z}] r${site.radius} -> ${file}`);
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
