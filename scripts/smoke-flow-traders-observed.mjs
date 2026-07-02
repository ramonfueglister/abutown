// S4 flow-trader-AT-OBSERVED-MARKET browser smoke (auction↔flow coupling).
//
// S3 lets ACTIVE (observed) markets join the inter-market macro flow, so a
// flow-trader now SPAWNS at an observed source market and ARRIVES at an observed
// destination — not merely transits between two dormant markets (the #70 case,
// covered by scripts/smoke-flow-traders.mjs).
//
// This inverts #70: instead of observing a transit chunk to keep the market chunks
// dormant, we OBSERVE the F_A market chunk (0,1) — making F_A active — while keeping
// F_B (2,1) outside the view (dormant). The cross-edge F_A→F_B then
// fires every interval (active source residual ask → dormant deficit pool), and the
// FlowShipment SPAWNS at F_A. We assert a `trader:` agent's FIRST observed sample is
// in the observed F_A chunk (0,1) — i.e. it appeared AT the market, not mid-route.
//
// Acceptance gate for the S3 frontend↔backend render boundary (CLAUDE.md): unit
// tests cannot catch that the gate-lift actually surfaces flow-traders at the
// markets you directly observe.
//
// Runs against a dev stack at $SMOKE_URL (default http://127.0.0.1:5175). Use the
// DB-free e2e_server backend (fresh seed) on :8080 + a vite dev server on :5175.

import { chromium } from '@playwright/test';
import { fromBinary } from '@bufbuild/protobuf';
import { tsImport } from 'tsx/esm/api';
const protoModule = await tsImport('../src/backend/proto/abutown_pb.ts', import.meta.url);
const { ServerMessageSchema, ClientMessageSchema } = protoModule;

const URL = process.env.SMOKE_URL ?? 'http://127.0.0.1:5175';
const PAGE_TIMEOUT_MS = 20000;

const CHUNK_SIZE = 32; // tiles per chunk edge

// F_A market @ tile (8,40) → chunk (0,1); F_B market @ tile (72,40) → chunk (2,1).
const FA_TILE_X = 8;
const FA_TILE_Y = 40;
const MARKET_FA_CHUNK = { x: 0, y: 1 };
const MARKET_FB_CHUNK = { x: 2, y: 1 };

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

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ viewport: { width: 1024, height: 768 } });
const page = await context.newPage();

const receivedBinary = [];
let textFramesReceived = 0;
let textFramesSent = 0;
const consoleErrors = [];
const subscribedChunks = new Set();

page.on('websocket', (ws) => {
  if (!ws.url().includes(':8080/')) return;
  ws.on('framesent', (ev) => {
    if (typeof ev.payload === 'string') {
      textFramesSent += 1;
      return;
    }
    const bytes = toBytes(ev.payload);
    if (!bytes) return;
    let msg;
    try {
      msg = fromBinary(ClientMessageSchema, bytes);
    } catch {
      return;
    }
    if (msg.body.case === 'chunkSubscribe') {
      for (const c of msg.body.value.coords) subscribedChunks.add(`${c.x},${c.y}`);
    } else if (msg.body.case === 'chunkUnsubscribe') {
      for (const c of msg.body.value.coords) subscribedChunks.delete(`${c.x},${c.y}`);
    }
  });
  ws.on('framereceived', (ev) => {
    if (typeof ev.payload === 'string') {
      textFramesReceived += 1;
      return;
    }
    const bytes = toBytes(ev.payload);
    if (bytes) receivedBinary.push(bytes);
  });
});
page.on('console', (msg) => {
  if (msg.type() === 'error') consoleErrors.push(msg.text());
});
page.on('pageerror', (err) => consoleErrors.push(err.message));

try {
  await page.goto(URL, { waitUntil: 'domcontentloaded', timeout: PAGE_TIMEOUT_MS });
} catch (e) {
  console.log(JSON.stringify({ status: 'page-load-failed', error: String(e) }, null, 2));
  await browser.close();
  process.exit(1);
}

await pause(2500);

// --- Zoom IN toward CAMERA_MAX_SCALE, centred so the camera tracks the cursor. ---
await page.mouse.move(512, 384);
for (let i = 0; i < 6; i += 1) {
  await page.mouse.wheel(0, -240);
  await pause(80);
}

