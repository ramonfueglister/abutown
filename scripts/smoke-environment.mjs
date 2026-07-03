// Environment smoke: proves the REAL wiring — (1) the client actually requests
// open-meteo and applies the parsed series, (2) the ?at/?wx state matrix lands
// in __ENV_STATE with the expected values. CLAUDE.md: mandatory before "complete".
//
// Usage: node scripts/smoke-environment.mjs

import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';
import { readFileSync } from 'node:fs';

const HOST = '127.0.0.1';
const PORT = 5175;

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

const dev = spawn('npm', ['run', 'dev'], {
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

let failed = false;
function fail(msg) {
  console.error(`SMOKE FAIL: ${msg}`);
  failed = true;
}

try {
  if (!(await waitForPort(30000))) {
    fail(`dev server not up.\n${devOut}`);
  } else {
    const fixture = readFileSync(new URL('../tests/fixtures/openMeteo.json', import.meta.url), 'utf8');
    const browser = await chromium.launch({
      headless: true,
      args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
    });
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const pageErrors = [];
    page.on('pageerror', (e) => pageErrors.push(`pageerror: ${e.message}`));
    page.on('console', (m) => {
      if (m.type() === 'error') pageErrors.push(`console: ${m.text()}`);
    });

    // Route intercept MUST be set up before the first goto (probe 1 has no
    // ?wx and triggers the real open-meteo fetch).
    let meteoRequested = false;
    await page.route('**/api.open-meteo.com/**', (route) => {
      meteoRequested = true;
      return route.fulfill({ status: 200, contentType: 'application/json', body: fixture });
    });

    const checks = [];
    const probe = async (query, assert) => {
      await page.goto(`http://${HOST}:${PORT}/look.html?${query}`, { waitUntil: 'load', timeout: 20000 });
      try {
        await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 45000 });
        const env = await page.evaluate(() => window.__ENV_STATE);
        const errors = assert(env);
        checks.push({ query, errors });
        for (const e of errors) console.error(`FAIL [${query}]: ${e}`);
      } catch (e) {
        checks.push({ query, errors: [`scene never became ready: ${e}`] });
        console.error(`FAIL [${query}]: scene never became ready: ${e}`);
      }
    };

    // 1) Live wiring: no ?wx → the client fetches open-meteo AND applies the
    // parsed series. ?at is before the fixture's time range, so sampleWeather
    // clamps to states[0] (cloud_cover 20% → 0.2). Per weatherLook:
    //   cloudCoverage = coverageMin + (coverageMax - coverageMin) * 0.2
    //                 = 0.15 + 0.70 * 0.2 = 0.29
    // The fetch is async and applied on a later frame, so wait for the series to
    // land instead of reading the first frame's CLEAR_SKY default.
    await probe('at=2026-07-03T11:00:00Z', () => []); // navigate + become ready
    let appliedCloud = NaN;
    try {
      await page.waitForFunction(
        () => window.__ENV_STATE && Math.abs(window.__ENV_STATE.cloudCoverage - 0.29) < 0.02,
        { timeout: 15000 },
      );
      appliedCloud = (await page.evaluate(() => window.__ENV_STATE)).cloudCoverage;
    } catch {
      appliedCloud = (await page.evaluate(() => window.__ENV_STATE?.cloudCoverage)) ?? NaN;
    }
    {
      const errs = [];
      if (!meteoRequested) errs.push('open-meteo was never requested');
      if (!(Math.abs(appliedCloud - 0.29) < 0.02))
        errs.push(`parsed series not applied: cloudCoverage=${appliedCloud} (expected ~0.29 from states[0] cloud 20%)`);
      const env = await page.evaluate(() => window.__ENV_STATE);
      if (env.sunElevDeg < 55) errs.push(`noon sun too low: ${env.sunElevDeg}`);
      checks.push({ query: 'at=2026-07-03T11:00:00Z (applied)', errors: errs });
      for (const e of errs) console.error(`FAIL [applied]: ${e}`);
    }
    // 2) State matrix (wx overrides, no network dependency)
    await probe('at=2026-07-03T04:00:00Z&wx=clear', (e) =>
      e.sunElevDeg > -8 && e.sunElevDeg < 12 && e.godraysMix >= 0 ? [] : [`dawn state off: elev=${e.sunElevDeg}`]
    );
    await probe('at=2026-07-03T19:45:00Z&wx=clear', (e) =>
      e.lampOn01 > 0.3 ? [] : ['dusk should start warming windows']
    );
    await probe('at=2026-07-03T23:30:00Z&wx=clear', (e) =>
      e.starVisibility > 0.7 && e.sunIntensity < 0.05 ? [] : [`night off: stars=${e.starVisibility}`]
    );
    await probe('at=2026-07-03T11:00:00Z&wx=overcast', (e) =>
      e.cloudCoverage > 0.7 && e.sunIntensity < 3 ? [] : [`overcast off: cov=${e.cloudCoverage}`]
    );
    await probe('at=2026-07-03T11:00:00Z&wx=rain', (e) =>
      e.precipType === 'rain' && e.precipIntensity > 0.5 ? [] : [`rain off: ${e.precipType}`]
    );
    await probe('at=2026-01-15T11:00:00Z&wx=snow', (e) => (e.precipType === 'snow' ? [] : [`snow off: ${e.precipType}`]));
    await probe('at=2026-07-03T11:00:00Z&wx=fog', (e) => (e.fogFar < 30 ? [] : [`fog off: far=${e.fogFar}`]));
    // 3) Winter check: at 17:30 CET in January it is already night
    await probe('at=2026-01-15T16:30:00Z&wx=clear', (e) =>
      e.sunElevDeg < 0 ? [] : ['winter 17:30 local should be after sunset']
    );

    await browser.close();

    if (pageErrors.length) {
      console.error('--- page errors ---');
      for (const e of pageErrors.slice(0, 12)) console.error(e);
    }

    const failedChecks = checks.filter((c) => c.errors.length > 0);
    console.log(`\nsmoke-environment: ${checks.length - failedChecks.length}/${checks.length} passed`);
    if (failedChecks.length > 0) failed = true;
  }
} finally {
  cleanup();
}
process.exit(failed ? 1 : 0);
