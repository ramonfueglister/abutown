# SOTA Trees Slice 1 — Forms + Wind Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the two fixed tree shapes with ~20 seeded procedural archetypes (trunk+branches+puff crown, clay style), per-instance shader variation, weather-coupled wind sway, octahedral impostors for the far field, and near-set compaction LOD — on the existing bake data.

**Architecture:** A boot-time deterministic archetype generator (`treeArchetypes.ts`) produces normalized geometries. `treeLayer.ts` renders them as one InstancedMesh per archetype (trunk+crown in one geometry, selective tint via an `aPuff` vertex attribute), with TSL vertex nodes for puff jitter, crown gradient, and wind (driven by a shared `windUniform` set from live weather). A throttled CPU compaction keeps only near-camera trees in the full meshes; a single always-on impostor InstancedMesh (camera-facing quads sampling a boot-baked hemi-octahedral atlas) covers the far field and collapses its quads near the camera in the vertex stage.

**Tech Stack:** three 0.185 `three/webgpu` + `three/tsl`, vitest, TypeScript. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-05-sota-trees-design.md` (Slice 1, incl. the two 2026-07-05 amendments).

## Global Constraints

- Determinism: all per-tree variation derives from world coordinates / explicit seeds. `Math.random()` and `Date.now()` are forbidden in the generator and layer (project convention, see nature.ts "no RNG").
- All tree meshes must have non-zero instance capacity even when a kind is absent — WebGPU rejects zero-size instance buffers (`Math.max(1, n)` + `.count = n`, see nature.ts comment).
- `tsc -p tsconfig.typecheck.json` covers src+tests+scripts — run it, `npm test` (vitest), and `npm run build` before claiming done.
- Browser smoke is MANDATORY before completion (CLAUDE.md) — the feature crosses render wiring.
- No cargo involvement in this slice.
- Silhouette contract: full mesh and impostor of the same archetype must share the same envelope (height 1, crown radius `crownRadius`) — LOD swaps must not pop.
- Existing hero-plate `excludeRect` behavior and greens/water rendering in nature.ts stay unchanged.

## File Structure

- Create `src/diorama/ksw/geo/treeArchetypes.ts` — deterministic archetype geometry generator (pure, no renderer).
- Create `src/diorama/ksw/windUniform.ts` — shared TSL wind uniforms (pattern: glowUniform.ts).
- Create `src/diorama/ksw/geo/treeLayer.ts` — instanced rendering, TSL material, compaction LOD.
- Create `src/diorama/ksw/geo/treeImpostors.ts` — hemi-oct mapping, atlas bake, impostor mesh.
- Modify `src/diorama/ksw/geo/nature.ts` — delete the tree section (keep greens/water), delegate to treeLayer.
- Modify `src/diorama/ksw/geo/lod.ts` — replace tree object lists with the tree-layer contract.
- Modify `src/diorama/ksw/main.ts` — wiring: layer build, impostor bake, per-frame/throttled updates, debug hook.
- Modify `src/diorama/ksw/applyCityEnvironment.ts` — set wind uniforms from live weather.
- Create `scripts/smoke-trees.mjs` — browser smoke + screenshots + FPS probe.
- Tests: `tests/diorama/treeArchetypes.test.ts`, `tests/diorama/windUniform.test.ts`, `tests/diorama/treeLayer.test.ts`, `tests/diorama/treeImpostors.test.ts`; modify `tests/diorama/*lod*` (LOD tests live in `tests/geo/lod.test.ts` if present — locate with `grep -rl applyCityLod tests/`).

---

### Task 1: Archetype generator (`treeArchetypes.ts`)

**Files:**
- Create: `src/diorama/ksw/geo/treeArchetypes.ts`
- Test: `tests/diorama/treeArchetypes.test.ts`

**Interfaces:**
- Consumes: nothing project-specific (three geometry utils only).
- Produces (used by Tasks 3, 4, 5):

```ts
export type TreeFamily = 'spreading' | 'oval' | 'tall' | 'conic' | 'slender';
export const BROAD_FAMILIES: readonly TreeFamily[]; // ['spreading','oval','tall']
export const CONIFER_FAMILIES: readonly TreeFamily[]; // ['conic','slender']
export const SEEDS_PER_FAMILY = 4;
export type TreeArchetype = {
  family: TreeFamily;
  seed: number; // 0..SEEDS_PER_FAMILY-1
  geometry: THREE.BufferGeometry; // normalized: y ∈ [0,1], attrs position/normal/aPuff(vec4)
  crownRadius: number;  // max horizontal radius in normalized units
  crownBaseY: number;   // normalized y where crown starts (impostor framing / trunk share)
};
export function buildArchetype(family: TreeFamily, seed: number): TreeArchetype;
export function allArchetypes(): TreeArchetype[]; // stable order: families × seeds; index IS the archetype id
export function archetypeIndexFor(x: number, z: number, kind: 'broad' | 'conifer'): number;
export function hash01(n: number): number; // exported for reuse/tests
```

Geometry contract (the whole slice hangs on it):
- Normalized height exactly 1 (scale y by instance `h` later), horizontal extent = `crownRadius` (scale xz by `r / crownRadius`).
- `aPuff` is a vec4 vertex attribute: `(cx, cy, cz, puffIndex)` = the puff's center for crown vertices; trunk/branch vertices get `(0,0,0,-1)`. Shader rule: `aPuff.w >= 0` ⇒ crown vertex (tint + jitter + full wind), `< 0` ⇒ wood (trunk color, minimal sway).

**Generator recipe (all seeded via `hash01`, splitmix-style mixing of family index + seed — NOT raw sin-hash on tiny ints, see stationary-age-seed lesson on FNV clustering):**
- Trunk: cylinder from y=0 to `crownBaseY`, radius ~0.02–0.035 normalized.
- Branches (broad families): 2 levels; level 1 = 3–5 cylinders leaving the trunk top at seeded azimuth/elevation, level 2 = 1–2 short cylinders per level-1 tip. Branch tips + trunk top define puff anchor points.
- Crown puffs (broad): 6–12 icosahedron puffs (detail 1) at anchors, radii seeded within a family envelope:
  - `spreading`: crownBaseY ≈ 0.35, wide flat envelope (crownRadius ≈ 0.55, puffs pushed outward),
  - `oval`: crownBaseY ≈ 0.30, oval envelope (crownRadius ≈ 0.40),
  - `tall`: crownBaseY ≈ 0.25, narrow tall envelope (crownRadius ≈ 0.30).
- Conifers: no branch skeleton — stacked cones/rings:
  - `conic`: 3–4 stacked cones (seeded ring radii), crownBaseY ≈ 0.10, crownRadius ≈ 0.30,
  - `slender`: 2–3 tighter cones, crownRadius ≈ 0.20.
  Cone vertices get `aPuff = (0, coneCenterY, 0, coneIndex)` so jitter/wind still have per-part identity.
- After merging, normalize: scale/translate so min y = 0, max y = 1; recompute `crownRadius` from actual positions; write `aPuff` BEFORE normalize using the same transform.

- [ ] **Step 1: Write the failing tests**

```ts
// tests/diorama/treeArchetypes.test.ts
import { describe, expect, it } from 'vitest';
import {
  BROAD_FAMILIES, CONIFER_FAMILIES, SEEDS_PER_FAMILY,
  allArchetypes, archetypeIndexFor, buildArchetype,
} from '../../src/diorama/ksw/geo/treeArchetypes';

const posHash = (g: import('three').BufferGeometry): string => {
  const a = g.getAttribute('position').array as Float32Array;
  let h = 0;
  for (let i = 0; i < a.length; i++) h = (h * 31 + Math.round(a[i] * 1e4)) | 0;
  return String(h);
};

describe('treeArchetypes', () => {
  it('is deterministic: same family+seed → identical geometry', () => {
    const a = buildArchetype('spreading', 2);
    const b = buildArchetype('spreading', 2);
    expect(posHash(a.geometry)).toBe(posHash(b.geometry));
  });

  it('different seeds of a family produce different geometry', () => {
    expect(posHash(buildArchetype('oval', 0).geometry)).not.toBe(posHash(buildArchetype('oval', 1).geometry));
  });

  it('normalizes to y ∈ [0,1] and reports a consistent crownRadius', () => {
    for (const arch of allArchetypes()) {
      const p = arch.geometry.getAttribute('position').array as Float32Array;
      let minY = Infinity, maxY = -Infinity, maxR = 0;
      for (let i = 0; i < p.length; i += 3) {
        minY = Math.min(minY, p[i + 1]);
        maxY = Math.max(maxY, p[i + 1]);
        maxR = Math.max(maxR, Math.hypot(p[i], p[i + 2]));
      }
      expect(minY).toBeCloseTo(0, 3);
      expect(maxY).toBeCloseTo(1, 3);
      expect(arch.crownRadius).toBeCloseTo(maxR, 3);
    }
  });

  it('carries aPuff vec4 with wood marked -1 and crown puffIndex >= 0', () => {
    const arch = buildArchetype('spreading', 0);
    const ap = arch.geometry.getAttribute('aPuff');
    expect(ap.itemSize).toBe(4);
    const w = ap.array as Float32Array;
    const flags = new Set<number>();
    for (let i = 3; i < w.length; i += 4) flags.add(Math.sign(Math.max(-1, w[i])));
    expect(flags.has(-1)).toBe(true); // trunk exists
    expect(flags.has(1) || flags.has(0)).toBe(true); // crown exists
  });

  it('allArchetypes is families × seeds in stable order', () => {
    const all = allArchetypes();
    expect(all.length).toBe((BROAD_FAMILIES.length + CONIFER_FAMILIES.length) * SEEDS_PER_FAMILY);
    expect(all[0].family).toBe(BROAD_FAMILIES[0]);
    expect(all[0].seed).toBe(0);
  });

  it('archetypeIndexFor is deterministic, kind-respecting, and spread out', () => {
    const broadRange = BROAD_FAMILIES.length * SEEDS_PER_FAMILY;
    const seen = new Set<number>();
    for (let i = 0; i < 200; i++) {
      const idx = archetypeIndexFor(i * 13.7, i * 7.3, 'broad');
      expect(idx).toBeGreaterThanOrEqual(0);
      expect(idx).toBeLessThan(broadRange);
      seen.add(idx);
    }
    expect(seen.size).toBeGreaterThan(broadRange / 2); // actually varied
    const all = allArchetypes().length;
    const cIdx = archetypeIndexFor(5, 5, 'conifer');
    expect(cIdx).toBeGreaterThanOrEqual(broadRange);
    expect(cIdx).toBeLessThan(all);
    expect(archetypeIndexFor(5, 5, 'conifer')).toBe(cIdx);
  });
});
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `npx vitest run tests/diorama/treeArchetypes.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `treeArchetypes.ts`**

Skeleton (fill the family tables per the recipe above; keep every constant named at top):

```ts
// src/diorama/ksw/geo/treeArchetypes.ts
// Deterministic procedural tree archetypes in the clay vocabulary: seeded
// branch skeleton + puff crown for broadleaf families, stacked cones for
// conifers. Normalized (y 0..1) so instances scale to real h/r meters.
import * as THREE from 'three/webgpu';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';

export type TreeFamily = 'spreading' | 'oval' | 'tall' | 'conic' | 'slender';
export const BROAD_FAMILIES = ['spreading', 'oval', 'tall'] as const satisfies readonly TreeFamily[];
export const CONIFER_FAMILIES = ['conic', 'slender'] as const satisfies readonly TreeFamily[];
export const SEEDS_PER_FAMILY = 4;

// splitmix32-style avalanche so nearby inputs decorrelate (stationary-age-seed
// lesson: naive hashes cluster in high bits)
export function hash01(n: number): number {
  let x = (n * 0x9e3779b9) | 0;
  x = Math.imul(x ^ (x >>> 16), 0x21f0aaad);
  x = Math.imul(x ^ (x >>> 15), 0x735a2d97);
  x = x ^ (x >>> 15);
  return (x >>> 0) / 4294967296;
}

export type TreeArchetype = { family: TreeFamily; seed: number; geometry: THREE.BufferGeometry; crownRadius: number; crownBaseY: number };

type FamilyParams = {
  crownBaseY: number; crownRadius: number;
  puffs: [min: number, max: number];           // broad: puff count band
  puffR: [min: number, max: number];           // puff radius band (normalized)
  branches: [min: number, max: number];        // level-1 branch count
  envelope: (t: number) => number;             // horizontal reach at crown height fraction t
};
// ... FAMILY_PARAMS: Record<TreeFamily, FamilyParams> — per the recipe table
// ... buildBroad(params, rng), buildConifer(params, rng) — cylinders/cones/icosahedra,
//     each part gets an aPuff Float32BufferAttribute before merge
// ... normalize(geo): scale/translate to y 0..1, return actual crownRadius

export function buildArchetype(family: TreeFamily, seed: number): TreeArchetype { /* ... */ }

