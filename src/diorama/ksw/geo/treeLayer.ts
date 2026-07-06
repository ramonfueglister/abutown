// src/diorama/ksw/geo/treeLayer.ts
// Instanced tree rendering over the procedural archetypes: one InstancedMesh
// per archetype (trunk+crown in one geometry, aPuff-selective tint), TSL
// per-instance jitter + crown gradient + weather-coupled wind. Deterministic.
//
// Scale contract (the point of this slice): a spec's `h`/`r` are real meters.
// The archetypes are normalized (y 0..1, horizontal reach = crownRadius), so
// the instance scale (r/crownRadius, h·squash, r/crownRadius) maps them 1:1
// onto world meters — heights and crown widths are literal.
//
// Per-instance colour: TSL on this three build exposes no `instanceColor`
// node, and setColorAt() only feeds the fixed-function pipeline — it does NOT
// reach a node material's colorNode. So each mesh carries its own `aTint`
// InstancedBufferAttribute (vec3, one RGB per instance) read via the
// `instancedBufferAttribute` node inside colorNode. instanceColor is never
// bound — the full mesh already uses 8/8 vertex buffers, so an ever-bound
// instanceColor would overflow to a 9th.

import * as THREE from 'three/webgpu';
import {
  attribute,
  floor,
  fract,
  float,
  instanceIndex,
  instancedBufferAttribute,
  mix,
  positionGeometry,
  positionLocal,
  select,
  sin,
  smoothstep,
  time,
  vec3,
} from 'three/tsl';
import { kswCity, kswCityStyle } from '../../designTokens';
import { clayMat } from '../props';
import { windAmpU, windDirU } from '../windUniform';
import { allArchetypes, archetypeIndexFor, effectiveTreeSize, hash01, type TreeArchetype } from './treeArchetypes';
import { buildImpostorMeshFor } from './treeImpostors';
import type { TreeSpec } from './geoData';

// Per-spot deterministic variety — ported verbatim from nature.ts so the
// palette carries over exactly across the LOD/renderer swap.
const SQUASH_MIN = 0.85; // y-squash band floor
const SQUASH_SPAN = 0.3; // 0.85 – 1.15
const TINT_L_SPREAD = 0.16; // lightness varies ±0.08
const HUE_SPREAD = 0.04; // hue nudge ±0.02 around the base green

const baseGreen = new THREE.Color(kswCity.treeGreen);
const baseWood = new THREE.Color(kswCity.woodGreen);

function tintFor(x: number, z: number, kind: TreeSpec['kind']): THREE.Color {
  const hh = hash01(x * 3.1 + z * 7.7);
  const dL = (hash01(x * 5.7 + z * 11.3) - 0.5) * TINT_L_SPREAD;
  const base = kind === 'conifer' ? baseWood : baseGreen;
  return base.clone().offsetHSL((hh - 0.5) * HUE_SPREAD, 0, dL);
}

function squashFor(x: number, z: number): number {
  const hv = hash01(x * 7.3 + z * 3.9);
  return SQUASH_MIN + SQUASH_SPAN * hv;
}

export type TreeInstance = {
  spec: TreeSpec;
  archetype: number;
  tint: THREE.Color;
  squash: number;
  y: number; // ground height at (x, z) — trees plant ON the DEM, not at y=0
};

// Task 4: near-set compaction. Full-detail trees are drawn out to 10% beyond
// the impostor-collapse radius (kswCityStyle.lod.nearR) — the overlap band
// means the full mesh is already up before the impostor takes over, so there
// is no visible gap during the LOD handoff.
export const NEAR_TREE_DIST = kswCityStyle.lod.nearR * 1.1;

// Task 5 capacity headroom for the GLOBAL full-detail meshes (see the cap
// comment in buildTreeLayer). Exported for the capacity test.
export const FULL_MESH_MIN_CAP = 4096;

// Coarse spatial grid over the tree set, built once at layer construction.
// Cell = 64 m; plain Map<string, TreeInstance[]> keyed by integer cell coords.
const GRID_CELL = 64;

function cellKey(cx: number, cz: number): string {
  return `${cx},${cz}`;
}

function cellOf(x: number, z: number): [number, number] {
  return [Math.floor(x / GRID_CELL), Math.floor(z / GRID_CELL)];
}

