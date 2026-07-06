// src/diorama/traffic/carLayer.ts
//
// The instanced car layer (FIX D2): Cities-Skylines-school cars — bodies,
// glass, and spinning/steering wheels, all instanced. THREE separate mesh
// families feed the frame:
//   * one BODY InstancedMesh per silhouette VARIANT (sedan / hatchback / wagon /
//     suv / van / pickup), selected per-vehicle by a stable id hash. Body faces
//     are white vertex-colour so the per-instance tint (setColorAt) shows
//     through; grille/lights/plates are baked in their own colours;
//   * one GLASS InstancedMesh per variant — a bright light-blue loft shell on a
//     low-roughness clearcoat-free physical material (envMapIntensity 1.6 so it
//     reads as reflective sky). Uniform: NO per-instance colour, casts no shadow;
//   * ONE wheel InstancedMesh shared by the whole fleet — a vertex-coloured
//     cylinder about the axle, 4 per car, scaled per variant, ROLLED by dead-
//     reckoned speed and STEERED (front axle) from the yaw rate (wheelSpin.ts).
//
// The paint material is a clearcoat MeshPhysicalMaterial (clearcoat 1.0) so the
// bodies get the glossy CS car-paint highlight; glass is a separate bright,
// low-roughness shell. NO textures — the diorama's clay/no-texture language:
// geometry + vertex colours only.
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
// except that each vehicle now writes its body into its variant's mesh, a
// matching glass instance, and four wheel instances into the shared wheel mesh.

import * as THREE from 'three/webgpu';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import { kswCity } from '../designTokens';
import { boxGeo } from '../ksw/geometryCache';
import { type TrafficNetGeom, type VehKinematics } from './deadReckon';
import { poseAtBlended } from './laneBlend';
import {
  CAR_VARIANTS,
  carColorForId,
  carVariantForId,
  CAR_PALETTE,
  wheelOffsets,
  buildWheelGeometry,
  WHEEL_GEO_RADIUS,
} from './carModels';
import { type SpinState, initSpin, advanceSpin } from './wheelSpin';

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
 * variant differences are sub-pixel, so one shape stands in for the fleet.
 * Returns a merged static geometry: sedan body + glass + 4 wheels. */
export function buildCarGeometry(): THREE.BufferGeometry {
  const sedan = CAR_VARIANTS[0];
  const parts: THREE.BufferGeometry[] = [
    sedan.buildBody(boxGeo).toNonIndexed(),
    sedan.buildGlass(),
  ];
  const s = sedan.wheels.radius / WHEEL_GEO_RADIUS;
  for (const off of wheelOffsets(sedan.wheels)) {
    const w = buildWheelGeometry().clone();
    w.scale(s, s, s);
    w.translate(off[0], off[1], off[2]);
    parts.push(w);
  }
  const merged = mergeGeometries(parts.map((p) => (p.index ? p.toNonIndexed() : p)), false);
  if (!merged) throw new Error('carLayer: impostor merge failed');
  merged.computeVertexNormals();
  merged.computeBoundingSphere();
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
   * The count of active instances is split across the variant meshes. */
  update(net: TrafficNetGeom, vehicles: Map<number, VehKinematics>, nowTick: number): void;
  /** Dev-only debug surface for the Task 10 browser smoke: per-variant body
   * counts, total wheel count, and one wheel instance matrix (JSON-safe). */
  debug: {
    variantCounts(): number[];
    wheelCount(): number;
    wheelMatrix(i: number): number[];
  };
}

/** CS car-paint material: white base × per-vertex white body / baked detail
 * colours × per-instance body tint (setColorAt). A clearcoat gives the glossy
 * highlight over a satin base coat — the Cities-Skylines car-paint look. */
function paintMaterial(): THREE.MeshPhysicalMaterial {
  const m = new THREE.MeshPhysicalMaterial({
    color: 0xffffff, vertexColors: true,
    roughness: 0.45, metalness: 0.1,
  });
  m.clearcoat = 1.0;
  m.clearcoatRoughness = 0.15;
  return m;
}

