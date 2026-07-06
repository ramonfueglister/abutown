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
//   (e) rush-hour vs night contrast (census wall-clock demand): with
//       ABUTOWN_TRAFFIC_AT=2026-07-03T07:30 (full form — pins day_kind to a
//       workday regardless of the run day) the client vehicle table exceeds
//       a demand_scale-derived floor within the observation window; the
//       backend is then relaunched at 03:00 of the same pinned date (same
//       seed) and the SAME scan-and-park procedure must show under a quarter
//       of the rush count.
//   (f) far-LOD flow channel (Task 11/12/13): after the client sends
//       subscribe_flow=true (always-on per trafficClient.ts), at least one
//       binary WS frame received over CDP decodes (via the SAME generated
//       proto the app uses, loaded in-page from vite so the decode is
//       byte-identical to production, not a re-implementation) to a
//       TrafficServerMsg carrying a `flow` field with `edges.length > 0`.
//   (h) heterogeneous fleet (S1): the decoded vehicle table carries classes
//       1/2 (Lieferwagen/LKW) at a plausible share, and the car layer draws
//       >=1 commercial silhouette (van/pickup/truck variant instance).
//   (g) far-LOD impostors actually DRAW while zoomed out: with the camera
//       pulled back far enough that the subscribed 3×3 AOI no longer covers
//       the busy corridor, `window.__traffic.flowCount() > 0` — the flow
//       layer's InstancedMesh.count read straight from the debug hook (Task
//       13), proving the impostors render, not just that the wire carries
//       flow data.
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

const TRAFFIC_PORT = Number(process.env.SMOKE_TRAFFIC_PORT ?? 8790);
const VITE_PORT = Number(process.env.SMOKE_VITE_PORT ?? 5187);
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

// ── scenario constants ────────────────────────────────────────────────────
// Wall-clock anchors: full YYYY-MM-DDTHH:MM form so day_kind is pinned too —
// 2026-07-03 is a Friday (Workday demand). An HH:MM-only override keeps the
// REAL run date and silently flips to weekend demand on Sat/Sun runs.
const AT_RUSH = '2026-07-03T07:30';
const AT_NIGHT = '2026-07-03T03:00';
// Candidate aim points on the Gemeinde census net's busiest central
// corridors at workday 07:30, found by a one-off central AOI sweep (top
// 128 m cells: 21 veh @ [-855,1850], 17 @ [-1330,1440], 14 @ [-1119,1237],
// 12 @ [-485,365]). NOTE the AOI subscription is a fixed 3×3 cell block
// around the camera target (trafficClient.ts) — `radius` only zooms the
// camera — so the vehicle table is capped at what a 384 m square holds, and
// instantaneous per-cell density wanders as platoons move through. Each
// scenario therefore SCANS these aims, parks on the busiest one, and
// observes there — identical procedure for rush and night, so the (e)
// contrast compares like with like.
const AIMS = [
  { x: -1100, z: 1550 },
  { x: -855, z: 1850 },
  { x: -1330, z: 1440 },
  { x: -485, z: 365 },
];
const CAM_RADIUS = 300;
// Identical observation window (measured from the first lookAt) for BOTH
// the 07:30 and the 03:00 scenario.
const OBSERVE_MS = 50000;
// (e) rush floor at demand_scale=1.0. Measured 07:30 workday runs hold
// 15–25 vehicles in the parked 3×3 AOI (the plan's authored ">40" predates
// the fixed-3×3-cell subscription cap); 8 is a robust floor under half the
// weakest observed run, while the weekend/03:00 regimes measure 0–4. The
// floor scales with the demand_scale the backend logs at boot.
const RUSH_MIN_BASE = 8;

/**
 * One full page session against the currently-running traffic backend:
 * opens ksw.html?traffic=1, aims the AOI at the dense corridor, and observes
 * the client vehicle table for OBSERVE_MS. With `full: true` it additionally
 * gathers the wire/motion evidence for assertions (a)–(d).
 *
 * @returns {Promise<{maxCount:number, sentBinary:number, recvBinary:number,
 *   moved:number, comparable:number, right:number, tested:number,
 *   flowFramesWithEdges:number, maxFlowCount:number, errors:string[]}>}
 */
