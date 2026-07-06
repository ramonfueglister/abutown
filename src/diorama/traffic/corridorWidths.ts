// src/diorama/traffic/corridorWidths.ts
//
// CORRIDOR width source — terrain-side only, NEVER for ribbon rendering.
//
// #134 removed the FIX-D1 render-width floor (the old roadWidths.ts
// `correctRoadWidths`): ribbons draw at their real OSM width and the traffic
// kernel bakes width-aware lane offsets instead. That philosophy governs the
// RENDER width. The TERRAIN corridor (grading, discard mask, corridor-snap,
// runtime ground sampler, §9 burial metric, §4.4 traffic-lane coverage) still
// must cover the traffic lane pairs actually driven on a street — a two-way
// pair of baked lanes needs a graded bench under it even where the OSM ribbon
// is narrower. So the corridor width is
//   max(OSM width, lanes_both_directions × LANE_W + SHOULDER_M)
// wherever a road matches a trafficnet edge — the same deterministic geometric
// match the old D1 used, now scoped exclusively to corridor consumers. The
// visible ribbon→mask gap this opens is covered by the platform apron
// (Bankett), by design.
//
// Matching: build a coarse spatial hash of trafficnet lane segments; for each
// road, project its midpoint to the nearest lane segment within MATCH_DIST_M
// and require the tangents to be near-parallel (a perpendicular crossing street
// must not borrow a lane count). The lane count is summed over BOTH directed
// edges sharing that undirected node pair, so a two-way street floors at 2×3.0.
// Deterministic: same inputs → same widths, stable across sessions.
//
// MIRROR: changes here must be mirrored in scripts/geo/lib/gradewidths.mjs
// (the plain-Node bake-side twin; parity test
// tests/geo/gradewidths-parity.test.ts).

import type { RoadPath } from '../ksw/geo/geoData';

/** Baked traffic lane width (metres). Must equal the `LANE_WIDTH` upper bound
 * the traffic-net bake uses (trafficnet.mjs). */
export const LANE_W = 3.0;
/** Extra shoulder/gutter width added on top of the lane pairs so the corridor
 * slightly overhangs the outermost lane rather than clipping it. */
export const SHOULDER_M = 0.8;
/** Max centreline distance (m) for a road midpoint to claim a traffic edge. */
export const MATCH_DIST_M = 5.0;
/** Tangents must agree to within this dot (|cos| ≥ this) to match — rejects a
 * perpendicular crossing street borrowing an unrelated edge's lane count. */
const PARALLEL_DOT = 0.5; // cos 60°

interface RawEdge {
  id: number;
  from: number;
  to: number;
  laneCount: number;
  lanes: number[];
}
interface RawLaneDoc {
  id: number;
  edge: number;
  pts: number[][];
}
export interface TrafficNetDoc {
  edges: RawEdge[];
  lanes: RawLaneDoc[];
}

/** One indexed lane segment: its two endpoints, unit tangent, and the total
 * lane count of the undirected node pair it belongs to. */
interface Seg {
  ax: number;
  az: number;
  bx: number;
  bz: number;
  tx: number; // unit tangent
  tz: number;
  lanes: number; // total lanes both directions at this edge's node pair
}

/** Undirected node-pair key (order-independent). */
function pairKey(a: number, b: number): string {
  return a < b ? `${a}_${b}` : `${b}_${a}`;
}

/**
 * Build the corridor widths. Pure over its inputs. Returns a NEW array, one
 * width per input road (footways/rails are passed through unchanged by the
 * caller — this only matters for carriage classes, but it is safe to run on
 * any road: a footway rarely matches a traffic edge and, if it did, the `max`
 * keeps its OSM width unless it is genuinely narrower than the lanes).
 *
 * `net` is a required parameter (no static trafficnet.json import here) so the
 * module stays bundle-light and callers control how the net is loaded.
 */
