// src/diorama/traffic/carLayer.ts
//
// The instanced car layer (FIX D2): Cities-Skylines-school clay cars. THREE
// InstancedMeshes — one per silhouette VARIANT (sedan / hatchback / van) —
// selected per-vehicle by a stable id hash, total capacity split across the
// three (4096). Each variant geometry is a merged clay shape with:
//   * a body with a hood/trunk step (sedan) or a hatch/cargo slope,
//   * a cabin with an INSET darker window band (baked via vertex colors),
//   * 4 wheel stubs (dark boxes, part of the merged geometry),
//   * a subtle roof highlight.
// NO textures — the diorama's clay/no-texture language. Windows + wheels are
// dark VERTEX colors; the body is white vertex color so the per-instance tint
// (setColorAt) shows through it. MeshPhysicalMaterial multiplies
// instanceColor × vertexColor, so the same instanced-colour wiring the KSW
// agent meshes use (agentMeshes.ts) still gives each car its own body colour
// while the glass/wheels stay dark (they pick up only a faint body tint).
//
// Palette (BODY_COLORS): a CS-like set of saturated-but-clay distinct car
// colours (whites/silvers/reds/blues/greens/yellow/black) — they pop against
// the tarmac under the soft GI without going neon.
//
// Cars sit ON the carriage road surface plus a small lift so the body clears
// the ribbon. Post-#119 the whole traffic plate is draped on real DEM terrain,
// so the layer samples the ground height PER VEHICLE via the `groundYAt(x, z)`
// callback (built from the world DEM in main.ts) and adds the carriage+lift
// offset. Without a sampler it falls back to the old flat carriage y (used by
// unit tests, which have no world). The lane polylines are already offset to
// the Swiss right-hand side (baked), so following them exactly puts cars on the
// correct side of two-way streets — the layer applies NO horizontal offset.
//
// The lane-change blend & bezier junction sweep (FIX-C2, laneBlend.ts) drive
// the pose exactly as before (poseAtBlended) — the update path is unchanged
// except that each vehicle now writes into its variant's mesh.

import * as THREE from 'three/webgpu';
import { kswCity, clay } from '../designTokens';
import { boxGeo } from '../ksw/geometryCache';
import { type TrafficNetGeom, type VehKinematics } from './deadReckon';
import { poseAtBlended } from './laneBlend';
import { CAR_VARIANTS, carColorForId, carVariantForId, CAR_PALETTE } from './carModels';

/** Instance capacity for VISIBLE cars (the AOI-subscribed cells only, not the
 * whole fleet — the server-side fleet cap is 30k since Task 8, but a browser
 * AOI sees a small fraction; the measured whole-Gemeinde morning peak is
 * ~2.4k alive at demand_scale 1.0). Split across the variant meshes. */
export const CAR_CAPACITY = 4096;

/** Per-variant capacity. A single busy view is nowhere near this per variant,
 * but the cap must cover a pathological all-one-variant fleet, so give each the
 * full fleet cap headroom. */
const PER_VARIANT_CAPACITY = 2048;

/** Ground clearance above the carriage ribbon so the wheels don't z-fight it. */
const CAR_LIFT = 0.04;

/** Re-export the palette for tests/tools. */
export { CAR_PALETTE };

/** A representative single car geometry for flowLayer.ts's far-LOD impostor
 * mesh (Task 12 brief: "second InstancedMesh reusing the clay-car geometry").
 * Delegates to variant 0 (the plain sedan) — at flow-LOD distances the
 * variant differences are sub-pixel, so one shape stands in for the fleet. */
export function buildCarGeometry(): THREE.BufferGeometry {
  return CAR_VARIANTS[0].build(boxGeo);
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
   * The count of active instances is split across the variant meshes. */
  update(net: TrafficNetGeom, vehicles: Map<number, VehKinematics>, nowTick: number): void;
}

