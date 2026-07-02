// tests/geo/style.test.ts
import { describe, expect, it } from 'vitest';
import { footprintValid, roofOutlineFootprint } from '../../scripts/geo/lib/style.mjs';
import { roofSkirts, roofUnderside } from '../../scripts/geo/lib/style.mjs';

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

describe('roofSkirts', () => {
  // gabled roof: two planes meeting at a ridge y=6, eaves at y=4
  const planes = [
    [[0, 4, 0], [10, 4, 0], [10, 6, 5], [0, 6, 5]],
    [[0, 6, 5], [10, 6, 5], [10, 4, 10], [0, 4, 10]],
  ];
  it('emits vertical skirts only for rising boundary edges, ridge deduped', () => {
    const skirts = roofSkirts(planes, 4);
    // boundary edges above eave: the two gable sides x=0 and x=10 (2 edges each
    // rising to the ridge) → 4 skirts; eave edges (y=4) skipped; ridge shared → deduped
    expect(skirts.length).toBe(4);
    for (const ring of skirts) {
      expect(ring.length).toBe(4);
      const ys = ring.map((p: number[]) => p[1]);
      expect(Math.min(...ys)).toBeCloseTo(4, 5);
    }
  });
});

describe('roofSkirts winding', () => {
  // gabled roof: two planes meeting at a ridge y=6, eaves at y=4
  const planes = [
    [[0, 4, 0], [10, 4, 0], [10, 6, 5], [0, 6, 5]],
    [[0, 6, 5], [10, 6, 5], [10, 4, 10], [0, 4, 10]],
  ];

  function centroidXZ(roofRings: number[][][]): [number, number] {
    let sx = 0, sz = 0, n = 0;
    for (const ring of roofRings) {
      for (const [x, , z] of ring) {
        sx += x; sz += z; n++;
      }
    }
    return [sx / n, sz / n];
  }

  // Newell's method — robust for the triangle-shaped (one repeated-vertex)
  // quads roofSkirts emits when the rising edge already touches the eave.
  function quadNormal(quad: number[][]): [number, number, number] {
    let nx = 0, ny = 0, nz = 0;
    for (let i = 0; i < quad.length; i++) {
      const [x0, y0, z0] = quad[i];
      const [x1, y1, z1] = quad[(i + 1) % quad.length];
      nx += (y0 - y1) * (z0 + z1);
      ny += (z0 - z1) * (x0 + x1);
      nz += (x0 - x1) * (y0 + y1);
    }
    return [nx, ny, nz];
  }

  function expectOutwardSkirts(skirts: number[][][], roofRings: number[][][]): void {
    const [cx, cz] = centroidXZ(roofRings);
    for (const quad of skirts) {
      const [nx, , nz] = quadNormal(quad);
      // centroid of the quad's own points as the reference "midpoint"
      const mx = quad.reduce((s, p) => s + p[0], 0) / quad.length;
      const mz = quad.reduce((s, p) => s + p[2], 0) / quad.length;
      const outX = mx - cx, outZ = mz - cz;
      const dot = nx * outX + nz * outZ;
      expect(dot).toBeGreaterThan(0);
    }
  }

  it('orients skirts outward for the normally-wound fixture', () => {
    const skirts = roofSkirts(planes, 4);
    expectOutwardSkirts(skirts, planes);
  });

  it('orients skirts outward even when every source ring is reversed', () => {
    const reversed = planes.map((ring) => [...ring].reverse());
    const skirts = roofSkirts(reversed, 4);
    expectOutwardSkirts(skirts, reversed);
  });
});

describe('roofUnderside', () => {
  it('copies each plane 0.22 lower with flipped winding', () => {
    const planes = [[[0, 5, 0], [4, 5, 0], [4, 5, 4], [0, 5, 4]]];
    const under = roofUnderside(planes);
    expect(under.length).toBe(1);
    expect(under[0][0][1]).toBeCloseTo(4.78, 5);
    expect(under[0][0][0]).toBe(planes[0][planes[0].length - 1][0]); // reversed order
  });
});
