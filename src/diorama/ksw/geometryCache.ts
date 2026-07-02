// Shared geometry cache for the KSW diorama's static clay vocabulary.
// Every box()/cylinder()/sphere()/torus() call used to allocate a brand new
// RoundedBoxGeometry/CylinderGeometry/SphereGeometry/TorusGeometry, even
// when called with identical params thousands of times (props, walls,
// roofs). This module keys by geometry type + exact params and hands back
// the SAME instance so identical shapes share one GPU buffer.
//
// Callers must never mutate a cached geometry in place (translate/rotate/
// scale on the geometry itself) — only Mesh-level position/rotation/scale
// are safe. See props.ts/building.ts for the call sites this replaces.

import * as THREE from 'three/webgpu';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';

const cache = new Map<string, THREE.BufferGeometry>();

function cached<T extends THREE.BufferGeometry>(kind: string, params: number[], build: () => T): T {
  const key = `${kind}|${params.join('|')}`;
  let geo = cache.get(key);
  if (!geo) {
    geo = build();
    cache.set(key, geo);
  }
  return geo as T;
}

export function roundedBox(w: number, h: number, d: number, segments: number, radius: number): RoundedBoxGeometry {
  return cached('roundedBox', [w, h, d, segments, radius], () => new RoundedBoxGeometry(w, h, d, segments, radius));
}

export function boxGeo(w: number, h: number, d: number): THREE.BoxGeometry {
  return cached('box', [w, h, d], () => new THREE.BoxGeometry(w, h, d));
}

export function cyl(rTop: number, rBot: number, h: number, seg: number): THREE.CylinderGeometry {
  return cached('cyl', [rTop, rBot, h, seg], () => new THREE.CylinderGeometry(rTop, rBot, h, seg));
}

export function sph(r: number, seg: number): THREE.SphereGeometry {
  return cached('sph', [r, seg], () => new THREE.SphereGeometry(r, seg, seg));
}

export function tor(r: number, tube: number, radialSeg: number, tubularSeg: number, arc: number): THREE.TorusGeometry {
  return cached('tor', [r, tube, radialSeg, tubularSeg, arc], () => new THREE.TorusGeometry(r, tube, radialSeg, tubularSeg, arc));
}
