// scripts/smoke-traffic.mjs
//
// Task 10 browser smoke: PROVES the winterthur-traffic wire end-to-end in a
// real headless-chromium WebGPU render of the Winterthur city view (ksw.html
// ?traffic=1). This is the gate CLAUDE.md mandates for any src/ ↔ backend/
// change.
//
// It launches the full stack (traffic binary + vite — see lib/traffic-stack),
// opens the city view, and asserts:
//   (a) the client SENDS subscribe frames  (CDP Network.webSocketFrameSent)
//   (b) the server SENDS CellFrames with vehicles (Network.webSocketFrameReceived
//       — protobuf binary, so we assert on frame COUNT + non-trivial byte size,
//       then corroborate "vehicles present" via the client's decoded table)
//   (c) sampled vehicle positions CHANGE over 5 s (cars actually move) — read
//       from window.__traffic.sample() (dead-reckoned poses, the same the car
//       layer draws)
//   (d) right-hand driving: for ≥90% of sampled MOVING vehicles on two-way
//       streets, the vehicle sits to the RIGHT of the oncoming (reverse-edge)
//       lane relative to its travel heading.
//
// Right-hand derivation (mirrors tests/geo/trafficnet.test.ts case (e), the
// curvature-robust antiparallel-pair form — NOT the straight node chord):
//   The ground plane is +x=EAST, +z=SOUTH — a left-handed (screen y-down)
//   frame, so "driver's right" is the +90° (visually clockwise) rotation of the
//   heading dir=(dx,dz) → right normal = (-dz, dx). A point to the right of the
//   heading has cross(dir, off) = dir.x·off.z − dir.z·off.x > 0.
//   For a moving vehicle we take `dir` from the local tangent of its OWN lane
//   polyline (nearest segment to the vehicle) — robust to curvature, unlike the
//   straight from→to chord. We find the vehicle's edge's REVERSE edge (to→from),
//   take that edge's lane 0 (the oncoming lane), and the nearest point `bp` on
//   it. The separation `sep = vehiclePos − bp` must point to the vehicle's right
//   (cross(dir, sep) > 0) — i.e. the oncoming traffic is on the vehicle's LEFT,
//   as required in right-hand-drive Switzerland. One-way edges have no reverse
//   lane and are (correctly) not testable this way.
//
// WebGPU headless flags + the window.__LOOK_READY gate follow scripts/smoke-ksw.mjs.

import { chromium } from 'playwright';
import { readFileSync } from 'node:fs';
import { startTrafficStack, HOST } from './lib/traffic-stack.mjs';

const TRAFFIC_PORT = 8790;
const VITE_PORT = 5187;
const SEED = 42;

// ── trafficnet geometry: lane → edge → (from,to) node centreline ────────────
const net = JSON.parse(
  readFileSync(new URL('../data/winterthur/trafficnet.json', import.meta.url), 'utf8'),
);
const edgeById = new Map(net.edges.map((e) => [e.id, e]));
const laneById = new Map(net.lanes.map((l) => [l.id, l]));
// Reverse-edge lookup keyed on (to->from): a two-way street's opposing edge.
const edgeByEndpoints = new Map(net.edges.map((e) => [`${e.from}->${e.to}`, e]));

/** The lane-0 polyline of the REVERSE edge (oncoming lane) for a given lane id,
 * or null if the lane's edge is one-way (no reverse) or data is missing. */
function oncomingLanePts(laneId) {
  const lane = laneById.get(laneId);
  if (!lane) return null;
  const edge = edgeById.get(lane.edge);
  if (!edge) return null;
  const rev = edgeByEndpoints.get(`${edge.to}->${edge.from}`);
  if (!rev) return null; // one-way
  const revLane0 = laneById.get(rev.lanes[0]);
  return revLane0?.pts ?? null;
}

/** Local unit travel tangent of a lane polyline at the point nearest (px,pz):
 * the direction of the closest polyline segment. Curvature-robust. */
