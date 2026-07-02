// Visible-traders browser smoke: verify the seeded economy trader is rendered as
// a real walking mobility agent and actually MOVES on the client.
//
// Asserts (over the binary protobuf wire, against the live dev stack):
// - An agent whose id starts `trader:` arrives in a chunk snapshot/delta.
// - That trader's world_coord CHANGES over a few seconds (it walks the route).
// - Per-tick mobility_chunk_delta frames flow.
// - Zero text frames; zero console errors.
//
// This is the acceptance gate for the frontend<->backend render boundary
// (the Phase-7a coord-mismatch lesson): unit tests cannot catch a spawn/chunk/
// coordinate/delta mistake that only shows up against a real browser client.

import { chromium } from '@playwright/test';
import { fromBinary } from '@bufbuild/protobuf';
import { tsImport } from 'tsx/esm/api';
const protoModule = await tsImport('../src/backend/proto/abutown_pb.ts', import.meta.url);
const { ServerMessageSchema } = protoModule;

const URL = 'http://127.0.0.1:5175';
const PAGE_TIMEOUT_MS = 15000;

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

page.on('websocket', (ws) => {
  if (!ws.url().includes(':8080/')) return; // backend WS only (skip vite HMR)
  ws.on('framesent', (ev) => {
    if (typeof ev.payload === 'string') textFramesSent += 1;
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

// Let the WS open + the initial subscribe poll fire.
await pause(2500);
// Zoom out so a wide area (including the market chunks near the origin) is
// subscribed regardless of the default camera center.
await page.mouse.move(512, 384);
for (let i = 0; i < 6; i += 1) {
  await page.mouse.wheel(0, 240);
  await pause(60);
}
// Observe for a few seconds so the trader visibly walks its route.
await pause(4500);

await browser.close();

function decodeServer(bytes) {
  try {
    return fromBinary(ServerMessageSchema, bytes);
  } catch {
    return null;
  }
}

const messages = receivedBinary.map(decodeServer).filter(Boolean);

// Collect every trader-agent sample (id starts `trader:`) from snapshots+deltas.
const traderSamples = new Map(); // id -> [{x, y}]
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
const traderMoved = traderIds.some((id) => {
  const s = traderSamples.get(id);
  if (s.length < 2) return false;
  const first = s[0];
  return s.some((p) => Math.abs(p.x - first.x) + Math.abs(p.y - first.y) > 0.5);
});

const checks = {
  page_loaded: receivedBinary.length > 0,
  got_chunk_deltas_per_tick: deltaCount > 0,
  trader_agent_present: traderIds.length > 0,
  trader_agent_moves: traderMoved,
  no_text_frames: textFramesReceived === 0 && textFramesSent === 0,
  no_console_errors: consoleErrors.length === 0,
};

const summary = {
  status: Object.values(checks).every(Boolean) ? 'ok' : 'failed',
  url: URL,
  received_frames: receivedBinary.length,
  delta_frames: deltaCount,
  trader_ids: traderIds,
  trader_sample_counts: Object.fromEntries(traderIds.map((id) => [id, traderSamples.get(id).length])),
  checks,
  console_errors: consoleErrors,
};

console.log(JSON.stringify(summary, null, 2));
process.exit(summary.status === 'ok' ? 0 : 1);
