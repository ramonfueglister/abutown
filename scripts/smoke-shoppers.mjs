// Shoppers browser smoke: verify that the demand-side shopper agents render
// and walk toward the observed market m_b (REF_B, tile (13,3) → chunk (0,0))
// when it has unmet FOOD demand.
//
// Strategy:
//   1. Launch headless Chromium against the dev stack.
//   2. ZOOM OUT (positive deltaY, like smoke-visible-traders) so the subscription
//      rectangle covers a wide area including chunk (0,0) where m_b lives.
//      m_b is thereby OBSERVED, its FOOD demand is unmet, and shopper agents
//      are spawned by the simulation.
//   3. Observe for ~12 s (≥120 ticks at 100 ms/tick) to give shoppers time to
//      spawn and walk.  Collect every agent whose id starts `shopper:` from
//      decoded mobility snapshot/delta frames; track world_coord over time.
//   4. Assert:
//        - page_loaded        (binary WS frames received)
//        - shopper_agent_present  (≥1 `shopper:` id seen in snapshot/delta)
//        - shopper_agent_moves    (≥1 shopper's world_coord changes > 0.5 tiles)
//        - market_chunk_subscribed  (chunk (0,0) is in the sent subscribe set)
//        - no_text_frames     (binary WS only)
//        - no_console_errors
//
// This is the acceptance gate for the frontend↔backend render boundary per
// CLAUDE.md: unit tests cannot catch a coordinate/chunk/projection mistake that
// only shows up against a real browser client.

import { chromium } from '@playwright/test';
import { fromBinary } from '@bufbuild/protobuf';
import { tsImport } from 'tsx/esm/api';
const protoModule = await tsImport('../src/backend/proto/abutown_pb.ts', import.meta.url);
const { ServerMessageSchema, ClientMessageSchema } = protoModule;

const URL = 'http://127.0.0.1:5175';
const PAGE_TIMEOUT_MS = 20000;

// m_b (REF_B) lives at tile (13,3) → chunk (0,0).
const MARKET_MB_CHUNK = { x: 0, y: 0 };

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

// Track every chunk_subscribe / chunk_unsubscribe the client sends so we can
// assert that chunk (0,0) (where m_b lives) is indeed subscribed after zoom-out.
const subscribedChunks = new Set();

page.on('websocket', (ws) => {
  if (!ws.url().includes(':8080/')) return; // backend WS only (skip vite HMR)
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
      for (const c of msg.body.value.coords) {
        subscribedChunks.add(`${c.x},${c.y}`);
      }
    } else if (msg.body.case === 'chunkUnsubscribe') {
      for (const c of msg.body.value.coords) {
        subscribedChunks.delete(`${c.x},${c.y}`);
      }
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

// Wait for the WS to open and the initial subscribe poll to fire.
await pause(2500);

// --- Zoom OUT so chunk (0,0) is within the subscription rectangle ---
// Positive deltaY = scroll down = zoom out (matches smoke-visible-traders).
// 6 steps × deltaY=240, centred on viewport centre, widens the subscribed
// rectangle enough to include chunk (0,0) regardless of the default camera.
await page.mouse.move(512, 384);
for (let i = 0; i < 6; i += 1) {
  await page.mouse.wheel(0, 240);
  await pause(60);
}

// Let the subscription poll settle (one full 200 ms poll cycle).
await pause(600);

// Snapshot the subscription set right after zoom-out, before we start observing.
const subscribedAfterZoom = new Set(subscribedChunks);

// --- Long observation window: ≥120 ticks (100 ms/tick) ≈ 12 s ---
// Shoppers are ephemeral agents; within this window at least one should spawn,
// appear in a snapshot/delta, and walk (world_coord changes > 0 tiles).
await pause(12000);

await browser.close();

// --- Decode protobuf frames ---
function decodeServer(bytes) {
  try {
    return fromBinary(ServerMessageSchema, bytes);
  } catch {
    return null;
  }
}

const messages = receivedBinary.map(decodeServer).filter(Boolean);

// --- Collect shopper-agent samples (id starts `shopper:`) from snapshots+deltas ---
const shopperSamples = new Map(); // id -> [{x, y}]
let deltaCount = 0;

const collect = (agents) => {
  for (const a of agents) {
    if (typeof a.id !== 'string' || !a.id.startsWith('shopper:')) continue;
    const arr = shopperSamples.get(a.id) ?? [];
    arr.push({ x: a.worldCoord?.x ?? 0, y: a.worldCoord?.y ?? 0 });
    shopperSamples.set(a.id, arr);
  }
};

for (const m of messages) {
  if (m.body.case === 'mobilityChunkSnapshot') collect(m.body.value.agents);
  if (m.body.case === 'mobilityChunkDelta') {
    deltaCount += 1;
    collect(m.body.value.changedAgents);
  }
}

const shopperIds = [...shopperSamples.keys()];

// A shopper "moves" if any two samples differ by > 0.5 tiles in Manhattan dist.
const shopperMoved = shopperIds.some((id) => {
  const samples = shopperSamples.get(id);
  if (samples.length < 2) return false;
  const first = samples[0];
  return samples.some((p) => Math.abs(p.x - first.x) + Math.abs(p.y - first.y) > 0.5);
});

// Assert chunk (0,0) — where m_b lives — was subscribed after the zoom-out.
const marketChunkSubscribed = subscribedAfterZoom.has(
  `${MARKET_MB_CHUNK.x},${MARKET_MB_CHUNK.y}`,
);

const checks = {
  page_loaded: receivedBinary.length > 0,
  got_chunk_deltas_per_tick: deltaCount > 0,
  market_chunk_subscribed: marketChunkSubscribed,
  shopper_agent_present: shopperIds.length > 0,
  shopper_agent_moves: shopperMoved,
  no_text_frames: textFramesReceived === 0 && textFramesSent === 0,
  no_console_errors: consoleErrors.length === 0,
};

const summary = {
  status: Object.values(checks).every(Boolean) ? 'ok' : 'failed',
  url: URL,
  received_binary_frames: receivedBinary.length,
  delta_frames: deltaCount,
  subscribed_after_zoom: [...subscribedAfterZoom].sort(),
  market_chunk_subscribed: marketChunkSubscribed,
  shopper_ids: shopperIds,
  shopper_sample_counts: Object.fromEntries(
    shopperIds.map((id) => [id, shopperSamples.get(id).length]),
  ),
  checks,
  console_errors: consoleErrors,
};

console.log(JSON.stringify(summary, null, 2));
process.exit(summary.status === 'ok' ? 0 : 1);
