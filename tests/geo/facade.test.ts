// tests/geo/facade.test.ts
import { describe, expect, it } from 'vitest';
import { facadeLayout } from '../../src/diorama/ksw/geo/facade';

const b = { footprint: [[0, 0], [24, 0], [24, 10], [0, 10]], height: 9.5, door: { x: 12, z: 0, yaw: Math.PI } };

describe('facadeLayout', () => {
  const out = facadeLayout(b);
  it('derives storeys and columns from real size', () => {
    // floors = round(9.5/3)=3; 24m edge → floor((24-0.8)/2.4)=9 cols; 10m edge → 3 cols
    // top storey windows must stay 0.4m under the eave → still 3 rows here
    expect(out.windows.length).toBeGreaterThan((9 * 2 + 3 * 2) * 2); // ≥2 rows everywhere
    const ys = [...new Set(out.windows.map((w) => Math.round(w.y * 10) / 10))];
    expect(ys.length).toBe(3);
  });
  it('door replaces the nearest ground-floor window on its edge', () => {
    expect(out.door).not.toBeNull();
    const gfSouth = out.windows.filter((w) => w.z === 0 && w.y < 3);
    for (const w of gfSouth) expect(Math.abs(w.x - 12)).toBeGreaterThan(1.1);
  });
  it('is deterministic', () => {
    expect(facadeLayout(b)).toEqual(out);
  });
  it('derives outward-pointing yaw regardless of ring winding', () => {
    // Same footprint, reversed winding order (CW vs CCW).
    const reversed = { ...b, footprint: [...b.footprint].reverse() };
    const outRev = facadeLayout(reversed);
    // Centroid of the rectangle is (12, 5). For every window, the yaw-derived
    // outward direction (sin(yaw), cos(yaw)) must point away from the centroid.
    const cx = 12;
    const cz = 5;
    for (const w of [...out.windows, ...outRev.windows]) {
      const dirX = Math.sin(w.yaw);
      const dirZ = Math.cos(w.yaw);
      const toCenterX = cx - w.x;
      const toCenterZ = cz - w.z;
      const dot = dirX * toCenterX + dirZ * toCenterZ;
      expect(dot).toBeLessThan(0); // outward direction points away from centroid
    }
  });
});
