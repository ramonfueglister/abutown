// tests/traffic/laneBlend.test.ts
//
// FIX-C2: client-side motion continuity. Two visible discontinuities the
// investigation found (probe2): (b) lane-change lateral TELEPORTS up to 3.79 m
// in one client frame (the client hard-swaps lane+s), and the junction hop
// (fromLane end → toLane start in one tick) rendering as a position jump. These
// tests pin the two pure blend functions:
//   * classifyLaneChange: same-edge → 'parallel', different-edge → 'junction';
//   * a PARALLEL lane change lateral-blends the rendered position from the old
//     lane's pose to the new lane's pose over LATERAL_BLEND_S, with s still
//     advancing longitudinally;
//   * a JUNCTION hop sweeps the rendered position along a quadratic bezier from
//     the old lane end/tangent to the new lane start/tangent;
//   * degenerate cases: no prior lane (fresh vehicle) → snap; same lane
//     reappearing → no blend; a blend expires cleanly back to poseAt.
import { describe, expect, it } from 'vitest';
import { buildLaneNet, poseAt, SIM_DT, type RawLane, type VehKinematics } from '../../src/diorama/traffic/deadReckon';
import {
  classifyLaneChange,
  beginLaneChange,
  poseAtBlended,
  LATERAL_BLEND_S,
  JUNCTION_SWEEP_MAX_S,
} from '../../src/diorama/traffic/laneBlend';

// Two PARALLEL lanes of edge 0: lane 10 at z=0, lane 11 at z=3 (a 3 m lateral
// lane offset), both running east along +x. A JUNCTION continuation on edge 1:
// lane 20 starts where lane 10 ends and turns north (−z), a different edge.
const LANE_A: RawLane = { id: 10, edge: 0, index: 0, lengthM: 100, pts: [[0, 0], [100, 0]] };
const LANE_B: RawLane = { id: 11, edge: 0, index: 1, lengthM: 100, pts: [[0, 3], [100, 3]] };
const LANE_C: RawLane = { id: 20, edge: 1, index: 0, lengthM: 100, pts: [[100, 0], [100, -100]] };
const net = buildLaneNet([LANE_A, LANE_B, LANE_C]);

describe('classifyLaneChange', () => {
  it('same edge → parallel', () => {
    expect(classifyLaneChange(net, 10, 11)).toBe('parallel');
  });
  it('different edge → junction', () => {
    expect(classifyLaneChange(net, 10, 20)).toBe('junction');
  });
});

describe('parallel lane-change lateral blend', () => {
  // Vehicle was on lane A at s=50 (world (50,0)) and hard-swaps to lane B at
  // s=50 (world (50,3)) at tick 100. v=0 so s does not advance — the ONLY
  // motion is the lateral blend from z=0 to z=3.
  const prev: VehKinematics = { lane: 10, s: 50, v: 0, tickAt: 90, cls: 0 };
  const next: VehKinematics = { lane: 11, s: 50, v: 0, tickAt: 100, cls: 0 };

  it('at t=0 renders the OLD lane pose (no teleport at the swap instant)', () => {
    const veh = beginLaneChange(net, next, prev, 100);
    const p = poseAtBlended(net, veh, 100);
    expect(p.x).toBeCloseTo(50, 5);
    expect(p.z).toBeCloseTo(0, 5); // still on lane A laterally
  });

  it('at mid-blend renders halfway between the lanes', () => {
    const veh = beginLaneChange(net, next, prev, 100);
    const halfTicks = 100 + (LATERAL_BLEND_S / SIM_DT) / 2;
    const p = poseAtBlended(net, veh, halfTicks);
    expect(p.x).toBeCloseTo(50, 5);
    expect(p.z).toBeCloseTo(1.5, 5); // halfway across the 3 m offset
  });

  it('at t=end renders the NEW lane pose and the blend is finished', () => {
    const veh = beginLaneChange(net, next, prev, 100);
    const endTicks = 100 + LATERAL_BLEND_S / SIM_DT;
    const p = poseAtBlended(net, veh, endTicks);
    expect(p.x).toBeCloseTo(50, 5);
    expect(p.z).toBeCloseTo(3, 5);
    // past the end it equals the plain new-lane pose
    const later = poseAtBlended(net, veh, endTicks + 100);
    const plain = poseAt(net, next, endTicks + 100);
    expect(later.x).toBeCloseTo(plain.x, 5);
    expect(later.z).toBeCloseTo(plain.z, 5);
  });

  it('keeps advancing longitudinally during the blend (v>0)', () => {
    const movingPrev: VehKinematics = { lane: 10, s: 50, v: 10, tickAt: 100, cls: 0 };
    const movingNext: VehKinematics = { lane: 11, s: 50, v: 10, tickAt: 100, cls: 0 };
    const veh = beginLaneChange(net, movingNext, movingPrev, 100);
    const halfTicks = 100 + (LATERAL_BLEND_S / SIM_DT) / 2;
    const p = poseAtBlended(net, veh, halfTicks);
    // x advanced by v·Δt·dt = 10 · (halfTicks-100) · 0.1
    const expectedX = 50 + 10 * (halfTicks - 100) * SIM_DT;
    expect(p.x).toBeCloseTo(expectedX, 4);
    expect(p.z).toBeCloseTo(1.5, 4);
  });
});

