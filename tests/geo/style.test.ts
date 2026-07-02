// tests/geo/style.test.ts
import { describe, expect, it } from 'vitest';
import { footprintValid, roofOutlineFootprint } from '../../scripts/geo/lib/style.mjs';

const roof = [
  [[0, 10, 0], [20, 10, 0], [20, 12, 15], [0, 12, 15]], // one 20x15 sloped plane
];

describe('footprintValid', () => {
  it('accepts a footprint that encloses the roof', () => {
    expect(footprintValid([[-1, -1], [21, -1], [21, 16], [-1, 16]], roof)).toBe(true);
  });
  it('rejects a stray-facet footprint (tiny vs roof)', () => {
    expect(footprintValid([[0, 0], [2, 0], [1, 2]], roof)).toBe(false);
  });
});

describe('roofOutlineFootprint', () => {
  it('returns the convex hull of the roof XZ points', () => {
    const hull = roofOutlineFootprint(roof);
    expect(hull.length).toBeGreaterThanOrEqual(4);
    const xs = hull.map((p: number[]) => p[0]);
    const zs = hull.map((p: number[]) => p[1]);
    expect(Math.min(...xs)).toBe(0);
    expect(Math.max(...xs)).toBe(20);
    expect(Math.max(...zs)).toBe(15);
  });
});