async function runScenario(browser, { label, full }) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(`console: ${m.text()}`);
  });

  try {
    // ── CDP WS instrumentation: capture frames without touching app code ────
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.enable');
    const wsSent = [];
    const wsRecv = [];
    // Raw base64 payloads of received binary frames, kept for the (f) in-page
    // proto decode below — capped so a long observation window doesn't grow
    // this unboundedly.
    const wsRecvPayloads = [];
    const MAX_KEPT_PAYLOADS = 400;
    cdp.on('Network.webSocketFrameSent', (ev) => {
      // opcode 2 = binary; payloadData is base64 for binary frames.
      wsSent.push({ len: ev.response?.payloadData?.length ?? 0, op: ev.response?.opcode });
    });
    cdp.on('Network.webSocketFrameReceived', (ev) => {
      wsRecv.push({ len: ev.response?.payloadData?.length ?? 0, op: ev.response?.opcode });
      if (ev.response?.opcode === 2 && ev.response?.payloadData && wsRecvPayloads.length < MAX_KEPT_PAYLOADS) {
        wsRecvPayloads.push(ev.response.payloadData);
      }
    });

    // Freeze the render clock at morning peak for a reproducible scene.
    const url =
      `http://${HOST}:${VITE_PORT}/ksw.html` +
      `?traffic=1&trafficWs=ws://${HOST}:${TRAFFIC_PORT}/traffic` +
      `&cam=bahnhof&at=2026-07-03T08:00:00Z&wx=clear`;
    console.log(`[smoke:${label}] opening ${url}`);
    await page.goto(url, { waitUntil: 'load', timeout: 60000 });
    // Post-#119 the boot streams the 77 MB world pyramid before __LOOK_READY;
    // a cold dev-server load takes well over the old 40 s. Env-tunable for CI.
    const readyMs = Number(process.env.SMOKE_READY_TIMEOUT_MS ?? 180000);
    try {
      await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: readyMs });
    } catch (e) {
      console.error(`[smoke:${label}] __LOOK_READY not set after ${readyMs}ms; page console errors so far:`);
      for (const line of errors) console.error(`  ${line}`);
      throw e;
    }
    await page.waitForFunction(() => typeof window.__traffic?.lookAt === 'function', { timeout: 20000 });

    // Let the warm-started fleet stream in.
    await page.waitForTimeout(8000);
    const tLook = Date.now();

    const count = () => page.evaluate(() => window.__traffic?.count() ?? 0);
    let maxCount = 0;
    const poll = async () => {
      const c = await count();
      if (c > maxCount) maxCount = c;
      return c;
    };

    // Scan the candidate corridors and PARK on the busiest one: aim the
    // camera — and thus the 3×3-cell AOI subscription (rig.target-driven) —
    // at each in turn (keyframes replace cell membership within ≤5 s).
    let best = { aim: AIMS[0], n: -1 };
    for (const aim of AIMS) {
      await page.evaluate(
        (a) => window.__traffic.lookAt(a.x, a.z, { radius: a.radius }),
        { ...aim, radius: CAM_RADIUS },
      );
      await page.waitForTimeout(4500);
      const n = await poll();
      if (n > best.n) best = { aim, n };
    }
    console.log(
      `[smoke:${label}] parking on busiest aim [${best.aim.x}, ${best.aim.z}] (${best.n} veh at scan time)`,
    );
    await page.evaluate(
      (a) => window.__traffic.lookAt(a.x, a.z, { radius: a.radius }),
      { ...best.aim, radius: CAM_RADIUS },
    );

    let moved = 0;
    let comparable = 0;
    let right = 0;
    let tested = 0;

    if (full) {
      // Wait for the AOI to re-populate at the parked target.
      await page
        .waitForFunction(() => (window.__traffic?.count() ?? 0) >= 3, { timeout: 20000 })
        .catch(() => {});
      await page.waitForTimeout(3000);
      await poll();

      // (c) evidence: sampled vehicle positions across a 5 s window.
      const sample = () => page.evaluate(() => window.__traffic?.sample() ?? []);
      const s0 = await sample();
      await page.waitForTimeout(5000);
      const s1 = await sample();
      await poll();

      const byId0 = new Map(s0.map((v) => [v.id, v]));
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
          // Only same-lane movers get an unambiguous local geometry; if the
          // lane changed across the 5 s window skip (curve/intersection
          // ambiguity).
          if (v0.lane === v1.lane) movers.push({ id: v1.id, lane: v1.lane, px: v1.x, pz: v1.z });
        }
      }

      // (d) evidence: right-hand driving on two-way streets — the oncoming
      // (reverse-edge) lane must be to the vehicle's LEFT ⇒ vehicle to the
      // right of its heading.
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
    }

    // (h) evidence: heterogeneous fleet — wire class mix in a fresh sample
    // plus drawn commercial silhouettes (van/pickup/truck variant instances).
    const hSample = await page.evaluate(() => window.__traffic?.sample() ?? []);
    const clsCounts = [0, 0, 0];
    for (const v of hSample) clsCounts[Math.min(v.cls ?? 0, 2)]++;
    const variantCounts = await page.evaluate(() => window.__traffic?.cars?.() ?? []);
    const commercialDrawn = variantCounts.slice(4).reduce((a, b) => a + b, 0);

    let flowFramesWithEdges = 0;
    let maxFlowCount = 0;

    if (full) {
      // (f) evidence: decode a sample of the raw binary WS frames captured
      // over CDP via the SAME generated proto module the app imports (loaded
      // in-page from vite, exactly like probe-density.mjs) — this proves the
      // WIRE carries a FlowFrame with edges, independent of the app's own
      // TrafficClient decode path.
      flowFramesWithEdges = await page.evaluate(async (payloads) => {
        // fromBinary lives in the @bufbuild/protobuf RUNTIME, not the
        // generated schema module — mirrors trafficClient.ts's import split
        // and scripts/probe-density.mjs's in-page decode.
        const { fromBinary } = await import(
          '/node_modules/@bufbuild/protobuf/dist/esm/index.js'
        ).catch(() => import('@bufbuild/protobuf'));
        const pb = await import('/src/proto/traffic_pb.ts');
        let n = 0;
        for (const b64 of payloads) {
          const bin = atob(b64);
          const bytes = new Uint8Array(bin.length);
          for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
          try {
            const msg = fromBinary(pb.TrafficServerMsgSchema, bytes);
            if (msg.flow && msg.flow.edges.length > 0) n++;
          } catch {
            // not every captured binary frame need be a TrafficServerMsg
            // (there are none other on this socket, but stay defensive).
          }
        }
        return n;
      }, wsRecvPayloads);
      console.log(
        `[smoke:${label}] (f) ${flowFramesWithEdges}/${wsRecvPayloads.length} sampled binary frames decode to a FlowFrame with edges`,
      );

      // (g) evidence: zoom the camera OUT far enough that the subscribed 3×3
      // AOI (128 m cells, radius 1) no longer covers the busy corridor we
      // parked on — the flow layer should then draw impostors for the traffic
      // that's still there, just outside the per-vehicle subscription.
      await page.evaluate(
        (a) => window.__traffic.lookAt(a.x, a.z, { radius: a.radius }),
        { ...best.aim, radius: 2400 },
      );
      await page.waitForTimeout(6000);
      const flowCounts = () => page.evaluate(() => window.__traffic?.flowCount?.() ?? 0);
      const tFlow = Date.now();
      while (Date.now() - tFlow < 12000) {
        const fc = await flowCounts();
        if (fc > maxFlowCount) maxFlowCount = fc;
        if (maxFlowCount > 0) break;
        await page.waitForTimeout(1000);
      }
      console.log(`[smoke:${label}] (g) max flowCount() while zoomed out (r=2400): ${maxFlowCount}`);

      // Re-park on the busy aim at the normal radius so the round-out window
      // below keeps observing the same scene the (a)-(e) assertions expect.
      await page.evaluate(
        (a) => window.__traffic.lookAt(a.x, a.z, { radius: a.radius }),
        { ...best.aim, radius: CAM_RADIUS },
      );
      await page.waitForTimeout(2000);
    }

    // Round the observation out to the fixed window, polling the table size.
    while (Date.now() - tLook < OBSERVE_MS) {
      await page.waitForTimeout(2000);
      await poll();
    }

    const sentBinary = wsSent.filter((f) => f.op === 2 && f.len > 0).length;
    const recvBinary = wsRecv.filter((f) => f.op === 2 && f.len > 0).length;
    console.log(`[smoke:${label}] max vehicle-table size over ${OBSERVE_MS / 1000}s: ${maxCount}`);
    return {
      maxCount,
      sentBinary,
      recvBinary,
      moved,
      comparable,
      right,
      tested,
      flowFramesWithEdges,
      maxFlowCount,
      clsCounts,
      commercialDrawn,
      errors,
    };
  } finally {
    await page.close().catch(() => {});
  }
}