function laneTangentAt(laneId, px, pz) {
  const lane = laneById.get(laneId);
  const pts = lane?.pts;
  if (!pts || pts.length < 2) return null;
  let best = Infinity;
  let dir = null;
  for (let i = 1; i < pts.length; i++) {
    const a = pts[i - 1];
    const b = pts[i];
    const ex = b[0] - a[0];
    const ez = b[1] - a[1];
    const len2 = ex * ex + ez * ez;
    if (len2 < 1e-12) continue;
    let t = ((px - a[0]) * ex + (pz - a[1]) * ez) / len2;
    t = Math.max(0, Math.min(1, t));
    const cx = a[0] + ex * t;
    const cz = a[1] + ez * t;
    const d = (px - cx) ** 2 + (pz - cz) ** 2;
    if (d < best) {
      best = d;
      const len = Math.sqrt(len2);
      dir = [ex / len, ez / len];
    }
  }
  return dir;
}

/** Nearest point on a polyline (array of [x,z]) to (px,pz). */
function nearestOnPolyline(pts, px, pz) {
  let best = Infinity;
  let bp = pts[0];
  for (let i = 1; i < pts.length; i++) {
    const a = pts[i - 1];
    const b = pts[i];
    const ex = b[0] - a[0];
    const ez = b[1] - a[1];
    const len2 = ex * ex + ez * ez;
    let t = len2 > 1e-12 ? ((px - a[0]) * ex + (pz - a[1]) * ez) / len2 : 0;
    t = Math.max(0, Math.min(1, t));
    const cx = a[0] + ex * t;
    const cz = a[1] + ez * t;
    const d = (px - cx) ** 2 + (pz - cz) ** 2;
    if (d < best) {
      best = d;
      bp = [cx, cz];
    }
  }
  return bp;
}