let cache: TreeArchetype[] | null = null;
export function allArchetypes(): TreeArchetype[] {
  if (!cache) {
    cache = [];
    for (const f of [...BROAD_FAMILIES, ...CONIFER_FAMILIES]) {
      for (let s = 0; s < SEEDS_PER_FAMILY; s++) cache.push(buildArchetype(f, s));
    }
  }
  return cache;
}

export function archetypeIndexFor(x: number, z: number, kind: 'broad' | 'conifer'): number {
  const h = hash01(Math.round(x * 8) * 92837111 + Math.round(z * 8) * 689287499);
  const broadN = BROAD_FAMILIES.length * SEEDS_PER_FAMILY;
  const conifN = CONIFER_FAMILIES.length * SEEDS_PER_FAMILY;
  return kind === 'broad' ? Math.floor(h * broadN) : broadN + Math.floor(h * conifN);
}
```

Implementation notes:
- Per-part `aPuff`: `new THREE.Float32BufferAttribute(new Float32Array(vertexCount * 4).map(...), 4)` — fill (cx,cy,cz,idx) for crown parts, (0,0,0,-1) for wood, BEFORE `mergeGeometries` (it concatenates same-named attributes).
- Seeded rng stream: `const rng = (() => { let i = 0; const base = familyIdx * 1013 + seed * 7919; return () => hash01(base + i++); })();`
- Keep total vertex count per archetype under ~1500 (icosahedron detail 1 = 42 verts/puff; 12 puffs + skeleton ≈ 700 — fine).

- [ ] **Step 4: Run tests, verify pass**

Run: `npx vitest run tests/diorama/treeArchetypes.test.ts`
Expected: PASS (all 6).

- [ ] **Step 5: Typecheck + commit**

```bash
npx tsc -p tsconfig.typecheck.json
git add src/diorama/ksw/geo/treeArchetypes.ts tests/diorama/treeArchetypes.test.ts
git commit -m "feat(trees): deterministic procedural archetype generator (5 families x 4 seeds)"
```

---

### Task 2: Wind uniforms + weather wiring

**Files:**
- Create: `src/diorama/ksw/windUniform.ts`
- Modify: `src/diorama/ksw/applyCityEnvironment.ts` (where `env.windSpeedMs` already flows into `t.precipitation.update(...)`, ~line 143)
- Test: `tests/diorama/windUniform.test.ts`

**Interfaces:**
- Consumes: `EnvironmentState`'s `windSpeedMs: number`, `windDirRad: number` (already present in the env object passed to applyCityEnvironment).
- Produces (used by Tasks 3, 4):

```ts
export const windAmpU: /* TSL uniform(float) */;        // 0..~1.2 normalized sway amplitude
export const windDirU: /* TSL uniform(vec2) */;         // unit XZ direction the wind blows TOWARD
export function windAmplitude(windSpeedMs: number): number; // pure mapping, tested
```

- [ ] **Step 1: Write the failing test**

```ts
// tests/diorama/windUniform.test.ts
import { describe, expect, it } from 'vitest';
import { windAmplitude } from '../../src/diorama/ksw/windUniform';

