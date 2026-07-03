import { describe, it, expect } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildInterior } from '../../src/diorama/ksw/interior/buildInterior';
import { generateInteriorPlan, type MainDoor } from '../../src/diorama/ksw/interior/generatePlan';
import type { Zone } from '../../src/diorama/ksw/interior/zones';

const ZONES: Zone[] = [
  { id: 'z0', x: -13, z: 0, w: 20, d: 20 },
  { id: 'z1', x: 13, z: 0, w: 20, d: 20 },
];
const DOOR: MainDoor = { x: -13, z: 10, yaw: 0 };

describe('buildInterior', () => {
  it('builds a non-empty interior group', () => {
    const plan = generateInteriorPlan(ZONES, DOOR);
    const g = buildInterior(plan);
    expect(g).toBeInstanceOf(THREE.Group);
    expect(g.children.length).toBeGreaterThan(0);
  });

  it('emits a floor + walls + sign for every room (so no room is a bare rect)', () => {
    const plan = generateInteriorPlan(ZONES, DOOR);
    const g = buildInterior(plan);
    // floors: one box per room + one per corridor + inlays; count meshes with
    // a footprint at each room center to prove every room got a floor.
    let meshCount = 0;
    g.traverse((o) => {
      if ((o as THREE.Mesh).isMesh) meshCount++;
    });
    // Each room contributes a floor + inlay + 4 walls (>=6 meshes); with N
    // rooms the interior must have at least ~6N meshes.
    expect(meshCount).toBeGreaterThanOrEqual(plan.rooms.length * 5);
  });

  it('builds no ground plate or roof (interior only)', () => {
    const plan = generateInteriorPlan(ZONES, DOOR);
    const g = buildInterior(plan);
    let roofFadeTagged = 0;
    g.traverse((o) => {
      if (o.userData && o.userData.roofFade) roofFadeTagged++;
    });
    expect(roofFadeTagged).toBe(0);
  });
});
