// src/diorama/ksw/interior/plaza.ts
// The outdoor forecourt of the real KSW (T19, S3d). Everything here is derived
// deterministically from the baked main-entrance door and the generated Notfall
// (emergency) zone — no RNG, no Date — so the plaza is identical on every boot.
//
//   • buildPlaza(mainDoor, erZone, roads):
//       - a hard slab in front of the real main door,
//       - a path from that slab to the nearest real city road,
//       - an ambulance under a canopy at the emergency zone's outer edge,
//       - 4–6 original props (benches / lampposts / trees) lined along the
//         forecourt edge.
//     The ambulance's roof light carries userData.blink; main.ts collects it
//     into the ANIMATED_TAGS pulse (same contract as the old hero hospital).
//
//   • buildHelipad(mainBuilding): the helicopter pad on the main building's
//     largest high flat roof face (normalY > 0.95, y > 20 m, from the baked
//     roof geometry). The rotor group carries userData.rotor so main.ts idles
//     it, and the whole group fades with the cutaway upperFade (returned as a
//     setter) exactly like the main roof.

import * as THREE from 'three/webgpu';
import { kswPalette, palette, radii } from '../../designTokens';
import { box, buildProp } from '../props';
import type { RoadPath, BakedBuilding } from '../geo/geoData';
import type { Zone } from './zones';

export type MainDoor = { x: number; z: number; yaw: number };

// Door outward normal in (x, z): matches the geo shell's window convention
// (windows.ts places facing meshes at sin(yaw)/cos(yaw)).
function doorOut(yaw: number): [number, number] {
  return [Math.sin(yaw), Math.cos(yaw)];
}

// Nearest point on any road polyline to (px, pz), scanning every segment.
function nearestRoadPoint(roads: RoadPath[], px: number, pz: number): [number, number] {
  let best: [number, number] = [px, pz];
  let bestD = Infinity;
  for (const r of roads) {
    const pts = r.pts;
    for (let i = 0; i < pts.length - 1; i++) {
      const [ax, az] = pts[i];
      const [bx, bz] = pts[i + 1];
      const dx = bx - ax;
      const dz = bz - az;
      const len2 = dx * dx + dz * dz || 1e-9;
      let t = ((px - ax) * dx + (pz - az) * dz) / len2;
      t = Math.max(0, Math.min(1, t));
      const cx = ax + t * dx;
      const cz = az + t * dz;
      const d = (px - cx) * (px - cx) + (pz - cz) * (pz - cz);
      if (d < bestD) {
        bestD = d;
        best = [cx, cz];
      }
    }
  }
  return best;
}

export function buildPlaza(mainDoor: MainDoor, erZone: Zone, roads: RoadPath[]): THREE.Group {
  const group = new THREE.Group();
  group.name = 'kswPlaza';

  const [ox, oz] = doorOut(mainDoor.yaw);
  // slab in front of the door, pushed a little outward so it doesn't clip the
  // wall/plinth.
  const slabW = 12;
  const slabD = 9;
  const slabCx = mainDoor.x + ox * (slabD / 2 + 0.5);
  const slabCz = mainDoor.z + oz * (slabD / 2 + 0.5);
  // orient the slab so its depth runs along the outward normal
  const yaw = Math.atan2(ox, oz);
  const slab = box(slabW, 0.1, slabD, kswPalette.plazaPath, radii.m);
  slab.position.set(slabCx, 0.05, slabCz);
  slab.rotation.y = yaw;
  slab.receiveShadow = true;
  group.add(slab);

  // path from the slab's outer edge to the nearest real road: a thin ribbon of
  // slabs stepped along the straight line (deterministic, axis-free).
  const startX = mainDoor.x + ox * (slabD + 0.5);
  const startZ = mainDoor.z + oz * (slabD + 0.5);
  const [roadX, roadZ] = nearestRoadPoint(roads, startX, startZ);
  const pathDx = roadX - startX;
  const pathDz = roadZ - startZ;
  const pathLen = Math.hypot(pathDx, pathDz);
  if (pathLen > 1) {
    const steps = Math.max(1, Math.round(pathLen / 3));
    const segLen = pathLen / steps + 0.3; // slight overlap so no gaps
    const segYaw = Math.atan2(pathDx, pathDz);
    for (let i = 0; i < steps; i++) {
      const f = (i + 0.5) / steps;
      const px = startX + pathDx * f;
      const pz = startZ + pathDz * f;
      const seg = box(3.2, 0.08, segLen, kswPalette.plazaPath, radii.s);
      seg.position.set(px, 0.04, pz);
      seg.rotation.y = segYaw;
      seg.receiveShadow = true;
      group.add(seg);
    }
  }

  // ── ambulance + canopy at the emergency zone's outer edge ─────────────────
  // Place it on the zone side facing away from the building center-ish: use the
  // door outward normal as the "street side" so the ambulance faces the arrival
  // road. Anchor at the emergency zone edge along that normal.
  const erEdgeX = erZone.x + ox * (erZone.w / 2 + 3);
  const erEdgeZ = erZone.z + oz * (erZone.d / 2 + 3);
  const canopyYaw = yaw;
  // canopy slab on two posts
  const canopy = box(6.5, 0.18, 4.5, palette.white, radii.s);
  canopy.position.set(erEdgeX, 3.1, erEdgeZ);
  canopy.rotation.y = canopyYaw;
  canopy.castShadow = true;
  group.add(canopy);
  for (const side of [-1, 1]) {
    const post = box(0.18, 3.0, 0.18, palette.white, radii.xs);
    // offset the two posts along the canopy's width (perpendicular to normal)
    const perpX = Math.cos(canopyYaw);
    const perpZ = -Math.sin(canopyYaw);
    post.position.set(erEdgeX + perpX * side * 2.8, 1.5, erEdgeZ + perpZ * side * 2.8);
    group.add(post);
  }
  const ambulance = buildProp({ kind: 'ambulance', x: erEdgeX, z: erEdgeZ, rotY: canopyYaw + Math.PI / 2 });
  group.add(ambulance);

  // ── 4–6 original props along the forecourt edge ───────────────────────────
  // Alternate bench / lamppost / tree along the slab's front edge, offset to
  // the sides of the entrance path. Deterministic count (6) and placement.
  const perpX = Math.cos(yaw);
  const perpZ = -Math.sin(yaw);
  const propKinds = ['waitingBench', 'lamppost', 'tree', 'waitingBench', 'lamppost', 'tree'];
  for (let i = 0; i < propKinds.length; i++) {
    const side = i % 2 === 0 ? -1 : 1;
    const along = (Math.floor(i / 2) - 1) * 3.2; // -3.2, 0, 3.2 depth positions
    const lateral = side * (slabW / 2 + 1.2);
    // base at the slab center, offset laterally (perp) and along the normal
    const px = slabCx + perpX * lateral + ox * along;
    const pz = slabCz + perpZ * lateral + oz * along;
    const prop = buildProp({ kind: propKinds[i], x: px, z: pz, rotY: yaw });
    group.add(prop);
  }

  return group;
}

