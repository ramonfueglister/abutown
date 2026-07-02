// tests/geo/cityMassing.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildCityMassing } from '../../src/diorama/ksw/geo/cityMassing';
import type { BakedBuilding } from '../../src/diorama/ksw/geo/geoData';

const cube = (x: number): BakedBuilding => ({
  id: `b${x}`, zone: 'city', footprint: [[x, 0], [x + 5, 0], [x + 5, 5], [x, 5]], height: 6,
  // one wall quad + one roof quad, cm ints
  wall: { pos: [x * 100, 0, 0, (x + 5) * 100, 0, 0, (x + 5) * 100, 600, 0, x * 100, 600, 0], idx: [0, 1, 2, 0, 2, 3] },
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