describe('windAmplitude', () => {
  it('is 0 in calm air and grows monotonically', () => {
    expect(windAmplitude(0)).toBe(0);
    expect(windAmplitude(3)).toBeGreaterThan(0);
    expect(windAmplitude(8)).toBeGreaterThan(windAmplitude(3));
  });
  it('saturates for storms (cap 1.2)', () => {
    expect(windAmplitude(40)).toBeCloseTo(1.2, 5);
    expect(windAmplitude(15)).toBeLessThanOrEqual(1.2);
  });
});
```

- [ ] **Step 2: Run test, verify it fails**

Run: `npx vitest run tests/diorama/windUniform.test.ts` — FAIL, module not found.

- [ ] **Step 3: Implement**

```ts
// src/diorama/ksw/windUniform.ts
// Shared wind uniforms driving tree sway (pattern: glowUniform.ts — one tiny
// module so builders and main.ts import without cycles). Set per env update
// in applyCityEnvironment from the live Open-Meteo wind.
import * as THREE from 'three/webgpu';
import { uniform } from 'three/tsl';

export const windAmpU = uniform(0);
export const windDirU = uniform(new THREE.Vector2(1, 0));

// 0 at calm; ~0.5 at a 5 m/s breeze; saturates at 1.2 for storms.
export function windAmplitude(windSpeedMs: number): number {
  return Math.min(1.2, windSpeedMs / 10);
}
```

In `applyCityEnvironment.ts`, next to the existing `t.precipitation.update(env.precipType, env.precipIntensity, env.windSpeedMs, env.windDirRad, dtSeconds)` call, add:

```ts
windAmpU.value = windAmplitude(env.windSpeedMs);
(windDirU.value as THREE.Vector2).set(Math.sin(env.windDirRad), Math.cos(env.windDirRad));
```

(with `import { windAmpU, windDirU, windAmplitude } from './windUniform';` — match the meteorological convention already used by precipitation: check how precipitation.ts converts `windDirRad` to a vector and copy that exact convention so rain streaks and tree sway agree.)

- [ ] **Step 4: Run tests + typecheck**

`npx vitest run tests/diorama/windUniform.test.ts` — PASS. `npx tsc -p tsconfig.typecheck.json` — clean.

- [ ] **Step 5: Commit**

```bash
git add src/diorama/ksw/windUniform.ts src/diorama/ksw/applyCityEnvironment.ts tests/diorama/windUniform.test.ts
git commit -m "feat(trees): shared wind uniforms driven by live weather"
```

---

### Task 3: Tree layer core (`treeLayer.ts`) — instances, tint, TSL variation + wind

**Files:**
- Create: `src/diorama/ksw/geo/treeLayer.ts`
- Test: `tests/diorama/treeLayer.test.ts`

**Interfaces:**
- Consumes: `TreeSpec` from `./geoData` (`{x, z, h, r, kind: 'broad'|'conifer'}`), `allArchetypes()/archetypeIndexFor()/hash01()` from `./treeArchetypes` (Task 1), `windAmpU/windDirU` from `../windUniform` (Task 2), `clayMat` from `../props`, `kswCity` tokens.
- Produces (used by Tasks 4, 5, 6):

```ts
export type TreeInstance = { spec: TreeSpec; archetype: number; tint: THREE.Color; squash: number };
export function assignTrees(trees: readonly TreeSpec[], excludeRect?: {x:number;z:number;w:number;d:number}): TreeInstance[];
export type TreeLayer = {
  group: THREE.Group;                    // name 'cityTrees'
  fullMeshes: THREE.InstancedMesh[];     // one per archetype, name `treeArch:${i}`
  instances: TreeInstance[];             // assignment result (impostors + compaction reuse it)
  setTreeShadows(on: boolean): void;
  compactNear(camX: number, camZ: number): void;  // Task 4 fills this; Task 3 ships it as full-refill
};
export function buildTreeLayer(trees: readonly TreeSpec[], opts?: { excludeRect?: ... }): TreeLayer;
```

Behavior:
- `assignTrees`: filter excludeRect (same predicate as nature.ts today), compute `archetype = archetypeIndexFor(x, z, kind)`, `squash = 0.85 + 0.3 * hash01(...)`, `tint = base(kind).offsetHSL((hh-0.5)*0.04, 0, dL)` — port the exact constants from nature.ts (SQUASH_MIN 0.85, span 0.3, TINT_L_SPREAD 0.16, hue ±0.02) so the palette carries over.
- Full meshes: capacity `Math.max(1, countOfArchetype)` (WebGPU zero-buffer rule), instance matrix `compose(pos(x, 0, z), identityQuat, scale(r / arch.crownRadius, h * squash, r / arch.crownRadius))` — **this is the scale-accuracy fix: `h` and `r` are meters and map 1:1 onto the normalized archetype.** `setColorAt(tint)`.
- Material (one per archetype mesh, cloned `clayMat(kswCity.treeGreen)`): TSL nodes —
  - `colorNode`: `select(aPuff.w < 0, trunkColor, instanceColor × gradient)` where `gradient = mix(0.82, 1.12, smoothstep(crownBaseY, 1, normalizedLocalY))` (two-tone crown, lit top lighter).
  - `positionNode`: puff jitter — for crown vertices, scale around the puff center: `p' = aPuff.xyz + (p − aPuff.xyz) × (0.85 + 0.3·hash(instanceIndex·13 + aPuff.w))` (hash via a TSL `fract(sin(...))` helper — visual-only, GPU float determinism is not required to match CPU); then wind sway — `offset = windDirU × windAmpU × swayY² × (sin(time·1.3 + phase) × 0.6 + sin(time·0.31 + phase·0.7) × 0.4) × 0.05` with `swayY = positionLocal.y` (normalized 0..1 by contract!), `phase = instanceIndex-hashed`; trunk vertices (`aPuff.w < 0`) get offset × 0.15.
  - Import nodes from `three/tsl`: `attribute, instanceIndex, instancedBufferColor OR vertexColor — use the same node agentMeshes/staticBatch use for per-instance color`, `positionLocal, select, float, vec2, vec3, mix, smoothstep, sin, fract, time`. **Copy the working import set from `agentMeshes.ts` — it already does instanceIndex-driven positionNode on this three version.**
