// Visual iteration harness (report tooling, not a gate).
//
// Captures the schematic map at three semantic-zoom presets (overview as
// loaded, economy band zoomed out, city band zoomed in) plus the
// render_game_to_text diagnostic at each, so renderer changes can be
// compared screenshot-by-screenshot.
//
// Usage:
//   FRONTEND_URL=http://127.0.0.1:5177 OUT_DIR=shots node scripts/capture-visuals.mjs
//
// Writes <OUT_DIR>/<preset>.png and <OUT_DIR>/<preset>.json.

import { chromium } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const FRONTEND_URL = process.env.FRONTEND_URL ?? 'http://127.0.0.1:5177';
const OUT_DIR = process.env.OUT_DIR ?? 'shots';
const SETTLE_MS = Number(process.env.SETTLE_MS ?? 5000);

mkdirSync(OUT_DIR, { recursive: true });

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
await page.goto(FRONTEND_URL, { waitUntil: 'domcontentloaded', timeout: 30000 });
await page.waitForFunction(
  () => {
    const fn = window.render_game_to_text;
    if (typeof fn !== 'function') return false;
    const diag = JSON.parse(fn());
    return diag.city?.mobility?.status === 'connected';
  },
  { timeout: 45000 },
);
await page.waitForTimeout(SETTLE_MS);

const cdp = await page.context().newCDPSession(page);

async function capture(name) {
  // Raw CDP screenshot instead of page.screenshot: the latter intermittently
  // stalls against the continuously-animating canvas under headless chromium.
  // CDP also captures DOM overlays (HUD/banners), unlike canvas.toDataURL.
  const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(join(OUT_DIR, `${name}.png`), Buffer.from(data, 'base64'));
  const diag = JSON.parse(await page.evaluate(() => window.render_game_to_text()));
  writeFileSync(join(OUT_DIR, `${name}.json`), JSON.stringify(diag, null, 2));
  const c = diag.city;
  console.log(
    `[capture] ${name}: peds=${c.pedestrians} cars=${c.cars} flows=${c.economyFlowCount ?? '?'} markets=${c.economyMarketCount ?? '?'}`,
  );
}

async function wheel(steps, deltaY) {
  for (let i = 0; i < steps; i += 1) {
    await page.mouse.wheel(0, deltaY);
    await page.waitForTimeout(60);
  }
  await page.waitForTimeout(1500); // let the camera lerp settle
}

// Preset 1: overview exactly as the app loads (fit-world camera).
await capture('overview');

// Preset 2: city band — zoom IN on the market corridor (screen centre-ish).
await page.mouse.move(730, 455);
await wheel(8, -120);
await capture('city');

// Preset 3: economy band — zoom OUT until the whole network is in view.
await wheel(20, 120);
await capture('economy');

await browser.close();
console.log(`[capture] wrote ${OUT_DIR}/{overview,city,economy}.{png,json}`);
