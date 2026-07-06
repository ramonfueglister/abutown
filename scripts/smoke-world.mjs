// scripts/smoke-world.mjs
//
// M1 Task 16 smoke: PROVES the persistent world end-to-end — TWO independent
// clients on the one-process sim-server's /live channel see the SAME world,
// the 4 h world day runs at 6× realtime, the SFC audit stays green, and (with
// a local test Postgres) a kill + relaunch RESUMES the world instead of
// reseeding it.
//
// Asserts (plan Task 16):
//   (1) two clients receive Vitals with identical `population` and a
//       `world_tick` difference < 100 (10 Hz ⇒ < 10 s apart),
//   (2) both see ≥1 CitizenCellFrame with citizens > 0 (AOI on the Altstadt
//       around the Bahnhof landmark),
//   (3) vitals `audit_ok == true` on every sample across a 30 s window,
//   (4) `s_of_world_day` grows ~6× realtime (4 h world day), measured over
//       ≥20 s, tolerance ±20%,
//   (5) restart-resume — ONLY when ABUTOWN_TEST_DATABASE_URL is set (a TEST
//       db; the launcher passes it explicitly so the inherited prod .env can
//       NEVER leak in, see lib/traffic-stack.mjs env-hygiene banner): after
//       ≥10 s of persistence (5 s flush cadence) the sim-server is killed and
//       relaunched; the new boot log must contain "resuming world-core from
//       persisted snapshot", /health must report resumed=true, the clients
//       must reconnect on their own, world_tick must continue monotonically
//       (new ≥ last-before-kill) and population must be unchanged.
//       Without the env var this part is SKIPPED with a log line.
//
// Modes (CI decision, Task 16 Step 2):
//   * default — real headless-chromium WebGPU render of ksw.html?live=1 (two
//     Playwright BrowserContexts). Requires the baked world pyramid
//     data/winterthur/world (gitignored, 77 MB) — LOCAL runs only.
//   * --no-render — CI mode. The CI e2e job cannot have the world bake (77 MB,
//     gitignored; geo:fetch + geo:bake-world in CI is far too heavy) and the
//     existing e2e job runs only the bake-free smoke-cardhand. So in CI this
//     smoke runs headless against /live + /health directly: two Node WS
//     clients (built-in WebSocket) that decode with the SAME generated proto
//     module the app imports (src/proto/live_pb.ts via Node's type
//     stripping — no re-implementation) and subscribe Altstadt cells derived
//     via the SAME shared CellGrid (src/diorama/traffic/cellGrid.ts). All 5
//     asserts hold in this mode too — (2) is proven on the wire (frames with
//     citizens) instead of on pixels. The browser mode stays the local gate
//     for the render path (CLAUDE.md browser-smoke rule is covered by the
//     local runs + smoke-traffic/smoke-ksw for the city view itself).
//
// Ports are non-default (8189/5191) and the launcher hard-aborts when they
// are taken (Task 15 trap #2: a busy port silently smokes a foreign server).

import { readFileSync } from 'node:fs';
import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import { LiveClientMsgSchema, LiveServerMsgSchema } from '../src/proto/live_pb.ts';
import { CellGrid } from '../src/diorama/traffic/cellGrid.ts';
import { startWorldStack, HOST } from './lib/traffic-stack.mjs';

const NO_RENDER = process.argv.includes('--no-render');
const TEST_DB = process.env.ABUTOWN_TEST_DATABASE_URL;

const SIM_PORT = 8189;
const VITE_PORT = 5191;
const SEED = 42;
// Pinned workday morning (2026-07-03 = Friday): citizens are up and about,
// and day_kind cannot silently flip on weekend runs (smoke-traffic lesson).
const AT_SIM = '2026-07-03T08:00';

// Altstadt anchor: the Bahnhof landmark from data/winterthur/meta.json —
// same coords the ksw.html `cam=bahnhof` preset aims at.
const meta = JSON.parse(readFileSync(new URL('../data/winterthur/meta.json', import.meta.url), 'utf8'));
const [ALTSTADT_X, ALTSTADT_Z] = meta.landmarks.bahnhof;