- `setTreeShadows(on)`: sets `castShadow` on all fullMeshes (impostors never cast).
- Task 3 ships `compactNear` as "write ALL instances" (full refill, no distance filter) so the layer is complete and renderable before Task 4 adds the near-set logic.

- [ ] **Step 1: Write the failing tests**

```ts
// tests/diorama/treeLayer.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { allArchetypes } from '../../src/diorama/ksw/geo/treeArchetypes';
import { assignTrees, buildTreeLayer } from '../../src/diorama/ksw/geo/treeLayer';
import type { TreeSpec } from '../../src/diorama/ksw/geo/geoData';

const specs: TreeSpec[] = Array.from({ length: 120 }, (_, i) => ({
  x: (i % 12) * 9.1, z: Math.floor(i / 12) * 7.3,
  h: 6 + (i % 7), r: 2 + (i % 4) * 0.6,
  kind: i % 3 === 0 ? 'conifer' : 'broad',
}));

describe('assignTrees', () => {
  it('is deterministic and respects kind partitions', () => {
    const a = assignTrees(specs);
    const b = assignTrees(specs);
    expect(a.map((t) => t.archetype)).toEqual(b.map((t) => t.archetype));
    const broadN = 3 * 4; // BROAD_FAMILIES × SEEDS_PER_FAMILY
    for (const t of a) {
      if (t.spec.kind === 'conifer') expect(t.archetype).toBeGreaterThanOrEqual(broadN);
      else expect(t.archetype).toBeLessThan(broadN);
    }
  });
  it('drops trees inside excludeRect', () => {
    const kept = assignTrees(specs, { x: specs[0].x, z: specs[0].z, w: 1, d: 1 });
    expect(kept.length).toBe(specs.length - 1);
  });
});

describe('buildTreeLayer', () => {
  it('creates one mesh per archetype with counts summing to the assignment', () => {
    const layer = buildTreeLayer(specs);
    expect(layer.fullMeshes.length).toBe(allArchetypes().length);
    const total = layer.fullMeshes.reduce((s, m) => s + m.count, 0);
    expect(total).toBe(layer.instances.length);
    for (const m of layer.fullMeshes) expect((m as THREE.InstancedMesh).instanceMatrix.count).toBeGreaterThanOrEqual(1);
  });
  it('maps h and r to world meters via the archetype envelope', () => {
    const layer = buildTreeLayer([{ x: 0, z: 0, h: 9, r: 3, kind: 'broad' }]);
    const mesh = layer.fullMeshes.find((m) => m.count === 1)!;
    const m4 = new THREE.Matrix4();
    mesh.getMatrixAt(0, m4);
    const s = new THREE.Vector3().setFromMatrixScale(m4);
    const arch = allArchetypes()[layer.instances[0].archetype];
    expect(s.y).toBeCloseTo(9 * layer.instances[0].squash, 3);       // height in meters (×squash band 0.85..1.15)
    expect(s.x * arch.crownRadius).toBeCloseTo(3, 3);                 // crown radius in meters
  });
  it('setTreeShadows toggles castShadow on all full meshes', () => {
    const layer = buildTreeLayer(specs);
    layer.setTreeShadows(true);
    expect(layer.fullMeshes.every((m) => m.castShadow)).toBe(true);
    layer.setTreeShadows(false);
    expect(layer.fullMeshes.every((m) => !m.castShadow)).toBe(true);
  });
});
```

