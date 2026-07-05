// tests/traffic/deadReckon.test.ts
//
// Cross-language contract test: the browser dead-reckoning MUST reproduce the
// server's `pos_at` semantics byte-for-byte (arc-length LUT segment search,
// clamped at both ends, unit tangent per segment). This is the Task-2 review
// carry-forward — the contract is pinned here, against hand-computed values,
// not left to visual inspection.
//
// The reference is backend/crates/traffic-net/src/lib.rs :: pos_at:
//   * s is clamped to [0, laneLen]
//   * segment i chosen so lut[i] <= s <= lut[i+1]; an EXACT hit at lut[i]
//     resolves to segment i (min'd to pts.len()-2), so at an interior vertex
//     the tangent is that of the FOLLOWING segment.
//   * position = a + (b-a)·t, t = (s - lut[i]) / segLen
//   * tangent  = unit(b - a)

import { describe, expect, it } from 'vitest';
import { buildLaneNet, posAt, poseAt, type TrafficNetGeom } from '../../src/diorama/traffic/deadReckon';

// An L-shaped 2-segment lane: (0,0) -> (10,0) -> (10,10).
// Arc LUT = [0, 10, 20]; total length 20.
const L_LANE: TrafficNetGeom = buildLaneNet([
  { id: 7, edge: 7, index: 0, lengthM: 20, pts: [[0, 0], [10, 0], [10, 10]] },
]);

const close = (a: number, b: number, eps = 1e-5): boolean => Math.abs(a - b) <= eps;

describe('posAt — port of server pos_at', () => {
  it('interpolates inside the first segment', () => {
    const { x, z, tx, tz } = posAt(L_LANE, 7, 5);
    expect(close(x, 5)).toBe(true);
    expect(close(z, 0)).toBe(true);
    expect(close(tx, 1)).toBe(true);
    expect(close(tz, 0)).toBe(true);
  });

  it('interpolates inside the second segment', () => {
    const { x, z, tx, tz } = posAt(L_LANE, 7, 15);
    expect(close(x, 10)).toBe(true);
    expect(close(z, 5)).toBe(true);
    expect(close(tx, 0)).toBe(true);
    expect(close(tz, 1)).toBe(true);
  });

  it('at the interior vertex, takes the FOLLOWING segment tangent (exact-hit rule)', () => {
    // s == lut[1] == 10 → segment 1, t = 0 → pos is the corner, tangent turns.
    const { x, z, tx, tz } = posAt(L_LANE, 7, 10);
    expect(close(x, 10)).toBe(true);
    expect(close(z, 0)).toBe(true);
    expect(close(tx, 0)).toBe(true);
    expect(close(tz, 1)).toBe(true);
  });

  it('clamps s below 0 to the lane start', () => {
    const { x, z, tx, tz } = posAt(L_LANE, 7, -5);
    expect(close(x, 0)).toBe(true);
    expect(close(z, 0)).toBe(true);
    expect(close(tx, 1)).toBe(true);
    expect(close(tz, 0)).toBe(true);
  });

  it('clamps s past the lane end (no overshoot) to the terminal vertex', () => {
    const { x, z, tx, tz } = posAt(L_LANE, 7, 25);
    expect(close(x, 10)).toBe(true);
    expect(close(z, 10)).toBe(true);
    expect(close(tx, 0)).toBe(true);
    expect(close(tz, 1)).toBe(true);
  });

  it('start vertex resolves to the first segment', () => {
    const { x, z, tx, tz } = posAt(L_LANE, 7, 0);
    expect(close(x, 0)).toBe(true);
    expect(close(z, 0)).toBe(true);
    expect(close(tx, 1)).toBe(true);
    expect(close(tz, 0)).toBe(true);
  });
});

describe('poseAt — dead-reckoning advance + yaw', () => {
  // A single straight east-west lane so advance is easy to hand-check.
  const STRAIGHT: TrafficNetGeom = buildLaneNet([
    { id: 3, edge: 3, index: 0, lengthM: 100, pts: [[0, 0], [100, 0]] },
  ]);

  it('advances s by v·(now - tickAt)·SIM_DT and yaws from the tangent', () => {
    // veh at s=10, v=5 m/s (quarter-m/s already decoded), tickAt=100.
    // 4 ticks elapsed → dt total = 4 * 0.1 s = 0.4 s → +2 m → s=12.
    const veh = { lane: 3, s: 10, v: 5, tickAt: 100 };
    const pose = poseAt(STRAIGHT, veh, 104);
    expect(close(pose.x, 12)).toBe(true);
    expect(close(pose.z, 0)).toBe(true);
    // heading +x (east). yaw = atan2(tangentX, tangentZ) = atan2(1, 0) = PI/2.
    expect(close(pose.yaw, Math.PI / 2)).toBe(true);
  });

  it('does not overshoot the lane end — waits clamped at the terminal vertex', () => {
    // s=95, v=50 m/s, 10 ticks → +50 m → 145, clamped to 100.
    const veh = { lane: 3, s: 95, v: 50, tickAt: 0 };
    const pose = poseAt(STRAIGHT, veh, 10);
    expect(close(pose.x, 100)).toBe(true);
    expect(close(pose.z, 0)).toBe(true);
  });

  it('never rewinds when nowTick is behind tickAt (clock skew guard)', () => {
    const veh = { lane: 3, s: 40, v: 10, tickAt: 200 };
    const pose = poseAt(STRAIGHT, veh, 190); // 10 ticks in the past
    expect(close(pose.x, 40)).toBe(true); // no negative advance
  });
});