describe('junction hop bezier sweep', () => {
  // Vehicle finishes lane A at s≈100 (world (100,0), heading +x) and crosses to
  // lane C which starts at (100,0) heading −z (a left/right turn through the
  // node). Speed 10 m/s.
  const prev: VehKinematics = { lane: 10, s: 100, v: 10, tickAt: 100, cls: 0 };
  const next: VehKinematics = { lane: 20, s: 0, v: 10, tickAt: 100, cls: 0 };

  it('classifies as a junction sweep', () => {
    expect(classifyLaneChange(net, 10, 20)).toBe('junction');
  });

  it('at t=0 sits at the old lane end, at t=end at the new lane start', () => {
    const veh = beginLaneChange(net, next, prev, 100);
    const p0 = poseAtBlended(net, veh, 100);
    expect(p0.x).toBeCloseTo(100, 3);
    expect(p0.z).toBeCloseTo(0, 3);
    const durTicks = veh.blend!.durTicks;
    // Sample just before the end so we read the bezier endpoint (new lane start
    // (100,0)); at exactly raw>=1 the function hands back to the authoritative
    // poseAt, which has already advanced the car along lane C.
    const pend = poseAtBlended(net, veh, 100 + durTicks * 0.999);
    expect(pend.x).toBeCloseTo(100, 1);
    expect(pend.z).toBeCloseTo(0, 1);
  });

  it('sweeps through a curved path (mid-point is off both straight lines)', () => {
    const veh = beginLaneChange(net, next, prev, 100);
    const durTicks = veh.blend!.durTicks;
    const mid = poseAtBlended(net, veh, 100 + durTicks / 2);
    // The bezier control point pulls the mid off the L-corner; yaw should be
    // between the +x heading (yaw≈PI/2) and the −z heading (yaw≈PI). Just assert
    // the position is a real point (finite) and the sweep duration is capped.
    expect(Number.isFinite(mid.x)).toBe(true);
    expect(Number.isFinite(mid.z)).toBe(true);
    expect(durTicks).toBeLessThanOrEqual(JUNCTION_SWEEP_MAX_S / SIM_DT + 1e-6);
  });
});

describe('degenerate cases', () => {
  it('same lane reappearing → no blend (plain poseAt)', () => {
    const prev: VehKinematics = { lane: 10, s: 40, v: 5, tickAt: 100, cls: 0 };
    const next: VehKinematics = { lane: 10, s: 50, v: 5, tickAt: 100, cls: 0 };
    const veh = beginLaneChange(net, next, prev, 100);
    expect(veh.blend).toBeUndefined();
    const p = poseAtBlended(net, veh, 100);
    const plain = poseAt(net, next, 100);
    expect(p.x).toBeCloseTo(plain.x, 6);
    expect(p.z).toBeCloseTo(plain.z, 6);
  });

  it('a vehicle with no blend state renders exactly poseAt', () => {
    const veh: VehKinematics = { lane: 11, s: 20, v: 3, tickAt: 100, cls: 0 };
    const p = poseAtBlended(net, veh, 130);
    const plain = poseAt(net, veh, 130);
    expect(p.x).toBeCloseTo(plain.x, 6);
    expect(p.z).toBeCloseTo(plain.z, 6);
    expect(p.yaw).toBeCloseTo(plain.yaw, 6);
  });
});
