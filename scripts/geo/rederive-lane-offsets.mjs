// scripts/geo/rederive-lane-offsets.mjs
//
// Width-aware lane re-derive on the ALREADY-BAKED data/winterthur/trafficnet.json,
// preserving every node/edge/lane/turn ID (same contract as
// rederive-traffic-conflicts.mjs — a full rebake no longer reproduces the
// committed net, see that script's banner).
//
// Root-cause fix for the FIX-D1 symptom chain: the original bake offset every
// lane by (idx + 0.5)·3.0 m regardless of the real carriage width, so cars on
// 4–5.5 m streets drove 1.5 m beside the centreline — half off the tarmac.
// FIX D1 then widened the RENDERED ribbons to cover the kernel error, which
// swallowed street-tree verges (68/221 trees inside ribbons at the
// [-708,700] probe) and pushed ribbons into facades. Here we fix the kernel
// side instead: each edge's centreline is recovered from lane 0
// (offsetRight by −1.5 m, the exact inverse of the bake), the edge's real
// width is matched geometrically against roads.json (same OSM source, 5 m
// radius + parallel-tangent gate like roadWidths.ts did), and the lanes are
// re-offset with the width-aware laneOffsets() now also used by the source
// bake (trafficnet.mjs). Turn conflict geometry depends on lane endpoints, so
// run rederive-traffic-conflicts.mjs AFTER this script, then re-bake
// trips.bin (the server's net-hash gate).
import { readFileSync, writeFileSync } from 'node:fs';
import { LANE_WIDTH, laneOffsets, offsetRight } from './lib/trafficnet.mjs';

const NET = 'data/winterthur/trafficnet.json';
const ROADS = 'data/winterthur/roads.json';
const MATCH_DIST_M = 5.0;
const PARALLEL_DOT = 0.5; // cos 60°, same gate as roadWidths.ts
const CELL = 32;

const net = JSON.parse(readFileSync(NET, 'utf8'));
const roadsDoc = JSON.parse(readFileSync(ROADS, 'utf8'));

const r2 = (n) => Math.round(n * 100) / 100;
const polylineLength = (pts) => {
  let s = 0;
  for (let i = 1; i < pts.length; i++) s += Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]);
  return s;
};

// ── spatial hash of carriage road segments (footways excluded) ─────────────
const FOOT = new Set(['footway', 'path', 'cycleway', 'steps', 'pedestrian', 'track']);
const hash = new Map(); // "cx,cz" -> [{ax,az,bx,bz,width}]
const cellKey = (x, z) => `${Math.floor(x / CELL)},${Math.floor(z / CELL)}`;
for (const rd of roadsDoc.roads) {
  if (FOOT.has(rd.class)) continue;
  for (let i = 1; i < rd.pts.length; i++) {
    const [ax, az] = rd.pts[i - 1];
    const [bx, bz] = rd.pts[i];
    const seg = { ax, az, bx, bz, width: rd.width };
    for (const [x, z] of [[ax, az], [bx, bz], [(ax + bx) / 2, (az + bz) / 2]]) {
      const k = cellKey(x, z);
      if (!hash.has(k)) hash.set(k, []);
      const arr = hash.get(k);
      if (arr[arr.length - 1] !== seg) arr.push(seg);
    }
  }
}

function widthAt(x, z, tx, tz) {
  let best = Infinity;
  let width = null;
  const ci = Math.floor(x / CELL);
  const cj = Math.floor(z / CELL);
  for (let di = -1; di <= 1; di++) for (let dj = -1; dj <= 1; dj++) {
    for (const s of hash.get(`${ci + di},${cj + dj}`) ?? []) {
      const dx = s.bx - s.ax;
      const dz = s.bz - s.az;
      const L = Math.hypot(dx, dz) || 1;
      if (Math.abs((dx * tx + dz * tz) / L) < PARALLEL_DOT) continue;
      const L2 = L * L;
      let t = ((x - s.ax) * dx + (z - s.az) * dz) / L2;
      t = Math.max(0, Math.min(1, t));
      const d = Math.hypot(x - (s.ax + dx * t), z - (s.az + dz * t));
      if (d < best) { best = d; width = s.width; }
    }
  }
  return best <= MATCH_DIST_M ? width : null;
}

// ── reverse-edge lookup for two-way detection ───────────────────────────────
const byEndpoints = new Map(net.edges.map((e) => [`${e.from}->${e.to}`, e]));
const laneById = new Map(net.lanes.map((l) => [l.id, l]));

let matched = 0;
let reoffset = 0;
for (const e of net.edges) {
  const rev = byEndpoints.get(`${e.to}->${e.from}`);
  const lane0 = laneById.get(e.lanes[0]);
  // exact inverse of the original bake: lane 0 sat +0.5·LANE_WIDTH right.
  const center = offsetRight(lane0.pts, -0.5 * LANE_WIDTH);
  // sample the matched width along the centreline (median of 3 probes)
  const widths = [];
  for (const f of [0.25, 0.5, 0.75]) {
    const i = Math.min(center.length - 2, Math.floor((center.length - 1) * f));
    const [ax, az] = center[i];
    const [bx, bz] = center[i + 1];
    const L = Math.hypot(bx - ax, bz - az) || 1;
    const w = widthAt((ax + bx) / 2, (az + bz) / 2, (bx - ax) / L, (bz - az) / L);
    if (w != null) widths.push(w);
  }
  widths.sort((a, b) => a - b);
  const widthM = widths.length ? widths[Math.floor(widths.length / 2)] : null;
  if (widthM != null) matched++;
  const offs = laneOffsets({
    laneCount: e.laneCount,
    reverseLaneCount: rev ? rev.laneCount : 0,
    widthM: widthM ?? undefined,
  });
  for (let idx = 0; idx < e.lanes.length; idx++) {
    const lane = laneById.get(e.lanes[idx]);
    const oldOff = (idx + 0.5) * LANE_WIDTH;
    if (Math.abs(offs[idx] - oldOff) < 1e-9) continue;
    lane.pts = offsetRight(center, offs[idx]).map(([x, z]) => [r2(x), r2(z)]);
    lane.lengthM = r2(polylineLength(lane.pts));
    reoffset++;
  }
}

writeFileSync(NET, JSON.stringify(net));
console.log(
  `re-derived lane offsets: ${matched}/${net.edges.length} edges width-matched, ` +
  `${reoffset}/${net.lanes.length} lanes re-offset — now run rederive-traffic-conflicts.mjs + demand-gen`,
);
