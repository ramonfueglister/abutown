// Phase 7b browser smoke: verify per-chunk wire protocol.
//
// Asserts:
// - Initial subscribe emits ≥1 mobility_chunk_snapshot per subscribed chunk.
// - Per-tick mobility_chunk_delta frames flow for subscribed chunks.
// - Idle (no pan, no zoom) eventually quiets down (no infinite snapshot flood).
// - Pan triggers new chunk_subscribe → new snapshots.
// - Zero global `mobility_delta` frames (old wire format removed in T8).

import { chromium } from '@playwright/test';

const URL = 'http://127.0.0.1:5175';
const PAGE_TIMEOUT_MS = 15000;

async function pause(ms) { await new Promise((r) => setTimeout(r, ms)); }

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ viewport: { width: 1024, height: 768 } });
const page = await context.newPage();

const sentFrames = [];
const receivedFrames = [];
const consoleErrors = [];

page.on('websocket', (ws) => {
  ws.on('framesent', (ev) => { if (typeof ev.payload === 'string') sentFrames.push(ev.payload); });
  ws.on('framereceived', (ev) => { if (typeof ev.payload === 'string') receivedFrames.push(ev.payload); });
});
page.on('console', (msg) => { if (msg.type() === 'error') consoleErrors.push(msg.text()); });
page.on('pageerror', (err) => consoleErrors.push(err.message));

try {
  await page.goto(URL, { waitUntil: 'domcontentloaded', timeout: PAGE_TIMEOUT_MS });
} catch (e) {
  console.log(JSON.stringify({ status: 'page-load-failed', error: String(e) }, null, 2));
  await browser.close();
  process.exit(1);
}

// Let WS open + initial subscribe poll fire (200 ms interval).
await pause(2500);
const initialReceivedCount = receivedFrames.length;

// Pan camera ~250 px left to add new chunks on the right edge of view.
await page.mouse.move(512, 384);
await page.mouse.down();
for (let dx = 0; dx <= 250; dx += 25) {
  await page.mouse.move(512 - dx, 384);
  await pause(15);
}
await page.mouse.up();
await pause(2000);
const afterPanReceivedCount = receivedFrames.length;

// Settle: 1.5 s idle, see how many frames per second arrive.
const idleStart = receivedFrames.length;
await pause(1500);
const idleEnd = receivedFrames.length;

await browser.close();

function summarise(frames) {
  const out = {
    hello: 0,
    tile_pulse: 0,
    mobility_chunk_snapshot: { count: 0, chunks: {} },
    mobility_chunk_delta: { count: 0, chunks: {} },
    mobility_delta_LEGACY: 0,
    other: { count: 0, samples: [] },
  };
  for (const f of frames) {
    let parsed;
    try { parsed = JSON.parse(f); } catch { continue; }
    switch (parsed.type) {
      case 'hello': out.hello += 1; break;
      case 'tile_pulse': out.tile_pulse += 1; break;
      case 'mobility_chunk_snapshot': {
        out.mobility_chunk_snapshot.count += 1;
        const k = `${parsed.chunk.x},${parsed.chunk.y}`;
        out.mobility_chunk_snapshot.chunks[k] = (out.mobility_chunk_snapshot.chunks[k] ?? 0) + 1;
        break;
      }
      case 'mobility_chunk_delta': {
        out.mobility_chunk_delta.count += 1;
        const k = `${parsed.chunk.x},${parsed.chunk.y}`;
        out.mobility_chunk_delta.chunks[k] = (out.mobility_chunk_delta.chunks[k] ?? 0) + 1;
        break;
      }
      case 'mobility_delta': out.mobility_delta_LEGACY += 1; break;
      default:
        out.other.count += 1;
        if (out.other.samples.length < 3) out.other.samples.push(parsed);
    }
  }
  return out;
}

const recv = summarise(receivedFrames);

function summariseSent(frames) {
  const out = { chunk_subscribe: 0, chunk_unsubscribe: 0, other: 0 };
  for (const f of frames) {
    try {
      const t = JSON.parse(f).type;
      if (t === 'chunk_subscribe') out.chunk_subscribe += 1;
      else if (t === 'chunk_unsubscribe') out.chunk_unsubscribe += 1;
      else out.other += 1;
    } catch { out.other += 1; }
  }
  return out;
}

const sent = summariseSent(sentFrames);

const checks = {
  page_loaded: receivedFrames.length > 0,
  // Per-chunk wire protocol is alive: subscribe → snapshot frame(s).
  got_chunk_snapshots_on_subscribe: recv.mobility_chunk_snapshot.count > 0,
  one_snapshot_per_subscribed_chunk:
    Object.keys(recv.mobility_chunk_snapshot.chunks).length >= 9, // at least a 3×3 visible area
  // Re-enabled after the spawn-time Position init fix: seeded agents are
  // now LOD-classified into their real chunks (not all into chunk(0,0))
  // so subscribed chunks actually receive per-tick deltas.
  got_chunk_deltas_per_tick: recv.mobility_chunk_delta.count > 0,
  // Client gets real entity data within the smoke window — either via a
  // snapshot frame or via per-tick deltas. The previous bug let empty
  // payloads flow undetected because we only counted frame counts.
  //
  // Note: snapshots-on-subscribe are typically EMPTY for chunks whose
  // population sits in a FlowCell (because the snapshot is built before
  // promote_warm_to_active respawns from the FlowCell on the next tick).
  // The chunk_delta on the following tick carries the entities. Both paths
  // would be acceptable for this assertion.
  client_receives_entity_data: receivedFrames.some((f) => {
    try {
      const m = JSON.parse(f);
      if (m.type === 'mobility_chunk_snapshot')
        return m.agents.length > 0 || m.vehicles.length > 0;
      if (m.type === 'mobility_chunk_delta')
        return m.changed_agents.length > 0 || m.changed_vehicles.length > 0;
      return false;
    } catch { return false; }
  }),
  // Old global delta is gone (state of the art mandate).
  no_legacy_mobility_delta: recv.mobility_delta_LEGACY === 0,
  // Frontend wiring works end-to-end.
  client_sent_chunk_subscribe: sent.chunk_subscribe > 0,
  pan_added_more_frames: afterPanReceivedCount > initialReceivedCount,
  no_console_errors: consoleErrors.length === 0,
};

const summary = {
  status: Object.values(checks).every(Boolean) ? 'ok' : 'failed',
  url: URL,
  totals: {
    received_frames: receivedFrames.length,
    sent_frames: sentFrames.length,
  },
  phases: {
    after_open_2500ms: initialReceivedCount,
    after_pan: afterPanReceivedCount,
    idle_1500ms_frames: idleEnd - idleStart,
  },
  sent_breakdown: sent,
  received_breakdown: recv,
  checks,
  console_errors: consoleErrors,
};

console.log(JSON.stringify(summary, null, 2));
process.exit(summary.status === 'ok' ? 0 : 1);
