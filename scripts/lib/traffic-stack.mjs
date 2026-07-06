// scripts/lib/traffic-stack.mjs
//
// Shared launcher for the Task 10 traffic browser harnesses (smoke-traffic.mjs
// + capture-traffic.mjs) AND the M1 world smoke (smoke-world.mjs): brings up
// BOTH halves of the wire —
//   1. the backend binary — winterthur-traffic (WS /traffic + /healthz on
//      TRAFFIC_PORT) for startTrafficStack, or the one-process sim-server
//      (/traffic + /live + /health on LISTEN_PORT) for startWorldStack,
//   2. the vite dev server that serves ksw.html (the city view that renders
//      cars behind ?traffic=1 and citizens behind ?live=1),
// waits until each is actually answering, and hands back a cleanup() that kills
// every process it started (detached process groups → no orphans). Follows the
// spawn/detached/SIGKILL teardown pattern of scripts/smoke-ksw.mjs.

import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, symlinkSync } from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const REPO_ROOT = fileURLToPath(new URL('../..', import.meta.url));
const HOST = '127.0.0.1';

/** Path to the release binary built up front by cargo-serial. */
const TRAFFIC_BIN = path.join(REPO_ROOT, 'backend/target/release/winterthur-traffic');

/** Path to the one-process world binary (M1 Task 13), built by startWorldStack. */
const SIM_BIN = path.join(REPO_ROOT, 'backend/target/release/sim-server');

function portOpen(host, port) {
  return new Promise((resolve) => {
    const s = net.createConnection({ host, port }, () => {
      s.end();
      resolve(true);
    });
    s.on('error', () => resolve(false));
    s.setTimeout(1000, () => {
      s.destroy();
      resolve(false);
    });
  });
}

async function waitForPort(host, port, timeoutMs) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await portOpen(host, port)) return true;
    await new Promise((r) => setTimeout(r, 200));
  }
  return false;
}

async function waitForHealthz(port, timeoutMs) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(`http://${HOST}:${port}/healthz`);
      if (res.ok && (await res.text()).trim() === 'ok') return true;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  return false;
}

/** sim-server readiness: /health answers 200 JSON with a numeric world_tick.
 * Per-attempt abort: the listener is bound BEFORE the (CH-router) sim build,
 * so a plain fetch would hang in the accept backlog instead of retrying. */
async function waitForWorldHealth(port, timeoutMs) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(`http://${HOST}:${port}/health`, {
        signal: AbortSignal.timeout(2000),
      });
      if (res.ok) {
        const body = await res.json();
        if (typeof body.world_tick === 'number') return true;
      }
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 300));
  }
  return false;
}

/** Hard-abort if `port` is already taken: a busy port means the smoke would
 * silently test a FOREIGN server (documented Task 15 trap #2). */
async function assertPortFree(port, label) {
  if (await portOpen(HOST, port)) {
    throw new Error(
      `${label} port :${port} is already in use — refusing to start (the smoke would ` +
        `silently talk to a foreign server). Pick another port or kill the holder ` +
        `(lsof -nP -iTCP:${port} -sTCP:LISTEN).`,
    );
  }
}

/** SIGKILL a spawned process and its detached group. */
function killGroup(p) {
  if (p.pid) {
    try {
      process.kill(-p.pid, 'SIGKILL');
    } catch {
      /* group already gone */
    }
  }
  try {
    p.kill('SIGKILL');
  } catch {
    /* already dead */
  }
}

/** ksw.html's loadWorld() fetches /winterthur-world/*.pb, served by vite from
 * the gitignored dev symlink public/winterthur-world → data/winterthur/world
 * (see .gitignore). Without it vite's SPA fallback answers those fetches
 * with HTML and the boot dies mid-decode ("cant skip wire type 4") before
 * __LOOK_READY — create the documented symlink if the bake exists.
 * With `required` the bake itself must exist (browser smokes). */
