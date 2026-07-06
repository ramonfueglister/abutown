// scripts/geo/lib/gradewidths.mjs
//
// Bake-side twin of src/diorama/traffic/corridorWidths.ts `corridorWidths`.
// Plain Node .mjs cannot import the TS module during the bake, so this is a
// line-by-line port (same constants, same spatial-hash + midpoint-projection
// algorithm). Consumed by the terrain-grading pass (grading.mjs) so the
// grading corridor width respects the traffic net's lane extents, not the
// OSM ribbon width. CORRIDOR width only — ribbons render at their real OSM
// width per #134; the lane floor never feeds rendering.
//
// MIRROR: changes here must be mirrored in src/diorama/traffic/corridorWidths.ts
// (parity test tests/geo/gradewidths-parity.test.ts).
//
// No fallbacks: a malformed trafficNetDoc (missing edges/lanes arrays)
// throws rather than silently defaulting.

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

/** Undirected node-pair key (order-independent). */
function pairKey(a, b) {
  return a < b ? `${a}_${b}` : `${b}_${a}`;
}

/**
 * Build the corrected carriage widths. Pure over its inputs. Returns a NEW
 * array, one width per input road (footways/rails are passed through unchanged
 * by the caller — this only matters for carriage classes, but it is safe to run
 * on any road: a footway rarely matches a traffic edge and, if it did, the
 * `max` keeps its OSM width unless it is genuinely narrower than the lanes).
 *
 * @param {{class: string, width: number, pts: number[][]}[]} roads
 * @param {{edges: {id:number, from:number, to:number, laneCount:number, lanes:number[]}[], lanes:{id:number, edge:number, pts:number[][]}[]}} net
 * @returns {number[]}
 */
export function laneFloorWidths(roads, net) {
  if (!net || !Array.isArray(net.edges) || !Array.isArray(net.lanes)) {
    throw new Error(
      'laneFloorWidths: malformed trafficNetDoc — expected { edges: [], lanes: [] }',
    );
  }

  // 1) Total lanes per undirected node pair (sum both directed edges).
  const lanesOfPair = new Map();
  for (const e of net.edges) {
    const k = pairKey(e.from, e.to);
    lanesOfPair.set(k, (lanesOfPair.get(k) ?? 0) + e.laneCount);
  }

  // 2) Index every lane segment into a coarse spatial hash, tagged with the
  //    node-pair lane total. edge id -> node pair via the edges list.
  const pairOfEdge = new Map();
  for (const e of net.edges) pairOfEdge.set(e.id, pairKey(e.from, e.to));

  const CELL = 16; // spatial-hash cell (m); MATCH_DIST_M is well under this
  const buckets = new Map();
  const cellKey = (x, z) => `${Math.floor(x / CELL)}_${Math.floor(z / CELL)}`;
  const addToBucket = (seg) => {
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
  const out = new Array(roads.length);
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
function pointSegDist2(px, pz, ax, az, bx, bz) {
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
