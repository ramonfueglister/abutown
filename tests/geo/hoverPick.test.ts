// tests/geo/hoverPick.test.ts
// Raycast is pure CPU math — runs headless in vitest. This test ALSO guards
// the three/webgpu ↔ three-mesh-bvh class-identity interop: if the BVH
// prototype patch missed the Mesh class the diorama uses, pick() returns null
// and this fails.
import * as THREE from 'three/webgpu';
import { describe, expect, it } from 'vitest';
import type { BakedBuilding, BakedMesh } from '../../src/diorama/ksw/geo/geoData';
import { mergeTinted } from '../../src/diorama/ksw/geo/cityMassing';
import { createHoverPicker } from '../../src/diorama/ksw/hoverPick';

// mergeTinted converts cm-int positions back to metres (÷100 — see
// cityMassing.ts and the `cube` fixture in cityMassing.test.ts), so this
// fixture bakes cm ints too: a 1×1 m horizontal quad at y=5 m, centred on
// (cx, cz) metres.
const flatQuad = (cx: number, cz: number): BakedMesh => ({
  pos: [
    (cx - 0.5) * 100, 500, (cz - 0.5) * 100,
    (cx + 0.5) * 100, 500, (cz - 0.5) * 100,
    (cx + 0.5) * 100, 500, (cz + 0.5) * 100,
    (cx - 0.5) * 100, 500, (cz + 0.5) * 100,
  ],
  idx: [0, 1, 2, 0, 2, 3],
});
const building = (id: string, cx: number, cz: number): BakedBuilding => ({
  id, zone: 'city', footprint: [[cx - 0.5, cz - 0.5], [cx + 0.5, cz - 0.5], [cx + 0.5, cz + 0.5], [cx - 0.5, cz + 0.5]],
  height: 5, eaveH: 5,
  wall: { ...flatQuad(cx, cz), fuv: new Array(8).fill(0) }, roof: flatQuad(cx, cz),
});

describe('createHoverPicker', () => {
  const buildings = [building('{A}', 0, 0), building('{B}', 20, 0)];
  const mesh = new THREE.Mesh(mergeTinted(buildings, (b) => b.roof, 0xffffff));
  const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 1000);
  camera.position.set(0, 100, 0);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  const picker = createHoverPicker({ camera, meshes: [mesh], buildings });

  it('straight down onto {A} at screen centre', () => {
    expect(picker.pick(0, 0)?.id).toBe('{A}');
  });
  it('empty sky → null', () => {
    expect(picker.pick(0.9, 0.9)).toBeNull();
  });
});