function ensureWorldBake({ required }) {
  const worldDir = path.join(REPO_ROOT, 'data/winterthur/world');
  const worldLink = path.join(REPO_ROOT, 'public/winterthur-world');
  if (!existsSync(worldDir)) {
    if (required) {
      throw new Error(
        `data/winterthur/world is missing — the browser city view cannot boot without the ` +
          `baked world pyramid (gitignored, 77 MB). Bake it once: npm run geo:fetch && ` +
          `npm run geo:bake-world (see memory: diorama-smoke-needs-world-bake).`,
      );
    }
    return;
  }
  if (!existsSync(worldLink)) {
    mkdirSync(path.dirname(worldLink), { recursive: true });
    symlinkSync('../data/winterthur/world', worldLink);
  }
}

/** Spawn the vite dev server on `vitePort`; logs stream into `appendOut`. */
function spawnVite(procs, appendOut, vitePort) {
  const dev = spawn('npm', ['run', 'dev', '--', '--port', String(vitePort), '--strictPort'], {
    cwd: REPO_ROOT,
    env: { ...process.env },
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true,
  });
  procs.push(dev);
  dev.stdout.on('data', (d) => appendOut(`[vite] ${d}`));
  dev.stderr.on('data', (d) => appendOut(`[vite] ${d}`));
  return dev;
}

/** Idempotent kill-everything teardown wired to exit/SIGINT. */
function makeCleanup(procs) {
  let cleaned = false;
  const cleanup = () => {
    if (cleaned) return;
    cleaned = true;
    for (const p of procs) killGroup(p);
  };
  process.on('exit', cleanup);
  process.on('SIGINT', () => {
    cleanup();
    process.exit(130);
  });
  return cleanup;
}

/**
 * Launch the traffic server + vite dev server.
 * @param {object} opts
 * @param {number} opts.trafficPort   TRAFFIC_PORT for the binary (default 8790)
 * @param {number} opts.vitePort      vite --port (default 5187)
 * @param {number} opts.seed          TRAFFIC_SEED (default 42, fixed for reproducibility)
 * @param {string} [opts.at]          ABUTOWN_TRAFFIC_AT wall-clock override:
 *                                    `HH:MM` (real date) or `YYYY-MM-DDTHH:MM`
 *                                    (pins the date + day_kind — reproducible)
 * @returns {Promise<{ vitePort:number, trafficPort:number, logs:()=>string,
 *   restartTraffic:(o?:{at?:string, seed?:number})=>Promise<void>, cleanup:()=>void }>}
 */
export async function startTrafficStack(opts = {}) {
  const trafficPort = opts.trafficPort ?? 8790;
  const vitePort = opts.vitePort ?? 5187;
  const seed = opts.seed ?? 42;

  ensureWorldBake({ required: false });

  await assertPortFree(trafficPort, 'traffic');
  await assertPortFree(vitePort, 'vite');

  let out = '';
  const procs = [];

  const spawnTraffic = ({ at, seed: s }) => {
    const env = { ...process.env, TRAFFIC_PORT: String(trafficPort), TRAFFIC_SEED: String(s) };
    if (at != null) env.ABUTOWN_TRAFFIC_AT = at;
    const p = spawn(TRAFFIC_BIN, [], {
      cwd: REPO_ROOT,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
      detached: true,
    });
    procs.push(p);
    p.stdout.on('data', (d) => (out += `[traffic] ${d}`));
    p.stderr.on('data', (d) => (out += `[traffic] ${d}`));
    return p;
  };

  // 1. traffic server (the release binary, run directly).
  let server = spawnTraffic({ at: opts.at, seed });

  // 2. vite dev server serving ksw.html.
  spawnVite(procs, (s) => (out += s), vitePort);

  const cleanup = makeCleanup(procs);
  const logs = () => out;

  /** Kill only the traffic binary and relaunch it with a new wall-clock
   * override / seed — vite (and any attached browser) stays up. Used by the
   * smoke's rush-hour-vs-night contrast scenario. */
  const restartTraffic = async ({ at, seed: s } = {}) => {
    killGroup(server);
    // Wait for the port to actually free before rebinding.
    const start = Date.now();
    while (Date.now() - start < 10000 && (await portOpen(HOST, trafficPort))) {
      await new Promise((r) => setTimeout(r, 200));
    }
    server = spawnTraffic({ at, seed: s ?? seed });
    if (!(await waitForHealthz(trafficPort, 30000))) {
      cleanup();
      throw new Error(`restarted traffic server /healthz not ok on :${trafficPort}\n${out}`);
    }
  };

  // Wait for both to answer.
  if (!(await waitForHealthz(trafficPort, 30000))) {
    cleanup();
    throw new Error(`traffic server /healthz not ok on :${trafficPort}\n${out}`);
  }
  if (!(await waitForPort(HOST, vitePort, 40000))) {
    cleanup();
    throw new Error(`vite dev server not up on :${vitePort}\n${out}`);
  }

  return { vitePort, trafficPort, logs, restartTraffic, cleanup };
}