// ── shared client interface ─────────────────────────────────────────────────
// Both modes expose: vitals() -> {worldTick, sOfWorldDay, population, auditOk}
// | null, citizensSeen() -> max citizens observed, close().

/** Node WS client (--no-render): same wire, same generated proto decode. */
function nodeLiveClient({ url, cells, label }) {
  let closed = false;
  let ws = null;
  let last = null;
  let maxCitizensInFrame = 0;
  const open = () => {
    ws = new WebSocket(url);
    ws.binaryType = 'arraybuffer';
    ws.addEventListener('open', () => {
      // Mirror liveClient.ts: (re)subscribe the full set + vitals on every open.
      const msg = create(LiveClientMsgSchema, {
        subscribeCells: cells,
        unsubscribeCells: [],
        subscribeVitals: true,
      });
      ws.send(toBinary(LiveClientMsgSchema, msg));
    });
    ws.addEventListener('message', (ev) => {
      if (!(ev.data instanceof ArrayBuffer)) return;
      const msg = fromBinary(LiveServerMsgSchema, new Uint8Array(ev.data));
      for (const f of msg.cells) {
        if (f.citizens.length > maxCitizensInFrame) maxCitizensInFrame = f.citizens.length;
      }
      if (msg.vitals) last = msg.vitals;
    });
    ws.addEventListener('close', () => {
      // Same simple reconnect policy as liveClient.ts.
      if (!closed) setTimeout(open, 1000);
    });
    ws.addEventListener('error', () => {
      /* close handler drives reconnect */
    });
  };
  open();
  return {
    label,
    errors: [],
    vitals: async () =>
      last == null
        ? null
        : {
            worldTick: Number(last.worldTick),
            sOfWorldDay: last.sOfWorldDay,
            population: Number(last.population),
            auditOk: last.auditOk === 1,
          },
    citizensSeen: async () => maxCitizensInFrame,
    close: async () => {
      closed = true;
      ws?.close();
    },
  };
}

/** Browser client: one Playwright BrowserContext on ksw.html?live=1, reading
 * the window.__live debug surface (Task 15). */
async function browserLiveClient(browser, { label }) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(`[${label}] pageerror: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(`[${label}] console: ${m.text()}`);
  });

  // cam=bahnhof parks the camera — and with it the 3×3-cell live AOI, which
  // follows the camera target — on the Altstadt. Render clock frozen for a
  // reproducible scene; the SIM clock is pinned separately via the launcher.
  const url =
    `http://${HOST}:${VITE_PORT}/ksw.html` +
    `?live=1&liveWs=ws://${HOST}:${SIM_PORT}/live` +
    `&cam=bahnhof&at=2026-07-03T08:00:00Z&wx=clear`;
  console.log(`[smoke:${label}] opening ${url}`);
  await page.goto(url, { waitUntil: 'load', timeout: 60000 });
  // The boot streams the 77 MB world pyramid before __LOOK_READY (post-#119).
  const readyMs = Number(process.env.SMOKE_READY_TIMEOUT_MS ?? 180000);
  try {
    await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: readyMs });
  } catch (e) {
    console.error(`[smoke:${label}] __LOOK_READY not set after ${readyMs}ms; page errors so far:`);
    for (const line of errors) console.error(`  ${line}`);
    throw e;
  }
  await page.waitForFunction(() => window.__live !== undefined, { timeout: 20000 });

  return {
    label,
    errors,
    vitals: () => page.evaluate(() => window.__live?.vitals() ?? null),
    citizensSeen: () => page.evaluate(() => window.__live?.citizenCount() ?? 0),
    close: async () => {
      await page.close().catch(() => {});
      await context.close().catch(() => {});
    },
  };
}

/** Poll `client.vitals()` until non-null. */
async function waitVitals(client, timeoutMs) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const v = await client.vitals();
    if (v != null) return v;
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`[${client.label}] no vitals received within ${timeoutMs}ms`);
}

/** s_of_world_day delta, wrap-safe (uint32 seconds, wraps at 86400). */
function worldDayDelta(s0, s1) {
  return s1 >= s0 ? s1 - s0 : s1 + 86400 - s0;
}

