// tests/geo/triangulate.test.ts
import { describe, expect, it } from 'vitest';
import { polygonNormal, triangulatePlanarPolygon } from '../../scripts/geo/lib/triangulate.mjs';

describe('triangulatePlanarPolygon', () => {
  it('triangulates a horizontal unit square into 2 triangles', () => {
    const r = triangulatePlanarPolygon([[0, 5, 0], [1, 5, 0], [1, 5, 1], [0, 5, 1]]);
    expect(r).not.toBeNull();
    expect(r!.positions.length).toBe(12); // 4 vertices × xyz
    expect(r!.indices.length).toBe(6); // 2 triangles
  });

  it('triangulates a vertical wall quad (dominant x-normal)', () => {
    const r = triangulatePlanarPolygon([[2, 0, 0], [2, 0, 4], [2, 3, 4], [2, 3, 0]]);
    expect(r!.indices.length).toBe(6);
  });

  it('handles a gabled roof plane (tilted normal)', () => {
    const r = triangulatePlanarPolygon([[0, 4, 0], [10, 4, 0], [10, 6, 3], [0, 6, 3]]);
    expect(r!.indices.length).toBe(6);
    // area of the tilted quad = 10 × hypot(2,3)
    let area = 0;
    const p = r!.positions;
    for (let i = 0; i < r!.indices.length; i += 3) {
      const [a, b, c] = [r!.indices[i] * 3, r!.indices[i + 1] * 3, r!.indices[i + 2] * 3];
      const ab = [p[b] - p[a], p[b + 1] - p[a + 1], p[b + 2] - p[a + 2]];
      const ac = [p[c] - p[a], p[c + 1] - p[a + 1], p[c + 2] - p[a + 2]];
      const cr = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
      ];
      area += Math.hypot(...cr) / 2;
    }
    expect(area).toBeCloseTo(10 * Math.hypot(2, 3), 5);
  });

  it('returns null for degenerate rings', () => {
    expect(triangulatePlanarPolygon([[0, 0, 0], [1, 0, 0]])).toBeNull();
    expect(triangulatePlanarPolygon([[0, 0, 0], [1, 0, 0], [2, 0, 0]])).toBeNull(); // collinear
  });

  it('polygonNormal points up for a CCW-from-above horizontal ring', () => {
    const n = polygonNormal([[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]]);
    expect(n[1]).toBeGreaterThan(0);
  });
});
