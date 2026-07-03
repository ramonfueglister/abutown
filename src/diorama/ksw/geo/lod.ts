// src/diorama/ksw/geo/lod.ts
// Semantic 3-ring LOD for the city (spec §2c): visibility + shadow policy by
// camera radius, with hysteresis so orbiting at a boundary never flickers.
import type * as THREE from 'three/webgpu';
import { kswCityStyle } from '../../designTokens';

export type CityLodRing = 'near' | 'mid' | 'far';

export function cityLodState(radius: number, prev: CityLodRing): CityLodRing {
  const { nearR, midR, hysteresis } = kswCityStyle.lod;
  const up = 1 + hysteresis;
  const dn = 1 - hysteresis;
  if (prev === 'near') return radius > nearR * up ? cityLodState(radius, 'mid') : 'near';
  if (prev === 'mid') {
    if (radius < nearR * dn) return 'near';
    return radius > midR * up ? 'far' : 'mid';
  }
  return radius < midR * dn ? cityLodState(radius, 'mid') : 'far';
}

// Refs are collected via getObjectByName, which can legitimately return
// undefined (a design-legal missing group/mesh) — applyCityLod must not
// assume any of these exist and must never throw on a partially-null refs
// object (e.g. in tests, or a bake that skips a mesh).
export type CityLodRefs = {
  // Windows are now a wall-shader raster (Task 13), not a separate object — the
  // far ring flips a uniform via setFacadeDetail instead of hiding a group.
  setFacadeDetail: (on: boolean) => void;
  lamps: THREE.Object3D | null;
  footways: THREE.Object3D | null;
  treesFull: THREE.Object3D[];
  treeImpostors: THREE.Object3D | null;
  setTreeShadows: (on: boolean) => void;
};

export function applyCityLod(ring: CityLodRing, r: CityLodRefs): void {
  const far = ring === 'far';
  r.setFacadeDetail(!far);
  if (r.lamps) r.lamps.visible = !far;
  if (r.footways) r.footways.visible = !far;
  for (const t of r.treesFull) if (t) t.visible = !far;
  if (r.treeImpostors) r.treeImpostors.visible = far;
  r.setTreeShadows(ring === 'near');
}
