// Schematic-renderer browser smoke: verify the schematic map feature is wired
// end-to-end over the real dev stack.
//
// This is the MANDATORY acceptance gate (CLAUDE.md) for the schematic renderer,
// which crosses the frontend<->backend wire boundary (EconomySnapshot.flows over
// the binary protobuf protocol, then projected/drawn on the canvas). "All unit
// tests pass" is NOT a substitute.
//
// Asserts (over the binary protobuf wire + canvas diagnostics):
// 1. WIRE — flows arrive: at least one `economySnapshot` ServerMessage frame is
//    received with `flows.length > 0`. Because the macro-flow engine ships goods
//    only after some interval ticks, we POLL generously (up to ~90s) re-reading
//    frames until a flows-bearing frame appears.
// 2. ECONOMY VIEW (zoomed out): after zooming out into the economy band, the
//    diagnostics JSON shows `economyFlowCount >= 1` (flow curves drawn last frame,
//    Task 13 wired this to flowsDrawnLastFrame()) AND `economyMarketCount >= 1`.
//    Screenshot saved to smoke-schematic-economy.png.
// 3. CITY VIEW (zoomed in): after zooming back in into the city band, the rendered
//    agent population is > 0 (diagnostics `city.mobility.agents`). Screenshot saved
//    to smoke-schematic-city.png.
// 4. Sanity: no console errors, no text WS frames.
//
// Stack management:
//   The smoke starts its own isolated dev stack (backend on BACKEND_PORT, frontend
//   on FRONTEND_PORT) against the LOCAL postgres (NOT the remote Supabase URL).
//   Set BACKEND_PORT / FRONTEND_PORT to override (defaults: 8083, 5177).
//   The cargo target defaults to the worktree's already-built `backend/target` so
//   the backend build is a fast incremental, not a multi-GB rebuild.
//   Set REUSE_STACK=1 to skip backend build + startup and connect to an already-
//   running stack at the configured ports.

