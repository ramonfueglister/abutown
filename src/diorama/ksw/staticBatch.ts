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
import { float, varyingProperty } from 'three/tsl';
import { clay, nightGlow, palette, roofFadePolicy } from '../designTokens';
import { claySheenNode } from './clayNodes';
import { ensureSequentialIndex } from './geometryCache';
import { lampGlowU } from './glowUniform';
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
// Glow geometry is ALWAYS routed into the glowNight bucket now; its intensity
// rides the shared lampGlowU uniform (0 at day = invisible), so the bake no
// longer decides day-vs-night — the runtime uniform does.
export function classifyMesh(mesh: THREE.Mesh): BucketName {
  const mat = mesh.material as THREE.Material;
  if (mesh.userData.roofFade) return 'roofFade';
  if ((mat as THREE.MeshBasicMaterial).isMeshBasicMaterial) return 'glow';
  if (mesh.userData.lampBulb) return 'glowNight';
  if ((mat as THREE.MeshPhysicalMaterial).isMeshPhysicalMaterial) {
    return mesh.castShadow ? 'clay' : 'clayNoCast';
  }
  if ((mat as THREE.MeshStandardMaterial).isMeshStandardMaterial) {
    // the shared glassMat: window panes, vehicle glass, domes
    if (mesh.userData.windowPane) {
      const wp = new THREE.Vector3().setFromMatrixPosition(mesh.matrixWorld);
      if (nightWindowHash(wp.x, wp.z) < NIGHT_WINDOW_SHARE) return 'glowNight';
    }
    return 'glass';
  }
  throw new Error(`staticBatch: unclassifiable material "${mat.type}" on mesh "${mesh.name}"`);
}

// userData tags marking objects main.ts animates per-frame: the ambulance
// blinker (userData.blink on the mesh, visibility toggled) and the heli
// rotor (userData.rotor on the parent group, rotated). Shared with main.ts's
// collection traverse — one string contract, not two.
export const ANIMATED_TAGS = ['blink', 'rotor'] as const;

// Meshes animated by main.ts keep their own draw call (see ANIMATED_TAGS).
function isAnimated(o: THREE.Object3D): boolean {
  let cur: THREE.Object3D | null = o;
  while (cur) {
    for (const tag of ANIMATED_TAGS) if (cur.userData[tag]) return true;
    cur = cur.parent;
  }
  return false;
}

// One MeshPhysicalNodeMaterial replaces the per-color clayMat map: diffuse
// comes from the per-instance batch color, and the clay sheen recipe
// (clayNodes.claySheenNode) moves into TSL so it stays per-instance too.
function clayBatchMaterial(transparent: boolean): THREE.MeshPhysicalNodeMaterial {
  const m = new THREE.MeshPhysicalNodeMaterial({
    color: palette.trueWhite, // multiplied by the per-instance batch color
    roughness: clay.roughness,
    metalness: clay.metalness,
    transparent,
  });
  m.sheenRoughness = clay.sheenRoughness;
  m.sheenNode = claySheenNode(batchColor.rgb);
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
        // depthWrite off: the batch draws with sortObjects=false, so
        // depth-writing transparent instances would depth-reject each other
        // in insertion order (popping between overlapping panes/domes).
        material: new THREE.MeshStandardMaterial({
          color: palette.glass,
          roughness: 0.4,
          metalness: 0,
          transparent: true,
          opacity: 0.16,
          depthWrite: false,
        }),
        perInstanceColor: false,
        castShadow: false,
        receiveShadow: false,
      };
    case 'glow':
      // unlit emissive-ish surfaces: device screens, op-light faces
      return {
        material: new THREE.MeshBasicMaterial({ color: palette.trueWhite }),
        perInstanceColor: true,
        castShadow: false,
        receiveShadow: false,
      };
    case 'glowNight': {
      // warm night glow: lamp bulbs + the hashed share of window panes. Opacity
      // rides the shared lampGlowU uniform (0 = day, fully transparent; 0.9 at
      // full night), so the geometry is always present but only shows at night.
      // depthWrite off for the same unsorted-transparency reason as glass.
      const glowMat = new THREE.MeshBasicNodeMaterial({ color: nightGlow.bulb, transparent: true, depthWrite: false });
      glowMat.opacityNode = float(0.9).mul(lampGlowU);
      return {
        material: glowMat,
        perInstanceColor: false,
        castShadow: false,
        receiveShadow: false,
      };
    }
    case 'roofFade':
      return { material: clayBatchMaterial(true), perInstanceColor: true, castShadow: true, receiveShadow: true };
  }
}

// Hoist every batchable Mesh under `group` into per-bucket BatchedMeshes,
// remove the originals (and the groups left empty), and return the batches
// plus the RoofControl driving the roofFade bucket.
export function batchHospital(group: THREE.Group): { batches: THREE.BatchedMesh[]; roofs: RoofControl } {
  group.updateMatrixWorld(true);

  const buckets = new Map<BucketName, THREE.Mesh[]>();
  group.traverse((o) => {
    const mesh = o as THREE.Mesh;
    if (!mesh.isMesh || isAnimated(mesh)) return;
    const name = classifyMesh(mesh);
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
      ensureSequentialIndex(geo);
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

  // Same thresholds as the pre-batching per-mesh RoofControl (shared via
  // designTokens.roofFadePolicy with main.ts's refresh triggers): shadows
  // drop early in the fade (while the lid is still clearly visible) so the
  // interior lights up smoothly instead of popping bright at the very end.
  let currentFade = 1;
  const roofs: RoofControl = {
    setFade(fade01: number) {
      currentFade = Math.min(Math.max(fade01, 0), 1);
      if (roofMaterial) {
        roofMaterial.opacity = currentFade;
        // depthWrite only while fully opaque: settled roofs must occlude
        // correctly (the lids overlap trim/walls), but during the fade the
        // batch draws unsorted (sortObjects=false) — depth-writing
        // half-transparent lids would depth-reject each other in insertion
        // order and pop. No depth writes while fading = stable blending.
        roofMaterial.depthWrite = currentFade >= roofFadePolicy.opaque;
      }
      if (roofBatch) {
        roofBatch.castShadow = currentFade > roofFadePolicy.castShadow;
        roofBatch.visible = currentFade > roofFadePolicy.visible;
      }
    },
    fade: () => currentFade,
  };
  return { batches, roofs };
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
