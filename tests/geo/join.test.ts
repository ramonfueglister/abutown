// tests/geo/join.test.ts
import { describe, expect, it } from 'vitest';
import { nameForFootprint, pointInRing, ringCentroid, roadStyle } from '../../scripts/geo/lib/join.mjs';

const square = [[0, 0], [10, 0], [10, 10], [0, 10]];

describe('geometry predicates', () => {
  it('pointInRing', () => {
    expect(pointInRing(5, 5, square)).toBe(true);
    expect(pointInRing(15, 5, square)).toBe(false);
  });
  it('ringCentroid of the square is its middle', () => {
    const [cx, cz] = ringCentroid(square);
    expect(cx).toBeCloseTo(5);
    expect(cz).toBeCloseTo(5);
  });
});

describe('nameForFootprint', () => {
  const osm = [
    { ring: [[-1, -1], [20, -1], [20, 20], [-1, 20]], tags: { building: 'hospital', name: 'Radio-Onkologie', healthcare: 'clinic' } },
  ];
  it('joins by centroid containment', () => {
    expect(nameForFootprint(square, osm)).toEqual({ name: 'Radio-Onkologie', usage: 'clinic' });
  });
  it('returns empty object when nothing contains the centroid', () => {
    expect(nameForFootprint([[100, 100], [110, 100], [110, 110], [100, 110]], osm)).toEqual({});
  });
});

describe('roadStyle', () => {
  it('classifies the hierarchy with descending widths', () => {
    const w = (t: Record<string, string>) => roadStyle(t)!.width;
    expect(w({ highway: 'primary' })).toBeGreaterThan(w({ highway: 'residential' }));
    expect(w({ highway: 'residential' })).toBeGreaterThan(w({ highway: 'footway' }));
  });
  it('classifies rails and rejects junk', () => {
    expect(roadStyle({ railway: 'rail' })!.class).toBe('rail');
    expect(roadStyle({ highway: 'proposed' })).toBeNull();
  });
});