/** CS glass: bright, low-roughness, mildly metallic reflective shell. The baked
 * light-blue vertex colour carries the sky tint; envMapIntensity boosts the
 * reflection so windows read as glass, not paint. Uniform — no per-instance
 * colour. */
function glassMaterial(): THREE.MeshPhysicalMaterial {
  const m = new THREE.MeshPhysicalMaterial({
    color: 0xffffff, vertexColors: true, // baked light-blue vertex colour carries the tint
    roughness: 0.05, metalness: 0.4,
  });
  m.envMapIntensity = 1.6;
  return m;
}

export function createCarLayer(groundYAt?: GroundYAt): CarLayer {
  const group = new THREE.Group();
  group.name = 'trafficCars';
  const paint = paintMaterial();
  const glass = glassMaterial();

  // One BODY InstancedMesh per silhouette variant. They share the paint
  // material; the per-vertex colours (baked into each geometry) differ, and the
  // per-instance body tint is written via setColorAt.
  const bodyMeshes: THREE.InstancedMesh[] = CAR_VARIANTS.map((variant) => {
    const mesh = new THREE.InstancedMesh(variant.buildBody(boxGeo), paint, PER_VARIANT_CAPACITY);
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

  // One GLASS InstancedMesh per variant. Uniform bright glass — no per-instance
  // colour, and no shadow casting (a glass shell throwing a hard shadow reads
  // wrong; the body already casts the car silhouette).
  const glassMeshes: THREE.InstancedMesh[] = CAR_VARIANTS.map((variant) => {
    const mesh = new THREE.InstancedMesh(variant.buildGlass(), glass, PER_VARIANT_CAPACITY);
    mesh.name = `trafficCarsGlass_${variant.name}`;
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    mesh.castShadow = false;
    mesh.receiveShadow = true;
    mesh.frustumCulled = false;
    mesh.count = 0;
    group.add(mesh);
    return mesh;
  });

  // ONE wheel InstancedMesh shared by the whole fleet: 4 wheels per car, up to
  // the fleet cap. Painted (rubber+rim baked vertex colours); instanceColor
  // filled once with white so the paint material's vertexColors path multiplies
  // cleanly. Wheels roll + steer per frame (wheelSpin.ts).
  const WHEEL_CAPACITY = 4 * CAR_CAPACITY;
  const wheelMesh = new THREE.InstancedMesh(buildWheelGeometry(), paint, WHEEL_CAPACITY);
  wheelMesh.name = 'trafficWheels';
  wheelMesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  wheelMesh.castShadow = true;
  wheelMesh.receiveShadow = true;
  wheelMesh.frustumCulled = false;
  wheelMesh.count = 0;
  wheelMesh.instanceColor = new THREE.InstancedBufferAttribute(
    new Float32Array(WHEEL_CAPACITY * 3),
    3,
  );
  {
    const white = wheelMesh.instanceColor.array as Float32Array;
    white.fill(1);
    wheelMesh.instanceColor.needsUpdate = true;
  }
  group.add(wheelMesh);

  // Precompute the 4 wheel offsets per variant once at layer build.
  const offsets: [number, number, number][][] = CAR_VARIANTS.map((v) => wheelOffsets(v.wheels));

  // Carriage-surface offset above the (possibly draped) ground. When no ground
  // sampler is supplied the ground is treated as flat y=0, reproducing the
  // pre-#119 single-plate behaviour that the unit tests assert.
  const surfaceOffset = kswCity.roadYs.carriage + CAR_LIFT;

  // Reused scratch — no per-frame allocation on the hot path.
  const bodyMat = new THREE.Matrix4();
  const pos = new THREE.Vector3();
  const quat = new THREE.Quaternion();
  const scl = new THREE.Vector3(1, 1, 1);
  const up = new THREE.Vector3(0, 1, 0);
  const col = new THREE.Color();
  const wheelMat = new THREE.Matrix4();
  const wpos = new THREE.Vector3();
  const wquat = new THREE.Quaternion();
  const wscl = new THREE.Vector3(1, 1, 1);
  const weuler = new THREE.Euler();

  // Stable per-id assignment (variant + colour): assign on first sight, keep it.
  const variantOfId = new Map<number, number>();
  const colorOfId = new Map<number, number>();
  // Per-id wheel spin/steer state, mutated in place each frame.
  const spinOfId = new Map<number, SpinState>();
  // Per-variant write cursor, reset each frame.
  const counts = new Array<number>(bodyMeshes.length).fill(0);

  function update(net: TrafficNetGeom, vehicles: Map<number, VehKinematics>, nowTick: number): void {
    counts.fill(0);
    let wheelCursor = 0;
    for (const [id, veh] of vehicles) {
      let variant = variantOfId.get(id);
      if (variant === undefined) {
        variant = carVariantForId(id);
        variantOfId.set(id, variant);
      }
      const i = counts[variant];
      if (i >= PER_VARIANT_CAPACITY) continue;

      const pose = poseAtBlended(net, veh, nowTick);
      const groundY = groundYAt ? groundYAt(pose.x, pose.z) : 0;
      pos.set(pose.x, groundY + surfaceOffset, pose.z);
      quat.setFromAxisAngle(up, pose.yaw);
      bodyMat.compose(pos, quat, scl);
      bodyMeshes[variant].setMatrixAt(i, bodyMat);
      glassMeshes[variant].setMatrixAt(i, bodyMat);

      let bodyColor = colorOfId.get(id);
      if (bodyColor === undefined) {
        bodyColor = carColorForId(id);
        colorOfId.set(id, bodyColor);
      }
      col.set(bodyColor);
      bodyMeshes[variant].setColorAt(i, col);
      counts[variant] = i + 1;

      // Wheels — roll + steer, then place four instances relative to the body.
      const layout = CAR_VARIANTS[variant].wheels;
      let spin = spinOfId.get(id);
      if (spin === undefined) { spin = initSpin(nowTick, pose.yaw); spinOfId.set(id, spin); }
      advanceSpin(spin, veh.v, pose.yaw, nowTick, layout.radius);
      const s = layout.radius / WHEEL_GEO_RADIUS;
      for (let w = 0; w < 4; w++) {
        const off = offsets[variant][w];
        wpos.set(off[0], off[1], off[2]);
        weuler.set(spin.theta, w < 2 ? spin.steer : 0, 0, 'YXZ'); // steer about y THEN roll about x
        wquat.setFromEuler(weuler);
        wscl.setScalar(s);
        wheelMat.compose(wpos, wquat, wscl).premultiply(bodyMat);
        wheelMesh.setMatrixAt(wheelCursor++, wheelMat);
      }
    }

    for (let v = 0; v < bodyMeshes.length; v++) {
      const body = bodyMeshes[v];
      const glassMesh = glassMeshes[v];
      body.count = counts[v];
      glassMesh.count = counts[v];
      body.instanceMatrix.needsUpdate = true;
      glassMesh.instanceMatrix.needsUpdate = true;
      if (body.instanceColor) body.instanceColor.needsUpdate = true;
    }
    wheelMesh.count = wheelCursor;
    wheelMesh.instanceMatrix.needsUpdate = true;

    // Bound the per-id maps so a long session with heavy slot recycling doesn't
    // grow them without limit: prune ids no longer present once they get large.
    if (colorOfId.size > CAR_CAPACITY * 2) {
      for (const id of colorOfId.keys()) {
        if (!vehicles.has(id)) {
          colorOfId.delete(id);
          variantOfId.delete(id);
          spinOfId.delete(id);
        }
      }
    }
  }

  return {
    object3d: group,
    update,
    debug: {
      variantCounts: () => bodyMeshes.map((m) => m.count),
      wheelCount: () => wheelMesh.count,
      wheelMatrix: (i: number) => {
        const m = new THREE.Matrix4();
        wheelMesh.getMatrixAt(Math.max(0, Math.min(wheelMesh.count - 1, i)), m);
        return Array.from(m.elements); // JSON-safe for the CDP smoke
      },
    },
  };
}
