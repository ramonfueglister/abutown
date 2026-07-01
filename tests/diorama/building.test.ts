import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildHospital, segmentWall, type WallOpening } from '../../src/diorama/ksw/building';
import { kswPlan } from '../../src/diorama/ksw/floorPlan';
import { kswScene } from '../../src/diorama/designTokens';

describe('segmentWall', () => {
  const H = kswScene.wallHeight;
  const sill = kswScene.openingSill;

  it('solid wall = top band + mid band + base band spanning full length', () => {
    const segs = segmentWall(8, H, []);
    expect(segs.length).toBe(3);
    for (const s of segs) {
      expect(s.c0).toBeCloseTo(-4);
      expect(s.c1).toBeCloseTo(4);
    }
  });

  it('a door splits base and mid bands but keeps the top band whole', () => {
    const door: WallOpening = { center: 0, width: 1.6, kind: 'door' };
    const segs = segmentWall(8, H, [door]);
    const base = segs.filter((s) => s.y0 === 0);
    const top = segs.filter((s) => s.y1 === H);
    expect(base.length).toBe(2);
    expect(top.length).toBe(1);
    // nothing solid inside the doorway
    for (const s of segs) {
      const insideDoor = s.c0 < 0.79 && s.c1 > -0.79;
      if (insideDoor) expect(s.y0).toBeGreaterThanOrEqual(kswScene.openingHead - 0.07);
    }
  });

  it('a window splits only the mid band — wall below the sill stays solid', () => {
    const window: WallOpening = { center: 1, width: 1.4, kind: 'window' };
    const segs = segmentWall(8, H, [window]);
    const base = segs.filter((s) => s.y0 === 0);
    expect(base.length).toBe(1);
    expect(base[0].c0).toBeCloseTo(-4);
    expect(base[0].c1).toBeCloseTo(4);
    const mid = segs.filter((s) => s.y0 === sill && s.y1 === kswScene.openingHead);
    expect(mid.length).toBe(2);
  });

  it('door and window on the same wall coexist', () => {
    const segs = segmentWall(10, H, [
      { center: -2, width: 1.6, kind: 'door' },
      { center: 2, width: 1.4, kind: 'window' },
    ]);
    const base = segs.filter((s) => s.y0 === 0);
    expect(base.length).toBe(2); // split by the door only
    const mid = segs.filter((s) => s.y0 === sill);
    expect(mid.length).toBe(3); // split by door and window
  });
});

describe('buildHospital', () => {
  const { group, roofs } = buildHospital(kswPlan);

  function meshes(root: THREE.Object3D): THREE.Mesh[] {
    const out: THREE.Mesh[] = [];
    root.traverse((o) => {
      if ((o as THREE.Mesh).isMesh) out.push(o as THREE.Mesh);
    });
    return out;
  }

  it('builds a substantial scene (walls, floors, props, people, roofs)', () => {
    expect(meshes(group).length).toBeGreaterThan(400);
  });

  it('covers every room and corridor with a shadow-casting roof', () => {
    roofs.setFade(1);
    const roofMeshes = meshes(group).filter((m) => m.castShadow && m.position.y > kswScene.wallHeight);
    expect(roofMeshes.length).toBeGreaterThanOrEqual(kswPlan.rooms.length + kswPlan.corridors.length);
  });

  it('setFade drives opacity, shadow casting, and visibility thresholds', () => {
    roofs.setFade(1);
    const roofMeshes = meshes(group).filter((m) => m.position.y > kswScene.wallHeight && (m.material as THREE.Material).transparent);
    expect(roofMeshes.length).toBeGreaterThan(0);
    expect(roofMeshes.every((m) => m.castShadow && m.visible)).toBe(true);

    roofs.setFade(0.4);
    expect(roofs.fade()).toBeCloseTo(0.4);
    expect(roofMeshes.every((m) => !m.castShadow && m.visible)).toBe(true);
    expect((roofMeshes[0].material as THREE.MeshPhysicalMaterial).opacity).toBeCloseTo(0.4);

    roofs.setFade(0.01);
    expect(roofMeshes.every((m) => !m.visible)).toBe(true);

    roofs.setFade(1); // restore
    expect(roofMeshes.every((m) => m.castShadow && m.visible)).toBe(true);
  });

  it('clamps fade input to [0, 1]', () => {
    roofs.setFade(7);
    expect(roofs.fade()).toBe(1);
    roofs.setFade(-2);
    expect(roofs.fade()).toBe(0);
    roofs.setFade(1);
  });
});
