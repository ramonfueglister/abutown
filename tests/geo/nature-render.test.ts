// tests/geo/nature-render.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildNature } from '../../src/diorama/ksw/geo/nature';
import type { CityNature } from '../../src/diorama/ksw/geo/geoData';

const nature: CityNature = {
  greens: [
    { kind: 'park', ring: [[0, 0], [40, 0], [40, 30], [0, 30]] },
    { kind: 'wood', ring: [[100, 0], [160, 0], [160, 50], [100, 50]] },
  ],
  waterAreas: [{ ring: [[-50, 0], [-20, 0], [-20, 20], [-50, 20]] }],
  rivers: [{ width: 6, pts: [[200, 0], [260, 0]] }],
  trees: [
    { x: 10, z: 10, h: 9, r: 3, kind: 'broad' },
    { x: 20, z: 15, h: 9, r: 3, kind: 'broad' },
    { x: 500, z: 500, h: 9, r: 3, kind: 'broad' },
  ],
};

describe('buildNature', () => {
  const group = buildNature(nature, { excludeRect: { x: 495, z: 495, w: 20, d: 20 } });
  const greens = group.getObjectByName('natureGreens') as THREE.Mesh;
  const water = group.getObjectByName('natureWater') as THREE.Mesh;
  const canopy = group.getObjectByName('treeCanopies') as THREE.InstancedMesh;
  const trunks = group.getObjectByName('treeTrunks') as THREE.InstancedMesh;

  it('triangulates green and water areas into single meshes', () => {
    expect(greens.geometry.index!.count).toBe(12); // 2 rects × 2 tris
    // water: 1 area rect (2 tris) + 1 river segment quad (2 tris)
    expect(water.geometry.index!.count).toBe(12);
  });

  it('instances trees, excluding the given rect (hero plate)', () => {
    expect(canopy.count).toBe(2); // third tree sits in the excluded rect
    expect(trunks.count).toBe(2);
  });

  it('greens sit below road level and receive shadows', () => {
    const pos = greens.geometry.getAttribute('position');
    expect(pos.getY(0)).toBeLessThan(0.04);
    expect(greens.receiveShadow).toBe(true);
    expect(greens.castShadow).toBe(false);
  });
});

