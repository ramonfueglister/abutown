// Batches the static KSW hospital scene into a handful of THREE.BatchedMesh
// buckets (Slice B of the 10k-perf design). The builders in building.ts/
// props.ts still assemble thousands of individual Meshes; this module hoists
// every static one into a per-material-class bucket, dedupes geometry via the
// Slice-A geometryCache (equal params = same BufferGeometry instance) and
// removes the originals. Result: the whole hospital renders in ~6 draw calls
// instead of thousands, with identical colors, clay sheen, roof fade and
// night window glow.
//
// Excluded (stay individual Meshes): anything main.ts animates per-frame —
// meshes tagged userData.blink (ambulance light) or inside a userData.rotor
// group (heli rotor) — and everything main.ts adds after building (people,
// mist, sky, discs, stars).

import * as THREE from 'three/webgpu';
import { float, mix, varyingProperty, vec3 } from 'three/tsl';
import { clay, nightGlow, palette } from '../designTokens';
import type { RoofControl } from './building';

// three r185 defines `batchColor` (three/src/nodes/accessors/Batch.js) but
// does not re-export it from 'three/tsl'. PropertyNodes hash by name, so
// recreating the varying under the same name resolves to the exact node the
// BatchedMesh vertex setup assigns the per-instance color to.
const batchColor = varyingProperty('vec4', 'vBatchColor');

export type BucketName = 'clay' | 'clayNoCast' | 'glass' | 'glow' | 'glowNight' | 'roofFade';

// Deterministic per-window night-glow choice (no RNG): hash the world
// position. Formula identical to the pre-batching material swap in main.ts.
export function nightWindowHash(x: number, z: number): number {
  return Math.abs(Math.sin(x * 12.9898 + z * 78.233) * 43758.5453) % 1;
}

// Share of window panes that glow warm from inside at night.
export const NIGHT_WINDOW_SHARE = 0.55;

// Which bucket a static mesh belongs to. Pure, given the mesh's material
// class, userData tags, shadow flags and (for night windows) matrixWorld.
export function classifyMesh(mesh: THREE.Mesh, opts: { lampGlow: boolean }): BucketName {
  const mat = mesh.material as THREE.Material;
  if (mesh.userData.roofFade) return 'roofFade';
  if ((mat as THREE.MeshBasicMaterial).isMeshBasicMaterial) return 'glow';
  if (opts.lampGlow && mesh.userData.lampBulb) return 'glowNight';
  if ((mat as THREE.MeshPhysicalMaterial).isMeshPhysicalMaterial) {
    return mesh.castShadow ? 'clay' : 'clayNoCast';
  }
  if ((mat as THREE.MeshStandardMaterial).isMeshStandardMaterial) {
    // the shared glassMat: window panes, vehicle glass, domes
    if (opts.lampGlow && mesh.userData.windowPane) {
      const wp = new THREE.Vector3().setFromMatrixPosition(mesh.matrixWorld);
      if (nightWindowHash(wp.x, wp.z) < NIGHT_WINDOW_SHARE) return 'glowNight';
    }
    return 'glass';
  }
  throw new Error(`staticBatch: unclassifiable material "${mat.type}" on mesh "${mesh.name}"`);
}

// Meshes animated by main.ts keep their own draw call: the ambulance blinker
// (userData.blink on the mesh) and the heli rotor (userData.rotor on the
// parent group whose children are plain clay meshes).
function isAnimated(o: THREE.Object3D): boolean {
  let cur: THREE.Object3D | null = o;
  while (cur) {
    if (cur.userData.blink || cur.userData.rotor) return true;
    cur = cur.parent;
  }
  return false;
}

// One MeshPhysicalNodeMaterial replaces the per-color clayMat map: diffuse
// comes from the per-instance batch color, and the clay sheen recipe
// (sheenColor = color lerped 50% to white, scaled by clay.sheen) moves into
// TSL so it stays per-instance too.
function clayBatchMaterial(transparent: boolean): THREE.MeshPhysicalNodeMaterial {
  const m = new THREE.MeshPhysicalNodeMaterial({
    color: 0xffffff, // multiplied by the per-instance batch color
    roughness: clay.roughness,
    metalness: clay.metalness,
    transparent,
  });
  m.sheenRoughness = clay.sheenRoughness;
  m.sheenNode = mix(batchColor.rgb, vec3(1, 1, 1), 0.5).mul(float(clay.sheen));
  return m;
}

type BucketSpec = {
  material: THREE.Material;
  perInstanceColor: boolean;
  castShadow: boolean;
  receiveShadow: boolean;
};

function bucketSpec(name: BucketName): BucketSpec {
  switch (name) {
    case 'clay':
      return { material: clayBatchMaterial(false), perInstanceColor: true, castShadow: true, receiveShadow: true };
    case 'clayNoCast':
      // room floor inlays: receive but never cast
      return { material: clayBatchMaterial(false), perInstanceColor: true, castShadow: false, receiveShadow: true };
    case 'glass':
      return {
        material: new THREE.MeshStandardMaterial({
          color: palette.glass,
          roughness: 0.4,
          metalness: 0,
          transparent: true,
          opacity: 0.16,
        }),
        perInstanceColor: false,
        castShadow: false,
        receiveShadow: false,
      };
    case 'glow':
      // unlit emissive-ish surfaces: device screens, op-light faces
      return {
        material: new THREE.MeshBasicMaterial({ color: 0xffffff }),
        perInstanceColor: true,
        castShadow: false,
        receiveShadow: false,
      };
    case 'glowNight':
      // warm night glow: lamp bulbs + the hashed share of window panes
      return {
        material: new THREE.MeshBasicMaterial({ color: nightGlow.bulb, transparent: true, opacity: 0.9 }),
        perInstanceColor: false,
        castShadow: false,
        receiveShadow: false,
      };
    case 'roofFade':
      return { material: clayBatchMaterial(true), perInstanceColor: true, castShadow: true, receiveShadow: true };
  }
}