- [ ] **Step 2: Run tests, verify fail** — `npx vitest run tests/diorama/treeLayer.test.ts` → module not found.

- [ ] **Step 3: Implement `treeLayer.ts`** per the interface/behavior block above. Structure:

```ts
// src/diorama/ksw/geo/treeLayer.ts
// Instanced tree rendering over the procedural archetypes: one InstancedMesh
// per archetype (trunk+crown in one geometry, aPuff-selective tint), TSL
// per-instance jitter + crown gradient + weather-coupled wind. Deterministic.
```

Keys:
- Group all meshes under `group.name = 'cityTrees'`; each mesh `name = 'treeArch:' + i`, `castShadow = false` initially, `receiveShadow = true`, `frustumCulled = false` (instances span the city — three would cull the whole mesh by the shared bounding sphere; compaction (Task 4) keeps counts small instead).
- Build the TSL material once per archetype index (crownBaseY differs) via a helper `treeMaterial(arch: TreeArchetype): THREE.MeshPhysicalMaterial` — clone `clayMat`, then assign `positionNode`/`colorNode`. Guard: only build node materials when `typeof (mat as any).positionNode !== 'undefined'` is fine — vitest runs in node without GPU but constructing TSL nodes is pure JS and safe (agentMeshes tests already run under vitest — follow whatever guard pattern `tests/diorama/agentMeshes.test.ts` uses).
- `compactNear(camX, camZ)`: v1 = write every instance of each archetype (grouped by archetype, `setMatrixAt/setColorAt`, `instanceMatrix.needsUpdate = true`, set `.count`).

