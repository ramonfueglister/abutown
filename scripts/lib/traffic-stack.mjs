// scripts/lib/traffic-stack.mjs
//
// Shared launcher for the Task 10 traffic browser harnesses (smoke-traffic.mjs
// + capture-traffic.mjs): brings up BOTH halves of the wire —
//   1. the winterthur-traffic binary (WS /traffic + /healthz on TRAFFIC_PORT),
//   2. the vite dev server that serves ksw.html (the city view that renders the
//      cars behind ?traffic=1),
// waits until each is actually answering, and hands back a cleanup() that kills
// every process it started (detached process groups → no orphans). Follows the
// spawn/detached/SIGKILL teardown pattern of scripts/smoke-ksw.mjs.

import { spawn } from 'node:child_process';
import net from 'node:net';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const REPO_ROOT = fileURLToPath(new URL('../..', import.meta.url));
const HOST = '127.0.0.1';

/** Path to the release binary built up front by cargo-serial. */
const TRAFFIC_BIN = path.join(REPO_ROOT, 'backend/target/release/winterthur-traffic');

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

/**
 * Launch the traffic server + vite dev server.
 * @param {object} opts
 * @param {number} opts.trafficPort   TRAFFIC_PORT for the binary (default 8790)
 * @param {number} opts.vitePort      vite --port (default 5187)
 * @param {number} opts.seed          TRAFFIC_SEED (default 42, fixed for reproducibility)
 * @returns {Promise<{ vitePort:number, trafficPort:number, logs:()=>string, cleanup:()=>void }>}
 */
export async function startTrafficStack(opts = {}) {
  const trafficPort = opts.trafficPort ?? 8790;
  const vitePort = opts.vitePort ?? 5187;
  const seed = opts.seed ?? 42;

  let out = '';
  const procs = [];

  // 1. traffic server (the release binary, run directly).
  const server = spawn(TRAFFIC_BIN, [], {
    cwd: REPO_ROOT,
    env: { ...process.env, TRAFFIC_PORT: String(trafficPort), TRAFFIC_SEED: String(seed) },
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true,
  });
  procs.push(server);
  server.stdout.on('data', (d) => (out += `[traffic] ${d}`));
  server.stderr.on('data', (d) => (out += `[traffic] ${d}`));

  // 2. vite dev server serving ksw.html.
  const dev = spawn('npm', ['run', 'dev', '--', '--port', String(vitePort), '--strictPort'], {
    cwd: REPO_ROOT,
    env: { ...process.env },
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true,
  });
  procs.push(dev);
  dev.stdout.on('data', (d) => (out += `[vite] ${d}`));
  dev.stderr.on('data', (d) => (out += `[vite] ${d}`));

  let cleaned = false;
  const cleanup = () => {
    if (cleaned) return;
    cleaned = true;
    for (const p of procs) {
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
  };
  process.on('exit', cleanup);
  process.on('SIGINT', () => {
    cleanup();
    process.exit(130);
  });

  const logs = () => out;

  // Wait for both to answer.
  if (!(await waitForHealthz(trafficPort, 30000))) {
    cleanup();
    throw new Error(`traffic server /healthz not ok on :${trafficPort}\n${out}`);
  }
  if (!(await waitForPort(HOST, vitePort, 40000))) {
    cleanup();
    throw new Error(`vite dev server not up on :${vitePort}\n${out}`);
  }

  return { vitePort, trafficPort, logs, cleanup };
}

export { HOST };