async function main() {
  const failures = [];
  const check = (name, ok, detail) => {
    console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  (${detail})` : ''}`);
    if (!ok) failures.push(name);
  };

  console.log(`[smoke] launching stack (traffic :${TRAFFIC_PORT} seed=${SEED}, vite :${VITE_PORT})…`);
  const stack = await startTrafficStack({ trafficPort: TRAFFIC_PORT, vitePort: VITE_PORT, seed: SEED });

  const browser = await chromium.launch({
    headless: true,
    args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
  });

  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const errors = [];
    page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
    page.on('console', (m) => {
      if (m.type() === 'error') errors.push(`console: ${m.text()}`);
    });

    // ── CDP WS instrumentation: capture frames without touching app code ──────
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.enable');
    const wsSent = [];
    const wsRecv = [];
    cdp.on('Network.webSocketFrameSent', (ev) => {
      // opcode 2 = binary; payloadData is base64 for binary frames.
      wsSent.push({ len: ev.response?.payloadData?.length ?? 0, op: ev.response?.opcode });
    });
    cdp.on('Network.webSocketFrameReceived', (ev) => {
      wsRecv.push({ len: ev.response?.payloadData?.length ?? 0, op: ev.response?.opcode });
    });

    // Freeze the render clock at morning peak for a reproducible scene.
    const url =
      `http://${HOST}:${VITE_PORT}/ksw.html` +
      `?traffic=1&trafficWs=ws://${HOST}:${TRAFFIC_PORT}/traffic` +
      `&cam=bahnhof&at=2026-07-03T08:00:00Z&wx=clear`;
    console.log(`[smoke] opening ${url}`);
    await page.goto(url, { waitUntil: 'load', timeout: 60000 });
    // Post-#119 the boot streams the 77 MB world pyramid before __LOOK_READY;
    // a cold dev-server load takes well over the old 40 s. Env-tunable for CI.
    const readyMs = Number(process.env.SMOKE_READY_TIMEOUT_MS ?? 180000);
    try {
      await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: readyMs });
    } catch (e) {
      console.error(`[smoke] __LOOK_READY not set after ${readyMs}ms; page console errors so far:`);
      for (const line of errors) console.error(`  ${line}`);
      throw e;
    }
    await page.waitForFunction(() => typeof window.__traffic?.lookAt === 'function', { timeout: 20000 });

    // Let the server fleet build up (spawns ~3.6+ veh/s), then aim the camera —
    // and thus the AOI subscription (rig.target-driven) — at the densest central
    // corridor found by scripts/probe-density.mjs ([8, 289] cluster, ~17 veh).
    // A moderate radius keeps the AOI over that corridor between the loop's 0.5 s
    // re-subscribes.
    const DENSE = { x: 8, z: 289, radius: 180 };
    await page.waitForTimeout(8000);
    await page.evaluate((d) => window.__traffic.lookAt(d.x, d.z, { radius: d.radius }), DENSE);
    // Wait for the AOI to populate at the new target (keyframes replace cell
    // membership within ≤5 s).
    await page.waitForFunction(() => (window.__traffic?.count() ?? 0) >= 3, { timeout: 20000 }).catch(() => {});
    await page.waitForTimeout(3000);

    // (a) client sends subscribe frames (binary, opcode 2, non-empty).
    const sentBinary = wsSent.filter((f) => f.op === 2 && f.len > 0);
    check('(a) client sends subscribe frames', sentBinary.length > 0, `${sentBinary.length} binary frames sent`);

    // (b) server sends CellFrames (binary, non-trivial size) AND the client's
    // decoded table has vehicles (proves the frames actually carried them).
    const recvBinary = wsRecv.filter((f) => f.op === 2 && f.len > 0);
    const vehCount0 = await page.evaluate(() => window.__traffic?.count() ?? 0);
    check(
      '(b) server sends CellFrames with vehicles',
      recvBinary.length > 0 && vehCount0 > 0,
      `${recvBinary.length} frames received, ${vehCount0} vehicles in client table`,
    );

    // (c) sampled vehicle positions CHANGE over 5 s.
    const sample = () => page.evaluate(() => window.__traffic?.sample() ?? []);
    const s0 = await sample();
    await page.waitForTimeout(5000);
    const s1 = await sample();

    const byId0 = new Map(s0.map((v) => [v.id, v]));
    let moved = 0;
    let comparable = 0;
    /** movers with their travel dir for the right-hand test */
    const movers = [];
    for (const v1 of s1) {
      const v0 = byId0.get(v1.id);
      if (!v0) continue;
      comparable++;
      const dx = v1.x - v0.x;
      const dz = v1.z - v0.z;
      const dist = Math.hypot(dx, dz);
      if (dist > 0.5) {
        moved++;
        // Only same-lane movers get an unambiguous local geometry; if the lane
        // changed across the 5 s window skip (curve/intersection ambiguity).
        if (v0.lane === v1.lane) movers.push({ id: v1.id, lane: v1.lane, px: v1.x, pz: v1.z });
      }
    }
    check(
      '(c) vehicle positions change over 5 s (cars move)',
      moved > 0 && comparable > 0,
      `${moved}/${comparable} tracked vehicles moved >0.5 m`,
    );

    // (d) right-hand driving on two-way streets: the oncoming (reverse-edge)
    // lane must be to the vehicle's LEFT ⇒ vehicle to the right of its heading.
    let right = 0;
    let tested = 0;
    for (const m of movers) {
      const onc = oncomingLanePts(m.lane);
      if (!onc) continue; // one-way edge — no oncoming lane to test against
      const dir = laneTangentAt(m.lane, m.px, m.pz);
      if (!dir) continue;
      const bp = nearestOnPolyline(onc, m.px, m.pz);
      const sepx = m.px - bp[0];
      const sepz = m.pz - bp[1];
      if (Math.hypot(sepx, sepz) < 0.1) continue; // coincident lanes — skip
      // cross(dir, sep) = dir.x·sep.z − dir.z·sep.x  > 0 ⇒ to the right.
      const cross = dir[0] * sepz - dir[1] * sepx;
      tested++;
      if (cross > 0) right++;
    }
    const ratio = tested > 0 ? right / tested : 0;
    check(
      '(d) ≥90% of moving vehicles drive on the right (two-way streets)',
      tested > 0 && ratio >= 0.9,
      `${right}/${tested} to the right of the oncoming lane (${(ratio * 100).toFixed(0)}%)`,
    );

    if (errors.length) {
      console.error('--- page/console errors ---');
      for (const e of errors.slice(0, 12)) console.error(e);
      failures.push('page errors');
    }

    await browser.close();
  } finally {
    await browser.close().catch(() => {});
    stack.cleanup();
  }

  if (failures.length) {
    console.error(`\nSMOKE FAIL: ${failures.join(', ')}`);
    process.exit(1);
  }
  console.log('\nSMOKE OK — cars stream, move, and drive on the right (verified in a real WebGPU browser)');
}

main().catch((err) => {
  console.error('SMOKE ERROR:', err);
  process.exit(1);
});