- [ ] **Step 4: Run tests, verify pass** — `npx vitest run tests/diorama/treeLayer.test.ts`.

- [ ] **Step 5: Typecheck + commit**

```bash
npx tsc -p tsconfig.typecheck.json
git add src/diorama/ksw/geo/treeLayer.ts tests/diorama/treeLayer.test.ts
git commit -m "feat(trees): instanced archetype tree layer with TSL variation and wind"
```

---

### Task 4: Near-set compaction LOD

**Files:**
- Modify: `src/diorama/ksw/geo/treeLayer.ts`
- Test: `tests/diorama/treeLayer.test.ts` (extend)

**Interfaces:**
- Consumes: `kswCityStyle.lod.nearR` from `designTokens` (the existing ring radius — reuse it as the full-detail distance).
- Produces: real `compactNear(camX, camZ)` — after the call, each full mesh contains ONLY instances within `NEAR_TREE_DIST` of the camera point; far trees cost zero vertices. Also export `NEAR_TREE_DIST` (= `kswCityStyle.lod.nearR × 1.1` — 10% slack beyond the impostor-collapse radius so the full tree is already drawn before the impostor collapses; the overlap band prevents gaps).

Implementation: at build time, bucket `instances` into a coarse spatial grid (cell = 64 m, plain `Map<string, TreeInstance[]>`); `compactNear` queries the grid cells overlapping the camera disc, groups hits by archetype, rewrites matrices/colors, sets counts (0-hit archetypes get `count = 0` — capacity stays ≥ 1 so buffers remain valid).

- [ ] **Step 1: Write the failing tests** (extend treeLayer.test.ts)

```ts
describe('compactNear', () => {
  it('keeps only trees within NEAR_TREE_DIST of the camera', () => {
    const far: TreeSpec = { x: 5000, z: 5000, h: 8, r: 2.5, kind: 'broad' };
    const near: TreeSpec = { x: 3, z: 4, h: 8, r: 2.5, kind: 'broad' };
    const layer = buildTreeLayer([near, far]);
    layer.compactNear(0, 0);
    expect(layer.fullMeshes.reduce((s, m) => s + m.count, 0)).toBe(1);
    layer.compactNear(5000, 5000);
    expect(layer.fullMeshes.reduce((s, m) => s + m.count, 0)).toBe(1);
  });
  it('restores instances when the camera returns (no destructive drop)', () => {
    const layer = buildTreeLayer(specs);
    const all = layer.instances.length;
    layer.compactNear(1e6, 1e6);
    expect(layer.fullMeshes.reduce((s, m) => s + m.count, 0)).toBe(0);
    layer.compactNear(specs[0].x, specs[0].z);
    expect(layer.fullMeshes.reduce((s, m) => s + m.count, 0)).toBeGreaterThan(0);
    expect(layer.instances.length).toBe(all); // assignment untouched
  });
});
```

- [ ] **Step 2: Run, verify fail** (first test fails: v1 writes everything).
- [ ] **Step 3: Implement** grid + query + per-archetype rewrite as described.
- [ ] **Step 4: Run full treeLayer suite** — PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(trees): near-set compaction LOD over a spatial grid"`.

---

### Task 5: Octahedral impostors (`treeImpostors.ts`)

**Files:**
- Create: `src/diorama/ksw/geo/treeImpostors.ts`
- Test: `tests/diorama/treeImpostors.test.ts`

**Interfaces:**
- Consumes: `TreeArchetype[]` (Task 1), `TreeInstance[]` + `NEAR_TREE_DIST` (Tasks 3/4), `clayMat`/tokens for bake colors.
- Produces (used by Task 6):

```ts
export const OCT_GRID = 4;              // 4×4 hemi-octahedral views per archetype
export const CELL_PX = 128;
export function atlasLayout(archCount: number): { cols: number; rows: number; width: number; height: number };
export function hemiOctUv(dir: THREE.Vector3): { u: number; v: number };   // pure: view dir → grid cell coords (0..OCT_GRID-1 space)
export function viewDirFor(ix: number, iy: number): THREE.Vector3;         // inverse: grid cell → bake camera direction (unit, y ≥ 0)
export async function bakeImpostorAtlas(renderer: THREE.WebGPURenderer, archetypes: TreeArchetype[]): Promise<THREE.Texture>;
export function buildImpostorMesh(instances: readonly TreeInstance[], atlas: THREE.Texture, archCount: number): THREE.InstancedMesh; // name 'treeImpostors'
```

