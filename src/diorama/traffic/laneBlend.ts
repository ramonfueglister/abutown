// src/diorama/traffic/laneBlend.ts
//
// FIX-C2 — client-side motion continuity for lane changes and junction hops.
//
// The wire delivers a vehicle's authoritative kinematics (lane, s, v, tickAt).
// When the sim re-seats a vehicle onto a new lane — either a MOBIL lane change
// to a PARALLEL lane of the same edge, or a junction crossing that hops it from
// one edge's lane end to the next edge's lane start in a single tick — the raw
// `poseAt` reads the new lane immediately, so the rendered car TELEPORTS: up to
// ~3.8 m sideways on a lane change (the investigation found 28 such teleports),
// or across the node on a junction hop (rendering as "driving through" the
// intersection instead of turning through it).
//
// This module adds a PURE, testable blend that smooths the RENDERED pose over a
// fraction of a second without touching the authoritative state:
//   * PARALLEL lane change → lateral blend. Both the old and new lane poses are
//     dead-reckoned forward in parallel over the blend, and the rendered point
//     lerps laterally from one to the other over LATERAL_BLEND_S. Longitudinal
//     motion continues normally (s advances on both lanes), so the car drifts
//     diagonally into the new lane instead of snapping across it.
//   * JUNCTION hop → quadratic-bezier sweep. The rendered point follows a bezier
//     from the old lane end (with its exit tangent) to the new lane start (with
//     its entry tangent); the control point is the tangent-line intersection
//     (midpoint fallback). The car visibly TURNS through the intersection — a
//     Cities-Skylines-style sweep — over a duration derived from speed and the
//     hop gap, capped at JUNCTION_SWEEP_MAX_S.
//
// A generation change (wire id changes → a genuinely different vehicle in a
// recycled slot) never reaches `beginLaneChange` with a prior pose, because the
// prior pose is looked up by the SAME id in the vehicle table; a new id has no
// prior entry, so it snaps (no blend) — exactly the desired teleport-heal
// behaviour.

import { poseAt, posAt, SIM_DT, type Pose, type TrafficNetGeom, type VehKinematics } from './deadReckon';

/** Lateral lane-change blend duration (seconds). ~0.7 s reads as a deliberate
 * merge without lagging the authoritative position. */
export const LATERAL_BLEND_S = 0.7;

/** Cap (seconds) on the junction sweep. The actual duration is derived from
 * speed and the hop gap but never exceeds this, so a slow/stopped car still
 * completes its turn promptly rather than crawling a frozen arc. */
export const JUNCTION_SWEEP_MAX_S = 0.7;

/** Floor (seconds) on the junction sweep. Some junctions are stitched from very
 * short internal edges whose lane end nearly coincides with the next lane's
 * start; a purely gap/speed-derived duration would collapse to ~0 there and the
 * hop would still render as a snap. Flooring the sweep keeps even a tiny hop
 * spread over a few frames so the motion stays continuous. */
export const JUNCTION_SWEEP_MIN_S = 0.25;

export type LaneChangeKind = 'parallel' | 'junction';

/** Classify a lane change: PARALLEL when both lanes belong to the same edge (a
 * lateral shift), JUNCTION otherwise (a hop across a node onto a new edge). If
 * edge info is missing for either lane we conservatively treat it as a junction
 * hop (the bezier sweep degrades gracefully to a short straight when the
 * endpoints are close). */
export function classifyLaneChange(net: TrafficNetGeom, fromLane: number, toLane: number): LaneChangeKind {
  const ea = net.edgeOf.get(fromLane);
  const eb = net.edgeOf.get(toLane);
  if (ea !== undefined && eb !== undefined && ea === eb) return 'parallel';
  return 'junction';
}

/** Build the post-change `VehKinematics` (a shallow copy of `next`) with a
 * blend state attached, given the vehicle's PRIOR kinematics (`prev`, from the
 * vehicle table before this upsert) and the current sim tick. Returns `next`
 * unchanged (no blend) when the lane did not actually change. Pure — allocates
 * a new object, never mutates its inputs. */
export function beginLaneChange(
  net: TrafficNetGeom,
  next: VehKinematics,
  prev: VehKinematics,
  nowTick: number,
): VehKinematics {
  if (prev.lane === next.lane) return { ...next, blend: undefined };

  const kind = classifyLaneChange(net, prev.lane, next.lane);
  if (kind === 'parallel') {
    return {
      ...next,
      blend: {
        kind: 'lateral',
        startTick: nowTick,
        durTicks: LATERAL_BLEND_S / SIM_DT,
        prev: { lane: prev.lane, s: prev.s, v: prev.v, tickAt: prev.tickAt },
      },
    };
  }

  // Junction sweep: departure = old lane pose+tangent at the change instant;
  // arrival = new lane start pose+tangent.
  const dep = posAt(net, prev.lane, prev.s + prev.v * Math.max(0, nowTick - prev.tickAt) * SIM_DT);
  const arr = posAt(net, next.lane, next.s);
  const p0: [number, number] = [dep.x, dep.z];
  const p1: [number, number] = [arr.x, arr.z];
  const t0: [number, number] = [dep.tx, dep.tz];
  const t1: [number, number] = [arr.tx, arr.tz];
  const ctrl = tangentIntersection(p0, t0, p1, t1);

  const gap = Math.hypot(p1[0] - p0[0], p1[1] - p0[1]);
  const speed = Math.max(next.v, 1e-3);
  // Duration derived from speed and gap, clamped into [MIN, MAX] so it neither
  // collapses to a snap on a near-zero micro-edge hop nor crawls on a slow car.
  const durS = Math.min(JUNCTION_SWEEP_MAX_S, Math.max(JUNCTION_SWEEP_MIN_S, gap / speed));
  const durTicks = durS / SIM_DT;

  return {
    ...next,
    blend: { kind: 'sweep', startTick: nowTick, durTicks, p0, ctrl, p1, t0, t1 },
  };
}

