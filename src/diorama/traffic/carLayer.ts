// src/diorama/traffic/carLayer.ts
//
// The instanced car layer: ONE InstancedMesh of a simple two-box clay car,
// capacity 4096, driven each frame by the TrafficClient's dead-reckoned poses.
//
// Design-language notes (designTokens): chunky clay, no textures, muted body
// colors. The car is a merged two-box shape (lower body + smaller cabin), built
// in local space so its wheels-line sits at y=0 and it points along +z (the yaw
// from deadReckon.poseAt is atan2(tangentX, tangentZ), so a car heading +x is
// rotated +PI/2 about y — a +z-facing base geometry lands correct).
//
// Cars sit ON the carriage road surface plus a small lift so the body clears
// the ribbon. Post-#119 the whole traffic plate is draped on real DEM terrain
// (it spans ~-1460..+200 x / ~-410..+1440 z, where the ground undulates from
// ~-10 m to ~+9 m relative to the anchor), so a single flat y left every car
// floating above or sunk below the visible road. The layer therefore samples
// the ground height PER VEHICLE via the `groundYAt(x, z)` callback the host
// passes in (built from the world DEM in main.ts) and adds the carriage+lift
// offset. Without a sampler it falls back to the old flat carriage y (used by
// unit tests, which have no world). The lane polylines are already offset to
// the Swiss right-hand side (baked), so following them exactly puts cars on the
// correct side of two-way streets — the layer applies NO horizontal offset.

import * as THREE from 'three/webgpu';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import { kswCity, palette } from '../designTokens';
import { boxGeo } from '../ksw/geometryCache';
import { clayMat } from '../ksw/props';
import { type TrafficNetGeom, type VehKinematics } from './deadReckon';
import { poseAtBlended } from './laneBlend';

/** Instance capacity for VISIBLE cars (the AOI-subscribed cells only, not the
 * whole fleet — the server-side fleet cap is 30k since Task 8, but a browser
 * AOI sees a small fraction; the measured whole-Gemeinde morning peak is
 * ~2.4k alive at demand_scale 1.0). */
export const CAR_CAPACITY = 4096;

/** Ground clearance above the carriage ribbon so the body doesn't z-fight it. */
const CAR_LIFT = 0.06;

/** Car footprint (metres). Small clay hatchback proportions. */
const BODY_W = 1.9;
const BODY_H = 0.7;
const BODY_L = 4.2;
const CABIN_W = 1.6;
const CABIN_H = 0.6;
const CABIN_L = 2.1;

/** A few muted body colors from the palette; picked deterministically per id so
 * a given vehicle keeps its colour across frames. */
const BODY_COLORS = [
  palette.metalMatt,
  palette.woodSoft,
  palette.sage,
  palette.coralSoft,
  palette.creamLight,
  palette.metalDark,
] as const;

/** Build the merged two-box car geometry, local origin at the wheel line,
 * long axis along +z. Exported so flowLayer.ts's far-LOD impostor mesh can
 * reuse the EXACT same shape (per the Task 12 brief: "second InstancedMesh
 * reusing the clay-car geometry") rather than duplicating the box-merge. */
export function buildCarGeometry(): THREE.BufferGeometry {
  const body = boxGeo(BODY_W, BODY_H, BODY_L).clone();
  body.translate(0, BODY_H / 2, 0);
  const cabin = boxGeo(CABIN_W, CABIN_H, CABIN_L).clone();
  // cabin sits on top, nudged toward the rear (+z here is forward; rear = -z)
  cabin.translate(0, BODY_H + CABIN_H / 2, -0.3);
  const merged = mergeGeometries([body, cabin], false);
  if (!merged) throw new Error('car geometry merge failed');
  return merged;
}

/** Ground-surface height (in the car layer's local frame) at a world (x, z) —
 * i.e. the visible draped road/terrain y at that point, BEFORE the carriage +
 * lift offset. In main.ts this is `heightAt(x,z) - anchorGroundHeight`, matching
 * the shifted terrainRoot; unit tests omit it and the layer falls back to flat. */