/** Build the sim-server release binary via the mandatory cargo-serial wrapper
 * (CLAUDE.md rule: never two cargo at once). No-op-fast when up to date. */
function buildSimServer() {
  const res = spawnSync(
    path.join(REPO_ROOT, 'scripts/cargo-serial.sh'),
    ['build', '--release', '--manifest-path', path.join(REPO_ROOT, 'backend/Cargo.toml'), '-p', 'sim-server'],
    { cwd: REPO_ROOT, stdio: 'inherit' },
  );
  if (res.status !== 0) {
    throw new Error(`cargo build --release -p sim-server failed (exit ${res.status})`);
  }
  if (!existsSync(SIM_BIN)) {
    throw new Error(`sim-server build reported success but ${SIM_BIN} is missing`);
  }
}

/**
 * Launch the one-process sim-server (M1 Task 13: /traffic + /live + card-hand
 * routes + /health on ONE port) plus — unless `vite: false` — the vite dev
 * server for ksw.html?live=1. Extends (rather than duplicates) the traffic
 * stack launcher above; same detached/SIGKILL teardown.
 *
 * ⚠️ ENV HYGIENE (documented Task 15 trap #1): sim-server calls
 * `dotenvy::dotenv()`, which walks ANCESTOR directories from its cwd for a
 * `.env` — a worktree under ~/Coding/abutown finds the parent checkout's
 * `.env` and with it the PROD `DATABASE_URL`. The child therefore gets
 *   (a) an env built FROM SCRATCH (never `...process.env` — a shell-exported
 *       DATABASE_URL must not leak through either), with DATABASE_URL set
 *       ONLY to the explicitly passed `opts.databaseUrl`, and
 *   (b) `cwd: os.tmpdir()` — outside every repo tree, so dotenvy finds
 *       nothing; all data artefact paths are passed absolute.
 *
 * @param {object} opts
 * @param {number}  [opts.simPort=8189]     LISTEN_PORT for sim-server
 * @param {number}  [opts.vitePort=5191]    vite --port (ignored with vite:false)
 * @param {boolean} [opts.vite=true]        false = headless /live+/health mode
 *                                          (CI --no-render; no world bake needed)
 * @param {number}  [opts.seed=42]          TRAFFIC_SEED
 * @param {string}  [opts.at]               ABUTOWN_TRAFFIC_AT boot wall-clock
 * @param {string}  [opts.databaseUrl]      Postgres URL (TEST db!) — omit for
 *                                          the in-memory (no-persistence) mode
 * @param {string}  [opts.worldId='winterthur'] ABUTOWN_WORLD_ID snapshot key
 * @returns {Promise<{ simPort:number, vitePort:number|null, logs:()=>string,
 *   restartSimServer:()=>Promise<string>, cleanup:()=>void }>}
 *   restartSimServer kills ONLY the sim-server (vite + browser stay up),
 *   relaunches it with the identical env, waits for /health, and returns the
 *   NEW process's boot log (for the resume-line assertion).
 */
