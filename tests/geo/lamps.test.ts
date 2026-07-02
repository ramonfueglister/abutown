// tests/geo/lamps.test.ts
import { describe, expect, it } from 'vitest';
import { lampSpots } from '../../src/diorama/ksw/geo/lamps';

describe('lampSpots', () => {
  it('spaces lamps along a residential road, alternating sides', () => {
    const spots = lampSpots([{ class: 'residential', width: 6, pts: [[0, 0], [120, 0]] }]);
    // 120 m / 35 m spacing → 4 lamps (t=0,35,70,105)
    expect(spots.length).toBe(4);
    expect(spots[0].z).toBeCloseTo(-(3 + 1.2));
    expect(spots[1].z).toBeCloseTo(3 + 1.2);
  });
  it('skips footways entirely', () => {
    expect(lampSpots([{ class: 'footway', width: 2.2, pts: [[0, 0], [100, 0]] }]).length).toBe(0);
  });
  it('is deterministic', () => {
    const r = [{ class: 'primary', width: 9, pts: [[0, 0], [50, 10], [90, 40]] }];
    expect(lampSpots(r)).toEqual(lampSpots(r));
  });
});