export type GroundYAt = (x: number, z: number) => number;

/** The car layer object + its per-frame update entry point. */
export interface CarLayer {
  /** Add this to the scene. */
  object3d: THREE.Object3D;
  /** Dead-reckon every live vehicle to `nowTick` and write instance matrices.
   * `count` becomes the number of active instances. */
  update(net: TrafficNetGeom, vehicles: Map<number, VehKinematics>, nowTick: number): void;
}

export function createCarLayer(groundYAt?: GroundYAt): CarLayer {
  const geometry = buildCarGeometry();
  const material = clayMat(BODY_COLORS[0]);
  const mesh = new THREE.InstancedMesh(geometry, material, CAR_CAPACITY);
  mesh.name = 'trafficCars';
  mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  mesh.castShadow = true;
  mesh.receiveShadow = true;
  // Instances are placed all over the net each frame, far from the base
  // geometry's origin-centred bounding sphere. Three's default per-object
  // frustum cull tests that stale sphere, so as soon as the camera looks away
  // from the world origin the ENTIRE car mesh is culled and no cars render
  // (Task 10 finding). Disable per-object culling — matches the agent instanced
  // meshes (agentMeshes.ts), which place instances the same way.
  mesh.frustumCulled = false;
  mesh.count = 0;
  // Per-instance colour (stable per vehicle id via the id->slot map below).
  mesh.instanceColor = new THREE.InstancedBufferAttribute(new Float32Array(CAR_CAPACITY * 3), 3);
  // Carriage-surface offset above the (possibly draped) ground. When no ground
  // sampler is supplied the ground is treated as flat y=0, reproducing the
  // pre-#119 single-plate behaviour that the unit tests assert.
  const surfaceOffset = kswCity.roadYs.carriage + CAR_LIFT;

  // Reused scratch — no per-frame allocation on the hot path.
  const mat = new THREE.Matrix4();
  const pos = new THREE.Vector3();
  const quat = new THREE.Quaternion();
  const scl = new THREE.Vector3(1, 1, 1);
  const up = new THREE.Vector3(0, 1, 0);
  const col = new THREE.Color();

  // Stable colour per id: assign on first sight, keep it.
  const colorOfId = new Map<number, number>();

  function update(net: TrafficNetGeom, vehicles: Map<number, VehKinematics>, nowTick: number): void {
    let i = 0;
    for (const [id, veh] of vehicles) {
      if (i >= CAR_CAPACITY) break;
      const pose = poseAtBlended(net, veh, nowTick);
      const groundY = groundYAt ? groundYAt(pose.x, pose.z) : 0;
      pos.set(pose.x, groundY + surfaceOffset, pose.z);
      quat.setFromAxisAngle(up, pose.yaw);
      mat.compose(pos, quat, scl);
      mesh.setMatrixAt(i, mat);

      let bodyColor = colorOfId.get(id);
      if (bodyColor === undefined) {
        bodyColor = BODY_COLORS[hashId(id) % BODY_COLORS.length];
        colorOfId.set(id, bodyColor);
      }
      col.set(bodyColor);
      mesh.setColorAt(i, col);
      i++;
    }
    mesh.count = i;
    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    // Bound the colour map so a long session with heavy slot recycling doesn't
    // grow it without limit: prune ids no longer present once it gets large.
    if (colorOfId.size > CAR_CAPACITY * 2) {
      for (const id of colorOfId.keys()) if (!vehicles.has(id)) colorOfId.delete(id);
    }
  }

  return { object3d: mesh, update };
}

/** Cheap integer hash so a vehicle's colour is stable and well-spread. */
function hashId(id: number): number {
  let h = id >>> 0;
  h = Math.imul(h ^ (h >>> 16), 0x45d9f3b);
  h = Math.imul(h ^ (h >>> 16), 0x45d9f3b);
  h = h ^ (h >>> 16);
  return h >>> 0;
}