/** The rendered pose for a vehicle, applying an active blend if present and not
 * yet expired, else the plain dead-reckoned `poseAt`. Pure. */
export function poseAtBlended(net: TrafficNetGeom, veh: VehKinematics, nowTick: number): Pose {
  const b = veh.blend;
  if (!b) return poseAt(net, veh, nowTick);

  const raw = (nowTick - b.startTick) / b.durTicks;
  if (raw >= 1 || raw < 0) return poseAt(net, veh, nowTick); // finished / not started → authoritative
  const alpha = smoothstep(raw);

  if (b.kind === 'lateral' && b.prev) {
    // Old-lane pose (dead-reckoned in parallel) and new-lane pose at `nowTick`.
    const oldPose = poseAt(
      net,
      { lane: b.prev.lane, s: b.prev.s, v: b.prev.v, tickAt: b.prev.tickAt, cls: veh.cls },
      nowTick,
    );
    const newPose = poseAt(
      net,
      { lane: veh.lane, s: veh.s, v: veh.v, tickAt: veh.tickAt, cls: veh.cls },
      nowTick,
    );
    return {
      x: lerp(oldPose.x, newPose.x, alpha),
      z: lerp(oldPose.z, newPose.z, alpha),
      // yaw follows the new lane once we are past the halfway mark, blended
      // before that — cars point where they're going as they settle in.
      yaw: lerpAngle(oldPose.yaw, newPose.yaw, alpha),
    };
  }

  if (b.kind === 'sweep' && b.p0 && b.ctrl && b.p1) {
    const x = bezier(b.p0[0], b.ctrl[0], b.p1[0], alpha);
    const z = bezier(b.p0[1], b.ctrl[1], b.p1[1], alpha);
    const dx = bezierDeriv(b.p0[0], b.ctrl[0], b.p1[0], alpha);
    const dz = bezierDeriv(b.p0[1], b.ctrl[1], b.p1[1], alpha);
    // Fall back to the arrival tangent if the derivative is ~0 (degenerate arc).
    const yaw =
      Math.hypot(dx, dz) > 1e-6
        ? Math.atan2(dx, dz)
        : b.t1
          ? Math.atan2(b.t1[0], b.t1[1])
          : poseAt(net, veh, nowTick).yaw;
    return { x, z, yaw };
  }

  return poseAt(net, veh, nowTick);
}

/** The intersection of the two tangent lines p0+u·t0 and p1+v·t1, used as the
 * bezier control point. Falls back to the segment midpoint when the tangents
 * are near-parallel (no well-conditioned intersection). */
function tangentIntersection(
  p0: [number, number],
  t0: [number, number],
  p1: [number, number],
  t1: [number, number],
): [number, number] {
  // Solve p0 + a·t0 = p1 + b·t1  →  a·t0 − b·t1 = p1 − p0.
  const det = t0[0] * -t1[1] - -t1[0] * t0[1];
  if (Math.abs(det) < 1e-6) {
    return [(p0[0] + p1[0]) / 2, (p0[1] + p1[1]) / 2];
  }
  const rx = p1[0] - p0[0];
  const rz = p1[1] - p0[1];
  const a = (rx * -t1[1] - -t1[0] * rz) / det;
  const cx = p0[0] + a * t0[0];
  const cz = p0[1] + a * t0[1];
  return [cx, cz];
}

function bezier(a: number, c: number, b: number, t: number): number {
  const mt = 1 - t;
  return mt * mt * a + 2 * mt * t * c + t * t * b;
}
function bezierDeriv(a: number, c: number, b: number, t: number): number {
  return 2 * (1 - t) * (c - a) + 2 * t * (b - c);
}
function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}
/** Shortest-arc angular lerp (handles the ±π wrap). */
function lerpAngle(a: number, b: number, t: number): number {
  let d = b - a;
  while (d > Math.PI) d -= 2 * Math.PI;
  while (d < -Math.PI) d += 2 * Math.PI;
  return a + d * t;
}
/** Cubic smoothstep easing so the blend eases in and out (no velocity jump at
 * the endpoints). */
function smoothstep(t: number): number {
  return t * t * (3 - 2 * t);
}
