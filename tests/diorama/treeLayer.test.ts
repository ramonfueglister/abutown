import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { allArchetypes, effectiveTreeSize } from '../../src/diorama/ksw/geo/treeArchetypes';
import { assignTrees, buildTreeLayer } from '../../src/diorama/ksw/geo/treeLayer';
import type { TreeSpec } from '../../src/diorama/ksw/geo/geoData';

const specs: TreeSpec[] = Array.from({ length: 120 }, (_, i) => ({
  x: (i % 12) * 9.1, z: Math.floor(i / 12) * 7.3,
  h: 6 + (i % 7), r: 2 + (i % 4) * 0.6,
  kind: i % 3 === 0 ? 'conifer' : 'broad',
}));

describe('assignTrees', () => {
  it('is deterministic and respects kind partitions', () => {
    const a = assignTrees(specs);
    const b = assignTrees(specs);
    expect(a.map((t) => t.archetype)).toEqual(b.map((t) => t.archetype));
    const broadN = 3 * 4; // BROAD_FAMILIES × SEEDS_PER_FAMILY
    for (const t of a) {
      if (t.spec.kind === 'conifer') expect(t.archetype).toBeGreaterThanOrEqual(broadN);
      else expect(t.archetype).toBeLessThan(broadN);
    }
  });
  it('drops trees inside excludeRect', () => {
    const kept = assignTrees(specs, { x: specs[0].x, z: specs[0].z, w: 1, d: 1 });
    expect(kept.length).toBe(specs.length - 1);
  });
});

describe('buildTreeLayer', () => {
  it('creates one mesh per archetype with counts summing to the assignment', () => {
    const layer = buildTreeLayer(specs);
    expect(layer.fullMeshes.length).toBe(allArchetypes().length);
    const total = layer.fullMeshes.reduce((s, m) => s + m.count, 0);
    expect(total).toBe(layer.instances.length);
    for (const m of layer.fullMeshes) expect((m as THREE.InstancedMesh).instanceMatrix.count).toBeGreaterThanOrEqual(1);
  });
  it('maps h and r to world meters via the archetype envelope', () => {
    const layer = buildTreeLayer([{ x: 0, z: 0, h: 9, r: 3, kind: 'broad' }]);
    const mesh = layer.fullMeshes.find((m) => m.count === 1)!;
    const m4 = new THREE.Matrix4();
    mesh.getMatrixAt(0, m4);
    const s = new THREE.Vector3().setFromMatrixScale(m4);
    const arch = allArchetypes()[layer.instances[0].archetype];
    // effectiveTreeSize (treeLook policy) slims/caps the baked h/r before the
    // envelope mapping — the matrix carries the EFFECTIVE metres.
    const eff = effectiveTreeSize(9, 3);
    expect(s.y).toBeCloseTo(eff.h * layer.instances[0].squash, 3);   // height in meters (×squash band 0.85..1.15)
    expect(s.x * arch.crownRadius).toBeCloseTo(eff.r, 3);            // crown radius in meters
  });
  it('setTreeShadows toggles castShadow on all full meshes', () => {
    const layer = buildTreeLayer(specs);
    layer.setTreeShadows(true);
    expect(layer.fullMeshes.every((m) => m.castShadow)).toBe(true);
    layer.setTreeShadows(false);
    expect(layer.fullMeshes.every((m) => !m.castShadow)).toBe(true);
  });
});

describe('compactNear', () => {
  it('keeps only trees within NEAR_TREE_DIST of the camera', () => {
    const far: TreeSpec = { x: 5000, z: 5000, h: 8, r: 2.5, kind: 'broad' };
    const near: TreeSpec = { x: 3, z: 4, h: 8, r: 2.5, kind: 'broad' };
    const layer = buildTreeLayer([near, far]);
    layer.compactNear(0, 0);
    expect(layer.fullMeshes.reduce((s, m) => s + m.count, 0)).toBe(1);
    layer.compactNear(5000, 5000);
    expect(layer.fullMeshes.reduce((s, m) => s + m.count, 0)).toBe(1);
  });
  it('restores instances when the camera returns (no destructive drop)', () => {
    const layer = buildTreeLayer(specs);
    const all = layer.instances.length;
    layer.compactNear(1e6, 1e6);
    expect(layer.fullMeshes.reduce((s, m) => s + m.count, 0)).toBe(0);
    layer.compactNear(specs[0].x, specs[0].z);
    expect(layer.fullMeshes.reduce((s, m) => s + m.count, 0)).toBeGreaterThan(0);
    expect(layer.instances.length).toBe(all); // assignment untouched
  });
});
