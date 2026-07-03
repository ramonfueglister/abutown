import { describe, expect, it } from 'vitest';
import { moonState, sceneDirFromAzEl, siderealAngleRad, sunState } from '../../src/diorama/environment/solar';

const utc = (s: string) => new Date(s);

describe('sunState (Winterthur golden values)', () => {
  it('summer solstice noon: elevation ~66°, sun due south (+z)', () => {
    // Solar noon Winterthur 2026-06-21 ≈ 13:25 CEST = 11:25 UTC. Max elevation 90-47.5+23.44 ≈ 65.9°.
    const s = sunState(utc('2026-06-21T11:25:00Z'));
    expect(s.elevDeg).toBeGreaterThan(64.9);
    expect(s.elevDeg).toBeLessThan(66.9);
    expect(s.dir[2]).toBeGreaterThan(0.3); // south component dominant
    expect(Math.abs(s.dir[0])).toBeLessThan(0.12); // barely east/west at noon
  });

  it('winter solstice noon: elevation ~19°', () => {
    const s = sunState(utc('2026-12-21T11:25:00Z'));
    expect(s.elevDeg).toBeGreaterThan(18.0);
    expect(s.elevDeg).toBeLessThan(20.1);
  });

  it('summer sunrise ~03:29 UTC (05:29 CEST): elevation crosses 0 rising, sun in the east (+x)', () => {
    const before = sunState(utc('2026-06-21T03:14:00Z'));
    const after = sunState(utc('2026-06-21T03:44:00Z'));
    expect(before.elevDeg).toBeLessThan(0);
    expect(after.elevDeg).toBeGreaterThan(0);
    expect(after.rising).toBe(true);
    expect(after.dir[0]).toBeGreaterThan(0.5); // east
  });

  it('summer sunset ~19:26 UTC: elevation crosses 0 falling, sun in the west (-x)', () => {
    const before = sunState(utc('2026-06-21T19:11:00Z'));
    const after = sunState(utc('2026-06-21T19:41:00Z'));
    expect(before.elevDeg).toBeGreaterThan(0);
    expect(after.elevDeg).toBeLessThan(0);
    expect(before.rising).toBe(false);
    expect(before.dir[0]).toBeLessThan(-0.5); // west
  });

  it('midnight: sun far below horizon', () => {
    expect(sunState(utc('2026-06-21T23:00:00Z')).elevDeg).toBeLessThan(-10);
  });
});

describe('sceneDirFromAzEl convention (+x east, +z south, +y up)', () => {
  it('due south at 45° elevation → (0, ~0.707, ~0.707)', () => {
    const [x, y, z] = sceneDirFromAzEl(Math.PI, Math.PI / 4);
    expect(x).toBeCloseTo(0, 5);
    expect(y).toBeCloseTo(Math.SQRT1_2, 5);
    expect(z).toBeCloseTo(Math.SQRT1_2, 5);
  });
  it('due east at horizon → (~1, 0, 0)', () => {
    const [x, y, z] = sceneDirFromAzEl(Math.PI / 2, 0);
    expect(x).toBeCloseTo(1, 5);
    expect(y).toBeCloseTo(0, 5);
    expect(z).toBeCloseTo(0, 5);
  });
});

describe('moonState', () => {
  it('phase and illumination are self-consistent: fraction ≈ (1 - cos(2π·phase)) / 2', () => {
    for (const d of ['2026-01-05', '2026-03-14', '2026-07-03', '2026-10-20']) {
      const m = moonState(utc(`${d}T22:00:00Z`));
      expect(m.phase).toBeGreaterThanOrEqual(0);
      expect(m.phase).toBeLessThan(1);
      const expected = (1 - Math.cos(2 * Math.PI * m.phase)) / 2;
      expect(m.illumination).toBeCloseTo(expected, 1);
    }
  });
  it('phase advances ~0.03/day', () => {
    const a = moonState(utc('2026-07-03T00:00:00Z')).phase;
    const b = moonState(utc('2026-07-05T00:00:00Z')).phase;
    const delta = (b - a + 1) % 1;
    expect(delta).toBeGreaterThan(0.04);
    expect(delta).toBeLessThan(0.09);
  });
});

describe('siderealAngleRad', () => {
  it('advances ~2π in one sidereal day (23h56m04s)', () => {
    const t0 = utc('2026-07-03T00:00:00Z');
    const t1 = new Date(t0.getTime() + 86164090); // sidereal day in ms
    const delta = siderealAngleRad(t1) - siderealAngleRad(t0);
    const wrapped = ((delta % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI);
    expect(Math.min(wrapped, 2 * Math.PI - wrapped)).toBeLessThan(0.01);
  });
});
