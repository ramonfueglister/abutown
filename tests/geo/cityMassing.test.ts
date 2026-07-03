// tests/geo/cityMassing.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildCityMassing } from '../../src/diorama/ksw/geo/cityMassing';
import type { BakedBuilding } from '../../src/diorama/ksw/geo/geoData';

const cube = (x: number): BakedBuilding => ({
  id: `b${x}`, zone: 'city', footprint: [[x, 0], [x + 5, 0], [x + 5, 5], [x, 5]], height: 6, eaveH: 6,
  // one wall quad + one roof quad, cm ints; wall carries fuv in 2-dm units
  // (u along the 5 m edge = 0,25,25,0; v = 0,0,30,30 for a 6 m wall)
  wall: {
    pos: [x * 100, 0, 0, (x + 5) * 100, 0, 0, (x + 5) * 100, 600, 0, x * 100, 600, 0],
    idx: [0, 1, 2, 0, 2, 3],
    fuv: [0, 0, 25, 0, 25, 30, 0, 30],
  },
  roof: { pos: [x * 100, 600, 0, (x + 5) * 100, 600, 0, (x + 5) * 100, 600, 500, x * 100, 600, 500], idx: [0, 1, 2, 0, 2, 3] },
});

describe('buildCityMassing', () => {
  const group = buildCityMassing([cube(0), cube(20)]);
  const walls = group.getObjectByName('cityWalls') as THREE.Mesh;
  const roofs = group.getObjectByName('cityRoofs') as THREE.Mesh;

  it('merges everything into exactly four meshes', () => {
    expect(group.children.length).toBe(4);
    expect(walls.geometry.getAttribute('position').count).toBe(8); // 2 buildings × 4 verts
    expect(roofs.geometry.index!.count).toBe(12); // 2 buildings × 2 tris
  });
  it('converts cm ints back to meters and stays finite', () => {
    const pos = walls.geometry.getAttribute('position');
    let maxX = -Infinity;
    for (let i = 0; i < pos.count; i++) {
      expect(Number.isFinite(pos.getX(i))).toBe(true);
      maxX = Math.max(maxX, pos.getX(i));
    }
    expect(maxX).toBe(25); // 2500 cm
  });
  it('casts and receives shadows', () => {
    expect(walls.castShadow && walls.receiveShadow).toBe(true);
    expect(roofs.castShadow && roofs.receiveShadow).toBe(true);
  });

  it('walls carry the fuv facade attribute aligned to the vertex count (Task 13)', () => {
    const pos = walls.geometry.getAttribute('position');
    const fuv = walls.geometry.getAttribute('fuv');
    const eaveH = walls.geometry.getAttribute('eaveH');
    expect(fuv).toBeTruthy();
    expect(fuv.itemSize).toBe(2);
    expect(fuv.count).toBe(pos.count); // one fuv per vertex
    expect(eaveH.count).toBe(pos.count);
    // dm→m conversion: first vertex u=0 v=0; third vertex u=5m v=6m
    expect(fuv.getX(0)).toBeCloseTo(0);
    expect(fuv.getX(2)).toBeCloseTo(5); // 50 dm → 5 m
    expect(fuv.getY(2)).toBeCloseTo(6); // 60 dm → 6 m
    expect(eaveH.getX(0)).toBeCloseTo(6);
  });

  it('exposes a setFacadeDetail LOD callback on the wall mesh', () => {
    expect(typeof walls.userData.setFacadeDetail).toBe('function');
    expect(() => (walls.userData.setFacadeDetail as (b: boolean) => void)(false)).not.toThrow();
    expect(() => (walls.userData.setFacadeDetail as (b: boolean) => void)(true)).not.toThrow();
  });

  it('adds plinth and eave band meshes', () => {
    const g2 = buildCityMassing([cube(0)]);
    expect(g2.getObjectByName('cityPlinths')).toBeTruthy();
    expect(g2.getObjectByName('cityEaves')).toBeTruthy();
    const plinth = g2.getObjectByName('cityPlinths') as THREE.Mesh;
    const pos = plinth.geometry.getAttribute('position');
    let minY = Infinity;
    for (let i = 0; i < pos.count; i++) minY = Math.min(minY, pos.getY(i));
    expect(minY).toBeLessThan(0); // sinks below the plate — nothing floats
  });
});
