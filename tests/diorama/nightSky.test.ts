import { describe, expect, it } from 'vitest';
import { kswScene, nightSkyLook } from '../../src/diorama/designTokens';
import { starDirections } from '../../src/diorama/environment/nightSky';

describe('starDirections', () => {
  it('is deterministic for a seed and unit-length', () => {
    const a = starDirections(200, 42);
    const b = starDirections(200, 42);
    expect(a).toEqual(b);
    for (const [x, y, z] of a) expect(Math.hypot(x, y, z)).toBeCloseTo(1, 6);
  });
  it('covers the full sphere (mean ~0 per axis, both hemispheres populated)', () => {
    const dirs = starDirections(2000, 7);
    const mean = dirs.reduce((m, d) => [m[0] + d[0], m[1] + d[1], m[2] + d[2]], [0, 0, 0]).map((v) => v / dirs.length);
    for (const v of mean) expect(Math.abs(v)).toBeLessThan(0.05);
    expect(dirs.filter((d) => d[1] < 0).length).toBeGreaterThan(600); // untere Halbkugel real besiedelt
  });
});

// The sun disc and moon disc are both celestial bodies on the CITY sky dome
// (kswCity.domeRadius). A stale ×kswScene.domeRadius (hero 400) left the sun disc
// at ~328 units — a white ball floating low over the city instead of at the
// horizon, while the moon correctly sat at ~1476. Guard the shared distance.
describe('city celestial dome — sun and moon share one distance', () => {
  it('the sun disc sits on the city dome, not the tiny hero dome', () => {
    expect(nightSkyLook.city.sunDistance).toBeGreaterThan(kswScene.domeRadius);
  });

  it('sun and moon are the same celestial distance (same sky dome)', () => {
    expect(nightSkyLook.city.sunDistance).toBe(nightSkyLook.city.moonDistance);
  });
});
