// src/diorama/ksw/hoverPick.ts
// BVH-accelerated building picking on the MERGED city meshes (one wall mesh +
// one roof mesh for ~800 buildings — per-mesh userData can't identify a
// building, so the merge carries a per-vertex buildingIdx attribute instead;
// see cityMassing.mergeTinted). three-mesh-bvh makes raycasting the ~240k-tri
// merge O(log n): the boundsTree is built once per geometry, firstHitOnly.
import * as THREE from 'three/webgpu';
import { acceleratedRaycast, computeBoundsTree, disposeBoundsTree } from 'three-mesh-bvh';
import type { BakedBuilding } from './geo/geoData';

// Patch once at module load. three r185 shares core classes between 'three'
// and 'three/webgpu' (three.core), so the prototype patch reaches both; the
// hoverPick unit test guards this interop.
THREE.BufferGeometry.prototype.computeBoundsTree = computeBoundsTree;
THREE.BufferGeometry.prototype.disposeBoundsTree = disposeBoundsTree;
THREE.Mesh.prototype.raycast = acceleratedRaycast;

export function createHoverPicker({
  camera,
  meshes,
  buildings,
}: {
  camera: THREE.Camera;
  meshes: THREE.Mesh[];
  buildings: BakedBuilding[];
}): { pick(ndcX: number, ndcY: number): BakedBuilding | null } {
  for (const mesh of meshes) {
    if (!mesh.geometry.boundsTree) mesh.geometry.computeBoundsTree();
  }
  const raycaster = new THREE.Raycaster();
  raycaster.firstHitOnly = true;
  const ndc = new THREE.Vector2();
  return {
    pick(ndcX: number, ndcY: number): BakedBuilding | null {
      ndc.set(ndcX, ndcY);
      raycaster.setFromCamera(ndc, camera);
      // Hover picking should hit a roof/wall triangle regardless of winding —
      // the baked meshes aren't guaranteed front-face-up for every roof facet,
      // and a hover ray can approach from any angle. Force DoubleSide for the
      // duration of this raycast only, restoring each material's own `side`
      // immediately after: three's raycast reads `material.side` off the hit
      // object internally (see three-mesh-bvh's acceleratedRaycast / the
      // vanilla Mesh.raycast), so there's no side-channel to pass this in.
      // A mesh's `material` can in principle be an array (multi-material
      // mesh) — skip the side-toggle for those rather than silently no-op
      // via a cast, so the behaviour is defined instead of accidental.
      const toggled = meshes.filter((m) => !Array.isArray(m.material)) as (THREE.Mesh & { material: THREE.Material })[];
      const prevSides = toggled.map((m) => m.material.side);
      let hits: THREE.Intersection[];
      try {
        for (const m of toggled) m.material.side = THREE.DoubleSide;
        hits = raycaster.intersectObjects(meshes, false);
      } finally {
        toggled.forEach((m, i) => {
          m.material.side = prevSides[i];
        });
      }
      const hit = hits[0];
      if (!hit || !hit.face) return null;
      const attr = (hit.object as THREE.Mesh).geometry.getAttribute('buildingIdx');
      if (!attr) return null;
      const idx = attr.getX(hit.face.a);
      return buildings[idx] ?? null;
    },
  };
}
