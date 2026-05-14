import { describe, expect, it } from 'vitest';
import {
  CURVE_EASING_WINDOW,
  JUNCTION_SPEED_FACTOR,
  TURN_SPEED_FACTOR,
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
    expect(pose.headingDelta.x).toBeGreaterThan(0);
    expect(pose.headingDelta.y).toBeGreaterThan(0);
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

    expect(pose.position.x).toBeLessThan(1);
    expect(pose.position.y).toBeGreaterThan(0);
    expect(pose.headingDelta.x).toBeGreaterThan(0);
    expect(pose.headingDelta.y).toBeGreaterThan(0);
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
});