Design:
- **Hemi-oct mapping** (standard): for a unit dir with `y ≥ 0`, `p = dir.xz / (|dir.x| + dir.y + |dir.z|)`, then rotate 45°: `u' = (p.x + p.y) * 0.5 + 0.5`, `v' = (p.y − p.x) * 0.5 + 0.5` → scale to grid. `viewDirFor` inverts it. Round-trip is the test.
- **Atlas bake** (boot, once): one throwaway scene per archetype — mesh with the REAL clay material colors (base treeGreen/woodGreen + trunk via the same colorNode minus per-instance tint), transparent clear (`renderer.setClearColor(0, 0)`), orthographic camera per view dir framing the unit envelope (height 1, width 2×crownRadius), rendered into the atlas RenderTarget cell via `renderer.setViewport/setScissor` + `renderAsync`. Atlas: `THREE.RenderTarget` with `depthBuffer: true`, format RGBA; return `rt.texture`.
- **Impostor mesh**: ONE InstancedMesh of a unit quad (PlaneGeometry 1×1, anchored at y 0..1), capacity = all instances, always visible. Per-instance attributes via `InstancedBufferAttribute`: `aArch` (float), plus the standard instanceMatrix carries position/scale (reuse the exact same compose as the full mesh so silhouettes align) and `setColorAt` the same tint (multiplied onto the sprite — slight trunk tinting accepted at distance).
- Impostor TSL: cylindrical billboard — yaw from `cameraPosition.sub(instancePos)` (instance position via the instance matrix translation node — copy the pattern used for per-instance world position in `agentMeshes.ts`); `opacityNode`/`alphaTest 0.5` from the atlas sample; UV = cell(aArch, hemiOctUv(viewDir)) + quad uv × cellSize; near collapse: `scale ×= step(NEAR_COLLAPSE, dist)` with `NEAR_COLLAPSE = kswCityStyle.lod.nearR` (full trees appear at `NEAR_TREE_DIST = nearR × 1.1`, so the bands overlap and nothing gaps).
- Material base: `MeshBasicNodeMaterial` (impostors are pre-lit by the bake; no live lighting needed — document this).

- [ ] **Step 1: Write the failing tests** (pure parts only — mapping + layout):

```ts
// tests/diorama/treeImpostors.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { OCT_GRID, atlasLayout, hemiOctUv, viewDirFor } from '../../src/diorama/ksw/geo/treeImpostors';

describe('hemi-octahedral mapping', () => {
  it('round-trips every grid cell', () => {
    for (let iy = 0; iy < OCT_GRID; iy++) {
      for (let ix = 0; ix < OCT_GRID; ix++) {
        const dir = viewDirFor(ix, iy);
        expect(dir.y).toBeGreaterThanOrEqual(-1e-6); // upper hemisphere
        expect(dir.length()).toBeCloseTo(1, 5);
        const { u, v } = hemiOctUv(dir);
        expect(Math.round(u)).toBe(ix);
        expect(Math.round(v)).toBe(iy);
      }
    }
  });
  it('maps the horizon ring to the grid border and zenith to center', () => {
    const zen = hemiOctUv(new THREE.Vector3(0, 1, 0));
    expect(zen.u).toBeCloseTo((OCT_GRID - 1) / 2, 1);
    expect(zen.v).toBeCloseTo((OCT_GRID - 1) / 2, 1);
  });
});

describe('atlasLayout', () => {
  it('fits archCount × OCT_GRID² cells in a near-square power-of-two atlas', () => {
    const l = atlasLayout(20);
    expect(l.cols * l.rows).toBeGreaterThanOrEqual(20 * OCT_GRID * OCT_GRID);
    expect(Math.log2(l.width) % 1).toBe(0);
    expect(Math.log2(l.height) % 1).toBe(0);
  });
});
```

- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** mapping/layout (pure) + `bakeImpostorAtlas` + `buildImpostorMesh` per the design block. The bake and mesh cannot be unit-tested headless — they are covered by the Task 7 smoke (screenshot from far zoom MUST show varied crowns).
- [ ] **Step 4: Run tests — PASS; typecheck clean.**
- [ ] **Step 5: Commit** — `git commit -m "feat(trees): boot-baked hemi-octahedral impostor atlas + far-field mesh"`.

---

### Task 6: Integration — nature.ts, lod.ts, main.ts

**Files:**
- Modify: `src/diorama/ksw/geo/nature.ts` — DELETE the whole tree section (spots/broad/conifers/trunks/impostors, `broadCanopyGeometry`…`coniferImpostorGeometry`, the tuned consts); keep greens/water. `buildNature` keeps its signature but drops tree handling; `NatureOptions.treeShadows` is removed (no legacy shim — project rule).
- Modify: `src/diorama/ksw/geo/lod.ts` — `CityLodRefs`: replace `treesFull/treeImpostors/setTreeShadows` with `setTreeShadows: (on: boolean) => void` only (trees no longer ring-toggle visibility; compaction + vertex collapse handle it). `applyCityLod` keeps `r.setTreeShadows(ring === 'near')` and loses the two visibility loops.
- Modify: `src/diorama/ksw/main.ts`:
  - Build: `const treeLayer = buildTreeLayer(cityNature.trees, { excludeRect: {…same rect…} }); cityRoot.add(treeLayer.group);`
  - Boot impostors (after renderer init): `const atlas = await bakeImpostorAtlas(renderer, allArchetypes()); treeLayer.group.add(buildImpostorMesh(treeLayer.instances, atlas, allArchetypes().length));`
  - lodRefs: `setTreeShadows: (on) => treeLayer.setTreeShadows(on)` (replace the getObjectByName lookups).
  - Animate loop: throttled compaction — reuse the existing camera-throttle pattern (`lastTrafficCamUpdate`, ~2 Hz): on camera target/radius change beyond 5 m or 500 ms, `treeLayer.compactNear(camera.position.x, camera.position.z)`.
  - Debug hook for the smoke: `window.__trees = { archetypes: allArchetypes().length, fullCount: () => treeLayer.fullMeshes.reduce((s,m)=>s+m.count,0), windAmp: () => windAmpU.value };`
