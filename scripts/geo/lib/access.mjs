// Zugangspunkt pro Gebäude: nächste befahrbare Kante (Klasse ≤6) in 80 m,
// Fallback Fußweg (7–8), sonst Sentinel. Segment-Lot + Bogenlängen-Offset.
// Grid-Bucketing (50 m) über Kanten-Segmente — O(n), wie das Door-Muster.
const NONE = 0xffffffff;
const CELL = 50;

// Shared primitive (Task 7 refactor): build the segment grid once for a
// graph and expose point→nearest-edge-segment projection. Buckets every
// segment across ALL grid cells its bounding box touches (not just the
// start cell) — this is the fix from commit 15e11ba for long/undensified
// segments whose midpoint can sit many cells from their start point.
// `accessPoints` and `transit.mjs`'s `nearestEdgePoint` both build on this,
// so the bucketing behavior lives in exactly one place.
export function buildSegmentGrid(graph) {
  const grid = new Map(); // "gx,gz" -> [{e, i}]
  const segsOf = (e) => {
    const start = graph.edgePtOffset[e];
    const end = e + 1 < graph.edgePtOffset.length ? graph.edgePtOffset[e + 1] : graph.edgePtX.length;
    return { start, end };
  };
  for (let e = 0; e < graph.edgeA.length; e++) {
    const { start, end } = segsOf(e);
    for (let i = start; i < end - 1; i++) {
      const ax = graph.edgePtX[i], az = graph.edgePtZ[i];
      const bx = graph.edgePtX[i + 1], bz = graph.edgePtZ[i + 1];
      const gx0 = Math.floor(Math.min(ax, bx) / CELL), gx1 = Math.floor(Math.max(ax, bx) / CELL);
      const gz0 = Math.floor(Math.min(az, bz) / CELL), gz1 = Math.floor(Math.max(az, bz) / CELL);
      for (let gx = gx0; gx <= gx1; gx++) for (let gz = gz0; gz <= gz1; gz++) {
        const k = `${gx},${gz}`;
        if (!grid.has(k)) grid.set(k, []);
        grid.get(k).push({ e, i });
      }
    }
  }
  const project = (px, pz, e, i) => {
    const ax = graph.edgePtX[i], az = graph.edgePtZ[i];
    const bx = graph.edgePtX[i + 1], bz = graph.edgePtZ[i + 1];
    const dx = bx - ax, dz = bz - az;
    const L2 = dx * dx + dz * dz || 1e-9;
    const t = Math.max(0, Math.min(1, ((px - ax) * dx + (pz - az) * dz) / L2));
    const qx = ax + t * dx, qz = az + t * dz;
    return { d: Math.hypot(px - qx, pz - qz), t };
  };
  const arcTo = (e, segIdx, t) => {
    const { start } = segsOf(e);
    let arc = 0;
    for (let i = start; i < segIdx; i++)
      arc += Math.hypot(graph.edgePtX[i + 1] - graph.edgePtX[i], graph.edgePtZ[i + 1] - graph.edgePtZ[i]);
    return arc + t * Math.hypot(graph.edgePtX[segIdx + 1] - graph.edgePtX[segIdx], graph.edgePtZ[segIdx + 1] - graph.edgePtZ[segIdx]);
  };
  const candidatesAt = (px, pz) => {
    const gx = Math.floor(px / CELL), gz = Math.floor(pz / CELL);
    const out = [];
    for (let dx = -2; dx <= 2; dx++) for (let dz = -2; dz <= 2; dz++)
      for (const c of grid.get(`${gx + dx},${gz + dz}`) ?? []) out.push(c);
    return out;
  };
  return { candidatesAt, project, arcTo };
}

// Nearest point on any edge whose class passes `classOk` (or, when omitted,
// any edge) within maxDist. Returns { edge, offsetM } or null. Used directly
// by transit.mjs (class ≤6 filter) and internally by accessPoints, which
// keeps its own two-tier drivable-beats-footway ranking on top.
//
// `grid` (optional): a pre-built buildSegmentGrid(graph) result, so a caller
// projecting many points onto the same graph (e.g. transit.mjs projecting
// thousands of stops) builds the segment grid ONCE instead of once per call.
// Rebuilding it per call is O(edges) each time — over a city-scale graph
// (tens of thousands of edges) times thousands of transit stops, that
// quadratic cost was observed to dominate bake-world's wall clock.
export function nearestEdgePoint(graph, x, z, maxDist, classOk = () => true, grid = null) {
  const { candidatesAt, project, arcTo } = grid ?? buildSegmentGrid(graph);
  let best = null;
  for (const { e, i } of candidatesAt(x, z)) {
    if (!classOk(graph.edgeClass[e])) continue;
    const p = project(x, z, e, i);
    if (p.d > maxDist) continue;
    if (!best || p.d < best.d) best = { d: p.d, edge: e, offsetM: arcTo(e, i, p.t) };
  }
  return best ? { edge: best.edge, offsetM: Math.round(best.offsetM * 10) / 10 } : null;
}

export function accessPoints({ graph, footprints }) {
  const { candidatesAt, project, arcTo } = buildSegmentGrid(graph);
  return footprints.map((fp) => {
    const cx = fp.reduce((s, [x]) => s + x, 0) / fp.length;
    const cz = fp.reduce((s, [, z]) => s + z, 0) / fp.length;
    let best = null;
    for (const { e, i } of candidatesAt(cx, cz)) {
      const p = project(cx, cz, e, i);
      if (p.d > 80) continue;
      const drivable = graph.edgeClass[e] <= 6;
      const rank = drivable ? 0 : 1;
      if (!best || rank < best.rank || (rank === best.rank && p.d < best.d))
        best = { rank, d: p.d, edge: e, offsetM: arcTo(e, i, p.t) };
    }
    return best ? { edge: best.edge, offsetM: Math.round(best.offsetM * 10) / 10 } : { edge: NONE, offsetM: 0 };
  });
}
