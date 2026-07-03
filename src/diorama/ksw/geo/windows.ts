// src/diorama/ksw/geo/windows.ts
// Instanced city DOORS only. Windows moved into the wall shader (Task 13,
// cityMassing.ts facadeMaterial) — the 154k instanced frames/panes/glow are
// gone. Doors stay instanced: a door is a single box per building, cheap, and
// its wood tone + placement don't fit a per-facet raster.
import * as THREE from 'three/webgpu';
import { kswCityStyle, palette } from '../../designTokens';
import { clayMat } from '../props';
import type { BakedBuilding } from './geoData';
import { facadeDoor, type WindowSlot } from './facade';

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

export function buildWindows(buildings: BakedBuilding[]): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityWindows';
  const s = kswCityStyle;
  const doors: WindowSlot[] = [];
  for (const b of buildings) {
    const door = facadeDoor(b);
    if (door) doors.push(door);
  }
  // Zero-instance InstancedMeshes create zero-size GPU uniform buffers, which
  // WebGPU rejects — only build the mesh when there is at least one door.
  if (doors.length === 0) return group;

  const doorGeo = new THREE.BoxGeometry(s.doorW, s.doorH, 0.14);
  const mesh = new THREE.InstancedMesh(doorGeo, clayMat(palette.woodSoft), doors.length);
  mesh.name = 'cityDoors';
  fill(mesh, doors, 0.08);
  mesh.castShadow = false;
  mesh.receiveShadow = false;
  group.add(mesh);
  return group;
}
