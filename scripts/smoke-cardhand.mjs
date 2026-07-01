// Card-hand-only browser smoke: verify the stripped app renders ONLY the
// card-hand shell + Supabase login, signed out, and issues NO requests to
// any removed sim route (world/base-world/chunks/mobility/economy/commands)
// or the /ws websocket. Adapted from scripts/smoke-7a.mjs's headless-chromium
// harness, replacing the WS-frame assertions with DOM + request-URL checks.
//
// Spawns `npm run dev` itself (with dummy Supabase env so
// mountCardHandView() doesn't bail out early), polls the port until ready,
// runs the checks, then tears the dev server down — so this script is
// runnable standalone: `node scripts/smoke-cardhand.mjs`.

import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';

const HOST = '127.0.0.1';
const PORT = 5175;
const TARGET_URL = `http://${HOST}:${PORT}`;
const PAGE_TIMEOUT_MS = 15000;
const SERVER_READY_TIMEOUT_MS = 30000;

const SIM_ROUTE_PATTERNS = [/\/world\b/, /\/base-world\b/, /\/chunks\//, /\/mobility\b/, /\/economy\b/, /\/commands\b/];

function fail(reason) {
  console.error(`SMOKE FAIL: ${reason}`);
  process.exitCode = 1;
}

async function pause(ms) {
  await new Promise((r) => setTimeout(r, ms));
}

function portOpen(host, port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host, port }, () => {
      socket.end();
      resolve(true);
    });
    socket.on('error', () => resolve(false));
    socket.setTimeout(1000, () => {
      socket.destroy();
      resolve(false);
    });
  });
}

async function waitForPort(host, port, timeoutMs) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await portOpen(host, port)) return true;
    await pause(200);
  }
  return false;
}

