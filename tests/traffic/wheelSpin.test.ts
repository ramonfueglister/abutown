import { describe, expect, it } from 'vitest';
import { advanceSpin, initSpin, MAX_STEER } from '../../src/diorama/traffic/wheelSpin';
import { SIM_DT } from '../../src/diorama/traffic/deadReckon';

describe('wheelSpin', () => {
  it('rolls theta by v·dt/r', () => {
    const st = initSpin(100, 0);
    advanceSpin(st, 10, 0, 110, 0.31); // 10 ticks → dt = 1 s
    expect(st.theta).toBeCloseTo((10 * 10 * SIM_DT) / 0.31);
    expect(st.lastTick).toBe(110);
  });

  it('accumulates across calls and rolls backward for v < 0', () => {
    const st = initSpin(0, 0);
    advanceSpin(st, 5, 0, 10, 0.3);
    const t1 = st.theta;
    advanceSpin(st, -5, 0, 20, 0.3);
    expect(st.theta).toBeCloseTo(0);
    expect(t1).toBeGreaterThan(0);
  });

  it('ignores non-positive dt (tick replay) without NaN', () => {
    const st = initSpin(50, 0);
    advanceSpin(st, 10, 0, 50, 0.3); // dt = 0
    advanceSpin(st, 10, 0, 40, 0.3); // dt < 0
    expect(st.theta).toBe(0);
    expect(Number.isFinite(st.theta)).toBe(true);
  });

  it('steers toward yaw change, clamped to ±MAX_STEER, and decays straight', () => {
    const st = initSpin(0, 0);
    advanceSpin(st, 10, 0.5, 1, 0.3); // hard yaw step in one tick
    expect(st.steer).toBeGreaterThan(0);
    expect(st.steer).toBeLessThanOrEqual(MAX_STEER);
    for (let t = 2; t < 40; t++) advanceSpin(st, 10, 0.5, t, 0.3); // yaw now constant
    expect(Math.abs(st.steer)).toBeLessThan(0.02); // filtered back to straight
  });

  it('handles yaw wrap (π → −π is a small left turn, not a full spin)', () => {
    const st = initSpin(0, Math.PI - 0.01);
    advanceSpin(st, 10, -Math.PI + 0.01, 1, 0.3);
    expect(Math.abs(st.steer)).toBeLessThanOrEqual(MAX_STEER);
    expect(Math.abs(st.steer)).toBeLessThan(0.2); // Δyaw was only 0.02 rad
  });
});
