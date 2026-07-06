// tests/geo/riverrings.test.ts
//
// Buffered-centreline water rings for polygon-less rivers (spec §4.4, Eulach).
import { describe, expect, it } from 'vitest';
import { bufferCenterline, riverCenterlineRings } from '../../scripts/geo/lib/riverrings.mjs';
import { pointInRing } from '../../scripts/geo/lib/join.mjs';

describe('bufferCenterline', () => {
  it('turns a straight 2-point line into a closed 4-corner ±3 m strip', () => {
    // Line along +x from (0,0) to (10,0). Right normal of +x is +z (south),
    // so the strip spans z in [-3, +3]. A 2-point line → right edge (2 pts)
    // + reversed left edge (2 pts) = 4 corners.
    const ring = bufferCenterline([[0, 0], [10, 0]], 3);
    expect(ring.length).toBe(4);
    // all corners at |z| = 3
    for (const [, z] of ring) expect(Math.abs(Math.abs(z) - 3)).toBeLessThan(1e-9);
    // x spans [0,10]
    const xs = ring.map(([x]) => x).sort((a, b) => a - b);
    expect(xs[0]).toBeCloseTo(0, 9);
    expect(xs[3]).toBeCloseTo(10, 9);
    // a point on the centreline is inside the ring; a point 5 m off is outside
    expect(pointInRing(5, 0, ring)).toBe(true);
    expect(pointInRing(5, 5, ring)).toBe(false);
  });

  it('is deterministic (byte-identical rings on repeat)', () => {
    const line = [[0, 0], [10, 2], [20, -1], [30, 5]];
    expect(bufferCenterline(line, 3)).toEqual(bufferCenterline(line, 3));
  });

  it('a multi-segment line yields a ring wider than the line (contains the centreline)', () => {
    const line = [[0, 0], [10, 0], [20, 10]];
    const ring = bufferCenterline(line, 3);
    expect(ring.length).toBeGreaterThanOrEqual(4);
    // midpoint of each segment lies inside the buffered ring
    expect(pointInRing(5, 0, ring)).toBe(true);
    expect(pointInRing(15, 5, ring)).toBe(true);
  });

  it('throws on a degenerate (single-point) line instead of silently defaulting', () => {
    expect(() => bufferCenterline([[1, 1]], 3)).toThrow();
    expect(() => bufferCenterline([[1, 1], [1, 1]], 3)).toThrow(); // dupes collapse to 1
  });
});

describe('riverCenterlineRings', () => {
  it('emits one ring per river line, preserving order', () => {
    const rivers = [
      { width: 5, pts: [[0, 0], [10, 0]] },
      { width: 2, pts: [[0, 100], [10, 100], [20, 100]] },
    ];
    const rings = riverCenterlineRings(rivers, 3);
    expect(rings.length).toBe(2);
    expect(pointInRing(5, 0, rings[0])).toBe(true);
    expect(pointInRing(10, 100, rings[1])).toBe(true);
  });

  it('skips degenerate river lines without throwing', () => {
    const rivers = [
      { width: 5, pts: [[0, 0]] }, // too short
      { width: 5, pts: [[0, 0], [10, 0]] },
    ];
    const rings = riverCenterlineRings(rivers, 3);
    expect(rings.length).toBe(1);
  });
});
