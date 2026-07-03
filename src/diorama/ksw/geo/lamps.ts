// src/diorama/ksw/geo/lamps.ts
// Street lamps along the REAL road polylines: class-based spacing, alternating
// sides — deterministic, no scattering. Night: warm bulbs like the original.
import * as THREE from 'three/webgpu';
import { kswCityStyle, palette } from '../../designTokens';
import { clayMat } from '../props';
import type { RoadPath } from './geoData';

export function lampSpots(roads: RoadPath[]): Array<{ x: number; z: number }> {
  const out: Array<{ x: number; z: number }> = [];
  for (const r of roads) {
    const spacing = kswCityStyle.lamp.spacing[r.class];
    if (!spacing) continue;
    const off = r.width / 2 + kswCityStyle.lamp.sideOffset;
    let travelled = 0;
    let next = 0;
    let side = -1;
    for (let i = 0; i < r.pts.length - 1; i++) {
      const [ax, az] = r.pts[i];
      const [bx, bz] = r.pts[i + 1];
      const dx = bx - ax;
      const dz = bz - az;
      const len = Math.hypot(dx, dz);
      if (len < 0.01) continue;
      while (next <= travelled + len) {
        const t = (next - travelled) / len;
        const nx = (-dz / len) * off * side;
        const nz = (dx / len) * off * side;
        out.push({ x: ax + dx * t + nx, z: az + dz * t + nz });
        side = -side;
        next += spacing;
      }
      travelled += len;
    }
  }
  return out;
}

export function buildLamps(roads: RoadPath[], opts: { lampGlow: boolean }): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityLamps';
  const spots = lampSpots(roads);
  // original lamppost proportions (props.ts): 2.9 m pole + head + bulb
  const pole = new THREE.CylinderGeometry(0.07, 0.1, 2.9, 6);
  pole.translate(0, 1.45, 0);
  const head = new THREE.CylinderGeometry(0.26, 0.34, 0.22, 8);
  head.translate(0, 2.98, 0);
  // Zero-instance InstancedMeshes create zero-size GPU uniform buffers, which
  // WebGPU rejects (GPUValidationError). Allocate at least 1 instance always
  // (Task 10 does getObjectByName on these meshes unconditionally), then set
  // the visible .count to the real spot count.
  const n = Math.max(1, spots.length);
  const posts = new THREE.InstancedMesh(pole, clayMat(palette.metalMatt), n);
  posts.name = 'lampPosts';
  posts.count = spots.length;
  const heads = new THREE.InstancedMesh(head, clayMat(palette.metalDark), n);
  heads.name = 'lampHeads';
  heads.count = spots.length;
  const bulbGeo = new THREE.SphereGeometry(0.15, 8, 6);
  bulbGeo.translate(0, 2.86, 0);
  const bulbs = new THREE.InstancedMesh(
    bulbGeo,
    opts.lampGlow ? new THREE.MeshBasicMaterial({ color: 0xffe3b0 }) : clayMat(palette.white),
    n,
  );
  bulbs.name = 'lampBulbs';
  bulbs.count = spots.length;
  const m = new THREE.Matrix4();
  spots.forEach((s, i) => {
    m.makeTranslation(s.x, 0, s.z);
    posts.setMatrixAt(i, m);
    heads.setMatrixAt(i, m);
    bulbs.setMatrixAt(i, m);
  });
  for (const mesh of [posts, heads, bulbs]) {
    mesh.castShadow = false;
    mesh.receiveShadow = false;
    group.add(mesh);
  }
  return group;
}
