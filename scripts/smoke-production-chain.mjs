// Production-chain browser smoke: verify the firms-as-buyers WOOD→TOOLS chain
// is wired end-to-end over the real dev stack (frontend↔backend wire boundary).
//
// Asserts (over the binary protobuf wire + canvas diagnostics):
// 1. Wire: an `economySnapshot` ServerMessage carries `producers` with exactly
//    one producer — actor 8031 at market 9001, in_good 2 (WOOD) → out_good 4
//    (TOOLS), in_qty 10 / out_qty 10, and `max_bid > 0` (the participation
//    bound is discovered on the first macro cadence, tick 10).
// 2. Wire: the macro flow carries a WOOD edge 9003 → 9001 (good_id 2, rate > 0)
//    — the producer is actually BUYING its input across markets.
// 3. Render: after zooming out so the near-origin corridor is observed, at
//    least one `trader:`-prefixed mobility agent materializes, and at least one
//    of them is the WOOD shipment trader: it first appears near market 9003
//    [8,40] and walks TOWARD market 9001 [8,8] (distance-to-9001 decreases).
// 4. Inspector: clicking market 9001's tile selects it (selectedMarketCoord ==
//    {8,8}); a screenshot of the inspector panel (with the producer rows
//    `recipe: 10 WOOD → 10 TOOLS`, `cash/target=...`, `max bid=...`) is saved
//    for eyeballing.
//
// This is the acceptance gate for the frontend<->backend wire boundary for the
// production-chains feature. "All unit tests pass" is NOT a substitute.
//
// Stack management:
//   The smoke starts its own isolated dev stack (backend on BACKEND_PORT,
//   frontend vite dev server on FRONTEND_PORT) so it does not conflict with any
//   other running dev server. Defaults: 8083 / 5177; override via env.
//   DATABASE_URL must point at a THROWAWAY local database (the backend runs
//   embedded migrations and seeds the abutopia world into it) — never the live
//   dev DB. SUPABASE_URL may be a dummy (JWKS refresh failure is tolerated).
//   Set REUSE_STACK=1 to connect to an already-running stack at the configured
//   ports instead.

import { chromium } from '@playwright/test';
import { fromBinary } from '@bufbuild/protobuf';
import { tsImport } from 'tsx/esm/api';
import { spawn, execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const protoModule = await tsImport('../src/backend/proto/abutown_pb.ts', import.meta.url);
const { ServerMessageSchema } = protoModule;

const BACKEND_PORT = parseInt(process.env.BACKEND_PORT ?? '8083', 10);
const FRONTEND_PORT = parseInt(process.env.FRONTEND_PORT ?? '5177', 10);
const BACKEND_URL = `http://127.0.0.1:${BACKEND_PORT}`;
const FRONTEND_URL = `http://127.0.0.1:${FRONTEND_PORT}`;
const PAGE_TIMEOUT_MS = 20000;
const REUSE_STACK = process.env.REUSE_STACK === '1';
const SCREENSHOT_PATH = process.env.SCREENSHOT_PATH ?? '/tmp/smoke-production-chain-inspector.png';

// Chain topology under test (data/worlds/abutopia/layers/markets.json).
const PRODUCER = { actorId: 8031, marketId: 9001, inGood: 2, outGood: 4, inQty: 10, outQty: 10 };
const WOOD_FLOW = { src: 9003, dst: 9001, goodId: 2 };
const MARKET_9001 = { x: 8, y: 8 };
const MARKET_9003 = { x: 8, y: 40 };

const killProcessGroup = process.platform !== 'win32';
let backendChild = null;
let frontendChild = null;
let shuttingDown = false;
const backendLogTail = [];

function terminate(child) {
  if (!child || child.killed) return;
  if (killProcessGroup && child.pid) {
    try {
      process.kill(-child.pid, 'SIGTERM');
      return;
    } catch {
      // fall through
    }
  }
  child.kill('SIGTERM');
}

function shutdown(code) {
  if (shuttingDown) return;
  shuttingDown = true;
  terminate(backendChild);
  terminate(frontendChild);
  setTimeout(() => process.exit(code), 800).unref();
}

process.on('SIGINT', () => shutdown(130));
process.on('SIGTERM', () => shutdown(143));

async function pause(ms) {
  await new Promise((r) => setTimeout(r, ms));
}

function toBytes(payload) {
  if (payload instanceof Buffer) {
    return new Uint8Array(payload.buffer, payload.byteOffset, payload.byteLength);
  }
  if (payload instanceof ArrayBuffer) return new Uint8Array(payload);
  if (payload instanceof Uint8Array) return payload;
  return null;
}

async function waitForHttpOk(url, timeoutMs, label) {
  const start = Date.now();
  while (true) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      // retry
    }
    if (Date.now() - start > timeoutMs) {
      throw new Error(`timed out waiting for ${label ?? url} (${timeoutMs}ms)`);
    }
    await pause(250);
  }
}