function gridAdd(grid: Map<string, TreeInstance[]>, inst: TreeInstance): void {
  const [cx, cz] = cellOf(inst.spec.x, inst.spec.z);
  const key = cellKey(cx, cz);
  let bucket = grid.get(key);
  if (!bucket) {
    bucket = [];
    grid.set(key, bucket);
  }
  bucket.push(inst);
}

// Identity-based removal — tile pools keep the exact TreeInstance refs they
// inserted, so indexOf finds them; empty buckets are dropped to keep the map
// from accumulating dead cells as tiles stream in and out.
function gridRemove(grid: Map<string, TreeInstance[]>, inst: TreeInstance): void {
  const [cx, cz] = cellOf(inst.spec.x, inst.spec.z);
  const key = cellKey(cx, cz);
  const bucket = grid.get(key);
  if (!bucket) return;
  const i = bucket.indexOf(inst);
  if (i >= 0) bucket.splice(i, 1);
  if (bucket.length === 0) grid.delete(key);
}

function buildGrid(instances: readonly TreeInstance[]): Map<string, TreeInstance[]> {
  const grid = new Map<string, TreeInstance[]>();
  for (const inst of instances) gridAdd(grid, inst);
  return grid;
}

// Query all instances within `dist` of (camX, camZ) by scanning the grid
// cells overlapping the camera disc's bounding square, then filtering by
// exact distance.
function queryNear(grid: Map<string, TreeInstance[]>, camX: number, camZ: number, dist: number): TreeInstance[] {
  const [cxMin, czMin] = cellOf(camX - dist, camZ - dist);
  const [cxMax, czMax] = cellOf(camX + dist, camZ + dist);
  const dist2 = dist * dist;
  const out: TreeInstance[] = [];
  for (let cx = cxMin; cx <= cxMax; cx++) {
    for (let cz = czMin; cz <= czMax; cz++) {
      const bucket = grid.get(cellKey(cx, cz));
      if (!bucket) continue;
      for (const inst of bucket) {
        const dx = inst.spec.x - camX;
        const dz = inst.spec.z - camZ;
        if (dx * dx + dz * dz <= dist2) out.push(inst);
      }
    }
  }
  return out;
}

export type TreeLayer = {
  group: THREE.Group; // name 'cityTrees'
  fullMeshes: THREE.InstancedMesh[]; // one per archetype, name `treeArch:${i}`
  instances: TreeInstance[]; // assignment result (impostors + compaction reuse it)
  setTreeShadows(on: boolean): void;
  compactNear(camX: number, camZ: number): void; // Task 4 fills this; Task 3 ships it as full-refill
  // ── Task 5 (M3 streaming): dynamic per-tile pools ────────────────────────
  // Full-detail meshes stay GLOBAL — compactNear pulls the near set from ALL
  // pools (boot stand + streamed tiles) via the shared spatial grid. Only the
  // far-field impostor quads are per-tile: each addTileTrees builds one small
  // InstancedMesh (shared atlas texture) that removeTileTrees disposes again.
  /** Shared impostor atlas for per-tile impostor meshes. Must be called once
   * (after bakeImpostorAtlas) before the first addTileTrees. */
  setImpostorContext(atlas: THREE.Texture, archCount: number): void;
  /** Registers a tile's trees: assignment + grid + per-tile impostor mesh.
   * Throws on a duplicate key or when the impostor context is missing. */
  addTileTrees(key: string, specs: TreeSpec[]): void;
  /** Unregisters a tile: instances leave grid + compaction; the tile's
   * impostor mesh is disposed (geometry + instance buffers + the per-mesh
   * NodeMaterial (#142) — the shared atlas TEXTURE is never disposed:
   * material.dispose() does not cascade into referenced textures). */
  removeTileTrees(key: string): void;
  /** Registered tile-pool keys, in insertion order (smoke assertion). */
  tileKeys(): string[];
};

// Same excludeRect predicate as nature.ts: a spec is dropped when it lies
// inside the axis-aligned rect (center x/z, size w/d).
function insideRect(x: number, z: number, ex: { x: number; z: number; w: number; d: number }): boolean {
  return Math.abs(x - ex.x) <= ex.w / 2 && Math.abs(z - ex.z) <= ex.d / 2;
}

