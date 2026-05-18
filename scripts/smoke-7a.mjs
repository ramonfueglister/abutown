// Phase 7a browser smoke: open the frontend, pan/zoom, log every WS frame
// the client sends to the server. Report counts + sampled payloads.

import { chromium } from '@playwright/test';

const URL = 'http://127.0.0.1:5175';
const PAGE_TIMEOUT_MS = 15000;

const events = [];
function log(kind, payload) {
  events.push({ t: Date.now(), kind, payload });
}

function summarise(frames) {
  const out = {
    chunk_subscribe: { count: 0, totalCoords: 0, samples: [] },
    chunk_unsubscribe: { count: 0, totalCoords: 0, samples: [] },
    other: { count: 0, samples: [] },
  };
  for (const f of frames) {
    let parsed;
    try { parsed = JSON.parse(f); } catch { continue; }
    const type = parsed.type;
    if (type === 'chunk_subscribe' || type === 'chunk_unsubscribe') {
      const bucket = out[type];
      bucket.count += 1;
      bucket.totalCoords += parsed.coords?.length ?? 0;
      if (bucket.samples.length < 3) {
        bucket.samples.push({ count: parsed.coords?.length, first: parsed.coords?.[0], last: parsed.coords?.[parsed.coords?.length - 1] });
      }
    } else {
      out.other.count += 1;
      if (out.other.samples.length < 3) out.other.samples.push(parsed);
    }
  }
  return out;
}

async function pause(ms) { await new Promise((r) => setTimeout(r, ms)); }

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ viewport: { width: 1024, height: 768 } });
const page = await context.newPage();

const sentFrames = [];
const receivedFrames = [];

page.on('websocket', (ws) => {
  log('ws-open', ws.url());
  ws.on('framesent', (ev) => {
    if (typeof ev.payload === 'string') sentFrames.push(ev.payload);
  });
  ws.on('framereceived', (ev) => {
    if (typeof ev.payload === 'string') receivedFrames.push(ev.payload);
  });
  ws.on('close', () => log('ws-close', ws.url()));
});

page.on('console', (msg) => {
  if (msg.type() === 'error') log('console-error', msg.text());
});
page.on('pageerror', (err) => log('page-error', err.message));

try {
  await page.goto(URL, { waitUntil: 'load', timeout: PAGE_TIMEOUT_MS });
} catch (e) {
  console.log(JSON.stringify({ status: 'page-load-failed', error: String(e) }, null, 2));
  await browser.close();
  process.exit(1);
}

// Let the WS open and the initial subscribe fire (200 ms poll).
await pause(1500);

const initialSentCount = sentFrames.length;
const initialReceivedCount = receivedFrames.length;

// Pan the camera by dragging the canvas: middle of viewport → 300 px left.
const box = { x: 512, y: 384 };
await page.mouse.move(box.x, box.y);
await page.mouse.down();
for (let dx = 0; dx <= 300; dx += 30) {
  await page.mouse.move(box.x - dx, box.y);
  await pause(20);
}
await page.mouse.up();

// Allow several poll cycles after the pan completes.
await pause(1500);

const afterPanSentCount = sentFrames.length;

// Zoom out: wheel up several times.
for (let i = 0; i < 8; i++) {
  await page.mouse.wheel(0, 200);
  await pause(50);
}
await pause(1500);

const afterZoomSentCount = sentFrames.length;

// Stay still and confirm polling stops emitting messages.
const stillStart = sentFrames.length;
await pause(2000);
const stillEnd = sentFrames.length;

await browser.close();

const summary = {
  status: 'ok',
  url: URL,
  totals: {
    sent_frames: sentFrames.length,
    received_frames: receivedFrames.length,
  },
  phases: {
    initial_after_open_1500ms: { sent: initialSentCount, received: initialReceivedCount },
    after_pan: { sent_delta: afterPanSentCount - initialSentCount, total_sent: afterPanSentCount },
    after_zoom_out: { sent_delta: afterZoomSentCount - afterPanSentCount, total_sent: afterZoomSentCount },
    idle_2000ms: { sent_delta: stillEnd - stillStart },
  },
  sent_frame_breakdown: summarise(sentFrames),
  console_and_page_errors: events.filter((e) => e.kind === 'console-error' || e.kind === 'page-error'),
};

console.log(JSON.stringify(summary, null, 2));
