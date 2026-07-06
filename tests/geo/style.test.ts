// tests/geo/style.test.ts
import { describe, expect, it } from 'vitest';
import { footprintValid, roofOutlineFootprint } from '../../scripts/geo/lib/style.mjs';
import { roofSkirts, roofUnderside } from '../../scripts/geo/lib/style.mjs';
import { doorForBuilding, forestFill, roadWidthFromTags, treeSpec } from '../../scripts/geo/lib/style.mjs';

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

describe('treeSpec', () => {
  it('real tags win, untouched by variance', () => {
    const t = treeSpec({ height: '17', diameter_crown: '8' }, 1, 2);
    expect(t.h).toBe(17);
    expect(t.r).toBe(4);
  });
  it('leaf_type default is deterministic (Slice-2 family/growth-curve sizing)', () => {
    const a = treeSpec({ leaf_type: 'needleleaved' }, 5, 5);
    const b = treeSpec({ leaf_type: 'needleleaved' }, 5, 5);
    expect(a).toEqual(b);
    expect(a.kind).toBe('conifer');
    expect(a.h).toBeGreaterThan(0);
  });
});

describe('forestFill', () => {
  const ring = [[0, 0], [120, 0], [120, 60], [0, 60]]; // 7200 m² → ~120 trees
  it('fills the polygon at ~1/60 m² density, deterministic', () => {
    const a = forestFill(ring, []);
    expect(a.length).toBeGreaterThan(70);
    expect(a.length).toBeLessThan(170);
    expect(a).toEqual(forestFill(ring, []));
    for (const t of a) {
      expect(t.x).toBeGreaterThanOrEqual(0);
      expect(t.x).toBeLessThanOrEqual(120);
    }
  });
  it('respects mapped trees (4 m clearance)', () => {
    const a = forestFill(ring, [{ x: 60, z: 30 }]);
    for (const t of a) expect(Math.hypot(t.x - 60, t.z - 30)).toBeGreaterThan(4);
  });
});

describe('roadWidthFromTags', () => {
  it('width tag > lanes > fallback', () => {
    expect(roadWidthFromTags({ width: '7.5' }, 5.5)).toBe(7.5);
    expect(roadWidthFromTags({ lanes: '3' }, 5.5)).toBeCloseTo(9.6);
    expect(roadWidthFromTags({}, 5.5)).toBe(5.5);
  });
});

describe('doorForBuilding', () => {
  it('places the door on the road-facing facade', () => {
    const fp = [[0, 0], [10, 0], [10, 10], [0, 10]];
    const road = [[5, -6], [5, -20]]; // south of the building (−z side)
    const d = doorForBuilding(fp, road)!;
    expect(d.z).toBeCloseTo(0); // south edge
    expect(d.x).toBeCloseTo(5);
  });
});
