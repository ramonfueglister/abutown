// tests/geo/roads.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildRoads, miterStrip } from '../../src/diorama/ksw/geo/roads';

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