export function assignTrees(
  trees: readonly TreeSpec[],
  excludeRect?: { x: number; z: number; w: number; d: number },
  groundYAt?: (x: number, z: number) => number,
): TreeInstance[] {
  const out: TreeInstance[] = [];
  for (const spec of trees) {
    if (excludeRect && insideRect(spec.x, spec.z, excludeRect)) continue;
    const kind = spec.kind === 'conifer' ? 'conifer' : 'broad';
    out.push({
      spec,
      archetype: archetypeIndexFor(spec.x, spec.z, kind, spec.family),
      tint: tintFor(spec.x, spec.z, kind),
      squash: squashFor(spec.x, spec.z),
      // Plant on the real terrain. Without this, every tree outside the flat
      // city plate is buried under the DEM (2026-07-06 waldrand finding: the
      // whole forest belt rendered as empty meadow — full meshes AND impostors).
      y: groundYAt ? groundYAt(spec.x, spec.z) : 0,
    });
  }
  return out;
}

// Node values are runtime-typed `any`: @types/three r185 doesn't model
// attribute-node swizzles (aPuff.w/.xyz) or float()-of-instanceIndex — the
// same un-modellable-node situation agentMeshes documents (its `TSLSlot = any`)
// and main.ts's PCSS block. The nodes are correct at runtime.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type TSLNode = any;

// GPU-side deterministic hash (visual-only) → [0,1). CPU/GPU float determinism
// is NOT required to match — this only needs to decorrelate per instance/puff.
//
// Deliberately transcendental-FREE. The classic `fract(sin(x)*43758)` blows up
// for large x: here the seed is `float(instanceIndex)*13 + puffIndex`, easily
// in the thousands, and float32 `sin()` of a large argument loses so much
// precision it returns garbage / non-finite values — which propagate as NaN
// through the puff-jitter scale and fling the ENTIRE crown of the affected
// instances off to infinity (the "naked-skeleton" trees). A pure fract-multiply
// hash (pre-fracted seed, no sin) is bounded and precise for ANY seed size.
const fractHash = (n: TSLNode): TSLNode => {
  const p = fract(n.mul(0.1031)); // bound the seed to [0,1) first
  const q = fract(p.add(0.1031).mul(p.add(19.19)).mul(103.71));
  return fract(q.mul(q.add(7.13)).mul(31.77));
};

const TRUNK_R = ((kswCity.treeTrunk >> 16) & 0xff) / 255;
const TRUNK_G = ((kswCity.treeTrunk >> 8) & 0xff) / 255;
const TRUNK_B = (kswCity.treeTrunk & 0xff) / 255;

