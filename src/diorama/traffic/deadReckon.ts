// src/diorama/traffic/deadReckon.ts
//
// Browser-side dead-reckoning along the baked lane polylines. This is a direct
// port of the server's arc-length interpolation so the client and server agree
// on where a vehicle is between the 5 Hz frames.
//
// Reference (binding): backend/crates/traffic-net/src/lib.rs :: pos_at
//   * precompute a per-lane cumulative arc-length LUT over the polyline verts;
//   * clamp s to [0, laneLen];
//   * binary-search the LUT for the segment `i` with lut[i] <= s <= lut[i+1]
//     (an EXACT hit at lut[i] resolves to segment i, min'd to pts.len()-2 — so
//     at an interior vertex the tangent is that of the FOLLOWING segment);
//   * pos = a + (b-a)·t, t = (s-lut[i]) / segLen;  tangent = unit(b-a).
//
// The lane geometry is loaded from data/winterthur/trafficnet.json — the SAME
// asset the Rust server bakes from — so the polylines (and therefore the LUT)
// are byte-identical on both sides. The coordinate frame is also identical:
// trafficnet pts are [x, z] in the diorama world frame (both projected via
// scripts/geo/lib/project.mjs, +x east / +z south), so no transform is applied
// (verified once at runtime by trafficClient — see the coord-check log there).

/** Sim tick period in seconds. The server steps the fleet at 10 Hz. */
export const SIM_DT = 0.1;

/** One baked lane: id, declared length (m), and its [x, z] polyline vertices. */
export interface RawLane {
  id: number;
  lengthM: number;
  pts: number[][];
}

/** Query-ready lane geometry: raw pts + the precomputed per-lane arc-length LUT,
 * indexed by lane id for O(1) lookup. */
export interface TrafficNetGeom {
  /** lane id -> polyline vertices ([x, z] each). */
  pts: Map<number, number[][]>;
  /** lane id -> cumulative arc length at each vertex (same length as pts). */
  arcLut: Map<number, number[]>;
}

/** A dead-reckoned vehicle's last-known kinematic state (units already decoded
 * from the wire: s in metres, v in m/s, tickAt in sim ticks). */
export interface VehKinematics {
  lane: number;
  s: number;
  v: number;
  tickAt: number;
}

/** World pose: ground position (x, z) and yaw about +y. */
export interface Pose {
  x: number;
  z: number;
  yaw: number;
}

/** Build the query-ready geometry (with arc-length LUTs) from raw baked lanes.
 * Mirrors `TrafficNet::from_doc`'s `arc_lut` construction exactly. */
export function buildLaneNet(lanes: RawLane[]): TrafficNetGeom {
  const pts = new Map<number, number[][]>();
  const arcLut = new Map<number, number[]>();
  for (const lane of lanes) {
    pts.set(lane.id, lane.pts);
    const acc: number[] = [0];
    let running = 0;
    for (let i = 1; i < lane.pts.length; i++) {
      const dx = lane.pts[i][0] - lane.pts[i - 1][0];
      const dz = lane.pts[i][1] - lane.pts[i - 1][1];
      running += Math.sqrt(dx * dx + dz * dz);
      acc.push(running);
    }
    arcLut.set(lane.id, acc);
  }
  return { pts, arcLut };
}

/** Position + unit tangent at arc length `s` on `lane`. `s` is clamped to
 * [0, laneLen]. Direct port of the server's `pos_at`. */
export function posAt(
  net: TrafficNetGeom,
  lane: number,
  s: number,
): { x: number; z: number; tx: number; tz: number } {
  const pts = net.pts.get(lane);
  const lut = net.arcLut.get(lane);
  if (!pts || !lut || pts.length < 2) {
    // Unknown/degenerate lane — never happens for live vehicles (their lane id
    // came from this same net). Return a null pose rather than throw so one bad
    // frame can't crash the render loop.
    return { x: 0, z: 0, tx: 0, tz: 0 };
  }
  const total = lut[lut.length - 1];
  const sc = Math.min(Math.max(s, 0), total);

  // Segment i with lut[i] <= sc <= lut[i+1]. Reproduce Rust's
  // binary_search_by: exact hit -> that index (min'd to pts.len()-2); miss ->
  // insertion point - 1.
  const lastSeg = pts.length - 2;
  let seg: number;
  const found = binarySearch(lut, sc);
  if (found >= 0) {
    seg = Math.min(found, lastSeg);
  } else {
    const ins = -found - 1; // insertion point (Err(i) in Rust)
    seg = Math.min(Math.max(ins - 1, 0), lastSeg);
  }

  const a = pts[seg];
  const b = pts[seg + 1];
  const segLen = lut[seg + 1] - lut[seg];
  const dx = b[0] - a[0];
  const dz = b[1] - a[1];
  const tanLen = Math.sqrt(dx * dx + dz * dz);
  const tx = tanLen > 1e-9 ? dx / tanLen : 0;
  const tz = tanLen > 1e-9 ? dz / tanLen : 0;

  let x: number;
  let z: number;
  if (segLen > 1e-9) {
    const t = (sc - lut[seg]) / segLen;
    x = a[0] + dx * t;
    z = a[1] + dz * t;
  } else {
    x = a[0];
    z = a[1];
  }
  return { x, z, tx, tz };
}

/** Dead-reckon a vehicle to `nowTick`: advance s by v·Δt·SIM_DT (Δt in ticks,
 * never negative — a stale/skewed clock must not rewind), clamp at the lane end
 * (cars wait rather than overshoot; the next frame corrects), and derive yaw
 * from the lane tangent. Yaw convention matches the diorama's agents: a heading
 * along +x (east) yaws to +PI/2, so yaw = atan2(tangentX, tangentZ). */
export function poseAt(net: TrafficNetGeom, veh: VehKinematics, nowTick: number): Pose {
  const dTicks = Math.max(0, nowTick - veh.tickAt);
  const s = veh.s + veh.v * dTicks * SIM_DT;
  const { x, z, tx, tz } = posAt(net, veh.lane, s);
  const yaw = Math.atan2(tx, tz);
  return { x, z, yaw };
}

/** Rust `slice::binary_search_by` equivalent over a sorted ascending array.
 * Returns the index on an exact match, or `-(insertionPoint) - 1` on a miss
 * (so a negative result encodes the Err(i) insertion point). Matches Rust's
 * behaviour including that with duplicate keys ANY matching index may be
 * returned — for the arc LUT (strictly increasing for non-degenerate lanes)
 * this is unambiguous. */
function binarySearch(arr: number[], target: number): number {
  let lo = 0;
  let hi = arr.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >>> 1;
    const v = arr[mid];
    if (v < target) lo = mid + 1;
    else if (v > target) hi = mid - 1;
    else return mid;
  }
  return -lo - 1;
}
