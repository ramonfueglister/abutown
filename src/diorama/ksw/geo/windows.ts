// src/diorama/ksw/geo/windows.ts
// Instanced diorama windows/doors for the city: white frames + glass panes
// slightly proud of the real facades, warm night glow in the original share.
import * as THREE from 'three/webgpu';
import { kswCityStyle, palette } from '../../designTokens';
import { NIGHT_WINDOW_SHARE, nightWindowHash } from '../staticBatch';
import { clayMat, glassMat } from '../props';
import type { BakedBuilding } from './geoData';
import { facadeLayout, type WindowSlot } from './facade';

function fill(mesh: THREE.InstancedMesh, slots: WindowSlot[], out: number): void {
  const m = new THREE.Matrix4();
  const e = new THREE.Euler();
  for (let i = 0; i < slots.length; i++) {
    const s = slots[i];
    e.set(0, s.yaw, 0);
    m.makeRotationFromEuler(e);
    m.setPosition(s.x + Math.sin(s.yaw) * out, s.y, s.z + Math.cos(s.yaw) * out);
    mesh.setMatrixAt(i, m);
  }
  mesh.instanceMatrix.needsUpdate = true;
}

export function buildWindows(buildings: BakedBuilding[], opts: { lampGlow: boolean }): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityWindows';
  const s = kswCityStyle;
  const all: WindowSlot[] = [];
  const doors: WindowSlot[] = [];
  for (const b of buildings) {
    const layout = facadeLayout(b);
    all.push(...layout.windows);
    if (layout.door) doors.push(layout.door);
  }
  const glow: WindowSlot[] = [];
  const plain: WindowSlot[] = [];
  for (const w of all) (opts.lampGlow && nightWindowHash(w.x, w.z) < NIGHT_WINDOW_SHARE ? glow : plain).push(w);

  const frameGeo = new THREE.BoxGeometry(s.windowW + 0.16, s.windowH + 0.16, 0.1);
  const paneGeo = new THREE.BoxGeometry(s.windowW, s.windowH, 0.06);
  const doorGeo = new THREE.BoxGeometry(s.doorW, s.doorH, 0.14);

  // Zero-instance InstancedMeshes create zero-size GPU uniform buffers, which
  // WebGPU rejects (GPUValidationError). Only build/add a mesh when it has
  // at least one instance — e.g. `glow` is empty whenever lampGlow is off.
  const specs: Array<{ name: string; geo: THREE.BufferGeometry; mat: THREE.Material; slots: WindowSlot[]; out: number }> = [
    { name: 'cityWindowFrames', geo: frameGeo, mat: clayMat(palette.white), slots: all, out: 0.07 },
    { name: 'cityWindowPanes', geo: paneGeo, mat: glassMat().clone(), slots: plain, out: 0.1 },
    { name: 'cityWindowGlow', geo: paneGeo, mat: new THREE.MeshBasicMaterial({ color: 0xffd9a0 }), slots: glow, out: 0.1 },
    { name: 'cityDoors', geo: doorGeo, mat: clayMat(palette.woodSoft), slots: doors, out: 0.08 },
  ];
  for (const spec of specs) {
    if (spec.slots.length === 0) continue;
    const mesh = new THREE.InstancedMesh(spec.geo, spec.mat, spec.slots.length);
    mesh.name = spec.name;
    fill(mesh, spec.slots, spec.out);
    mesh.castShadow = false;
    mesh.receiveShadow = false;
    group.add(mesh);
  }
  return group;
}
