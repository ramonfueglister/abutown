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
// `instancedBufferAttribute` node inside colorNode. setColorAt is still called
// for parity with any fixed-function consumer, but aTint is the source of truth
// the shader reads.

import * as THREE from 'three/webgpu';
import {
  attribute,
  fract,
  float,
  instanceIndex,
  instancedBufferAttribute,
  mix,
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
import { allArchetypes, archetypeIndexFor, hash01, type TreeArchetype } from './treeArchetypes';
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

export type TreeInstance = { spec: TreeSpec; archetype: number; tint: THREE.Color; squash: number };

// Task 4: near-set compaction. Full-detail trees are drawn out to 10% beyond
// the impostor-collapse radius (kswCityStyle.lod.nearR) — the overlap band
// means the full mesh is already up before the impostor takes over, so there
// is no visible gap during the LOD handoff.
export const NEAR_TREE_DIST = kswCityStyle.lod.nearR * 1.1;

// Coarse spatial grid over the tree set, built once at layer construction.
// Cell = 64 m; plain Map<string, TreeInstance[]> keyed by integer cell coords.
const GRID_CELL = 64;

function cellKey(cx: number, cz: number): string {
  return `${cx},${cz}`;
}

function cellOf(x: number, z: number): [number, number] {
  return [Math.floor(x / GRID_CELL), Math.floor(z / GRID_CELL)];
}

function buildGrid(instances: readonly TreeInstance[]): Map<string, TreeInstance[]> {
  const grid = new Map<string, TreeInstance[]>();
  for (const inst of instances) {
    const [cx, cz] = cellOf(inst.spec.x, inst.spec.z);
    const key = cellKey(cx, cz);
    let bucket = grid.get(key);
    if (!bucket) {
      bucket = [];
      grid.set(key, bucket);
    }
    bucket.push(inst);
  }
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
};

// Same excludeRect predicate as nature.ts: a spec is dropped when it lies
// inside the axis-aligned rect (center x/z, size w/d).
function insideRect(x: number, z: number, ex: { x: number; z: number; w: number; d: number }): boolean {
  return Math.abs(x - ex.x) <= ex.w / 2 && Math.abs(z - ex.z) <= ex.d / 2;
}

export function assignTrees(
  trees: readonly TreeSpec[],
  excludeRect?: { x: number; z: number; w: number; d: number },
): TreeInstance[] {
  const out: TreeInstance[] = [];
  for (const spec of trees) {
    if (excludeRect && insideRect(spec.x, spec.z, excludeRect)) continue;
    const kind = spec.kind === 'conifer' ? 'conifer' : 'broad';
    out.push({
      spec,
      archetype: archetypeIndexFor(spec.x, spec.z, kind),
      tint: tintFor(spec.x, spec.z, kind),
      squash: squashFor(spec.x, spec.z),
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
function treeMaterial(arch: TreeArchetype, aTintNode: TSLNode): THREE.MeshPhysicalMaterial {
  const mat = clayMat(kswCity.treeGreen).clone();

  // aPuff (vec4): xyz = puff center (normalized), w = puff index (>=0 crown) or
  // -1 (wood/trunk). The archetype geometry stamps it per vertex.
  const aPuff: TSLNode = attribute('aPuff', 'vec4');
  const idxF: TSLNode = float(instanceIndex);
  const isWood = aPuff.w.lessThan(float(0));

  // ── colorNode: two-tone crown, trunk keeps clay wood colour ─────────────
  // gradient lightens the crown top; the per-instance tint modulates the base.
  // positionLocal.y is normalized 0..1 by the archetype contract.
  const gradient = mix(float(0.82), float(1.12), smoothstep(float(arch.crownBaseY), float(1), positionLocal.y));
  const trunkColor = vec3(float(TRUNK_R), float(TRUNK_G), float(TRUNK_B));
  mat.colorNode = select(isWood, trunkColor, aTintNode.mul(gradient));

  // ── positionNode: puff jitter (crown only) then wind sway ───────────────
  // Puff jitter: scale each crown vertex around its puff center by a per-puff,
  // per-instance factor in [0.94, 1.06]. Wood vertices skip jitter.
  const jitterSeed = idxF.mul(float(13)).add(aPuff.w);
  // Gentle per-puff scale variety, centred on 1.0 so it never SHRINKS a puff
  // enough to open a gap and expose the branch skeleton (the puffs are tuned to
  // just-overlap; a 0.85× shrink was enough to break marginal crowns). Kept
  // small — the crown must always read as one solid mass.
  const jitter = float(0.94).add(fractHash(jitterSeed).mul(float(0.12)));
  const fromCenter: TSLNode = positionLocal.sub(aPuff.xyz);
  const jittered = aPuff.xyz.add(fromCenter.mul(jitter));
  const crownP = select(isWood, positionLocal, jittered);

  // Wind sway: horizontal offset in windDir, growing with height² so the crown
  // top leans most; two detuned sines for a natural gust, phase per instance.
  // Trunk vertices get a damped 0.15× share so the base barely moves.
  const swayY = positionLocal.y; // normalized 0..1
  const phase = fractHash(idxF.mul(float(0.7))).mul(float(6.2831853));
  const wave = sin(time.mul(float(1.3)).add(phase)).mul(float(0.6)).add(sin(time.mul(float(0.31)).add(phase.mul(float(0.7)))).mul(float(0.4)));
  const swayMag = windAmpU.mul(swayY).mul(swayY).mul(wave).mul(float(0.05));
  const windXZ = windDirU.mul(swayMag);
  const damp = select(isWood, float(0.15), float(1));
  const swayOffset = vec3(windXZ.x.mul(damp), float(0), windXZ.y.mul(damp));

  mat.positionNode = crownP.add(swayOffset);
  return mat;
}

export function buildTreeLayer(
  trees: readonly TreeSpec[],
  opts: { excludeRect?: { x: number; z: number; w: number; d: number } } = {},
): TreeLayer {
  const archetypes = allArchetypes();
  const instances = assignTrees(trees, opts.excludeRect);

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
    // WebGPU zero-buffer rule: allocate at least one instance, set count after.
    const cap = Math.max(1, n);

    // Per-instance tint attribute (vec3) — the node colorNode reads this.
    const tintArray = new Float32Array(cap * 3);
    const tintAttr = new THREE.InstancedBufferAttribute(tintArray, 3);
    tintAttr.setUsage(THREE.DynamicDrawUsage);
    const aTintNode = instancedBufferAttribute(tintAttr, 'vec3');

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

  const layer: TreeLayer = {
    group,
    fullMeshes,
    instances,
    setTreeShadows(on: boolean) {
      for (const m of fullMeshes) m.castShadow = on;
    },
    // Near-set compaction: query the grid for instances within NEAR_TREE_DIST
    // of the camera point, group the hits by archetype, and rewrite each
    // mesh's matrices/tint in the SAME order for both attributes so instance
    // k's matrix and tint always describe the same tree post-compaction.
    compactNear(camX: number, camZ: number) {
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
        const tintArray = tintAttrs[i].array as Float32Array;
        for (let k = 0; k < list.length; k++) {
          const { spec, tint, squash } = list[k];
          const s = spec.r / arch.crownRadius;
          pos.set(spec.x, 0, spec.z);
          scl.set(s, spec.h * squash, s);
          m.compose(pos, q, scl);
          mesh.setMatrixAt(k, m);
          mesh.setColorAt(k, tint);
          tintArray[k * 3] = tint.r;
          tintArray[k * 3 + 1] = tint.g;
          tintArray[k * 3 + 2] = tint.b;
        }
        // 0-hit archetypes get count = 0; capacity (cap = Math.max(1, n))
        // stays >= 1 — buffers are never shrunk, only the visible count.
        mesh.count = list.length;
        mesh.instanceMatrix.needsUpdate = true;
        if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
        tintAttrs[i].needsUpdate = true;
      }
    },
  };

  // Initial fill so the layer is renderable immediately (camera args unused).
  layer.compactNear(0, 0);
  return layer;
}