async function main() {
  const failures = [];
  const check = (name, ok, detail) => {
    console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  (${detail})` : ''}`);
    if (!ok) failures.push(name);
  };

  console.log(
    `[smoke] mode=${NO_RENDER ? 'no-render (wire-only)' : 'browser'} ` +
      `persistence=${TEST_DB ? 'TEST postgres' : 'in-memory (restart part will be skipped)'}`,
  );
  const stack = await startWorldStack({
    simPort: SIM_PORT,
    vitePort: VITE_PORT,
    vite: !NO_RENDER,
    seed: SEED,
    at: AT_SIM,
    databaseUrl: TEST_DB, // undefined ⇒ in-memory; NEVER inherited from .env
  });

  let browser = null;
  let clients = [];
  try {
    if (NO_RENDER) {
      // Altstadt AOI: 5×5 cells around the Bahnhof, derived via the SAME
      // CellGrid the frontend uses (shared trafficnet.json lanes derivation).
      const net = JSON.parse(
        readFileSync(new URL('../data/winterthur/trafficnet.json', import.meta.url), 'utf8'),
      );
      const grid = CellGrid.build(net.lanes);
      const cells = [...grid.cellsAround(ALTSTADT_X, ALTSTADT_Z, 2)];
      const url = `ws://${HOST}:${SIM_PORT}/live`;
      clients = [
        nodeLiveClient({ url, cells, label: 'A' }),
        nodeLiveClient({ url, cells, label: 'B' }),
      ];
    } else {
      const { chromium } = await import('playwright');
      browser = await chromium.launch({
        headless: true,
        args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
      });
      clients = [
        await browserLiveClient(browser, { label: 'A' }),
        await browserLiveClient(browser, { label: 'B' }),
      ];
    }
    const [A, B] = clients;

    await waitVitals(A, 30000);
    await waitVitals(B, 30000);

    // ── (1) same world: identical population, world_tick within 100 ─────────
    const [v1a, v1b] = await Promise.all([A.vitals(), B.vitals()]);
    const tickDiff = Math.abs(v1a.worldTick - v1b.worldTick);
    check(
      '(1) two clients share one world (population identical, |Δworld_tick| < 100)',
      v1a.population === v1b.population && v1a.population > 0 && tickDiff < 100,
      `population A=${v1a.population} B=${v1b.population}, world_tick A=${v1a.worldTick} B=${v1b.worldTick} (Δ=${tickDiff})`,
    );

    // ── (2) ≥1 CitizenCellFrame with citizens > 0 on both clients ───────────
    const tCitizens = Date.now();
    let seenA = 0;
    let seenB = 0;
    while (Date.now() - tCitizens < 90000 && (seenA === 0 || seenB === 0)) {
      [seenA, seenB] = await Promise.all([A.citizensSeen(), B.citizensSeen()]);
      if (seenA > 0 && seenB > 0) break;
      await new Promise((r) => setTimeout(r, 1000));
    }
    check(
      '(2) both clients see a CitizenCellFrame with citizens > 0 (Altstadt AOI)',
      seenA > 0 && seenB > 0,
      `citizens A=${seenA} B=${seenB}`,
    );

    // ── (3)+(4) one 30 s window: audit_ok on every sample, 6× world-day rate ─
    const WINDOW_MS = 30000;
    const t0 = Date.now();
    const [s0a, s0b] = await Promise.all([A.vitals(), B.vitals()]);
    let auditSamples = 0;
    let auditFailures = 0;
    while (Date.now() - t0 < WINDOW_MS) {
      await new Promise((r) => setTimeout(r, 2000));
      for (const v of await Promise.all([A.vitals(), B.vitals()])) {
        if (v == null) continue;
        auditSamples++;
        if (!v.auditOk) auditFailures++;
      }
    }
    const t1 = Date.now();
    const [s1a, s1b] = await Promise.all([A.vitals(), B.vitals()]);
    check(
      '(3) vitals audit_ok == true on every sample across 30 s (both clients)',
      auditSamples >= 20 && auditFailures === 0,
      `${auditSamples - auditFailures}/${auditSamples} samples ok`,
    );

    const elapsedS = (t1 - t0) / 1000;
    const rateA = worldDayDelta(s0a.sOfWorldDay, s1a.sOfWorldDay) / elapsedS;
    const rateB = worldDayDelta(s0b.sOfWorldDay, s1b.sOfWorldDay) / elapsedS;
    const rateOk = (r) => r >= 6 * 0.8 && r <= 6 * 1.2;
    check(
      '(4) s_of_world_day grows ~6× realtime (4 h world day, ±20%)',
      elapsedS >= 20 && rateOk(rateA) && rateOk(rateB),
      `rate A=${rateA.toFixed(2)}× B=${rateB.toFixed(2)}× over ${elapsedS.toFixed(1)}s`,
    );

    // ── (5) restart-resume (only with a TEST database) ──────────────────────
    if (TEST_DB) {
      // The stack has been up well over 10 s by now (asserts 1–4), so at the
      // 5 s flush cadence at least one snapshot write has landed.
      const before = await A.vitals();
      console.log(
        `[smoke] killing sim-server (last seen world_tick=${before.worldTick}, population=${before.population})…`,
      );
      const bootLogRaw = await stack.restartSimServer();
      // tracing emits ANSI colour codes even into a pipe — strip before matching.
      // eslint-disable-next-line no-control-regex
      const bootLog = bootLogRaw.replace(/\x1b\[[0-9;]*m/g, '');
      const resumeLine = bootLog
        .split('\n')
        .find((l) => l.includes('resuming world-core from persisted snapshot'));
      check(
        '(5a) new boot log proves resume (not a reseed)',
        resumeLine !== undefined,
        resumeLine ? resumeLine.replace(/^\[sim\]\s*/, '').trim() : 'resume line MISSING in boot log',
      );

      const health = await (
        await fetch(`http://${HOST}:${SIM_PORT}/health`, { signal: AbortSignal.timeout(5000) })
      ).json();
      check('(5b) /health reports resumed=true', health.resumed === true, JSON.stringify(health));

      // Clients reconnect on their own (liveClient/nodeLiveClient retry every
      // 1 s); world_tick must catch up past the last pre-kill value (10 Hz —
      // the resumed snapshot may be up to one 5 s flush older) and population
      // must be exactly what it was.
      const tResume = Date.now();
      let vA = null;
      let vB = null;
      while (Date.now() - tResume < 120000) {
        [vA, vB] = await Promise.all([A.vitals(), B.vitals()]);
        if (vA != null && vB != null && vA.worldTick >= before.worldTick && vB.worldTick >= before.worldTick) break;
        await new Promise((r) => setTimeout(r, 1000));
      }
      check(
        '(5c) clients reconnected, world_tick monotonic across the restart',
        vA != null && vB != null && vA.worldTick >= before.worldTick && vB.worldTick >= before.worldTick,
        `world_tick before-kill=${before.worldTick}, after A=${vA?.worldTick} B=${vB?.worldTick}`,
      );
      check(
        '(5d) population unchanged across the restart',
        vA?.population === before.population && vB?.population === before.population,
        `population before=${before.population}, after A=${vA?.population} B=${vB?.population}`,
      );
    } else {
      console.log(
        'SKIP  (5) restart-resume — ABUTOWN_TEST_DATABASE_URL not set (in-memory run, nothing to resume)',
      );
    }

    const errors = clients.flatMap((c) => c.errors);
    if (errors.length) {
      console.error('--- page/console errors ---');
      for (const e of errors.slice(0, 12)) console.error(e);
      failures.push('page errors');
    }
  } finally {
    for (const c of clients) await c.close().catch(() => {});
    if (browser) await browser.close().catch(() => {});
    stack.cleanup();
  }

  if (failures.length) {
    console.error(`\nSMOKE FAIL: ${failures.join(', ')}`);
    process.exit(1);
  }
  console.log(
    `\nSMOKE OK — two clients share one live world (6× world day, audit green` +
      `${TEST_DB ? ', restart-resume proven' : ''})`,
  );
}

main().catch((err) => {
  console.error('SMOKE ERROR:', err);
  process.exit(1);
});