describe('buildNature — trees v2 (broad/conifer/impostor split)', () => {
  const nature2: CityNature = {
    greens: [], waterAreas: [], rivers: [],
    trees: [
      { x: 10, z: 10, h: 9, r: 3, kind: 'broad' },
      { x: 20, z: 15, h: 14, r: 2, kind: 'conifer' },
    ],
  };

  it('splits broadleaf/conifer/impostor instances with real sizes', () => {
    const g = buildNature(nature2, {});
    const broad = g.getObjectByName('treeCanopies') as THREE.InstancedMesh;
    const conif = g.getObjectByName('treeConifers') as THREE.InstancedMesh;
    const imp = g.getObjectByName('treeImpostors') as THREE.InstancedMesh;
    const impC = g.getObjectByName('treeImpostorsConifer') as THREE.InstancedMesh;
    expect(broad.count).toBe(1);
    expect(conif.count).toBe(1);
    expect(imp.count).toBe(1); // broad impostors only — conifers have their own mesh
    expect(impC.count).toBe(1);
    expect(imp.visible).toBe(false);
    expect(impC.visible).toBe(false);
    const m = new THREE.Matrix4();
    broad.getMatrixAt(0, m);
    const s = new THREE.Vector3();
    m.decompose(new THREE.Vector3(), new THREE.Quaternion(), s);
    expect(s.x).toBeCloseTo(3 / 0.75, 1); // canopy geo authored at 0.75 puff radius → scale = r/0.75
  });

  it('impostor silhouettes match the near forms: multi-puff broad, cone-ish conifer', () => {
    const g = buildNature(nature2, {});
    const imp = g.getObjectByName('treeImpostors') as THREE.InstancedMesh;
    const impC = g.getObjectByName('treeImpostorsConifer') as THREE.InstancedMesh;
    // broad impostor = low-poly merge of the 4-puff layout, NOT a single ball:
    // strictly more vertices than one Icosahedron(1, 0)
    const ballVerts = new THREE.IcosahedronGeometry(1, 0).getAttribute('position').count;
    expect(imp.geometry.getAttribute('position').count).toBeGreaterThan(ballVerts);
    // conifer impostor is its own cone-ish geometry, not the puff merge
    expect(impC.geometry).not.toBe(imp.geometry);
    expect(impC.geometry.getAttribute('position').count).not.toBe(
      imp.geometry.getAttribute('position').count,
    );
  });

  it('impostors keep per-kind tints (broad green != conifer wood-green)', () => {
    const g = buildNature(nature2, {});
    const imp = g.getObjectByName('treeImpostors') as THREE.InstancedMesh;
    const impC = g.getObjectByName('treeImpostorsConifer') as THREE.InstancedMesh;
    const cb = new THREE.Color();
    const cc = new THREE.Color();
    imp.getColorAt(0, cb);
    impC.getColorAt(0, cc);
    expect(cb.getHex()).not.toBe(cc.getHex());
  });

  it('variety is deterministic from x/z and consistent full↔impostor (no LOD pop)', () => {
    const g = buildNature(nature2, {});
    const broad = g.getObjectByName('treeCanopies') as THREE.InstancedMesh;
    const imp = g.getObjectByName('treeImpostors') as THREE.InstancedMesh;
    const mB = new THREE.Matrix4();
    const mI = new THREE.Matrix4();
    broad.getMatrixAt(0, mB);
    imp.getMatrixAt(0, mI);
    const sB = new THREE.Vector3();
    const sI = new THREE.Vector3();
    const pB = new THREE.Vector3();
    const pI = new THREE.Vector3();
    mB.decompose(pB, new THREE.Quaternion(), sB);
    mI.decompose(pI, new THREE.Quaternion(), sI);
    // same position + same y-squash ratio → the far↔near swap doesn't pop
    expect(pI.distanceTo(pB)).toBeLessThan(1e-6);
    expect(sI.y / sI.x).toBeCloseTo(sB.y / sB.x, 5);
    // squash stays inside the authored 0.85–1.15 band
    const squash = sB.y / sB.x;
    expect(squash).toBeGreaterThanOrEqual(0.85);
    expect(squash).toBeLessThanOrEqual(1.15);
    // and full canopy tint == impostor tint (consistent variety)
    const cB = new THREE.Color();
    const cI = new THREE.Color();
    broad.getColorAt(0, cB);
    imp.getColorAt(0, cI);
    expect(cB.getHex()).toBe(cI.getHex());
  });
});

describe('buildNature — empty tree kind keeps a non-zero instance buffer', () => {
  // Only broadleaf trees in this bake (e.g. a future re-bake with no conifers
  // mapped yet). treeConifers must still exist by name (Task 10's LOD does
  // getObjectByName on all four tree mesh names) but must not allocate a
  // zero-size WebGPU instance buffer (the windows.ts GPUValidationError class
  // of bug, Task 6) — here the mesh can't just be skipped like windows.ts
  // does, since the name contract requires it to be present.
  const natureNoConifers: CityNature = {
    greens: [], waterAreas: [], rivers: [],
    trees: [
      { x: 10, z: 10, h: 9, r: 3, kind: 'broad' },
      { x: 20, z: 15, h: 9, r: 2, kind: 'broad' },
    ],
  };

  it('treeConifers exists with count 0 but an allocated instance buffer', () => {
    const g = buildNature(natureNoConifers, {});
    const broad = g.getObjectByName('treeCanopies') as THREE.InstancedMesh;
    const conif = g.getObjectByName('treeConifers') as THREE.InstancedMesh;
    const trunksMesh = g.getObjectByName('treeTrunks') as THREE.InstancedMesh;
    const imp = g.getObjectByName('treeImpostors') as THREE.InstancedMesh;
    const impC = g.getObjectByName('treeImpostorsConifer') as THREE.InstancedMesh;

    expect(broad).toBeDefined();
    expect(conif).toBeDefined();
    expect(trunksMesh).toBeDefined();
    expect(imp).toBeDefined();
    expect(impC).toBeDefined();

    expect(conif.count).toBe(0);
    expect(impC.count).toBe(0);
    expect(imp.count).toBe(2);
    // capacity must stay >= 1 even though drawn count is 0
    expect(conif.instanceMatrix.count).toBeGreaterThanOrEqual(1);
    expect(broad.instanceMatrix.count).toBeGreaterThanOrEqual(1);
    expect(trunksMesh.instanceMatrix.count).toBeGreaterThanOrEqual(1);
    expect(imp.instanceMatrix.count).toBeGreaterThanOrEqual(1);
    expect(impC.instanceMatrix.count).toBeGreaterThanOrEqual(1);
  });
});
