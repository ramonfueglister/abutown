// tests/geo/facade.test.ts
// Windows moved into the wall shader (Task 13); facade.ts now only derives the
// instanced door slot from the baked door point.
import { describe, expect, it } from 'vitest';
import { facadeDoor } from '../../src/diorama/ksw/geo/facade';
import { kswCityStyle } from '../../src/diorama/designTokens';

describe('facadeDoor', () => {
  it('lifts the baked door to half the door height and keeps x/z/yaw', () => {
    const slot = facadeDoor({ door: { x: 12, z: 3, yaw: Math.PI } });
    expect(slot).not.toBeNull();
    expect(slot!.x).toBe(12);
    expect(slot!.z).toBe(3);
    expect(slot!.yaw).toBe(Math.PI);
    expect(slot!.y).toBeCloseTo(kswCityStyle.doorH / 2);
  });

  it('returns null when the bake found no door', () => {
    expect(facadeDoor({})).toBeNull();
  });
});
