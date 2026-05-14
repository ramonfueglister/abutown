import { describe, expect, it } from 'vitest';
import {
  CURVE_EASING_WINDOW,
  JUNCTION_SPEED_FACTOR,
  MIN_VEHICLE_GAP_TILES,
  TURN_SPEED_FACTOR,
  vehicleFollowingAdvanceLimit,
  vehicleFollowingSpeedFactor,
  vehicleRenderPose,
  vehicleSpeedFactor,
} from '../../src/render/vehicleMotion';

describe('vehicleRenderPose', () => {
  it('keeps straight segments linear for ECS-friendly rendering', () => {
    const pose = vehicleRenderPose({
      path: [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 2, y: 0 },
      ],
      offset: 0.5,
    });

    expect(pose.position).toEqual({ x: 0.5, y: 0 });
    expect(pose.headingDelta).toEqual({ x: 1, y: 0 });
  });

  it('eases into a ninety-degree turn before reaching the junction tile', () => {
    const pose = vehicleRenderPose({
      path: [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 1, y: 1 },
      ],
      offset: 1 - CURVE_EASING_WINDOW / 2,
    });

    expect(pose.position.x).toBeLessThan(1);
    expect(pose.position.y).toBeGreaterThan(0);
    expect(pose.position.y).toBeLessThan(0.1);
    expect(pose.headingDelta.x).toBeGreaterThan(0);
    expect(pose.headingDelta.y).toBeGreaterThan(0);
    expect(pose.headingDelta.x).toBeGreaterThan(pose.headingDelta.y);
  });

  it('eases out of a ninety-degree turn after leaving the junction tile', () => {
    const pose = vehicleRenderPose({
      path: [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 1, y: 1 },
      ],
      offset: 1 + CURVE_EASING_WINDOW / 2,
    });

    expect(pose.position.x).toBeGreaterThan(0.9);
    expect(pose.position.x).toBeLessThan(1);
    expect(pose.position.y).toBeGreaterThan(0);
    expect(pose.position.y).toBeGreaterThan(0.1);
    expect(pose.headingDelta.x).toBeGreaterThan(0);
    expect(pose.headingDelta.y).toBeGreaterThan(0);
    expect(pose.headingDelta.y).toBeGreaterThan(pose.headingDelta.x);
  });

  it('does not smooth u-turn ping-pong path endpoints into sideways arcs', () => {
    const pose = vehicleRenderPose({
      path: [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 0, y: 0 },
      ],
      offset: 1 - CURVE_EASING_WINDOW / 2,
    });

    expect(pose.position.y).toBe(0);
    expect(pose.headingDelta).toEqual({ x: 1, y: 0 });
  });

  it('keeps straight road speed unchanged for ECS-friendly progress updates', () => {
    expect(vehicleSpeedFactor({
      path: [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 2, y: 0 },
      ],
      offset: 0.5,
    })).toBe(1);
  });

  it('brakes before a ninety-degree curve and accelerates after it', () => {
    const path = [
      { x: 0, y: 0 },
      { x: 1, y: 0 },
      { x: 1, y: 1 },
      { x: 1, y: 2 },
    ];

    expect(vehicleSpeedFactor({ path, offset: 0.2 })).toBe(1);
    expect(vehicleSpeedFactor({ path, offset: 0.7 })).toBeLessThan(1);
    expect(vehicleSpeedFactor({ path, offset: 1 })).toBe(TURN_SPEED_FACTOR);
    expect(vehicleSpeedFactor({ path, offset: 1.25 })).toBeGreaterThan(TURN_SPEED_FACTOR);
    expect(vehicleSpeedFactor({ path, offset: 1.8 })).toBe(1);
  });

  it('adds a softer visual caution slowdown for marked junction tiles', () => {
    const path = [
      { x: 0, y: 0 },
      { x: 1, y: 0 },
      { x: 2, y: 0 },
    ];

    expect(vehicleSpeedFactor({
      path,
      offset: 0.6,
      cautionTileKeys: new Set(['1:0']),
    })).toBeLessThan(1);
    expect(vehicleSpeedFactor({
      path,
      offset: 1,
      cautionTileKeys: new Set(['1:0']),
    })).toBe(JUNCTION_SPEED_FACTOR);
  });

  it('lets vehicles drive freely when no leader is on their path', () => {
    expect(vehicleFollowingSpeedFactor({
      offset: 4,
      pathLength: 12,
    })).toBe(1);
  });

  it('stops a vehicle before it visually overlaps its leader', () => {
    expect(vehicleFollowingSpeedFactor({
      offset: 4,
      leaderOffset: 4 + MIN_VEHICLE_GAP_TILES / 2,
      pathLength: 12,
    })).toBe(0);
  });

  it('slows a vehicle down as it approaches its leader', () => {
    const factor = vehicleFollowingSpeedFactor({
      offset: 4,
      leaderOffset: 5.05,
      pathLength: 12,
    });

    expect(factor).toBeGreaterThan(0);
    expect(factor).toBeLessThan(1);
  });

  it('handles leaders across the path loop seam', () => {
    expect(vehicleFollowingSpeedFactor({
      offset: 11.75,
      leaderOffset: 0.15,
      pathLength: 12,
    })).toBe(0);
  });

  it('caps progress so a follower cannot cross into its leader gap', () => {
    expect(vehicleFollowingAdvanceLimit({
      offset: 4,
      leaderOffset: 5,
      pathLength: 12,
    })).toBeCloseTo(1 - MIN_VEHICLE_GAP_TILES, 3);
    expect(vehicleFollowingAdvanceLimit({
      offset: 11.75,
      leaderOffset: 0.75,
      pathLength: 12,
    })).toBeCloseTo(1 - MIN_VEHICLE_GAP_TILES, 3);
  });
});
