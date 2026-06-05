// Economy-markets browser smoke: verify the on-map economy feature is wired
// end-to-end over the real dev stack.
//
// Asserts (over the binary protobuf wire + canvas diagnostics):
// 1. Wire: at least one `economySnapshot` ServerMessage is received with >= 4 markets.
// 2. Glyph: after zooming out so market chunks near the origin are in view,
//    the `runtimeDiagnostics` field `economyMarketCount` >= 1 (a market glyph is in view).
// 3. Inspector: a synthetic click at a visible market's screen position selects the
//    market, confirmed by `selectedMarketCoord != null` in the diagnostics JSON.
//
// This is the acceptance gate for the frontend<->backend wire boundary for the
// on-map economy feature. "All unit tests pass" is NOT a substitute.
//
// Stack management:
//   The smoke starts its own isolated dev stack (backend on BACKEND_PORT, frontend
//   on FRONTEND_PORT) so it does not conflict with any other running dev server.
//   Set BACKEND_PORT and FRONTEND_PORT env vars to override (defaults: 8082, 5176).
//   Set CARGO_TARGET_DIR to use an isolated cargo output dir (default: /tmp/abutown-vtraders-target).
//   Set REUSE_STACK=1 to skip backend build + startup and connect to an already-
//   running stack at the configured ports (useful in CI where the stack is pre-started).