/** Vertex-coloured clay material for the cars: same clay recipe as clayMat but
 * with `vertexColors` on so the baked white-body / dark-glass / dark-wheel
 * vertex colours multiply the per-instance body tint. Base colour white so the
 * instance tint is unmodified on the body faces. */
function carMaterial(): THREE.MeshPhysicalMaterial {
  const m = new THREE.MeshPhysicalMaterial({
    color: 0xffffff,
    roughness: clay.roughness,
    metalness: clay.metalness,
    vertexColors: true,
  });
  m.sheen = clay.sheen;
  m.sheenRoughness = clay.sheenRoughness;
  m.sheenColor = new THREE.Color(0xffffff).lerp(new THREE.Color(0xffffff), clay.sheenLerp);
  return m;
}

export function createCarLayer(groundYAt?: GroundYAt): CarLayer {
  const group = new THREE.Group();
  group.name = 'trafficCars';
  const material = carMaterial();

  // One InstancedMesh per silhouette variant. They share the clay material; the
  // per-vertex colours (baked into each geometry) differ, and the per-instance
  // body tint is written via setColorAt.
  const meshes: THREE.InstancedMesh[] = CAR_VARIANTS.map((variant) => {
    const geometry = variant.build(boxGeo);
    const mesh = new THREE.InstancedMesh(geometry, material, PER_VARIANT_CAPACITY);
    mesh.name = `trafficCars_${variant.name}`;
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    // Instances are placed all over the net each frame, far from the base
    // geometry's origin-centred bounding sphere, so Three's per-object frustum
    // cull (stale sphere) would drop the whole mesh as soon as the camera looks
    // away from the world origin. Disable it — matches agentMeshes.ts.
    mesh.frustumCulled = false;
    mesh.count = 0;
    mesh.instanceColor = new THREE.InstancedBufferAttribute(
      new Float32Array(PER_VARIANT_CAPACITY * 3),
      3,
    );
    group.add(mesh);
    return mesh;
  });

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

  // Stable per-id assignment (variant + colour): assign on first sight, keep it.
  const variantOfId = new Map<number, number>();
  const colorOfId = new Map<number, number>();
  // Per-variant write cursor, reset each frame.
  const counts = new Array<number>(meshes.length).fill(0);

  function update(net: TrafficNetGeom, vehicles: Map<number, VehKinematics>, nowTick: number): void {
    counts.fill(0);
    for (const [id, veh] of vehicles) {
      let variant = variantOfId.get(id);
      if (variant === undefined) {
        variant = carVariantForId(id);
        variantOfId.set(id, variant);
      }
      const mesh = meshes[variant];
      const i = counts[variant];
      if (i >= PER_VARIANT_CAPACITY) continue;

      const pose = poseAtBlended(net, veh, nowTick);
      const groundY = groundYAt ? groundYAt(pose.x, pose.z) : 0;
      pos.set(pose.x, groundY + surfaceOffset, pose.z);
      quat.setFromAxisAngle(up, pose.yaw);
      mat.compose(pos, quat, scl);
      mesh.setMatrixAt(i, mat);

      let bodyColor = colorOfId.get(id);
      if (bodyColor === undefined) {
        bodyColor = carColorForId(id);
        colorOfId.set(id, bodyColor);
      }
      col.set(bodyColor);
      mesh.setColorAt(i, col);
      counts[variant] = i + 1;
    }
    for (let v = 0; v < meshes.length; v++) {
      const mesh = meshes[v];
      mesh.count = counts[v];
      mesh.instanceMatrix.needsUpdate = true;
      if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    }
    // Bound the per-id maps so a long session with heavy slot recycling doesn't
    // grow them without limit: prune ids no longer present once they get large.
    if (colorOfId.size > CAR_CAPACITY * 2) {
      for (const id of colorOfId.keys()) {
        if (!vehicles.has(id)) {
          colorOfId.delete(id);
          variantOfId.delete(id);
        }
      }
    }
  }

  return { object3d: group, update };
}