// One TSL node material per archetype: crownBaseY differs, and each mesh owns
// its own aTint attribute node. clayMat is cached — clone before assigning
// nodes (nature.ts convention). Constructing the nodes is pure JS and safe
// under vitest (no GPU) — same as agentMeshes' node materials.
// SPACE CONTRACT (the 2026-07-06 "naked skeleton" root cause): three's
// NodeMaterial applies the instancedMesh() transform to `positionLocal`
// BEFORE evaluating a user positionNode (NodeMaterial.setupPosition). Inside
// this material `positionLocal` is therefore already in WORLD-scale city
// coordinates (meters, translated to the tree's spot), while geometry
// attributes like aPuff.xyz stay in the normalized archetype space (y 0..1).
// Mixing the two spaces — e.g. `aPuff.xyz + (positionLocal − aPuff.xyz)·j` —
// displaces crown vertices by (1−j)·(distance to origin) ≈ tens of meters,
// which scattered crowns off their trunks (fully naked skeletons for
// instances whose hash landed far from j=1). All displacement is now computed
// in GEOMETRY space (positionGeometry) and lifted to world scale with the
// per-instance scale carried in aTint.w — valid because the instance matrices
// are compose(pos, IDENTITY quaternion, scale): axis-aligned scale + translation
// only (see compactNear). If instance rotation is ever introduced, this lift
// needs the rotation applied to the delta as well.
function treeMaterial(arch: TreeArchetype, aTintNode: TSLNode): THREE.MeshPhysicalMaterial {
  const mat = clayMat(kswCity.treeGreen).clone();
  // The clay sheen recipe lerps 50% toward white — right for buildings, but
  // it bleached every canopy to pale sage. Foliage keeps a whisper of sheen,
  // tinted green, so the crowns hold their saturation (SOTA pass 2026-07-06).
  mat.sheen = 0.2;
  mat.sheenColor = new THREE.Color(kswCity.treeGreen);

  // aPuff (vec4): xyz = puff center (normalized), w = puff index (>=0 crown) or
  // -1 (wood/trunk). The archetype geometry stamps it per vertex.
  const aPuff: TSLNode = attribute('aPuff', 'vec4');
  const idxF: TSLNode = float(instanceIndex);
  const isWood = aPuff.w.lessThan(float(0));

  // Per-instance world scale, unpacked from aTint.w (packScales in compactNear):
  // sx = horizontal scale (r/crownRadius), sy = vertical scale (h·squash).
  const sx = floor(aTintNode.w.div(float(4096))).div(float(100));
  const sy = aTintNode.w.mod(float(4096)).div(float(100));

  // ── colorNode: two-tone crown, trunk keeps clay wood colour ─────────────
  // gradient lightens the crown top; the per-instance tint modulates the base.
  // positionGeometry.y is the RAW attribute — normalized 0..1 by the archetype
  // contract (positionLocal is already instance-transformed here, see above).
  // This matches the impostor bake exactly (bakeMaterial renders the archetype
  // un-instanced, where local y IS 0..1) — same gradient across the LOD handoff.
  const gradient = mix(float(0.82), float(1.12), smoothstep(float(arch.crownBaseY), float(1), positionGeometry.y));
  const trunkColor = vec3(float(TRUNK_R), float(TRUNK_G), float(TRUNK_B));
  mat.colorNode = select(isWood, trunkColor, aTintNode.xyz.mul(gradient));

  // ── positionNode: puff jitter (crown only) then wind sway ───────────────
  // Puff jitter: scale each crown vertex around its puff center by a per-puff,
  // per-instance factor in [0.94, 1.06]. Computed as a GEOMETRY-space delta
  // (p − c)·(j − 1), lifted to world scale with (sx, sy, sx), then added to the
  // instanced positionLocal. Wood vertices skip jitter.
  const jitterSeed = idxF.mul(float(13)).add(aPuff.w);
  // Gentle per-puff scale variety, centred on 1.0 so it never SHRINKS a puff
  // enough to open a gap and expose the branch skeleton (the puffs are tuned to
  // just-overlap; a 0.85× shrink was enough to break marginal crowns). Kept
  // small — the crown must always read as one solid mass.
  const jitter = float(0.94).add(fractHash(jitterSeed).mul(float(0.12)));
  const dLocal: TSLNode = positionGeometry.sub(aPuff.xyz).mul(jitter.sub(float(1)));
  const dWorld = vec3(dLocal.x.mul(sx), dLocal.y.mul(sy), dLocal.z.mul(sx));
  const crownOffset = select(isWood, vec3(float(0), float(0), float(0)), dWorld);

  // Wind sway: horizontal WORLD-space offset in windDir, growing with height²
  // so the crown top leans most; two detuned sines for a natural gust, phase
  // per instance. Amplitude scales with sx (≈ crown radius in meters) so big
  // trees displace proportionally — the original local-space intent. Trunk
  // vertices get a damped 0.15× share so the base barely moves.
  const swayY = positionGeometry.y; // normalized 0..1
  const phase = fractHash(idxF.mul(float(0.7))).mul(float(6.2831853));
  const wave = sin(time.mul(float(1.3)).add(phase)).mul(float(0.6)).add(sin(time.mul(float(0.31)).add(phase.mul(float(0.7)))).mul(float(0.4)));
  const swayMag = windAmpU.mul(swayY).mul(swayY).mul(wave).mul(float(0.05)).mul(sx);
  const windXZ = windDirU.mul(swayMag);
  const damp = select(isWood, float(0.15), float(1));
  const swayOffset = vec3(windXZ.x.mul(damp), float(0), windXZ.y.mul(damp));

  mat.positionNode = positionLocal.add(crownOffset).add(swayOffset);
  return mat;
}

// Pack the per-instance world scales into one float32: floor(sx·100)·4096 +
// floor(sy·100). Exact in f32 up to 2^24; sx/sy are clamped to < 40.95 m
// (well above any real tree). The shader unpacks with floor/mod (see above).
export function packScales(sx: number, sy: number): number {
  const q = (v: number) => Math.min(4095, Math.max(0, Math.round(v * 100)));
  return q(sx) * 4096 + q(sy);
}