async function main() {
  console.log(`Spawning dev server (npm run dev) on ${TARGET_URL} with dummy Supabase env...`);
  const devServer = spawn('npm', ['run', 'dev'], {
    cwd: new URL('..', import.meta.url).pathname,
    env: {
      ...process.env,
      VITE_SUPABASE_URL: 'https://dummy.supabase.co',
      VITE_SUPABASE_PUBLISHABLE_KEY: 'sb_publishable_dummy',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
    // Own process group so cleanup can kill the whole tree (npm → vite → esbuild).
    // A plain SIGTERM to `npm` leaves the `vite` grandchild alive, whose open
    // pipes keep this script's event loop from ever exiting (CI hangs forever).
    detached: true,
  });

  let devServerOutput = '';
  devServer.stdout.on('data', (d) => {
    devServerOutput += d.toString();
  });
  devServer.stderr.on('data', (d) => {
    devServerOutput += d.toString();
  });

  let cleanedUp = false;
  const cleanup = () => {
    if (cleanedUp) return;
    cleanedUp = true;
    // SIGKILL the whole process group (negative pid) so the detached
    // npm → vite → esbuild tree dies, not just the npm wrapper.
    if (devServer.pid) {
      try {
        process.kill(-devServer.pid, 'SIGKILL');
      } catch {
        /* group already gone */
      }
    }
    try {
      devServer.kill('SIGKILL');
    } catch {
      /* already dead */
    }
  };
  process.on('exit', cleanup);

  try {
    const ready = await waitForPort(HOST, PORT, SERVER_READY_TIMEOUT_MS);
    if (!ready) {
      fail(`dev server did not open ${HOST}:${PORT} within ${SERVER_READY_TIMEOUT_MS}ms.\n--- dev server output ---\n${devServerOutput}`);
      return;
    }
    // Give vite a brief extra moment past the raw TCP accept to finish
    // its own startup logging / first compile.
    await pause(300);

    const browser = await chromium.launch({ headless: true });
    const context = await browser.newContext({ viewport: { width: 1024, height: 768 } });
    const page = await context.newPage();

    const requestUrls = [];
    const consoleErrors = [];
    const pageErrors = [];
    const allWebSockets = [];
    let sawAppWebSocket = null;

    page.on('request', (req) => {
      requestUrls.push(req.url());
    });
    page.on('websocket', (ws) => {
      allWebSockets.push(ws.url());
      // vite's own HMR client opens a websocket to the dev server root
      // (`ws://host:port/?token=...`) — that's dev-server plumbing, not an
      // app request. The app's sim websocket (now removed) would connect
      // to the `/ws` path specifically.
      let pathname;
      try {
        pathname = new URL(ws.url()).pathname;
      } catch {
        pathname = ws.url();
      }
      if (pathname === '/ws' || pathname.startsWith('/ws/')) {
        sawAppWebSocket = ws.url();
      }
    });
    page.on('console', (msg) => {
      if (msg.type() === 'error') consoleErrors.push(msg.text());
    });
    page.on('pageerror', (err) => {
      pageErrors.push(err.message);
    });

    try {
      await page.goto(TARGET_URL, { waitUntil: 'load', timeout: PAGE_TIMEOUT_MS });
    } catch (e) {
      fail(`page failed to load: ${e}`);
      await browser.close();
      return;
    }

    // Wait for the card-hand shell + login button to mount.
    let cardHandEl;
    let authButtonEl;
    try {
      cardHandEl = await page.waitForSelector('[data-card-hand]', { timeout: PAGE_TIMEOUT_MS });
      authButtonEl = await page.waitForSelector('[data-card-auth-button]', { timeout: PAGE_TIMEOUT_MS });
    } catch (e) {
      fail(`card-hand shell / login button did not appear in DOM: ${e}`);
      await browser.close();
      return;
    }

    // Let any async post-mount work (getSession, etc.) settle before we
    // snapshot the request log — a real regression would fire fetches here.
    await pause(1500);

    const buttonText = (await authButtonEl.textContent())?.trim();
    const statusEl = await page.$('[data-card-hand-status]');
    const statusHidden = statusEl ? await statusEl.isHidden() : null;
    const statusDataStatus = statusEl ? await statusEl.getAttribute('data-status') : null;
    const cardHandChildCount = await cardHandEl.evaluate((el) => el.childElementCount);

    await browser.close();

    // --- Assertions ---
    let ok = true;

    if (buttonText !== 'Login') {
      fail(`expected Login button text "Login", got ${JSON.stringify(buttonText)}`);
      ok = false;
    }

    if (statusDataStatus !== 'signed_out') {
      fail(`expected card-hand status data-status="signed_out", got ${JSON.stringify(statusDataStatus)}`);
      ok = false;
    }
    if (statusHidden !== true) {
      fail(`expected card-hand status element hidden in signed-out state, isHidden()=${statusHidden}`);
      ok = false;
    }
    if (cardHandChildCount !== 0) {
      fail(`expected empty card-hand (signed out, no cards), found ${cardHandChildCount} children`);
      ok = false;
    }

    if (sawAppWebSocket) {
      fail(`unexpected app websocket connection to /ws opened: ${sawAppWebSocket}`);
      ok = false;
    }

    const simRequests = requestUrls.filter((u) => SIM_ROUTE_PATTERNS.some((re) => re.test(u)));
    if (simRequests.length > 0) {
      fail(`sim-route requests observed: ${JSON.stringify(simRequests)}`);
      ok = false;
    }

    if (pageErrors.length > 0) {
      fail(`uncaught page errors: ${JSON.stringify(pageErrors)}`);
      ok = false;
    }

    console.log(
      JSON.stringify(
        {
          url: TARGET_URL,
          total_requests: requestUrls.length,
          requests: requestUrls,
          sim_route_requests: simRequests,
          all_websockets: allWebSockets,
          app_websocket_to_ws: sawAppWebSocket,
          login_button_text: buttonText,
          card_hand_status_data_status: statusDataStatus,
          card_hand_status_hidden: statusHidden,
          card_hand_child_count: cardHandChildCount,
          console_errors: consoleErrors,
          page_errors: pageErrors,
        },
        null,
        2,
      ),
    );

    if (ok) {
      console.log('SMOKE PASS');
    } else {
      process.exitCode = 1;
    }
  } finally {
    cleanup();
  }
}

await main();
// Force-exit: even after the process group is killed in cleanup(), a lingering
// handle could keep node alive. Exit explicitly so CI never hangs.
process.exit(process.exitCode ?? 0);