export function corridorWidths(roads: RoadPath[], net: TrafficNetDoc): number[] {
  // 1) Total lanes per undirected node pair (sum both directed edges).
  const lanesOfPair = new Map<string, number>();
  for (const e of net.edges) {
    const k = pairKey(e.from, e.to);
    lanesOfPair.set(k, (lanesOfPair.get(k) ?? 0) + e.laneCount);
  }

  // 2) Index every lane segment into a coarse spatial hash, tagged with the
  //    node-pair lane total. edge id -> node pair via the edges list.
  const pairOfEdge = new Map<number, string>();
  for (const e of net.edges) pairOfEdge.set(e.id, pairKey(e.from, e.to));

  const CELL = 16; // spatial-hash cell (m); MATCH_DIST_M is well under this
  const buckets = new Map<string, Seg[]>();
  const cellKey = (x: number, z: number): string =>
    `${Math.floor(x / CELL)}_${Math.floor(z / CELL)}`;
  const addToBucket = (seg: Seg): void => {
    // register the segment in every cell its bbox (padded by MATCH_DIST) touches
    const minX = Math.min(seg.ax, seg.bx) - MATCH_DIST_M;
    const maxX = Math.max(seg.ax, seg.bx) + MATCH_DIST_M;
    const minZ = Math.min(seg.az, seg.bz) - MATCH_DIST_M;
    const maxZ = Math.max(seg.az, seg.bz) + MATCH_DIST_M;
    for (let cx = Math.floor(minX / CELL); cx <= Math.floor(maxX / CELL); cx++) {
      for (let cz = Math.floor(minZ / CELL); cz <= Math.floor(maxZ / CELL); cz++) {
        const k = `${cx}_${cz}`;
        let arr = buckets.get(k);
        if (!arr) buckets.set(k, (arr = []));
        arr.push(seg);
      }
    }
  };

  for (const lane of net.lanes) {
    const pk = pairOfEdge.get(lane.edge);
    if (pk === undefined) continue;
    const lanes = lanesOfPair.get(pk) ?? 0;
    for (let i = 1; i < lane.pts.length; i++) {
      const a = lane.pts[i - 1];
      const b = lane.pts[i];
      const dx = b[0] - a[0];
      const dz = b[1] - a[1];
      const len = Math.hypot(dx, dz) || 1;
      addToBucket({
        ax: a[0],
        az: a[1],
        bx: b[0],
        bz: b[1],
        tx: dx / len,
        tz: dz / len,
        lanes,
      });
    }
  }

  // 3) For each road, project its representative midpoint to the nearest
  //    parallel lane segment; floor its width to the lane pairs if matched.
  const out: number[] = new Array(roads.length);
  for (let r = 0; r < roads.length; r++) {
    const road = roads[r];
    out[r] = road.width;
    const pts = road.pts;
    if (pts.length < 2) continue;

    // Representative point + tangent: the middle vertex of the polyline.
    const mi = pts.length >> 1;
    const mx = pts[mi][0];
    const mz = pts[mi][1];
    const pa = pts[Math.max(0, mi - 1)];
    const pb = pts[Math.min(pts.length - 1, mi + 1)];
    let rtx = pb[0] - pa[0];
    let rtz = pb[1] - pa[1];
    const rl = Math.hypot(rtx, rtz) || 1;
    rtx /= rl;
    rtz /= rl;

    const bucket = buckets.get(cellKey(mx, mz));
    if (!bucket) continue;

    let bestD2 = MATCH_DIST_M * MATCH_DIST_M;
    let bestLanes = 0;
    for (const seg of bucket) {
      // parallel-tangent gate (allow either direction)
      const dot = Math.abs(seg.tx * rtx + seg.tz * rtz);
      if (dot < PARALLEL_DOT) continue;
      const d2 = pointSegDist2(mx, mz, seg.ax, seg.az, seg.bx, seg.bz);
      if (d2 < bestD2) {
        bestD2 = d2;
        bestLanes = seg.lanes;
      }
    }
    if (bestLanes > 0) {
      const need = bestLanes * LANE_W + SHOULDER_M;
      if (need > out[r]) out[r] = need;
    }
  }
  return out;
}

/** Squared distance from point (px,pz) to segment (ax,az)-(bx,bz). */
function pointSegDist2(
  px: number,
  pz: number,
  ax: number,
  az: number,
  bx: number,
  bz: number,
): number {
  const dx = bx - ax;
  const dz = bz - az;
  const l2 = dx * dx + dz * dz;
  let t = l2 > 0 ? ((px - ax) * dx + (pz - az) * dz) / l2 : 0;
  t = t < 0 ? 0 : t > 1 ? 1 : t;
  const cx = ax + t * dx;
  const cz = az + t * dz;
  const ex = px - cx;
  const ez = pz - cz;
  return ex * ex + ez * ez;
}
