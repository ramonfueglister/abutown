// src/diorama/ksw/geo/lamps.ts
// Street lamps along the REAL road polylines: class-based spacing, alternating
// sides — deterministic, no scattering. Night: warm bulbs like the original.
import * as THREE from 'three/webgpu';
import { float, mix, normalView, uv, vec3 } from 'three/tsl';
import { kswCityStyle, nightGlow, palette } from '../../designTokens';
import { clayMat } from '../props';
import { lampGlowU } from '../glowUniform';
import type { RoadPath } from './geoData';

// rgb01 tuple for a 0xRRGGBB hex — TSL vec3 needs normalized channels.
function rgb01(hex: number): [number, number, number] {
  const c = new THREE.Color(hex);
  return [c.r, c.g, c.b];
}

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

export function buildLamps(roads: RoadPath[], groundYAt?: (x: number, z: number) => number): THREE.Group {
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
  // The bulb is always built with an unlit node material; by day it reads as a
  // dim glass housing (a fullbright white sphere × 17 938 lamps painted the
  // whole city with white dot rows), and mixes toward an HDR warm tint as
  // lampGlowU rises — bright enough (nightGlow.bulbHdr) to clear the bloom
  // threshold, so night lamps read as glowing light sources.
  const bulbMat = new THREE.MeshBasicNodeMaterial();
  const dayGlass = vec3(...rgb01(palette.metalMatt)).mul(float(0.85));
  const warmHdr = vec3(...rgb01(nightGlow.lampHead)).mul(float(nightGlow.bulbHdr));
  bulbMat.colorNode = mix(dayGlass, warmHdr, lampGlowU);
  const bulbs = new THREE.InstancedMesh(bulbGeo, bulbMat, n);
  bulbs.name = 'lampBulbs';
  bulbs.count = spots.length;
  // Light pool: an instanced additive disc under each lamp — the stylized-city
  // stand-in for per-lamp point lights. Radial (1−d)² falloff, warm 2700K-ish
  // tint, scaled by lampGlowU (black by day → additive no-op). fog=false: fog
  // on an ADDITIVE material would add the fog colour itself at distance.
  const poolGeo = new THREE.CircleGeometry(nightGlow.pool.radius, 24);
  poolGeo.rotateX(-Math.PI / 2);
  poolGeo.translate(0, nightGlow.pool.lift, 0);
  const poolMat = new THREE.MeshBasicNodeMaterial();
  poolMat.transparent = true;
  poolMat.depthWrite = false;
  poolMat.blending = THREE.AdditiveBlending;
  poolMat.fog = false;
  const poolDist = uv().sub(0.5).length().mul(2); // 0 centre → 1 rim
  const poolFall = float(1).sub(poolDist).clamp(0, 1);
  poolMat.colorNode = vec3(...rgb01(nightGlow.pool.color))
    .mul(poolFall.mul(poolFall))
    .mul(float(nightGlow.pool.peak))
    .mul(lampGlowU);
  const pools = new THREE.InstancedMesh(poolGeo, poolMat, n);
  pools.name = 'lampPools';
  pools.count = spots.length;
  // Halo: additive glow sphere around the head — brightness follows the
  // view-facing normal (center bright, silhouette soft), a view-robust
  // budget volumetric. Black by day via lampGlowU (additive no-op).
  const haloGeo = new THREE.SphereGeometry(nightGlow.halo.radius, 10, 8);
  haloGeo.translate(0, 2.86, 0);
  const haloMat = new THREE.MeshBasicNodeMaterial();
  haloMat.transparent = true;
  haloMat.depthWrite = false;
  haloMat.blending = THREE.AdditiveBlending;
  haloMat.fog = false;
  const facing = normalView.z.clamp(0, 1);
  haloMat.colorNode = vec3(...rgb01(nightGlow.lampHead))
    .mul(facing.mul(facing).mul(facing))
    .mul(float(nightGlow.halo.peak))
    .mul(lampGlowU);
  const halos = new THREE.InstancedMesh(haloGeo, haloMat, n);
  halos.name = 'lampHalos';
  halos.count = spots.length;
  const m = new THREE.Matrix4();
  spots.forEach((s, i) => {
    m.makeTranslation(s.x, groundYAt ? groundYAt(s.x, s.z) : 0, s.z);
    posts.setMatrixAt(i, m);
    heads.setMatrixAt(i, m);
    bulbs.setMatrixAt(i, m);
    pools.setMatrixAt(i, m);
    halos.setMatrixAt(i, m);
  });
  for (const mesh of [posts, heads, bulbs, pools, halos]) {
    mesh.castShadow = false;
    mesh.receiveShadow = false;
    group.add(mesh);
  }
  return group;
}