// --- Pan toward the F_A market chunk (0,1) until it is SUBSCRIBED (observed). ---
// F_A sits at the far-left of row 1 (tile 16,48). Drag the world right (mouse moves
// left→right) to reveal the left edge; repeat (adaptive) and stop once chunk (0,1)
// appears in the subscription set, so the test does not depend on a hand-tuned pan
// distance. A small downward bias keeps the camera on tile-row y≈48 (chunk row 1).
for (let attempt = 0; attempt < 16 && !subscribedChunks.has('0,1'); attempt += 1) {
  await page.mouse.move(260, 360);
  await page.mouse.down();
  await page.mouse.move(820, 400, { steps: 8 }); // drag world right+down → reveal upper-left
  await page.mouse.up();
  await pause(400); // let the 200 ms subscription poll fire
}
await pause(600);

const subscribedAfterZoom = new Set(subscribedChunks);

// --- Long observation window: ≥120 ticks (~12 s), several flow-interval cycles. ---
await pause(12000);

await browser.close();

function decodeServer(bytes) {
  try {
    return fromBinary(ServerMessageSchema, bytes);
  } catch {
    return null;
  }
}

const messages = receivedBinary.map(decodeServer).filter(Boolean);

// Collect trader-agent samples (id starts `trader:`) IN ARRIVAL ORDER (frames are
// appended in receive order), so traderSamples[id][0] is the FIRST observed sample.
const traderSamples = new Map();
let deltaCount = 0;
const collect = (agents) => {
  for (const a of agents) {
    if (typeof a.id !== 'string' || !a.id.startsWith('trader:')) continue;
    const arr = traderSamples.get(a.id) ?? [];
    arr.push({ x: a.worldCoord?.x ?? 0, y: a.worldCoord?.y ?? 0 });
    traderSamples.set(a.id, arr);
  }
};
for (const m of messages) {
  if (m.body.case === 'mobilityChunkSnapshot') collect(m.body.value.agents);
  if (m.body.case === 'mobilityChunkDelta') {
    deltaCount += 1;
    collect(m.body.value.changedAgents);
  }
}

const chunkOf = (p) => ({ x: Math.floor(p.x / CHUNK_SIZE), y: Math.floor(p.y / CHUNK_SIZE) });
const inFaChunk = (p) => {
  const c = chunkOf(p);
  return c.x === MARKET_FA_CHUNK.x && c.y === MARKET_FA_CHUNK.y;
};

const traderIds = [...traderSamples.keys()];

// The S4 claim: some flow-trader's FIRST observed sample is AT the observed source
// market F_A (chunk 0,1) — i.e. it spawned at the market, not in transit.
const traderSpawnedAtFa = traderIds.some((id) => {
  const samples = traderSamples.get(id);
  return samples.length > 0 && inFaChunk(samples[0]);
});
// And it visibly walks away from the spawn (not a static artifact).
const spawnedTraderMoves = traderIds.some((id) => {
  const s = traderSamples.get(id);
  return s.length >= 2 && inFaChunk(s[0]) && s.some((p) => Math.abs(p.x - s[0].x) + Math.abs(p.y - s[0].y) > 0.5);
});

const faSubscribed = subscribedAfterZoom.has(`${MARKET_FA_CHUNK.x},${MARKET_FA_CHUNK.y}`);
const fbSubscribed = subscribedAfterZoom.has(`${MARKET_FB_CHUNK.x},${MARKET_FB_CHUNK.y}`);

const checks = {
  page_loaded: receivedBinary.length > 0,
  got_chunk_deltas_per_tick: deltaCount > 0,
  market_fa_subscribed: faSubscribed, // F_A observed -> active
  market_fb_dormant: !fbSubscribed, // F_B dormant -> the import deficit
  trader_agent_present: traderIds.length > 0,
  trader_spawned_at_observed_fa: traderSpawnedAtFa,
  spawned_trader_moves: spawnedTraderMoves,
  no_text_frames: textFramesReceived === 0 && textFramesSent === 0,
  no_console_errors: consoleErrors.length === 0,
};

const summary = {
  status: Object.values(checks).every(Boolean) ? 'ok' : 'failed',
  url: URL,
  received_binary_frames: receivedBinary.length,
  delta_frames: deltaCount,
  subscribed_after_zoom: [...subscribedAfterZoom].sort(),
  trader_ids: traderIds,
  trader_first_samples: Object.fromEntries(traderIds.map((id) => [id, traderSamples.get(id)[0]])),
  trader_first_chunks: Object.fromEntries(traderIds.map((id) => [id, chunkOf(traderSamples.get(id)[0])])),
  checks,
  console_errors: consoleErrors,
};

console.log(JSON.stringify(summary, null, 2));
process.exit(summary.status === 'ok' ? 0 : 1);
