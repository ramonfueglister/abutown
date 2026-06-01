// Flow-trader browser smoke: verify that a dormant cross-market flow produces a
// visible `trader:` agent transiting the observed chunk (3,1) along the dormant
// route (F_A at chunk (0,1) → F_B at chunk (6,1), y ≈ row 1 tiles 32-63).
//
// Strategy:
//   1. Launch headless Chromium against the dev stack.
//   2. Zoom IN toward CAMERA_MAX_SCALE and pan to center the viewport on the
//      transit-chunk neighborhood (tile ≈ 112, 48).  This keeps market chunks
//      (0,1) and (6,1) well outside the subscribed rectangle, so those markets
//      stay dormant and the macro flow fires every interval.
//   3. Collect subscription messages to assert (0,1) and (6,1) are NOT
//      subscribed (so markets are dormant), and that the transit chunk (3,1)
//      IS subscribed.
//   4. Observe for ~12 s (≥120 ticks at 100 ms/tick), spanning several
//      flow-interval cycles (interval=10 ticks, travel_ticks ≈ 48 ticks for
//      6-chunk-wide route at ~4 tiles/tick).  Assert at least one `trader:`
//      agent appears in snapshot/delta frames and its world_coord changes
//      (i.e., it visibly walks along the route).
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

// Geometry constants (must match seed.rs + main.ts).
const TILE_PX = 18;          // MINIMAL_MAP_TILE_SIZE width/height
const CHUNK_SIZE = 32;       // tiles per chunk edge

// Transit chunk (3,1) center tile: x = 3*32 + 16 = 112, y = 1*32 + 16 = 48.
// Market F_A at chunk (0,1): node near tile (16, 48).
// Market F_B at chunk (6,1): node near tile (208, 48).
const TRANSIT_TILE_X = 112;
const TRANSIT_TILE_Y = 48;

// Market chunk coords that MUST stay dormant (not subscribed).
const MARKET_FA_CHUNK = { x: 0, y: 1 };
const MARKET_FB_CHUNK = { x: 6, y: 1 };

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

// Track every chunk_subscribe coord set we send to the server.
// key = "x,y", value = last subscription seen.
const subscribedChunks = new Set();

page.on('websocket', (ws) => {
  if (!ws.url().includes(':8080/')) return; // backend WS only (skip vite HMR)
  ws.on('framesent', (ev) => {
    if (typeof ev.payload === 'string') {
      textFramesSent += 1;
      return;
    }
    // Binary sent frames are client→server protobuf ClientMessage (chunkSubscribe /
    // chunkUnsubscribe).  Decode and track subscribed chunks.
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

// Wait for WS to open and the initial subscribe poll to fire.
await pause(2500);

// --- Zoom IN toward CAMERA_MAX_SCALE ---
// Negative deltaY = scroll up = zoom in.  Each step multiplies scale by
// exp(240 * 0.0015) ≈ 1.43.  Starting at ≈0.32 we reach ~2.8 in 6 steps.
// Centre the zoom on the transit-tile screen position so the camera tracks it.
await page.mouse.move(512, 384);
for (let i = 0; i < 6; i += 1) {
  await page.mouse.wheel(0, -240);
  await pause(80);
}

// --- Pan to re-center the viewport on the transit chunk row (y≈48 tiles). ---
// At max scale 2.8, the transit chunk center (tile 112, 48) is at world pixel
// (112*18+9, 48*18+9) = (2025, 873).  On screen: ≈ (512, 384) after zoom-at-
// centre, but a small pan corrects any residual offset to centre y=48.
// We drag slowly to avoid overshooting; the poll loop will resend subscription.
await page.mouse.move(512, 384);
await page.mouse.down();
// Pan right and down slightly to centre on row y=1 tile-row (chunk row 1).
// The world starts at y=0; chunk row 1 is tiles 32-63, centre ≈ tile 48.
// At scale ≈2.8 each tile is 2.8*18 ≈ 50 px.  A correction of ~(0, +10 tiles)
// is handled by dragging up ≈500 px, but a gentle drift is enough.
for (let dy = 0; dy <= 80; dy += 10) {
  await page.mouse.move(512, 384 - dy);
  await pause(30);
}
await page.mouse.up();

// Let the subscription poll settle (200 ms poll interval, one full cycle).
await pause(600);

// Snapshot the subscription state immediately before the long observation.
const subscribedAfterZoom = new Set(subscribedChunks);

// --- Long observation window: ≥120 ticks at 100 ms/tick ≈ 12 s. ---
// flow_interval=10 ticks, travel_ticks≈48.  Within 120 ticks we expect several
// flow fires and at least one shipment to transit the observed chunk (3,1).
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

// --- Collect trader-agent samples (id starts `trader:`) from snapshots+deltas ---
const traderSamples = new Map(); // id -> [{x, y, tick_approx}]
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

const traderIds = [...traderSamples.keys()];

// A trader "moves" if any two samples differ by > 0.5 tiles in Manhattan dist.
const traderMoved = traderIds.some((id) => {
  const samples = traderSamples.get(id);
  if (samples.length < 2) return false;
  const first = samples[0];
  return samples.some((p) => Math.abs(p.x - first.x) + Math.abs(p.y - first.y) > 0.5);
});

// --- Subscription assertions ---
const transitChunkKey = `${TRANSIT_TILE_X / CHUNK_SIZE | 0},${TRANSIT_TILE_Y / CHUNK_SIZE | 0}`;
// chunk (3,1):
const transitSubscribed = subscribedAfterZoom.has('3,1');
// Market chunks must NOT be subscribed (they must stay dormant):
const marketFaSubscribed = subscribedAfterZoom.has(`${MARKET_FA_CHUNK.x},${MARKET_FA_CHUNK.y}`);
const marketFbSubscribed = subscribedAfterZoom.has(`${MARKET_FB_CHUNK.x},${MARKET_FB_CHUNK.y}`);

const checks = {
  page_loaded: receivedBinary.length > 0,
  got_chunk_deltas_per_tick: deltaCount > 0,
  transit_chunk_subscribed: transitSubscribed,
  market_fa_not_subscribed: !marketFaSubscribed,
  market_fb_not_subscribed: !marketFbSubscribed,
  trader_agent_present: traderIds.length > 0,
  trader_agent_moves: traderMoved,
  no_text_frames: textFramesReceived === 0 && textFramesSent === 0,
  no_console_errors: consoleErrors.length === 0,
};

const summary = {
  status: Object.values(checks).every(Boolean) ? 'ok' : 'failed',
  url: URL,
  received_binary_frames: receivedBinary.length,
  delta_frames: deltaCount,
  subscribed_after_zoom: [...subscribedAfterZoom].sort(),
  transit_subscribed: transitSubscribed,
  market_fa_subscribed: marketFaSubscribed,
  market_fb_subscribed: marketFbSubscribed,
  trader_ids: traderIds,
  trader_sample_counts: Object.fromEntries(traderIds.map((id) => [id, traderSamples.get(id).length])),
  checks,
  console_errors: consoleErrors,
};

console.log(JSON.stringify(summary, null, 2));
process.exit(summary.status === 'ok' ? 0 : 1);