export async function startWorldStack(opts = {}) {
  const simPort = opts.simPort ?? 8189;
  const withVite = opts.vite !== false;
  const vitePort = withVite ? (opts.vitePort ?? 5191) : null;
  const seed = opts.seed ?? 42;

  if (withVite) ensureWorldBake({ required: true });

  buildSimServer();

  await assertPortFree(simPort, 'sim-server');
  if (withVite) await assertPortFree(vitePort, 'vite');

  let out = '';
  const procs = [];

  const simEnv = () => {
    // Built from scratch — see the env-hygiene banner above. PATH/HOME kept
    // for dyld/TLS lookups; everything else explicit.
    const env = {
      PATH: process.env.PATH ?? '',
      HOME: process.env.HOME ?? '',
      LISTEN_HOST: HOST,
      LISTEN_PORT: String(simPort),
      TRAFFIC_SEED: String(seed),
      ABUTOWN_WORLD_ID: opts.worldId ?? 'winterthur',
      TRAFFICNET_JSON: path.join(REPO_ROOT, 'data/winterthur/trafficnet.json'),
      TRIPS_BIN: path.join(REPO_ROOT, 'data/winterthur/trips.bin'),
      SIMWORLD_JSON: path.join(REPO_ROOT, 'data/winterthur/simworld.json'),
      ECONOMY_JSON: path.join(REPO_ROOT, 'data/winterthur/economy.json'),
    };
    if (opts.at != null) env.ABUTOWN_TRAFFIC_AT = opts.at;
    if (opts.databaseUrl != null) {
      env.DATABASE_URL = opts.databaseUrl;
      // Required alongside DATABASE_URL (card-hand auth); the smoke never
      // logs in, so a dummy loopback URL is correct here.
      env.SUPABASE_URL = 'http://127.0.0.1:9999';
      env.CORS_ALLOWED_ORIGINS = vitePort != null ? `http://${HOST}:${vitePort}` : `http://${HOST}:5173`;
    }
    return env;
  };

  const spawnSim = () => {
    const p = spawn(SIM_BIN, [], {
      cwd: os.tmpdir(), // OUTSIDE every repo tree — dotenvy must find no .env
      env: simEnv(),
      stdio: ['ignore', 'pipe', 'pipe'],
      detached: true,
    });
    procs.push(p);
    p.stdout.on('data', (d) => (out += `[sim] ${d}`));
    p.stderr.on('data', (d) => (out += `[sim] ${d}`));
    return p;
  };

  let server = spawnSim();
  if (withVite) spawnVite(procs, (s) => (out += s), vitePort);

  const cleanup = makeCleanup(procs);
  const logs = () => out;

  /** restartTraffic analogue: kill ONLY sim-server, relaunch identically,
   * return the fresh boot log (resume-line proof lives in there). */
  const restartSimServer = async () => {
    killGroup(server);
    const start = Date.now();
    while (Date.now() - start < 10000 && (await portOpen(HOST, simPort))) {
      await new Promise((r) => setTimeout(r, 200));
    }
    const mark = out.length;
    server = spawnSim();
    if (!(await waitForWorldHealth(simPort, 120000))) {
      cleanup();
      throw new Error(`restarted sim-server /health not ok on :${simPort}\n${out.slice(mark)}`);
    }
    return out.slice(mark);
  };

  // The CH router build makes a cold boot take tens of seconds — generous gate.
  if (!(await waitForWorldHealth(simPort, 180000))) {
    cleanup();
    throw new Error(`sim-server /health not ok on :${simPort}\n${out}`);
  }
  if (withVite && !(await waitForPort(HOST, vitePort, 40000))) {
    cleanup();
    throw new Error(`vite dev server not up on :${vitePort}\n${out}`);
  }

  return { simPort, vitePort, logs, restartSimServer, cleanup };
}

export { HOST };