export function buildTreeLayer(
  trees: readonly TreeSpec[],
  opts: {
    excludeRect?: { x: number; z: number; w: number; d: number };
    groundYAt?: (x: number, z: number) => number;
  } = {},
): TreeLayer {
  const archetypes = allArchetypes();
  const instances = assignTrees(trees, opts.excludeRect, opts.groundYAt);

  const group = new THREE.Group();
  group.name = 'cityTrees';

  // Group instances by archetype so each mesh's capacity is exact.
  const byArch: TreeInstance[][] = archetypes.map(() => []);
  for (const inst of instances) byArch[inst.archetype].push(inst);

  // Coarse spatial grid, built once — compactNear queries it per call instead
  // of rescanning every instance.
  const grid = buildGrid(instances);

  const fullMeshes: THREE.InstancedMesh[] = [];
  // Per-mesh aTint attributes, kept so compactNear can rewrite them.
  const tintAttrs: THREE.InstancedBufferAttribute[] = [];

  for (let i = 0; i < archetypes.length; i++) {
    const arch = archetypes[i];
    const n = byArch[i].length;
    // Capacity contract (Task 5): the full-detail meshes are GLOBAL and serve
    // ALL pools, so the boot-stand size alone no longer bounds the near set —
    // streamed tiles add instances at runtime. Only trees < NEAR_TREE_DIST
    // (165 m ring) ever land in the full meshes, so compaction bounds the
    // count naturally; FULL_MESH_MIN_CAP is a documented, asserted headroom
    // (compactNear throws if a near set ever exceeds capacity) — 4096 per
    // archetype ≈ 82k near trees total, far above any plausible 165 m ring.
    // (WebGPU zero-buffer rule is subsumed: capacity is always ≥ 1.)
    const cap = Math.max(FULL_MESH_MIN_CAP, n);

    // Per-instance tint+scale attribute (vec4: rgb + packScales(sx, sy)) — the
    // node colorNode reads .xyz, the positionNode's space lift reads .w.
    const tintArray = new Float32Array(cap * 4);
    const tintAttr = new THREE.InstancedBufferAttribute(tintArray, 4);
    tintAttr.setUsage(THREE.DynamicDrawUsage);
    const aTintNode = instancedBufferAttribute(tintAttr, 'vec4');

    // arch.geometry is shared across the app; clone it so the per-mesh aTint
    // instanced attribute doesn't mutate the shared geometry. (position/normal/
    // aPuff carry over — the material reads aPuff from here.)
    const geo = arch.geometry.clone();
    geo.setAttribute('aTint', tintAttr);

    const mat = treeMaterial(arch, aTintNode);
    const mesh = new THREE.InstancedMesh(geo, mat, cap);
    mesh.name = `treeArch:${i}`;
    mesh.castShadow = false;
    mesh.receiveShadow = true;
    // Instances span the whole city; the shared bounding sphere would cull the
    // entire mesh. Compaction (Task 4) keeps counts small instead of culling.
    mesh.frustumCulled = false;
    mesh.count = n;

    fullMeshes.push(mesh);
    tintAttrs.push(tintAttr);
    group.add(mesh);
  }

  // ── Task 5 state: per-tile pools + shared impostor context ───────────────
  // Each pool keeps the exact TreeInstance refs it pushed into `instances`
  // and the grid, so removal is identity-exact; the impostor mesh is the
  // tile's own far-field draw (shared atlas texture).
  const tilePools = new Map<string, { instances: TreeInstance[]; impostor: THREE.InstancedMesh }>();
  let impostorAtlas: THREE.Texture | null = null;
  let impostorArchCount = 0;
  // Last compaction point: add/remove re-run compactNear here so the full
  // meshes never draw stale (removed) instances between camera moves.
  let lastCamX = 0;
  let lastCamZ = 0;

  const layer: TreeLayer = {
    group,
    fullMeshes,
    instances,
    setTreeShadows(on: boolean) {
      for (const m of fullMeshes) m.castShadow = on;
    },
    setImpostorContext(atlas: THREE.Texture, archCount: number) {
      impostorAtlas = atlas;
      impostorArchCount = archCount;
    },
    addTileTrees(key: string, specs: TreeSpec[]) {
      if (tilePools.has(key)) {
        throw new Error(`treeLayer.addTileTrees: duplicate tile key ${key}`);
      }
      if (!impostorAtlas) {
        throw new Error(
          'treeLayer.addTileTrees: impostor context missing — call setImpostorContext(atlas, archCount) after bakeImpostorAtlas',
        );
      }
      // Same assignment path as the boot stand (excludeRect/groundYAt from
      // the layer opts) — tile trees plant on the DEM and skip the hero plate
      // exactly like boot trees.
      const tileInstances = assignTrees(specs, opts.excludeRect, opts.groundYAt);
      for (const inst of tileInstances) {
        instances.push(inst);
        gridAdd(grid, inst);
      }
      const impostor = buildImpostorMeshFor(tileInstances, impostorAtlas, impostorArchCount);
      impostor.name = `treeImpostors:${key}`;
      group.add(impostor);
      tilePools.set(key, { instances: tileInstances, impostor });
      layer.compactNear(lastCamX, lastCamZ);
    },
    removeTileTrees(key: string) {
      const pool = tilePools.get(key);
      if (!pool) {
        throw new Error(`treeLayer.removeTileTrees: unknown tile key ${key}`);
      }
      tilePools.delete(key);
      const gone = new Set(pool.instances);
      for (const inst of pool.instances) gridRemove(grid, inst);
      // In-place compaction of the shared `instances` array (callers hold the
      // array ref — never swap it). Boot instances keep their relative order.
      let w = 0;
      for (let r = 0; r < instances.length; r++) {
        if (!gone.has(instances[r])) instances[w++] = instances[r];
      }
      instances.length = w;
      // Dispose the tile's impostor draw: geometry + instance buffers + the
      // per-mesh NodeMaterial (#142: buildImpostorMeshFor creates ONE material
      // per tile mesh; never disposing it leaked a material + WebGPU pipeline-
      // cache entry per tile churn cycle). material.dispose() only fires the
      // material's own dispose event — three NEVER cascades it into referenced
      // textures, so the SHARED atlas texture (and the shared impostorLightU
      // uniform node) survive for every other live/future tile mesh. The
      // pool-subkey re-add test proves a later addTileTrees still renders off
      // the same atlas.
      group.remove(pool.impostor);
      pool.impostor.geometry.dispose();
      (pool.impostor.material as THREE.Material).dispose();
      pool.impostor.dispose();
      layer.compactNear(lastCamX, lastCamZ);
    },
    tileKeys() {
      return [...tilePools.keys()];
    },
    // Near-set compaction: query the grid for instances within NEAR_TREE_DIST
    // of the camera point, group the hits by archetype, and rewrite each
    // mesh's matrices/tint in the SAME order for both attributes so instance
    // k's matrix and tint always describe the same tree post-compaction.
    compactNear(camX: number, camZ: number) {
      lastCamX = camX;
      lastCamZ = camZ;
      const near = queryNear(grid, camX, camZ, NEAR_TREE_DIST);
      const nearByArch: TreeInstance[][] = archetypes.map(() => []);
      for (const inst of near) nearByArch[inst.archetype].push(inst);

      const m = new THREE.Matrix4();
      const q = new THREE.Quaternion();
      const pos = new THREE.Vector3();
      const scl = new THREE.Vector3();
      for (let i = 0; i < fullMeshes.length; i++) {
        const mesh = fullMeshes[i];
        const arch = archetypes[i];
        const list = nearByArch[i];
        // Asserted headroom (see the cap comment above): tile pools can grow
        // the near set past the boot size, but never past capacity — fail
        // loudly instead of writing out of bounds.
        if (list.length > mesh.instanceMatrix.count) {
          throw new Error(
            `treeLayer.compactNear: near set for archetype ${i} (${list.length}) exceeds capacity ${mesh.instanceMatrix.count}`,
          );
        }
        const tintArray = tintAttrs[i].array as Float32Array;
        for (let k = 0; k < list.length; k++) {
          const { spec, tint, squash, y } = list[k];
          const eff = effectiveTreeSize(spec.h, spec.r);
          const s = eff.r / arch.crownRadius;
          pos.set(spec.x, y, spec.z);
          scl.set(s, eff.h * squash, s);
          // NOTE: identity quaternion — treeMaterial's geometry-space→world
          // displacement lift relies on the matrices staying rotation-free.
          m.compose(pos, q, scl);
          mesh.setMatrixAt(k, m);
          tintArray[k * 4] = tint.r;
          tintArray[k * 4 + 1] = tint.g;
          tintArray[k * 4 + 2] = tint.b;
          tintArray[k * 4 + 3] = packScales(s, eff.h * squash);
        }
        // 0-hit archetypes get count = 0; capacity (cap = Math.max(1, n))
        // stays >= 1 — buffers are never shrunk, only the visible count.
        mesh.count = list.length;
        mesh.instanceMatrix.needsUpdate = true;
        tintAttrs[i].needsUpdate = true;
      }
    },
  };

  // Initial fill so the layer is renderable immediately (camera args unused).
  layer.compactNear(0, 0);
  return layer;
}