async function main() {
  const failures = [];
  const check = (name, ok, detail) => {
    console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  (${detail})` : ''}`);
    if (!ok) failures.push(name);
  };

  console.log(
    `[smoke] launching stack (traffic :${TRAFFIC_PORT} seed=${SEED} at=${AT_RUSH}, vite :${VITE_PORT})…`,
  );
  const stack = await startTrafficStack({
    trafficPort: TRAFFIC_PORT,
    vitePort: VITE_PORT,
    seed: SEED,
    at: AT_RUSH,
  });

  const browser = await chromium.launch({
    headless: true,
    args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
  });

  try {
    // ── scenario 1: morning rush (07:30) — full wire/motion assertions ──────
    const rush = await runScenario(browser, { label: 'rush-07:30', full: true });

    // (a) client sends subscribe frames (binary, opcode 2, non-empty).
    check('(a) client sends subscribe frames', rush.sentBinary > 0, `${rush.sentBinary} binary frames sent`);

    // (b) server sends CellFrames (binary, non-trivial size) AND the client's
    // decoded table has vehicles (proves the frames actually carried them).
    check(
      '(b) server sends CellFrames with vehicles',
      rush.recvBinary > 0 && rush.maxCount > 0,
      `${rush.recvBinary} frames received, ${rush.maxCount} vehicles in client table`,
    );

    // (c) sampled vehicle positions CHANGE over 5 s.
    check(
      '(c) vehicle positions change over 5 s (cars move)',
      rush.moved > 0 && rush.comparable > 0,
      `${rush.moved}/${rush.comparable} tracked vehicles moved >0.5 m`,
    );

    // (d) right-hand driving.
    const ratio = rush.tested > 0 ? rush.right / rush.tested : 0;
    check(
      '(d) ≥90% of moving vehicles drive on the right (two-way streets)',
      rush.tested > 0 && ratio >= 0.9,
      `${rush.right}/${rush.tested} to the right of the oncoming lane (${(ratio * 100).toFixed(0)}%)`,
    );

    // (f) far-LOD flow channel: ≥1 sampled binary frame decodes to a
    // TrafficServerMsg with a FlowFrame carrying ≥1 edge.
    check(
      '(f) subscribe_flow wire carries a FlowFrame with edges',
      rush.flowFramesWithEdges > 0,
      `${rush.flowFramesWithEdges} frames with edges.length > 0`,
    );

    // (g) impostors actually draw while zoomed out past the subscribed AOI.
    check(
      '(g) flowCount() > 0 while zoomed out',
      rush.maxFlowCount > 0,
      `max flowCount()=${rush.maxFlowCount}`,
    );

    // (h) heterogeneous fleet: the wire carries classes 1/2 (S1) and the car
    // layer draws commercial silhouettes. The urban mix bakes ~11% commercial,
    // the A1 corridor ~14% — accept a broad 1–40% band on the sampled AOI to
    // stay location-robust, but require presence in a rush-sized sample.
    const clsTotal = rush.clsCounts.reduce((a, b) => a + b, 0);
    const commercial = rush.clsCounts[1] + rush.clsCounts[2];
    const commercialShare = clsTotal > 0 ? commercial / clsTotal : 0;
    // Presence, not exact share: the parked AOI often holds only ~10-30
    // vehicles, so the baked 11% urban mix yields a handful of commercial
    // vehicles — require >=1 on the wire and >=1 drawn, and a sane upper
    // bound (a majority-commercial AOI would mean the mapping ran wild).
    check(
      '(h) wire carries vehicle classes + commercial silhouettes draw',
      clsTotal >= 10 && commercial > 0 && commercialShare < 0.5 && rush.commercialDrawn > 0,
      `sample=${clsTotal} [car,van,truck]=${JSON.stringify(rush.clsCounts)} share=${(commercialShare * 100).toFixed(1)}% drawnCommercial=${rush.commercialDrawn}`,
    );

    // ── scenario 2: dead of night (03:00), fresh backend, same seed ─────────
    console.log(`[smoke] restarting traffic backend at ${AT_NIGHT} (same seed)…`);
    await stack.restartTraffic({ at: AT_NIGHT, seed: SEED });
    const night = await runScenario(browser, { label: 'night-03:00', full: false });

    // (e) rush-hour vs night contrast on the census demand curve. The rush
    // floor scales with the demand_scale the backend logged at boot.
    // tracing emits ANSI colour codes even into a pipe — strip before matching.
    // eslint-disable-next-line no-control-regex
    const plainLogs = stack.logs().replace(/\x1b\[[0-9;]*m/g, '');
    const dsMatch = plainLogs.match(/demand_scale=([0-9.]+)/);
    const demandScale = dsMatch ? Number(dsMatch[1]) : 1.0;
    const rushMin = Math.max(4, Math.round(RUSH_MIN_BASE * demandScale));
    check(
      `(e) rush-hour vs night demand contrast (N_rush > ${rushMin}, N_night < N_rush/4)`,
      rush.maxCount > rushMin && night.maxCount < rush.maxCount / 4,
      `N_rush=${rush.maxCount}, N_night=${night.maxCount}, demand_scale=${demandScale}`,
    );

    const errors = [...rush.errors, ...night.errors];
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
  console.log(
    '\nSMOKE OK — cars stream, move, drive on the right, follow the census rush-hour curve, and the far-LOD flow channel carries edges + draws impostors zoomed out (verified in a real WebGPU browser)',
  );
}

main().catch((err) => {
  console.error('SMOKE ERROR:', err);
  process.exit(1);
});
