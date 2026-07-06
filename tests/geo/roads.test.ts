// tests/geo/roads.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildRoads, miterStrip, skirtStrip } from '../../src/diorama/ksw/geo/roads';

describe('miterStrip', () => {
  it('builds a continuous strip: 2 verts per point, no seams', () => {
    const g = miterStrip([[0, 0], [10, 0], [10, 10]], 6, 0.04);
    expect(g.positions.length / 3).toBe(6); // 3 pts × 2
    expect(g.indices.length).toBe(12); // 2 segments × 2 tris
  });
  it('miter joint bisects a right angle (outer corner further than half width)', () => {
    const g = miterStrip([[0, 0], [10, 0], [10, 10]], 6, 0);
    // corner verts are at index 2,3: bisector direction (±(1,-1)/√2) × 3√2
    const cx = [g.positions[6], g.positions[9]];
    for (const x of cx) expect(Math.abs(x - 10)).toBeCloseTo(3, 3);
  });
  it('caps extreme spikes', () => {
    const g = miterStrip([[0, 0], [10, 0], [0, 0.4]], 6, 0); // ~176° turn
    for (let i = 0; i < g.positions.length; i += 3) {
      expect(Math.abs(g.positions[i])).toBeLessThan(25); // no infinite miter spike
    }
  });
});

describe('skirtStrip', () => {
  // A straight ribbon along +x, width 6, layer offset y=0.04, draped flat on a
  // profile at ground=10 (groundYAt constant). The skirt drops DROP_M below the
  // profile at each edge, so vertex ys must span [profile − DROP_M, profile + y].
  const GROUND = 10;
  const Y = 0.04;
  const DROP = 1.5;
  const ground = () => GROUND;
  const s = skirtStrip([[0, 0], [10, 0]], 6, Y, ground, DROP);

  it('emits two vertical skirt strips (both ribbon edges)', () => {
    // 2 edges × 2 points × 2 verts (top+bottom) = 8 verts; 2 edges × 1 seg × 2 tris = 4 tris
    expect(s.positions.length / 3).toBe(8);
    expect(s.indices.length).toBe(4 * 3);
  });

  it('vertex ys span [profile − dropM, profile + lift]', () => {
    const ys = [];
    for (let i = 1; i < s.positions.length; i += 3) ys.push(s.positions[i]);
    const top = Math.max(...ys);
    const bottom = Math.min(...ys);
    expect(top).toBeCloseTo(GROUND + Y, 5); // ribbon-edge top
    expect(bottom).toBeCloseTo(GROUND - DROP, 5); // apron foot below the profile
  });

  it('skirt x/z lie on the ribbon EDGES (miter-consistent), never the centreline', () => {
    // width 6 → half-width 3 for a road along +x; the skirt offset is the SAME
    // miter offset miterStrip uses, so open-ended vertices carry the ×2 endcap
    // scale (|z|=6) and interior vertices |z|=3 — but NEVER z=0 (the centreline).
    const midStrip = skirtStrip([[0, 0], [10, 0], [20, 0]], 6, Y, ground, DROP);
    const zs = [];
    for (let i = 2; i < midStrip.positions.length; i += 3) zs.push(Math.abs(Math.round(midStrip.positions[i] * 100) / 100));
    expect(zs.every((z) => z === 3 || z === 6)).toBe(true);
    expect(zs.some((z) => z === 0)).toBe(false);
  });
});

describe('buildRoads', () => {
  const group = buildRoads(
    [
      { class: 'residential', width: 6, pts: [[0, 0], [10, 0]] },
      { class: 'footway', width: 2.2, pts: [[0, 5], [10, 5]] },
    ],
    [{ class: 'rail', width: 3, pts: [[0, 9], [10, 9]] }],
  );
  it('splits carriage / footway / rail(+bed) into named layers', () => {
    for (const n of ['carriageRibbons', 'footwayRibbons', 'railBeds', 'railRibbons'])
      expect(group.getObjectByName(n)).toBeTruthy();
  });
  it('layers sit on distinct heights (no z-fight)', () => {
    const y = (n: string) => (group.getObjectByName(n) as THREE.Mesh).geometry.getAttribute('position').getY(0);
    expect(y('carriageRibbons')).not.toBeCloseTo(y('footwayRibbons'), 3);
  });
});