import { chromium } from '@playwright/test';
import { fromBinary } from '@bufbuild/protobuf';
import { tsImport } from 'tsx/esm/api';
import { spawn, execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { isAbsolute, resolve, join } from 'node:path';

const protoModule = await tsImport('../src/backend/proto/abutown_pb.ts', import.meta.url);
const { ServerMessageSchema } = protoModule;

const BACKEND_PORT = parseInt(process.env.BACKEND_PORT ?? '8082', 10);
const FRONTEND_PORT = parseInt(process.env.FRONTEND_PORT ?? '5176', 10);
const BACKEND_URL = `http://127.0.0.1:${BACKEND_PORT}`;
const FRONTEND_URL = `http://127.0.0.1:${FRONTEND_PORT}`;
const PAGE_TIMEOUT_MS = 20000;
const REUSE_STACK = process.env.REUSE_STACK === '1';

// Resolve the cargo target dir for the backend build.
const cargoTargetDir = (() => {
  const configured = process.env.CARGO_TARGET_DIR;
  if (configured) return isAbsolute(configured) ? configured : resolve(process.cwd(), configured);
  return '/tmp/abutown-vtraders-target';
})();

const viteBin = fileURLToPath(new URL('../node_modules/vite/bin/vite.js', import.meta.url));
const backendBinary = join(cargoTargetDir, 'debug', 'sim-server');
const backendHealthUrl = `${BACKEND_URL}/health`;

const killProcessGroup = process.platform !== 'win32';
let backendChild = null;
let frontendChild = null;
let shuttingDown = false;

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
  // 1. Build the backend.
  console.error(`[smoke] building backend (target: ${cargoTargetDir}) ...`);
  try {
    execSync(
      `CARGO_TARGET_DIR=${cargoTargetDir} TMPDIR=/tmp/abutown-vtraders-tmp ` +
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

  // 2. Start the backend.
  // Load .env for DATABASE_URL / SUPABASE_URL; the backend requires them.
  let backendEnv;
  try {
    const dotenvContent = execSync('cat .env 2>/dev/null || true', { cwd: process.cwd() }).toString();
    const envPairs = {};
    for (const line of dotenvContent.split('\n')) {
      const m = line.match(/^([A-Z_][A-Z0-9_]*)=(.*)$/);
      if (m) envPairs[m[1]] = m[2].replace(/^["']|["']$/g, '');
    }
    backendEnv = {
      ...process.env,
      ...envPairs,
      CARGO_TARGET_DIR: cargoTargetDir,
      LISTEN_PORT: String(BACKEND_PORT),
      RUST_LOG: process.env.RUST_LOG ?? 'error',
      // Explicitly allow this smoke's frontend origin, overriding .env.
      CORS_ALLOWED_ORIGINS: FRONTEND_URL,
    };
  } catch {
    backendEnv = {
      ...process.env,
      LISTEN_PORT: String(BACKEND_PORT),
      RUST_LOG: process.env.RUST_LOG ?? 'error',
      CORS_ALLOWED_ORIGINS: FRONTEND_URL,
      // Note: DATABASE_URL and SUPABASE_URL must be in process.env in this case.
    };
  }

  console.error(`[smoke] starting backend on port ${BACKEND_PORT} ...`);
  backendChild = spawn(backendBinary, [], {
    env: backendEnv,
    detached: killProcessGroup,
    stdio: 'pipe',
  });
  backendChild.stderr.on('data', () => {}); // suppress stderr

  try {
    await waitForHttpOk(backendHealthUrl, 30_000, 'backend');
  } catch (err) {
    console.log(JSON.stringify({ status: 'stack-failed', phase: 'backend-start', error: String(err) }, null, 2));
    shutdown(1);
    await pause(1000);
    process.exit(1);
  }
  console.error(`[smoke] backend healthy at ${BACKEND_URL}`);

  // 3. Start the vite preview server on FRONTEND_PORT with the correct backend URL.
  //    The existing dist/ is built with the default backend URL; we need to serve it
  //    through a Vite preview that transparently rewrites the backend base URL.
  //    Strategy: rebuild the frontend with the correct VITE_ABUTOWN_BACKEND_URL,
  //    then serve via vite preview. We keep the build in a temp dir so we do not
  //    disturb the main dist/ output.
  console.error(`[smoke] rebuilding frontend with VITE_ABUTOWN_BACKEND_URL=${BACKEND_URL} ...`);
  try {
    execSync(
      `VITE_ABUTOWN_BACKEND_URL=${BACKEND_URL} VITE_SKIP_PUBLIC_COPY=1 ` +
      `node ${viteBin} build --outDir /tmp/abutown-smoke-dist --emptyOutDir`,
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
    [viteBin, 'preview', '--host', '127.0.0.1', '--port', String(FRONTEND_PORT), '--outDir', '/tmp/abutown-smoke-dist'],
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

// Wait for the WS to open and the initial subscribe poll + economy snapshot to arrive.
await pause(3000);

// --- Zoom OUT so market chunks near the origin enter the viewport ---
// Seeded markets REF_A(2,3) and REF_B(13,3) are near tile origin.
// The default camera starts zoomed in; scroll OUT to see the full map including origin.
await page.mouse.move(640, 400);
for (let i = 0; i < 8; i += 1) {
  await page.mouse.wheel(0, 240); // positive deltaY = zoom out
  await pause(60);
}

// Let the subscription poll fire and the server send chunk data + economy snapshot.
await pause(2000);

// --- Read diagnostics to check economyMarketCount and get market screen positions ---
const diagRaw = await page.evaluate(() => window.render_game_to_text?.() ?? '');
let diag = null;
if (diagRaw) {
  try {
    diag = JSON.parse(diagRaw);
  } catch {
    // will report below
  }
}

const economyMarketCountAfterZoom = diag?.city?.economyMarketCount ?? 0;
const economyMarketsInView = (diag?.city?.economyMarkets ?? []).filter(
  (m) =>
    m.screen.x > 16 &&
    m.screen.x < 1264 &&
    m.screen.y > 16 &&
    m.screen.y < 784,
);

// --- Assertion 3: click a visible market to open inspector ---
let inspectorOpened = false;
let selectedMarketCoordAfterClick = null;

if (economyMarketsInView.length > 0) {
  const target = economyMarketsInView[0];
  // Click the market glyph screen position.
  await page.mouse.click(target.screen.x, target.screen.y);
  await pause(300);

  const diagAfterClickRaw = await page.evaluate(() => window.render_game_to_text?.() ?? '');
  let diagAfterClick = null;
  if (diagAfterClickRaw) {
    try {
      diagAfterClick = JSON.parse(diagAfterClickRaw);
    } catch {
      // will report as not opened
    }
  }
  selectedMarketCoordAfterClick = diagAfterClick?.city?.selectedMarketCoord ?? null;
  inspectorOpened = selectedMarketCoordAfterClick !== null;
}

await browser.close();

// --- Decode protobuf frames to verify wire-level economySnapshot ---
function decodeServer(bytes) {
  try {
    return fromBinary(ServerMessageSchema, bytes);
  } catch {
    return null;
  }
}

const messages = receivedBinary.map(decodeServer).filter(Boolean);

let economyFrameCount = 0;
let economyFrameMaxMarkets = 0;
let wireMarketIds = [];
for (const m of messages) {
  if (m.body.case === 'economySnapshot') {
    economyFrameCount += 1;
    const marketCount = m.body.value.markets?.length ?? 0;
    if (marketCount > economyFrameMaxMarkets) {
      economyFrameMaxMarkets = marketCount;
      wireMarketIds = (m.body.value.markets ?? []).map((mk) => mk.marketId);
    }
  }
}

const checks = {
  // Assertion 1: wire received >= 1 economySnapshot frames with >= 4 markets
  wire_economy_snapshot_received: economyFrameCount > 0,
  wire_economy_snapshot_has_4_markets: economyFrameMaxMarkets >= 4,
  // Assertion 2: glyph in view (after zoom-out, diagnostics show >= 1 market)
  economy_glyph_in_view: economyMarketCountAfterZoom >= 1,
  // Assertion 3: inspector opened on click
  inspector_opened_on_click: inspectorOpened,
  // Sanity checks
  no_text_frames: textFramesReceived === 0 && textFramesSent === 0,
  no_console_errors: consoleErrors.length === 0,
};

const summary = {
  status: Object.values(checks).every(Boolean) ? 'ok' : 'failed',
  frontend_url: FRONTEND_URL,
  backend_url: BACKEND_URL,
  received_binary_frames: receivedBinary.length,
  economy_frame_count: economyFrameCount,
  economy_frame_max_markets: economyFrameMaxMarkets,
  wire_market_ids: wireMarketIds,
  economy_market_count_after_zoom: economyMarketCountAfterZoom,
  economy_markets_in_view: economyMarketsInView.map((m) => ({ tileX: m.tileX, tileY: m.tileY, screen: m.screen })),
  selected_market_coord_after_click: selectedMarketCoordAfterClick,
  inspector_opened: inspectorOpened,
  checks,
  console_errors: consoleErrors,
};

console.log(JSON.stringify(summary, null, 2));
shutdown(summary.status === 'ok' ? 0 : 1);
// Give shutdown a moment to terminate child processes before exiting.
await pause(900);
process.exit(summary.status === 'ok' ? 0 : 1);