// --- Start isolated stack (unless REUSE_STACK=1) ---
if (!REUSE_STACK) {
  if (!process.env.DATABASE_URL) {
    console.log(JSON.stringify({
      status: 'stack-failed',
      phase: 'config',
      error: 'DATABASE_URL is required and must point at a THROWAWAY local database',
    }, null, 2));
    process.exit(1);
  }

  // 1. Build the backend (serialized; no-op when already built).
  console.error('[smoke] building backend via cargo-serial ...');
  try {
    execSync('scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server', {
      stdio: 'inherit',
      cwd: process.cwd(),
    });
  } catch (err) {
    console.log(JSON.stringify({ status: 'stack-failed', phase: 'backend-build', error: String(err) }, null, 2));
    process.exit(1);
  }

  // 2. Start the backend against the throwaway DB.
  const backendBinary = join(process.cwd(), 'backend/target/debug/sim-server');
  console.error(`[smoke] starting backend on port ${BACKEND_PORT} ...`);
  backendChild = spawn(backendBinary, [], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      LISTEN_PORT: String(BACKEND_PORT),
      SUPABASE_URL: process.env.SUPABASE_URL ?? 'http://127.0.0.1:9',
      CORS_ALLOWED_ORIGINS: FRONTEND_URL,
      RUST_LOG: process.env.RUST_LOG ?? 'info',
    },
    detached: killProcessGroup,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  const keepTail = (chunk) => {
    backendLogTail.push(...chunk.toString().split('\n').filter(Boolean));
    while (backendLogTail.length > 40) backendLogTail.shift();
  };
  backendChild.stdout.on('data', keepTail);
  backendChild.stderr.on('data', keepTail);

  try {
    await waitForHttpOk(`${BACKEND_URL}/health`, 60_000, 'backend');
  } catch (err) {
    console.log(JSON.stringify({
      status: 'stack-failed',
      phase: 'backend-start',
      error: String(err),
      backend_log_tail: backendLogTail,
    }, null, 2));
    shutdown(1);
    await pause(1000);
    process.exit(1);
  }
  console.error(`[smoke] backend healthy at ${BACKEND_URL}`);

  // 3. Start a vite DEV server pointing the frontend at this backend.
  const viteBin = fileURLToPath(new URL('../node_modules/vite/bin/vite.js', import.meta.url));
  console.error(`[smoke] starting vite dev on port ${FRONTEND_PORT} ...`);
  frontendChild = spawn(
    process.execPath,
    [viteBin, 'dev', '--host', '127.0.0.1', '--port', String(FRONTEND_PORT), '--strictPort'],
    {
      cwd: process.cwd(),
      env: { ...process.env, VITE_ABUTOWN_BACKEND_URL: BACKEND_URL },
      detached: killProcessGroup,
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
  frontendChild.stdout.on('data', () => {});
  frontendChild.stderr.on('data', () => {});

  try {
    await waitForHttpOk(FRONTEND_URL, 30_000, 'frontend');
  } catch (err) {
    console.log(JSON.stringify({ status: 'stack-failed', phase: 'frontend-start', error: String(err) }, null, 2));
    shutdown(1);
    await pause(1000);
    process.exit(1);
  }
  console.error(`[smoke] frontend serving at ${FRONTEND_URL}`);
}

// --- Browser smoke ---
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
const page = await context.newPage();

const receivedBinary = [];
let decodedUpTo = 0;
const consoleErrors = [];

page.on('websocket', (ws) => {
  if (!ws.url().includes(`:${BACKEND_PORT}/`)) return; // backend WS only (skip vite HMR)
  ws.on('framereceived', (ev) => {
    if (typeof ev.payload === 'string') return;
    const bytes = toBytes(ev.payload);
    if (bytes) receivedBinary.push(bytes);
  });
});
page.on('console', (msg) => {
  if (msg.type() === 'error') consoleErrors.push(msg.text());
});
page.on('pageerror', (err) => consoleErrors.push(err.message));

function decodeServer(bytes) {
  try {
    return fromBinary(ServerMessageSchema, bytes);
  } catch {
    return null;
  }
}

// Incremental wire-state accumulated from decoded frames.
let economyFrameCount = 0;
let producersObserved = null; // last non-empty producers array (plain objects)
let woodFlowObserved = null; // first matching 9003→9001 WOOD flow
const traderTracks = new Map(); // id -> { spriteKey, samples: [{x, y, tick}] }
let mobilityDeltaCount = 0;

function dist(ax, ay, bx, by) {
  return Math.hypot(ax - bx, ay - by);
}

function drainFrames() {
  for (; decodedUpTo < receivedBinary.length; decodedUpTo += 1) {
    const m = decodeServer(receivedBinary[decodedUpTo]);
    if (!m) continue;
    if (m.body.case === 'economySnapshot') {
      economyFrameCount += 1;
      const snap = m.body.value;
      if ((snap.producers?.length ?? 0) > 0) {
        producersObserved = snap.producers.map((p) => ({
          actorId: Number(p.actorId),
          marketId: p.marketId,
          inGood: p.inGood,
          outGood: p.outGood,
          retainedEarnings: Number(p.retainedEarnings),
          wcTarget: Number(p.wcTarget),
          maxBid: Number(p.maxBid),
          inQty: Number(p.inQty),
          outQty: Number(p.outQty),
          tick: Number(snap.tick),
        }));
      }
      for (const f of snap.flows ?? []) {
        if (f.srcMarketId === WOOD_FLOW.src && f.dstMarketId === WOOD_FLOW.dst && f.goodId === WOOD_FLOW.goodId) {
          woodFlowObserved ??= {
            srcMarketId: f.srcMarketId,
            dstMarketId: f.dstMarketId,
            goodId: f.goodId,
            rate: Number(f.rate),
            tick: Number(snap.tick),
          };
        }
      }
    }
    const collect = (agents, tick) => {
      for (const a of agents ?? []) {
        if (typeof a.id !== 'string' || !a.id.startsWith('trader:')) continue;
        const track = traderTracks.get(a.id) ?? { spriteKey: a.spriteKey ?? '', samples: [] };
        track.samples.push({ x: a.worldCoord?.x ?? 0, y: a.worldCoord?.y ?? 0, tick });
        traderTracks.set(a.id, track);
      }
    };
    if (m.body.case === 'mobilityChunkSnapshot') collect(m.body.value.agents, Number(m.body.value.tick ?? 0));
    if (m.body.case === 'mobilityChunkDelta') {
      mobilityDeltaCount += 1;
      collect(m.body.value.changedAgents, Number(m.body.value.tick ?? 0));
    }
  }
}

// A WOOD shipment trader spawns near 9003 [8,40] and walks toward 9001 [8,8]
// (distance to 9001 strictly shrinks). The coexisting 9003→9004 FOOD trader also
// spawns at 9003 but walks AWAY from 9001, and 9001→9002 traders spawn at 9001 —
// neither satisfies both conditions.
function findWoodTrader() {
  for (const [id, track] of traderTracks) {
    if (track.samples.length < 2) continue;
    const first = track.samples[0];
    const last = track.samples[track.samples.length - 1];
    const spawnNear9003 = dist(first.x, first.y, MARKET_9003.x, MARKET_9003.y) <= 6;
    const closedIn = dist(first.x, first.y, MARKET_9001.x, MARKET_9001.y)
      - dist(last.x, last.y, MARKET_9001.x, MARKET_9001.y) >= 3;
    if (spawnNear9003 && closedIn) {
      return { id, spriteKey: track.spriteKey, first, last, samples: track.samples.length };
    }
  }
  return null;
}

let pageLoadFailed = null;
try {
  await page.goto(FRONTEND_URL, { waitUntil: 'domcontentloaded', timeout: PAGE_TIMEOUT_MS });
} catch (e) {
  pageLoadFailed = String(e);
}
if (pageLoadFailed) {
  console.log(JSON.stringify({ status: 'page-load-failed', error: pageLoadFailed, backend_log_tail: backendLogTail }, null, 2));
  await browser.close();
  shutdown(1);
  await pause(1000);
  process.exit(1);
}

// Wait for the WS to open + first economy snapshot.
await pause(3000);

// --- Phase 1: producers + WOOD flow on the wire (poll up to 45 s) ---
{
  const deadline = Date.now() + 45_000;
  while (Date.now() < deadline) {
    drainFrames();
    if (producersObserved && producersObserved.some((p) => p.maxBid > 0) && woodFlowObserved) break;
    await pause(1000);
  }
  drainFrames();
}

// --- Phase 2: zoom out so the 9003→9001 corridor chunks are observed, then
// watch mobility frames for the WOOD flow-trader (up to 60 s). ---
await page.mouse.move(640, 400);
for (let i = 0; i < 8; i += 1) {
  await page.mouse.wheel(0, 240); // positive deltaY = zoom out
  await pause(60);
}
{
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    drainFrames();
    if (findWoodTrader()) break;
    await pause(1000);
  }
  drainFrames();
}
const woodTrader = findWoodTrader();

// --- Phase 3: click market 9001 and screenshot the inspector panel ---
let selectedMarketCoord = null;
let screenshotTaken = false;
let market9001Screen = null;
let market9001Tile = null;

// The wire carries the market's GRAPH-SNAPPED node tile, not the authored
// anchor — match the entry nearest to the authored anchor (within 4 tiles)
// instead of exact tile equality.
async function readMarket9001Entry() {
  const diagRaw = await page.evaluate(() => window.render_game_to_text?.() ?? '');
  try {
    const diag = JSON.parse(diagRaw);
    const candidates = (diag?.city?.economyMarkets ?? [])
      .map((m) => ({ m, d: dist(m.tileX, m.tileY, MARKET_9001.x, MARKET_9001.y) }))
      .filter((c) => c.d <= 4)
      .sort((a, b) => a.d - b.d);
    return candidates[0]?.m ?? null;
  } catch {
    return null;
  }
}

for (let attempt = 0; attempt < 6 && selectedMarketCoord === null; attempt += 1) {
  const entry = await readMarket9001Entry();
  if (!entry) {
    await pause(500);
    continue;
  }
  market9001Screen = entry.screen;
  market9001Tile = { x: entry.tileX, y: entry.tileY };
  const inView = entry.screen.x > 8 && entry.screen.x < 1272 && entry.screen.y > 8 && entry.screen.y < 792;
  if (!inView) {
    // Pan the camera so the market's screen position moves toward the viewport
    // center (drag is clamped to stay inside the viewport).
    const center = { x: 640, y: 400 };
    const dx = Math.max(-560, Math.min(560, center.x - entry.screen.x));
    const dy = Math.max(-360, Math.min(360, center.y - entry.screen.y));
    await page.mouse.move(center.x, center.y);
    await page.mouse.down();
    await page.mouse.move(center.x + dx, center.y + dy, { steps: 10 });
    await page.mouse.up();
    await pause(400);
    continue;
  }
  await page.mouse.click(entry.screen.x, entry.screen.y);
  await pause(300);
  const afterRaw = await page.evaluate(() => window.render_game_to_text?.() ?? '');
  try {
    const after = JSON.parse(afterRaw);
    selectedMarketCoord = after?.city?.selectedMarketCoord ?? null;
  } catch {
    // retry
  }
  if (selectedMarketCoord === null) await pause(500);
}
if (selectedMarketCoord !== null) {
  await page.screenshot({ path: SCREENSHOT_PATH });
  screenshotTaken = true;
}

await browser.close();

const producer = producersObserved?.[0] ?? null;
const checks = {
  // Assertion 1: producers on the wire with the seeded chain identity + a live participation bound.
  wire_producers_present: (producersObserved?.length ?? 0) === 1,
  wire_producer_identity:
    producer !== null &&
    producer.actorId === PRODUCER.actorId &&
    producer.marketId === PRODUCER.marketId &&
    producer.inGood === PRODUCER.inGood &&
    producer.outGood === PRODUCER.outGood &&
    producer.inQty === PRODUCER.inQty &&
    producer.outQty === PRODUCER.outQty,
  wire_producer_max_bid_positive: producer !== null && producer.maxBid > 0,
  // Assertion 2: the WOOD macro-flow edge 9003→9001 exists with rate > 0.
  wire_wood_flow_9003_to_9001: woodFlowObserved !== null && woodFlowObserved.rate > 0,
  // Assertion 3: a trader-prefixed agent materializes and walks the WOOD corridor.
  trader_agents_materialized: traderTracks.size > 0,
  wood_trader_walks_9003_to_9001: woodTrader !== null,
  // Assertion 4: inspector opens on market 9001 (selection echoes the clicked
  // entry's graph-snapped tile, which may differ from the authored anchor).
  inspector_selected_market_9001:
    selectedMarketCoord !== null &&
    market9001Screen !== null &&
    selectedMarketCoord.x === market9001Tile?.x &&
    selectedMarketCoord.y === market9001Tile?.y,
  no_console_errors: consoleErrors.length === 0,
};

const summary = {
  status: Object.values(checks).every(Boolean) ? 'ok' : 'failed',
  frontend_url: FRONTEND_URL,
  backend_url: BACKEND_URL,
  received_binary_frames: receivedBinary.length,
  economy_frame_count: economyFrameCount,
  mobility_delta_frames: mobilityDeltaCount,
  producers_observed: producersObserved,
  wood_flow_observed: woodFlowObserved,
  trader_ids_observed: [...traderTracks.keys()],
  wood_trader: woodTrader,
  market_9001_screen: market9001Screen,
  market_9001_tile: market9001Tile,
  selected_market_coord: selectedMarketCoord,
  inspector_screenshot: screenshotTaken ? SCREENSHOT_PATH : null,
  checks,
  console_errors: consoleErrors,
  backend_log_tail: checks.wire_producers_present ? undefined : backendLogTail,
};

console.log(JSON.stringify(summary, null, 2));
shutdown(summary.status === 'ok' ? 0 : 1);
await pause(900);
process.exit(summary.status === 'ok' ? 0 : 1);