- Test: update the existing LOD test (`grep -rl applyCityLod tests/` — adjust `CityLodRefs` construction), update/remove nature tree assertions (`grep -rl buildNature tests/`).

**Interfaces:** consumes everything produced in Tasks 1–5; produces the running app.

- [ ] **Step 1: Update the LOD + nature tests to the new contracts (they should FAIL against current impl).**
- [ ] **Step 2: Run them, verify fail for the right reason.** `npx vitest run tests/geo tests/diorama`
- [ ] **Step 3: Apply the nature.ts / lod.ts / main.ts modifications.**
- [ ] **Step 4: Full frontend gate.**

Run: `npx tsc -p tsconfig.typecheck.json && npx vitest run && npm run build`
Expected: all green. (Build needs `npm run generate:proto` implicitly — it's part of `npm run build`.)

- [ ] **Step 5: Commit** — `git commit -m "feat(trees): wire archetype tree layer into city, retire fixed-shape trees"`.

---

### Task 7: Browser smoke + screenshots + FPS (`scripts/smoke-trees.mjs`)

**Files:**
- Create: `scripts/smoke-trees.mjs` (start from `scripts/smoke-ksw.mjs` / `scripts/capture-env.mjs` — same launch/`__LOOK_READY` mechanics)

**Preflight (documented at the top of the script):** the diorama boot hangs at `__LOOK_READY` until the world bake exists — `data/winterthur/world/*.pb` must be baked (`npm run geo:fetch && npm run geo:bake-world`) and symlinked (`ln -s ../data/winterthur/world public/winterthur-world`). "can't skip wire type 4" from the client = HTML-instead-of-proto = missing symlink.

**Checks (assert, don't eyeball):**
1. Boot to `__LOOK_READY` with no console errors.
2. `window.__trees.archetypes === 20` and `window.__trees.fullCount() > 0` after boot.
3. Camera near a park (use the `lookAt`-style debug hook or rig): `fullCount()` small (< 2000) — compaction works; zoom out to establishing radius: `fullCount()` drops (near set shrinks around a high camera) while the screenshot still shows crowns (impostors).
4. Wind: `window.__trees.windAmp()` ≥ 0; force `?wx=storm`-equivalent if the env harness supports it (check `?at/?wx` params from the env smoke) and assert amp > 0.5.
5. Screenshots via CDP (NOT `page.screenshot` — hangs on live canvas, memory lesson): `establishing.png` (far), `street.png` (near, street-level), `forest-edge.png`. Saved to `scratch/trees/` for human review.
6. FPS probe: reuse `scripts/fps-check.mjs` logic or its helper — assert ≥ 55 fps at the establishing shot on the dev machine (relative bar; record the number in the PR).

- [ ] **Step 1: Write the script (adapt smoke-ksw.mjs).**
- [ ] **Step 2: Run preflight + script:** `node scripts/smoke-trees.mjs` — all checks green, screenshots written.
- [ ] **Step 3: LOOK at the three screenshots.** Acceptance: distinct silhouettes visible side by side (no two identical neighbors in the street shot), sizes visibly varied, far field shows structured crowns (not uniform blobs), nothing floats or z-fights.
- [ ] **Step 4: Commit** — `git add scripts/smoke-trees.mjs && git commit -m "test(trees): browser smoke — variety, compaction, wind, fps"`.

---

### Task 8: Gate + PR

- [ ] **Step 1:** Full local gate again from clean: `npx tsc -p tsconfig.typecheck.json && npx vitest run && npm run build && node scripts/smoke-trees.mjs`.
- [ ] **Step 2:** `git push -u origin <branch>` and open a PR against `main` titled `Trees Slice 1: procedural archetypes, wind, octahedral impostors, compaction LOD`; body includes the three screenshots + FPS number. End body with the Claude Code attribution line.
- [ ] **Step 3:** Wait for ALL CI checks green (`gh pr checks --watch --exit-status`) — never merge on UNSTABLE (memory rule). Merge, delete branch, verify `origin/main`.

---

## Self-Review (done at write time)

- Spec coverage: archetype generator (T1), per-instance TSL variation + gradient (T3), weather wind (T2/T3), scale mapping h/r→meters (T3), compaction LOD amendment (T4), octahedral impostors (T5), LOD-ring recoupling (T6), mandatory smoke + fps (T7). Slice-1 items all covered; Slices 2/3 explicitly out of scope.
- Placeholders: generator internals are specified by recipe + tables rather than full listing (a complete listing would be ~400 lines of geometry code; the recipe, contracts, and tests pin the behavior). All public interfaces, tests, and commands are concrete.
- Type consistency: `TreeInstance`, `TreeLayer`, archetype index space (broad `0..11`, conifer `12..19`), `NEAR_TREE_DIST = nearR×1.1` vs `NEAR_COLLAPSE = nearR` cross-checked across T3–T6.