// Hoist every batchable Mesh under `group` into per-bucket BatchedMeshes,
// remove the originals (and the groups left empty), and return the batches
// plus the RoofControl driving the roofFade bucket.
export function batchHospital(group: THREE.Group, opts: { lampGlow: boolean }): { batches: THREE.BatchedMesh[]; roofs: RoofControl } {
  group.updateMatrixWorld(true);

  const buckets = new Map<BucketName, THREE.Mesh[]>();
  group.traverse((o) => {
    const mesh = o as THREE.Mesh;
    if (!mesh.isMesh || isAnimated(mesh)) return;
    const name = classifyMesh(mesh, opts);
    const list = buckets.get(name);
    if (list) list.push(mesh);
    else buckets.set(name, [mesh]);
  });

  const batches: THREE.BatchedMesh[] = [];
  let roofBatch: THREE.BatchedMesh | null = null;
  let roofMaterial: THREE.Material | null = null;
  const color = new THREE.Color();

  for (const [name, meshes] of buckets) {
    // exact capacities: instances, plus vertex/index counts summed over the
    // *unique* geometries (Slice A guarantees equal params = same instance)
    const unique = new Set(meshes.map((m) => m.geometry));
    let vertexCount = 0;
    let indexCount = 0;
    for (const geo of unique) {
      ensureIndexed(geo);
      vertexCount += geo.attributes.position.count;
      indexCount += geo.index!.count;
    }
    const spec = bucketSpec(name);
    const batch = new THREE.BatchedMesh(meshes.length, vertexCount, indexCount, spec.material);
    batch.name = `ksw-${name}`;
    batch.castShadow = spec.castShadow;
    batch.receiveShadow = spec.receiveShadow;
    // The scene is fully static: freeze the multi-draw ranges. With sorting
    // and per-instance culling enabled, BatchedMesh re-walks every instance
    // (matrix fetch + bounding-sphere transform + sort) on EVERY pass of
    // every frame — measured as the dominant CPU cost here, for a scene
    // where the overview camera sees nearly everything anyway.
    batch.sortObjects = false;
    batch.perObjectFrustumCulled = false;

    const geometryIds = new Map<THREE.BufferGeometry, number>();
    for (const mesh of meshes) {
      let geometryId = geometryIds.get(mesh.geometry);
      if (geometryId === undefined) {
        geometryId = batch.addGeometry(mesh.geometry);
        geometryIds.set(mesh.geometry, geometryId);
      }
      const instanceId = batch.addInstance(geometryId);
      batch.setMatrixAt(instanceId, mesh.matrixWorld);
      if (spec.perInstanceColor) {
        batch.setColorAt(instanceId, color.copy((mesh.material as THREE.MeshStandardMaterial).color));
      }
    }
    // three r185 WebGPU footgun: the backend patches Uint16 index attributes
    // to Uint32 IN PLACE on first upload (WebGPUAttributeUtils "patch for
    // UINT16"). Our frozen multi-draw starts are byte offsets computed from
    // the index's BYTES_PER_ELEMENT — built once at boot with 2, divided by 4
    // after the patch, drawing garbage ranges. Force Uint32 up front so the
    // byte math is stable across the whole lifetime.
    const batchIndex = batch.geometry.getIndex();
    if (batchIndex !== null && batchIndex.array instanceof Uint16Array) {
      batch.geometry.setIndex(new THREE.BufferAttribute(new Uint32Array(batchIndex.array), 1));
    }
    if (name === 'roofFade') {
      roofBatch = batch;
      roofMaterial = spec.material;
    }
    batches.push(batch);
  }

  for (const meshes of buckets.values()) {
    for (const mesh of meshes) mesh.removeFromParent();
  }
  pruneEmptyGroups(group);
  for (const batch of batches) group.add(batch);

  // Same thresholds as the pre-batching per-mesh RoofControl: shadows drop
  // early in the fade (while the lid is still clearly visible) so the
  // interior lights up smoothly instead of popping bright at the very end.
  let currentFade = 1;
  const roofs: RoofControl = {
    setFade(fade01: number) {
      currentFade = Math.min(Math.max(fade01, 0), 1);
      if (roofMaterial) roofMaterial.opacity = currentFade;
      if (roofBatch) {
        roofBatch.castShadow = currentFade > 0.6;
        roofBatch.visible = currentFade > 0.02;
      }
    },
    fade: () => currentFade,
  };
  return { batches, roofs };
}

// BatchedMesh requires every geometry in a batch to consistently have an
// index. RoundedBoxGeometry is non-indexed while the other cached primitives
// are indexed; giving it a trivial sequential index renders identically and
// makes the buckets homogeneous. (Safe on shared cache instances: the index
// doesn't change what's drawn for remaining individual meshes either.)
function ensureIndexed(geo: THREE.BufferGeometry): void {
  if (geo.index !== null) return;
  const count = geo.attributes.position.count;
  const index = new (count > 65535 ? Uint32Array : Uint16Array)(count);
  for (let i = 0; i < count; i++) index[i] = i;
  geo.setIndex(new THREE.BufferAttribute(index, 1));
}

// Batching empties most builder Groups (walls, signs, props); drop them so
// the renderer doesn't walk hundreds of dead nodes every frame. Groups that
// still hold excluded meshes (blinker, rotor) survive.
function pruneEmptyGroups(root: THREE.Object3D): void {
  for (const child of [...root.children]) pruneEmptyGroups(child);
  if (root.parent !== null && root.children.length === 0 && !(root as THREE.Mesh).isMesh) {
    root.removeFromParent();
  }
}
