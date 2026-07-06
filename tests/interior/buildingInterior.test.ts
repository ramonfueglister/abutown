// buildBuildingInterior (Phase A): per-storey groups stacked at k·storeyH,
// per-storey material clones so setStoreyFades drives opacity independently.
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildBuildingInterior } from '../../src/diorama/ksw/interior/buildInterior';
import { generateBuildingPlan } from '../../src/diorama/ksw/interior/generatePlan';
import type { Zone } from '../../src/diorama/ksw/interior/zones';

const zones: Zone[] = [{ id: 'z0', x: 0, z: 0, w: 40, d: 24 }];
const bp = generateBuildingPlan(zones, { x: 0, z: 12, yaw: 0 }, 10.2); // 3 storeys, H=3.4

function storeyGroups(group: THREE.Group): THREE.Group[] {
  return group.children.filter((c): c is THREE.Group => c.name.startsWith('storey-'));
}

describe('buildBuildingInterior', () => {
  it('builds one group per storey at level·storeyH with userData.level', () => {
    const { group } = buildBuildingInterior(bp, { angle: 0.4 });
    const storeys = storeyGroups(group);
    expect(storeys).toHaveLength(3);
    storeys.forEach((s, k) => {
      expect(s.userData.level).toBe(k);
      expect(s.position.y).toBeCloseTo(k * bp.storeyH, 6);
    });
    expect(group.rotation.y).toBeCloseTo(0.4, 9);
  });

  it('materials are NOT shared across storeys (fades must be independent)', () => {
    const { group } = buildBuildingInterior(bp, { angle: 0 });
    const [s0, s1] = storeyGroups(group);
    const mats = (g: THREE.Object3D): Set<THREE.Material> => {
      const out = new Set<THREE.Material>();
      g.traverse((o) => {
        const m = (o as THREE.Mesh).material;
        if (m) (Array.isArray(m) ? m : [m]).forEach((mm) => out.add(mm as THREE.Material));
      });
      return out;
    };
    const m0 = mats(s0);
    for (const m of mats(s1)) expect(m0.has(m)).toBe(false);
  });

  it('setStoreyFades drives per-storey opacity and visibility', () => {
    const { group, setStoreyFades } = buildBuildingInterior(bp, { angle: 0 });
    setStoreyFades([1, 0.5, 0]);
    const [s0, s1, s2] = storeyGroups(group);
    expect(s0.visible).toBe(true);
    expect(s1.visible).toBe(true);
    expect(s2.visible).toBe(false);
    let sawHalf = false;
    s1.traverse((o) => {
      const m = (o as THREE.Mesh).material as THREE.Material | undefined;
      if (m && !Array.isArray(m)) {
        expect((m as THREE.MeshStandardMaterial).opacity).toBeCloseTo(0.5, 6);
        sawHalf = true;
      }
    });
    expect(sawHalf).toBe(true);
  });
});