// ── helipad on the real roof ────────────────────────────────────────────────
// Largest flat (|normalY| > 0.95) roof face above 20 m, computed from the baked
// roof geometry (cm → m like cityMassing.mergeTinted). Area-weighted centroid
// of the highest-area 1 m y-band is the pad center. Returns the group plus a
// fade setter so main.ts drives it with the cutaway upperFade (like the roof).
export function buildHelipad(mainBuilding: BakedBuilding): { group: THREE.Group; setFade: (o: number) => void } {
  const p = mainBuilding.roof.pos;
  const idx = mainBuilding.roof.idx;
  const bands = new Map<number, { area: number; cx: number; cz: number; y: number }>();
  for (let i = 0; i < idx.length; i += 3) {
    const a = idx[i] * 3;
    const b = idx[i + 1] * 3;
    const c = idx[i + 2] * 3;
    const ax = p[a] / 100;
    const ay = p[a + 1] / 100;
    const az = p[a + 2] / 100;
    const bx = p[b] / 100;
    const by = p[b + 1] / 100;
    const bz = p[b + 2] / 100;
    const cx = p[c] / 100;
    const cy = p[c + 1] / 100;
    const cz = p[c + 2] / 100;
    const ux = bx - ax;
    const uy = by - ay;
    const uz = bz - az;
    const vx = cx - ax;
    const vy = cy - ay;
    const vz = cz - az;
    const nx = uy * vz - uz * vy;
    const ny = uz * vx - ux * vz;
    const nz = ux * vy - uy * vx;
    const cross = Math.hypot(nx, ny, nz);
    if (cross < 1e-9) continue;
    const nyN = ny / cross;
    if (Math.abs(nyN) < 0.95) continue;
    const yavg = (ay + by + cy) / 3;
    if (yavg <= 20) continue;
    const triArea = cross / 2;
    const key = Math.round(yavg);
    const cxx = (ax + bx + cx) / 3;
    const czz = (az + bz + cz) / 3;
    const bd = bands.get(key) ?? { area: 0, cx: 0, cz: 0, y: 0 };
    bd.area += triArea;
    bd.cx += cxx * triArea;
    bd.cz += czz * triArea;
    bd.y += yavg * triArea;
    bands.set(key, bd);
  }
  const group = new THREE.Group();
  group.name = 'kswHelipad';
  const setFade = (o: number): void => {
    group.visible = o > 0.001;
  };
  if (bands.size === 0) return { group, setFade };
  let best = { area: -1, cx: 0, cz: 0, y: 25 };
  for (const bd of bands.values()) {
    if (bd.area > best.area) best = { area: bd.area, cx: bd.cx / bd.area, cz: bd.cz / bd.area, y: bd.y / bd.area };
  }
  const pad = buildProp({ kind: 'helipad', x: best.cx, z: best.cz });
  pad.position.y = best.y + 0.1;
  group.add(pad);
  const heli = buildProp({ kind: 'helicopter', x: best.cx, z: best.cz, rotY: 0.6 });
  heli.position.y = best.y + 0.2;
  group.add(heli);
  return { group, setFade };
}
