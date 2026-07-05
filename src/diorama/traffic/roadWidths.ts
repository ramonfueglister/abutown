// src/diorama/traffic/roadWidths.ts
//
// FIX D1 — carriage render width fits the traffic lane pairs.
//
// The baked roads.json ribbons are drawn at their OSM `width`, which for many
// residential/service streets is 2.2–5 m. But the traffic kernel bakes those
// same streets as a two-way pair of 3.0 m lanes (data/winterthur/trafficnet.json).
// A car is 1.9 m wide; two opposing 1.9 m cars span 3.8 m of tarmac, so on a
// ribbon narrower than the baked lanes the cars visibly overhang onto the grass
// (bug-report item 4). The mesh/kernel/lane sizes are mutually consistent
// (4.2/4.5 m long, 1.9 m in a 3.0 m lane); only the *ribbon render width* is
// wrong for the lane count.
//
// This module widens the carriage ribbon at RUNTIME to at least
//   lanes_both_directions * LANE_W + SHOULDER_M
// wherever a road matches a trafficnet edge. It is a pure, deterministic
// geometric match — no rebake of roads.json (whose fetch is now Gemeinde-wide
// and whose plate-scoped bake is fragile, see FIX-C rebake note), no
// hard dependency on ?traffic=1 (the trafficnet.json asset is already in the
// bundle via trafficClient; we import it the same static way). Roads with no
// nearby traffic edge keep their OSM width untouched.
//
// Matching: build a coarse spatial hash of trafficnet lane segments; for each
// road, project its midpoint to the nearest lane segment within MATCH_DIST_M
// and require the tangents to be near-parallel (a perpendicular crossing street
// must not borrow a lane count). The lane count is summed over BOTH directed
// edges sharing that undirected node pair, so a two-way street floors at 2×3.0.
// Deterministic: same inputs → same widths, stable across sessions.

import type { RoadPath } from '../ksw/geo/geoData';
import trafficNetJson from '../../../data/winterthur/trafficnet.json';

/** Baked traffic lane width (metres). Must equal the `LANE_W`/lane offset the
 * traffic-net bake uses (bug report: lanes baked at 3.0 m). */
export const LANE_W = 3.0;
/** Extra shoulder/gutter width added on top of the lane pairs so the drawn
 * tarmac slightly overhangs the outermost lane rather than clipping it. */
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
interface TrafficNetDoc {
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
 * Build the corrected carriage widths. Pure over its inputs. Returns a NEW
 * array, one width per input road (footways/rails are passed through unchanged
 * by the caller — this only matters for carriage classes, but it is safe to run
 * on any road: a footway rarely matches a traffic edge and, if it did, the
 * `max` keeps its OSM width unless it is genuinely narrower than the lanes).
 */
export function correctRoadWidths(
  roads: RoadPath[],
  net: TrafficNetDoc = trafficNetJson as unknown as TrafficNetDoc,
): number[] {
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