import { chromium } from '@playwright/test';
import { fromBinary } from '@bufbuild/protobuf';
import { tsImport } from 'tsx/esm/api';
import { spawn, execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { isAbsolute, resolve, join } from 'node:path';

const protoModule = await tsImport('../src/backend/proto/abutown_pb.ts', import.meta.url);
const { ServerMessageSchema } = protoModule;

const BACKEND_PORT = parseInt(process.env.BACKEND_PORT ?? '8083', 10);
const FRONTEND_PORT = parseInt(process.env.FRONTEND_PORT ?? '5177', 10);
const BACKEND_URL = `http://127.0.0.1:${BACKEND_PORT}`;
const FRONTEND_URL = `http://127.0.0.1:${FRONTEND_PORT}`;
const PAGE_TIMEOUT_MS = 20000;
const REUSE_STACK = process.env.REUSE_STACK === '1';

// Local postgres — do NOT use the remote Supabase URL (slow, shared/prod).
const LOCAL_DATABASE_URL =
  process.env.SMOKE_DATABASE_URL ??
  'postgresql://ramonfuglister@127.0.0.1:5432/abutown';

// How long to keep the page open polling for a flows-bearing frame.
const FLOW_POLL_TIMEOUT_MS = parseInt(process.env.FLOW_POLL_TIMEOUT_MS ?? '90000', 10);

// Resolve the cargo target dir for the backend build. Default to the worktree's
// already-built backend/target to avoid a multi-GB rebuild.
const cargoTargetDir = (() => {
  const configured = process.env.CARGO_TARGET_DIR;
  if (configured) return isAbsolute(configured) ? configured : resolve(process.cwd(), configured);
  return resolve(process.cwd(), 'backend/target');
})();

const ECONOMY_SCREENSHOT = resolve(process.cwd(), 'smoke-schematic-economy.png');
const CITY_SCREENSHOT = resolve(process.cwd(), 'smoke-schematic-city.png');

const viteBin = fileURLToPath(new URL('../node_modules/vite/bin/vite.js', import.meta.url));
const backendBinary = join(cargoTargetDir, 'debug', 'sim-server');
const backendHealthUrl = `${BACKEND_URL}/health`;

const killProcessGroup = process.platform !== 'win32';
let backendChild = null;
let frontendChild = null;
let shuttingDown = false;

// Capture backend stderr so we can report flow-engine liveness if flows never ship.
const backendLogLines = [];
function recordBackendLog(buf) {
  const text = buf.toString();
  for (const line of text.split('\n')) {
    if (line.trim().length > 0) backendLogLines.push(line);
  }
  // Keep only the last 400 lines to bound memory.
  if (backendLogLines.length > 400) backendLogLines.splice(0, backendLogLines.length - 400);
}

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
  // 1. Build the backend (incremental against the worktree's target).
  console.error(`[smoke] building backend (target: ${cargoTargetDir}) ...`);
  try {
    execSync(
      `CARGO_TARGET_DIR=${cargoTargetDir} ` +
      `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server`,
      { stdio: 'inherit', cwd: process.cwd() },
    );
  } catch (err) {
    console.log(JSON.stringify({
      status: 'stack-failed',
      phase: 'backend-build',
      error: String(err),
    }, null, 2));
    process.exit(1);
  }

  // 2. Start the backend against LOCAL postgres.
  //    Explicitly override DATABASE_URL to the local PG; clear SUPABASE_URL /
  //    PGSSLROOTCERT and any VITE_* / supabase vars so the backend uses plain
  //    local PG (no remote, no TLS). Keep CORS for this smoke's frontend origin.
  const sanitizedEnv = { ...process.env };
  for (const key of Object.keys(sanitizedEnv)) {
    if (key.startsWith('VITE_') || key.startsWith('SUPABASE')) {
      sanitizedEnv[key] = '';
    }
  }
  const backendEnv = {
    ...sanitizedEnv,
    CARGO_TARGET_DIR: cargoTargetDir,
    LISTEN_PORT: String(BACKEND_PORT),
    DATABASE_URL: LOCAL_DATABASE_URL,
    SUPABASE_URL: '',
    PGSSLROOTCERT: '',
    RUST_LOG: process.env.RUST_LOG ?? 'info',
    // Explicitly allow this smoke's frontend origin.
    CORS_ALLOWED_ORIGINS: FRONTEND_URL,
  };

  console.error(`[smoke] starting backend on port ${BACKEND_PORT} (local PG) ...`);
  backendChild = spawn(backendBinary, [], {
    env: backendEnv,
    detached: killProcessGroup,
    stdio: 'pipe',
  });
  backendChild.stderr.on('data', recordBackendLog);
  backendChild.stdout.on('data', recordBackendLog);

  try {
    await waitForHttpOk(backendHealthUrl, 30_000, 'backend');
  } catch (err) {
    console.log(JSON.stringify({
      status: 'stack-failed',
      phase: 'backend-start',
      error: String(err),
      backend_log_tail: backendLogLines.slice(-40),
    }, null, 2));
    shutdown(1);
    await pause(1000);
    process.exit(1);
  }
  console.error(`[smoke] backend healthy at ${BACKEND_URL}`);

  // 3. Rebuild the frontend with the correct backend URL, serve via vite preview.
  console.error(`[smoke] rebuilding frontend with VITE_ABUTOWN_BACKEND_URL=${BACKEND_URL} ...`);
  try {
    execSync(
      `VITE_ABUTOWN_BACKEND_URL=${BACKEND_URL} VITE_SKIP_PUBLIC_COPY=1 ` +
      `node ${viteBin} build --outDir /tmp/abutown-schematic-smoke-dist --emptyOutDir`,
      { stdio: 'pipe', cwd: process.cwd() },
    );
  } catch (err) {
    console.log(JSON.stringify({ status: 'stack-failed', phase: 'frontend-build', error: String(err) }, null, 2));
    shutdown(1);
    await pause(1000);
    process.exit(1);
  }

  console.error(`[smoke] starting frontend preview on port ${FRONTEND_PORT} ...`);
  frontendChild = spawn(
    process.execPath,
    [viteBin, 'preview', '--host', '127.0.0.1', '--port', String(FRONTEND_PORT), '--outDir', '/tmp/abutown-schematic-smoke-dist'],
    { cwd: process.cwd(), env: process.env, detached: killProcessGroup, stdio: 'pipe' },
  );
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
const context2 = await browser.newContext({ viewport: { width: 1280, height: 800 } });
const page = await context2.newPage();

const receivedBinary = [];
let textFramesReceived = 0;
let textFramesSent = 0;
const consoleErrors = [];

page.on('websocket', (ws) => {
  if (!ws.url().includes(`:${BACKEND_PORT}/`)) return; // backend WS only (skip vite HMR)
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
  await page.goto(FRONTEND_URL, { waitUntil: 'domcontentloaded', timeout: PAGE_TIMEOUT_MS });
} catch (e) {
  console.log(JSON.stringify({ status: 'page-load-failed', error: String(e) }, null, 2));
  await browser.close();
  shutdown(1);
  await pause(1000);
  process.exit(1);
}

// Wait for the WS to open and the initial subscribe poll to arrive.
await pause(3000);

// --- Helper: decode protobuf frames and extract the max flows-bearing snapshot ---
function decodeServer(bytes) {
  try {
    return fromBinary(ServerMessageSchema, bytes);
  } catch {
    return null;
  }
}

function scanFlows() {
  let economyFrameCount = 0;
  let maxFlows = 0;
  let sampleFlow = null;
  for (const bytes of receivedBinary) {
    const m = decodeServer(bytes);
    if (!m || m.body.case !== 'economySnapshot') continue;
    economyFrameCount += 1;
    const flows = m.body.value.flows ?? [];
    if (flows.length > maxFlows) {
      maxFlows = flows.length;
      const f = flows[0];
      sampleFlow = {
        srcMarketId: f.srcMarketId,
        dstMarketId: f.dstMarketId,
        goodId: f.goodId,
        rate: typeof f.rate === 'bigint' ? Number(f.rate) : f.rate,
      };
    }
  }
  return { economyFrameCount, maxFlows, sampleFlow };
}

// --- Zoom OUT into the economy band so flow curves + market glyphs are drawn ---
// Flow-demo markets 9003 (16,48) and 9004 (208,48) span the map; zooming out
// brings both into view and crosses into the semantic-zoom economy band.
await page.mouse.move(640, 400);
for (let i = 0; i < 10; i += 1) {
  await page.mouse.wheel(0, 240); // positive deltaY = zoom out
  await pause(60);
}

// --- Assertion 1: POLL for a flows-bearing frame (macro-flow needs interval ticks) ---
let flowScan = scanFlows();
const pollStart = Date.now();
while (flowScan.maxFlows === 0 && Date.now() - pollStart < FLOW_POLL_TIMEOUT_MS) {
  await pause(2000);
  flowScan = scanFlows();
}
const elapsedFlowPollMs = Date.now() - pollStart;

// Let the renderer draw a couple of frames with the flows present, then sample diagnostics.
await pause(1500);

// --- Assertion 2: economy-band diagnostics (flow curves + markets drawn) ---
const economyDiagRaw = await page.evaluate(() => window.render_game_to_text?.() ?? '');
let economyDiag = null;
if (economyDiagRaw) {
  try {
    economyDiag = JSON.parse(economyDiagRaw);
  } catch {
    // will report below
  }
}
const economyFlowCount = economyDiag?.city?.economyFlowCount ?? 0;
const economyMarketCount = economyDiag?.city?.economyMarketCount ?? 0;

await page.screenshot({ path: ECONOMY_SCREENSHOT });

// --- Assertion 3: zoom back IN into the city band, assert rendered agents > 0 ---
await page.mouse.move(640, 400);
for (let i = 0; i < 14; i += 1) {
  await page.mouse.wheel(0, -240); // negative deltaY = zoom in
  await pause(60);
}
await pause(3000);

const cityDiagRaw = await page.evaluate(() => window.render_game_to_text?.() ?? '');
let cityDiag = null;
if (cityDiagRaw) {
  try {
    cityDiag = JSON.parse(cityDiagRaw);
  } catch {
    // will report below
  }
}
// Agent population rendered from the backend mobility stream.
const cityAgentCount = cityDiag?.city?.mobility?.agents ?? 0;
const cityPedestrians = cityDiag?.city?.pedestrians ?? 0;

await page.screenshot({ path: CITY_SCREENSHOT });

await browser.close();

// Final flow scan (in case the last frames carried flows).
flowScan = scanFlows();

const checks = {
  // Assertion 1: wire delivered at least one flow.
  wire_flows_received: flowScan.maxFlows > 0,
  // Assertion 2: economy view drew flow curves + market glyphs.
  economy_flow_count_ge_1: economyFlowCount >= 1,
  economy_market_count_ge_1: economyMarketCount >= 1,
  // Assertion 3: city view rendered the agent population.
  city_agents_rendered: cityAgentCount > 0,
  // Sanity checks.
  no_text_frames: textFramesReceived === 0 && textFramesSent === 0,
  no_console_errors: consoleErrors.length === 0,
};

const summary = {
  status: Object.values(checks).every(Boolean) ? 'ok' : 'failed',
  frontend_url: FRONTEND_URL,
  backend_url: BACKEND_URL,
  database_url: LOCAL_DATABASE_URL,
  received_binary_frames: receivedBinary.length,
  economy_frame_count: flowScan.economyFrameCount,
  max_flows_seen: flowScan.maxFlows,
  sample_flow: flowScan.sampleFlow,
  flow_poll_elapsed_ms: elapsedFlowPollMs,
  economy_flow_count_at_economy_zoom: economyFlowCount,
  economy_market_count_at_economy_zoom: economyMarketCount,
  city_agent_count_at_city_zoom: cityAgentCount,
  city_pedestrians_at_city_zoom: cityPedestrians,
  economy_screenshot: ECONOMY_SCREENSHOT,
  city_screenshot: CITY_SCREENSHOT,
  checks,
  console_errors: consoleErrors,
};

// If flows never arrived, attach a backend log tail to aid the diagnosis.
if (flowScan.maxFlows === 0) {
  summary.backend_log_tail = backendLogLines.slice(-60);
}

console.log(JSON.stringify(summary, null, 2));
shutdown(summary.status === 'ok' ? 0 : 1);
// Give shutdown a moment to terminate child processes before exiting.
await pause(900);
process.exit(summary.status === 'ok' ? 0 : 1);
