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

describe('miterStrip terrain draping', () => {
  // Rolling ground: 3 m amplitude, ~63 m period — the Gemeinde bake has 447
  // road segments longer than 50 m, so an undivided chord cuts metres deep.
  const ground = (x: number, _z: number) => Math.sin(x / 10) * 3;

  it('subdivides long segments so the ribbon follows the terrain (no chord burial)', () => {
    const g = miterStrip([[0, 0], [100, 0]], 6, 0.04, ground);
    // Walk the centreline rows (vertex pairs) and check the ribbon height
    // against the ground everywhere between rows via dense sampling.
    const rows: Array<{ x: number; y: number }> = [];
    for (let i = 0; i < g.positions.length; i += 6) {
      rows.push({ x: (g.positions[i] + g.positions[i + 3]) / 2, y: g.positions[i + 1] });
    }
    expect(rows.length).toBeGreaterThan(4); // actually subdivided
    let worst = 0;
    for (let k = 1; k < rows.length; k++) {
      const a = rows[k - 1];
      const b = rows[k];
      for (let t = 0; t <= 1; t += 0.1) {
        const x = a.x + (b.x - a.x) * t;
        const y = a.y + (b.y - a.y) * t;
        worst = Math.max(worst, Math.abs(y - (ground(x, 0) + 0.04)));
      }
    }
    expect(worst).toBeLessThan(0.5); // undivided chord is ~5.9 m off here
  });

  it('keeps flat strips without a ground sampler byte-identical (no subdivision)', () => {
    const g = miterStrip([[0, 0], [100, 0]], 6, 0.04);
    expect(g.positions.length / 3).toBe(4); // 2 pts × 2 — untouched fast path
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
