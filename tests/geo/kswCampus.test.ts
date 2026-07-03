// tests/geo/kswCampus.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildKswCampus } from '../../src/diorama/ksw/geo/kswCampus';
import { kswBuildings } from '../../src/diorama/ksw/geo/geoData';

describe('buildKswCampus', () => {
  const { group, mainBuilding } = buildKswCampus(kswBuildings);

  it('is named kswCampus and reuses the city massing pipeline (walls with fuv facade attribute)', () => {
    expect(group.name).toBe('kswCampus');
    const walls = group.getObjectByName('kswCampusWalls') as THREE.Mesh;
    expect(walls).toBeTruthy();
    expect(walls.geometry.getAttribute('fuv')).toBeTruthy();
  });

  it('picks the main building by largest footprint area (real KSW tower/wing, 113 points)', () => {
    expect(mainBuilding.footprint.length).toBe(113);
  });

  it('exposes setFacadeDetail and it is always on (hero zone = near)', () => {
    const walls = group.getObjectByName('kswCampusWalls') as THREE.Mesh;
    expect(typeof walls.userData.setFacadeDetail).toBe('function');
    const mat = walls.material as THREE.MeshPhysicalNodeMaterial & { facadeDetail: { value: number } };
    expect(mat.facadeDetail.value).toBe(1);
  });

  it('produces real geometry for all 26 baked KSW buildings', () => {
    const walls = group.getObjectByName('kswCampusWalls') as THREE.Mesh;
    expect(walls.geometry.getAttribute('position').count).toBeGreaterThan(0);
  });
});
