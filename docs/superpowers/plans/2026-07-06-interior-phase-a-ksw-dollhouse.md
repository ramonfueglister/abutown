# Phase A: KSW Multi-Storey Dollhouse Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the two KSW interior bugs — rooms that don't fit the shell and the non-seamless zoom — by making the generated interior multi-storey (stacked per real storey, vertically zoned like a real hospital), decomposing zones along the footprint's dominant wall angle at near-full coverage, and replacing the boolean-pop cutaway with a storey-by-storey coordinated peel.

**Architecture:** Pure, deterministic TypeScript modules (`peelState`, oriented zone decomposition, multi-storey plan generation) drive a per-storey THREE.Group interior and an extended TSL facade shader with a dissolve band. The camera orbit radius maps to a continuous peel progress `p ∈ [0, storeyCount]`; each unit of `p` removes one layer (roof first, then storeys top-down), and the SAME ramp that dissolves a shell band fades the interior below it in.

**Tech Stack:** TypeScript, three.js WebGPU + TSL nodes, vitest, playwright-core (browser smoke).

**Spec:** `docs/superpowers/specs/2026-07-06-interior-generator-design.md` (§1, §6 step 1–2, §8, §12 Phase A).

## Global Constraints

- Branch: `feat/interior-generator`, based on `origin/main` @ eb4a3b9. PR against `origin/main`. NEVER touch local `main`.
- Determinism: no `Date.now()`, no `Math.random()` in any generator/decomposition/peel code. Same input → same output.
- No silent fallbacks (repo rule): errors surface loudly.
- Browser smoke is MANDATORY before claiming Phase A complete (CLAUDE.md rule) — "all vitest green" is NOT sufficient for a frontend-wiring feature.
- Frontend gate before push: `npm run typecheck && npm test && npm run build`. (No Rust is touched in Phase A; if you do touch backend, use `scripts/cargo-serial.sh`.)
- Existing tests must stay green: `tests/interior/zones.test.ts`, `tests/interior/nav-zones.test.ts`, `tests/diorama/floorPlan.test.ts`, `tests/diorama/cameraRig.test.ts`. Only `tests/interior/cutaway.test.ts` is REPLACED (its contract changes by design).
- `tsconfig.json` has `"include": ["src"]` — tests are NOT type-checked by `tsc`. When a test calls a production function, double-check the signature by reading the production file, not from memory.

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `src/diorama/designTokens.ts` | Modify | add `kswPeel` token block; shrink `kswS3` to seam-only |
| `src/diorama/ksw/interior/cutaway.ts` | Rewrite | pure `peelState(radius, cfg)` → per-storey peel state |
| `src/diorama/ksw/interior/zones.ts` | Extend | `dominantAngle`, `PlanFrame`, `decomposeOriented`, coverage opts |
| `src/diorama/ksw/interior/generatePlan.ts` | Extend | `storeyDeptQueue` (vertical zoning), `generateBuildingPlan` |
| `src/diorama/ksw/interior/buildInterior.ts` | Extend | `buildBuildingInterior` — per-storey groups + `setStoreyFades` |
| `src/diorama/ksw/geo/cityMassing.ts` | Modify | facade cutaway shader v2: `discardAbove`/`bandLo`/`bandFade` dissolve |
| `src/diorama/ksw/geo/kswCampus.ts` | Modify | export `largestBuilding`; `setCutaway` v2 signature |
| `src/diorama/ksw/main.ts` | Modify | wire it all: one main-building source, frame transforms, per-frame peel |
| `tests/interior/cutaway.test.ts` | Rewrite | peelState contract |
| `tests/interior/zones-oriented.test.ts` | Create | oriented decomposition invariants |
| `tests/interior/buildingPlan.test.ts` | Create | multi-storey plan invariants |
| `tests/interior/buildingInterior.test.ts` | Create | per-storey group structure + fades |
| `scripts/smoke-ksw-peel.mjs` | Create | browser smoke: peel drives storeys, screenshots |

---

### Task 1: Peel math — `peelState` replaces `cutawayState`

**Files:**
- Modify: `src/diorama/designTokens.ts:404` (the `kswS3` line)
- Rewrite: `src/diorama/ksw/interior/cutaway.ts`
- Rewrite: `tests/interior/cutaway.test.ts`

**Interfaces:**
- Consumes: nothing new.
- Produces (used by Tasks 5, 6):

```ts
export type PeelCfg = {
  storeyCount: number; // L ≥ 1
  storeyH: number;     // metres per storey slab
  baseY: number;       // world-y of the building's ground slab (KSW: 0)
  startR: number;      // orbit radius where peeling begins (p = 0)
  endR: number;        // orbit radius where fully peeled (p = L)
};
export type PeelState = {
  p: number;              // continuous peel progress, 0 (closed) .. L (EG only)
  roofFade: number;       // roof+eave opacity: 1 closed → 0 gone (first peel unit)
  discardAbove: number;   // shell hard cut world-y; 1e6 = off
  bandLo: number;         // dissolve band bottom world-y; 1e6 = no band
  bandFade: number;       // dissolve progress of the band 0 (solid) → 1 (gone)
  storeyFades: number[];  // interior opacity per level, index 0 = EG, length L
};
export function peelState(radius: number, cfg: PeelCfg): PeelState;
export function closedPeel(cfg: PeelCfg): PeelState; // p=0 state, for "camera not over building"
export function storeyLayout(eaveH: number): { storeyCount: number; storeyH: number };
```

**Peel model (the contract):** total peel units = `L = storeyCount`. `t = clamp((startR − radius)/(startR − endR), 0, 1)`, `p = t·L`.
Unit 0 (p ∈ [0,1]) fades the roof+eave out and the TOP storey's interior in (`roofFade = 1−p`, `storeyFades[L−1] = p`). Unit j (p ∈ [j, j+1], 1 ≤ j ≤ L−1) dissolves the shell band of storey `L−j` (band `[baseY+(L−j)·H, baseY+(L−j+1)·H]`, `bandFade = frac`, hard `discardAbove = baseY+(L−j+1)·H`) while the interior of storey `L−j` fades OUT and the interior of storey `L−j−1` fades IN with the same `frac`. The EG (level 0) never fades out. Closed-form per level k:
`storeyFades[k] = clamp(p − (L−1−k), 0, 1) − (k > 0 ? clamp(p − (L−k), 0, 1) : 0)`.

- [ ] **Step 1: Add the `kswPeel` token block and shrink `kswS3`**

In `src/diorama/designTokens.ts` replace the line
`export const kswS3 = { cutHeight: 3.2, cutSeam: 0.25, fadeStartR: 90, fadeEndR: 55, seamColor: 0xf3e2c8 } as const;`
with:

```ts
export const kswS3 = { cutSeam: 0.25, seamColor: 0xf3e2c8 } as const;
// Storey-peel dollhouse (Phase A): the orbit-radius window over which the main
// building peels open storey-by-storey (startR: closed; endR: only the EG
// remains), and the storey-height model derived from the baked eave height.
export const kswPeel = { startR: 110, endR: 40, nominalStoreyH: 3.4, minStoreyH: 2.4, maxStoreyH: 4.5, maxStoreys: 12 } as const;
```

Then `grep -rn "kswS3" src/ tests/` — the only remaining `kswS3.` field accesses allowed after this task are `cutSeam` and `seamColor` (in `cityMassing.ts`). `cutHeight`/`fadeStartR`/`fadeEndR` consumers are removed in this task (cutaway.ts) and Task 6 (main.ts — if main.ts references them directly, note it for Task 6; do not fix main.ts here).

- [ ] **Step 2: Write the failing tests** — replace the entire content of `tests/interior/cutaway.test.ts`:

```ts
// TDD for peelState (Phase A): the pure orbit-radius → storey-peel mapping.
// Contract (plan Task 1): L peel units; unit 0 fades roof out + top interior
// in; unit j (j≥1) dissolves the shell band of storey L−j while its interior
// fades out and the storey below fades in. EG never fades out.
import { describe, expect, it } from 'vitest';
import { peelState, closedPeel, storeyLayout, type PeelCfg } from '../../src/diorama/ksw/interior/cutaway';
import { kswPeel } from '../../src/diorama/designTokens';

const cfg: PeelCfg = { storeyCount: 4, storeyH: 3.5, baseY: 0, startR: kswPeel.startR, endR: kswPeel.endR };
const rAt = (p: number): number => cfg.startR - (p / cfg.storeyCount) * (cfg.startR - cfg.endR);

describe('peelState', () => {
  it('is fully closed at and above startR', () => {
    for (const r of [cfg.startR, cfg.startR + 1, 500, 1500]) {
      const s = peelState(r, cfg);
      expect(s.p).toBe(0);
      expect(s.roofFade).toBe(1);
      expect(s.discardAbove).toBe(1e6);
      expect(s.bandFade).toBe(0);
      expect(s.storeyFades).toEqual([0, 0, 0, 0]);
    }
  });

  it('closedPeel equals the closed state', () => {
    expect(closedPeel(cfg)).toEqual(peelState(cfg.startR + 100, cfg));
  });

  it('unit 0: roof fades out while ONLY the top storey interior fades in', () => {
    const s = peelState(rAt(0.5), cfg);
    expect(s.p).toBeCloseTo(0.5, 5);
    expect(s.roofFade).toBeCloseTo(0.5, 5);
    expect(s.discardAbove).toBe(1e6); // no wall slicing during the roof unit
    expect(s.storeyFades[3]).toBeCloseTo(0.5, 5);
    expect(s.storeyFades[0]).toBe(0);
    expect(s.storeyFades[1]).toBe(0);
    expect(s.storeyFades[2]).toBe(0);
  });

  it('unit j dissolves the band of storey L−j with coordinated interior swap', () => {
    // p = 1.5 → unit 1, frac 0.5: storey 3 (top) shell band half-dissolved,
    // interior 3 half-out (1−0.5), interior 2 half-in (0.5).
    const s = peelState(rAt(1.5), cfg);
    expect(s.roofFade).toBe(0);
    expect(s.bandLo).toBeCloseTo(0 + 3 * 3.5, 5);       // baseY+(L−j)·H, j=1
    expect(s.discardAbove).toBeCloseTo(0 + 4 * 3.5, 5); // baseY+(L−j+1)·H
    expect(s.bandFade).toBeCloseTo(0.5, 5);
    expect(s.storeyFades[3]).toBeCloseTo(0.5, 5);
    expect(s.storeyFades[2]).toBeCloseTo(0.5, 5);
    expect(s.storeyFades[1]).toBe(0);
    expect(s.storeyFades[0]).toBe(0);
  });

  it('fully open at endR: only EG interior at 1, shell cut above storey 1', () => {
    const s = peelState(cfg.endR, cfg);
    expect(s.p).toBeCloseTo(4, 5);
    expect(s.storeyFades).toEqual([1, 0, 0, 0]);
    expect(s.bandFade).toBeCloseTo(1, 5);
    expect(s.bandLo).toBeCloseTo(3.5, 5);       // band of storey 1 fully gone
    expect(s.discardAbove).toBeCloseTo(7, 5);   // = baseY + 2·H
  });

  it('every storeyFade stays in [0,1] and EG is monotonic non-decreasing', () => {
    let prevEg = -1;
    for (let r = cfg.endR - 5; r <= cfg.startR + 5; r += 0.5) {
      const s = peelState(r, cfg);
      for (const f of s.storeyFades) {
        expect(f).toBeGreaterThanOrEqual(0);
        expect(f).toBeLessThanOrEqual(1);
      }
      expect(s.p).toBeGreaterThanOrEqual(0);
      expect(s.p).toBeLessThanOrEqual(cfg.storeyCount);
    }
    for (let r = cfg.startR + 5; r >= cfg.endR - 5; r -= 0.5) {
      const eg = peelState(r, cfg).storeyFades[0];
      expect(eg).toBeGreaterThanOrEqual(prevEg - 1e-9);
      prevEg = eg;
    }
  });

  it('single-storey building: roof fade IS the whole peel, never any wall cut', () => {
    const c1: PeelCfg = { ...cfg, storeyCount: 1 };
    for (const p of [0, 0.3, 0.7, 1]) {
      const r = c1.startR - p * (c1.startR - c1.endR);
      const s = peelState(r, c1);
      expect(s.roofFade).toBeCloseTo(1 - p, 5);
      expect(s.storeyFades).toHaveLength(1);
      expect(s.storeyFades[0]).toBeCloseTo(p, 5);
      expect(s.discardAbove).toBe(1e6);
    }
  });

  it('is deterministic', () => {
    for (const r of [45, 60, 77.5, 110, 400]) expect(peelState(r, cfg)).toEqual(peelState(r, cfg));
  });
});

describe('storeyLayout', () => {
  it('derives count from eave height at the nominal pitch, clamped', () => {
    expect(storeyLayout(3.0)).toEqual({ storeyCount: 1, storeyH: 3.0 });
    expect(storeyLayout(14)).toEqual({ storeyCount: 4, storeyH: 3.5 });
    expect(storeyLayout(17)).toEqual({ storeyCount: 5, storeyH: 3.4 });
  });
  it('clamps storeyH into [minStoreyH, maxStoreyH] via the count', () => {
    const tall = storeyLayout(100); // would be 29 storeys at nominal → capped
    expect(tall.storeyCount).toBeLessThanOrEqual(12);
    const low = storeyLayout(2.0); // below minStoreyH → still 1 storey
    expect(low.storeyCount).toBe(1);
    expect(low.storeyH).toBe(2.0);
  });
});
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `npx vitest run tests/interior/cutaway.test.ts`
Expected: FAIL — `peelState` is not exported.

- [ ] **Step 4: Rewrite `src/diorama/ksw/interior/cutaway.ts`**

```ts
// Storey-peel dollhouse state (Phase A). Pure, deterministic mapping from the
// camera orbit radius to the per-storey peel of the main building: L peel
// units over the [startR, endR] radius window. Unit 0 fades the roof + eave
// out while the TOP storey's interior fades in; unit j (1 ≤ j ≤ L−1)
// dissolves the shell band of storey L−j (screen-door dissolve between
// bandLo and discardAbove, progress bandFade) while that storey's interior
// fades out and the storey below fades in — the SAME ramp drives both sides,
// so there is never a boolean pop. The EG (level 0) never fades out.
import { kswPeel } from '../../designTokens';

export type PeelCfg = {
  storeyCount: number;
  storeyH: number;
  baseY: number;
  startR: number;
  endR: number;
};

export type PeelState = {
  p: number;
  roofFade: number;
  discardAbove: number;
  bandLo: number;
  bandFade: number;
  storeyFades: number[];
};

const OFF = 1e6;
const clamp01 = (v: number): number => Math.min(1, Math.max(0, v));

export function peelState(radius: number, cfg: PeelCfg): PeelState {
  const L = cfg.storeyCount;
  const t = clamp01((cfg.startR - radius) / (cfg.startR - cfg.endR));
  const p = t * L;

  const roofFade = 1 - clamp01(p);

  // Shell dissolve band: only during units j ≥ 1. q = p clamped a hair below
  // L so floor() lands on the last unit at p = L exactly.
  let discardAbove = OFF;
  let bandLo = OFF;
  let bandFade = 0;
  if (p > 1 && L > 1) {
    const q = Math.min(p, L);
    const j = q >= L ? L - 1 : Math.floor(q);
    bandFade = q >= L ? 1 : q - j;
    bandLo = cfg.baseY + (L - j) * cfg.storeyH;
    discardAbove = cfg.baseY + (L - j + 1) * cfg.storeyH;
  }

  const storeyFades: number[] = new Array(L);
  for (let k = 0; k < L; k++) {
    const fadeIn = clamp01(p - (L - 1 - k));
    const fadeOut = k > 0 ? clamp01(p - (L - k)) : 0;
    storeyFades[k] = fadeIn - fadeOut;
  }

  return { p, roofFade, discardAbove, bandLo, bandFade, storeyFades };
}

export function closedPeel(cfg: PeelCfg): PeelState {
  return peelState(cfg.startR, cfg);
}

// Storey count + slab pitch from the baked eave height: round to the nominal
// 3.4 m pitch, clamp the count to [1, maxStoreys]; the resulting pitch is
// eaveH / count (a 1-storey shed keeps its real low eave as its pitch).
export function storeyLayout(eaveH: number): { storeyCount: number; storeyH: number } {
  const nominal = Math.round(eaveH / kswPeel.nominalStoreyH);
  const storeyCount = Math.min(kswPeel.maxStoreys, Math.max(1, nominal));
  return { storeyCount, storeyH: eaveH / storeyCount };
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `npx vitest run tests/interior/cutaway.test.ts`
Expected: PASS (all).
Note: `src/diorama/ksw/main.ts` still imports `cutawayState` — the full `npm run typecheck` breaks until Task 6. That is expected mid-branch; the vitest file above must pass now. Add a temporary compatibility export ONLY if you want typecheck green per-task:

```ts
// TEMPORARY (removed in Task 6): legacy single-slice adapter for main.ts.
export function cutawayState(radius: number): { cutH: number; upperFade: number } {
  const s = peelState(radius, { storeyCount: 1, storeyH: 3.2, baseY: 0, startR: 90, endR: 55 });
  return { cutH: s.roofFade < 0.15 ? 3.2 : 1e6, upperFade: s.roofFade };
}
```

Include this adapter (it keeps every intermediate commit green — repo CI-gate rule).

- [ ] **Step 6: Verify the whole suite + commit**

Run: `npx vitest run && npm run typecheck`
Expected: PASS.

```bash
git add src/diorama/designTokens.ts src/diorama/ksw/interior/cutaway.ts tests/interior/cutaway.test.ts
git commit -m "feat(interior): storey-peel state machine — peelState replaces single-slice cutawayState"
```

---

### Task 2: Oriented full-coverage zone decomposition

**Files:**
- Modify: `src/diorama/ksw/interior/zones.ts`
- Create: `tests/interior/zones-oriented.test.ts`

**Interfaces:**
- Consumes: existing `decomposeToZones(footprint, opts)`, `Zone`.
- Produces (used by Tasks 3, 6):

```ts
export type PlanFrame = {
  angle: number; // radians; THREE group.rotation.y = angle maps plan-local → world
  toLocal(x: number, z: number): [number, number];
  toWorld(x: number, z: number): [number, number];
};
export function dominantAngle(footprint: number[][]): number;
export function decomposeOriented(footprint: number[][]): { zones: Zone[]; frame: PlanFrame };
export type DecomposeOpts = { maxZones?: number; minSize?: number; stopCoverage?: number }; // stopCoverage NEW
```

Rotation convention (MUST match THREE's `rotation.y`): for `group.rotation.y = angle`, a group-local point `(lx, lz)` lands at world `x = lx·cos(angle) + lz·sin(angle)`, `z = −lx·sin(angle) + lz·cos(angle)`. So `toWorld` uses those formulas and `toLocal` is the inverse (`lx = x·cos(angle) − z·sin(angle)`, `lz = x·sin(angle) + z·cos(angle)`).

- [ ] **Step 1: Write the failing tests** — create `tests/interior/zones-oriented.test.ts`:

```ts
// Oriented decomposition (Phase A): zones are extracted in a frame rotated to
// the footprint's dominant wall angle, so rooms run parallel to the facade and
// coverage approaches the full footprint (the old axis-aligned decomposition
// plateaued at ~61% on the diagonal KSW complex).
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { decomposeOriented, decomposeToZones, dominantAngle, type Zone } from '../../src/diorama/ksw/interior/zones';

function pointInRing(x: number, z: number, ring: number[][]): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    if (zi > z !== zj > z && x < ((xj - xi) * (z - zi)) / (zj - zi) + xi) inside = !inside;
  }
  return inside;
}

function shoelaceArea(ring: number[][]): number {
  let a = 0;
  const n = ring.length;
  for (let i = 0, j = n - 1; i < n; j = i++) a += ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
  return Math.abs(a) / 2;
}

// a 40×24 rectangle rotated by 23° around the origin
function rotatedRect(deg: number): number[][] {
  const a = (deg * Math.PI) / 180;
  const pts: number[][] = [[-20, -12], [20, -12], [20, 12], [-20, 12]];
  return pts.map(([x, z]) => [x * Math.cos(a) + z * Math.sin(a), -x * Math.sin(a) + z * Math.cos(a)]);
}

function realKswFootprint(): number[][] {
  const raw = JSON.parse(readFileSync(resolve(__dirname, '../../data/winterthur/buildings.json'), 'utf8'));
  const ksw = raw.buildings.filter((b: { zone?: string }) => b.zone === 'ksw');
  let best = ksw[0];
  for (const b of ksw) if (shoelaceArea(b.footprint) > shoelaceArea(best.footprint)) best = b;
  return best.footprint;
}

describe('dominantAngle', () => {
  it('recovers the rotation of a rotated rectangle (mod 90°)', () => {
    const a = dominantAngle(rotatedRect(23));
    const deg = ((a * 180) / Math.PI + 180) % 90;
    expect(Math.min(deg, 90 - deg)).toBeLessThan(0.5); // 23° folds to 23 or 67 — check |a|≈23° mod 90
    const folded = Math.abs((((a * 180) / Math.PI) % 90) + 90) % 90;
    expect(Math.min(Math.abs(folded - 23), Math.abs(folded - 67))).toBeLessThan(0.5);
  });
  it('returns ~0 for an axis-aligned rectangle', () => {
    const a = dominantAngle([[0, 0], [30, 0], [30, 10], [0, 10]]);
    const deg = Math.abs((a * 180) / Math.PI) % 90;
    expect(Math.min(deg, 90 - deg)).toBeLessThan(0.5);
  });
  it('is deterministic', () => {
    const fp = realKswFootprint();
    expect(dominantAngle(fp)).toBe(dominantAngle(fp));
  });
});

describe('decomposeOriented', () => {
  it('frame round-trips: toWorld(toLocal(p)) ≈ p', () => {
    const { frame } = decomposeOriented(rotatedRect(23));
    for (const [x, z] of [[3.2, -7.7], [0, 0], [-15, 4]]) {
      const [lx, lz] = frame.toLocal(x, z);
      const [wx, wz] = frame.toWorld(lx, lz);
      expect(wx).toBeCloseTo(x, 9);
      expect(wz).toBeCloseTo(z, 9);
    }
  });

  it('covers ≥ 95% of a rotated rectangle (the old path could not)', () => {
    const fp = rotatedRect(23);
    const { zones } = decomposeOriented(fp);
    const covered = zones.reduce((s, z) => s + z.w * z.d, 0);
    expect(covered / shoelaceArea(fp)).toBeGreaterThan(0.95);
  });

  it('covers ≥ 80% of the real KSW footprint (old: ~61%)', () => {
    const fp = realKswFootprint();
    const { zones } = decomposeOriented(fp);
    const covered = zones.reduce((s, z) => s + z.w * z.d, 0);
    expect(covered / shoelaceArea(fp)).toBeGreaterThan(0.8);
  });

  it('every zone corner, mapped to world, lies inside the original footprint', () => {
    const fp = realKswFootprint();
    const { zones, frame } = decomposeOriented(fp);
    const eps = 1e-6;
    for (const z of zones) {
      for (const [cx, cz] of [
        [z.x - z.w / 2 + eps, z.z - z.d / 2 + eps],
        [z.x + z.w / 2 - eps, z.z - z.d / 2 + eps],
        [z.x + z.w / 2 - eps, z.z + z.d / 2 - eps],
        [z.x - z.w / 2 + eps, z.z + z.d / 2 - eps],
      ]) {
        const [wx, wz] = frame.toWorld(cx, cz);
        expect(pointInRing(wx, wz, fp)).toBe(true);
      }
    }
  });

  it('zones never overlap each other', () => {
    const { zones } = decomposeOriented(realKswFootprint());
    const e = 1e-6;
    for (let i = 0; i < zones.length; i++) {
      for (let j = i + 1; j < zones.length; j++) {
        const a = zones[i] as Zone;
        const b = zones[j] as Zone;
        const overlap =
          a.x - a.w / 2 < b.x + b.w / 2 - e && b.x - b.w / 2 < a.x + a.w / 2 - e &&
          a.z - a.d / 2 < b.z + b.d / 2 - e && b.z - b.d / 2 < a.z + a.d / 2 - e;
        expect(overlap).toBe(false);
      }
    }
  });

  it('legacy decomposeToZones is unchanged for existing callers', () => {
    const fp = [[0, 0], [40, 0], [40, 20], [0, 20]];
    const zones = decomposeToZones(fp);
    expect(zones.length).toBeGreaterThan(0);
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run tests/interior/zones-oriented.test.ts`
Expected: FAIL — `decomposeOriented` / `dominantAngle` not exported.

- [ ] **Step 3: Implement in `src/diorama/ksw/interior/zones.ts`**

Extend `DecomposeOpts` (`stopCoverage?: number`, default keeps 0.15) and change the extraction loop condition to use it: replace `const minCoverageCells = totalInside * 0.15;` with `const minCoverageCells = totalInside * (opts.stopCoverage ?? 0.15);`.

Append to the file:

```ts
// ── Oriented decomposition (Phase A) ────────────────────────────────────────
// Real footprints rarely run parallel to the world axes (KSW: ~23°). Extract
// zones in a frame rotated to the footprint's dominant wall angle so the
// rectangles hug the facade; the interior THREE group then simply rotates by
// frame.angle to land back in world space.

export type PlanFrame = {
  angle: number;
  toLocal(x: number, z: number): [number, number];
  toWorld(x: number, z: number): [number, number];
};

// Length-weighted dominant edge direction, folded to a 90°-periodic domain
// via the ×4 angle-doubling trick (a wall and its perpendicular vote for the
// same orientation). Deterministic — pure arithmetic over the ring.
export function dominantAngle(footprint: number[][]): number {
  let sx = 0;
  let sz = 0;
  const n = footprint.length;
  for (let i = 0; i < n; i++) {
    const [ax, az] = footprint[i];
    const [bx, bz] = footprint[(i + 1) % n];
    const ex = bx - ax;
    const ez = bz - az;
    const len = Math.hypot(ex, ez);
    if (len < 0.05) continue;
    const a4 = 4 * Math.atan2(ez, ex);
    sx += len * Math.cos(a4);
    sz += len * Math.sin(a4);
  }
  return Math.atan2(sz, sx) / 4;
}

export function makeFrame(angle: number): PlanFrame {
  const c = Math.cos(angle);
  const s = Math.sin(angle);
  return {
    angle,
    // matches THREE group.rotation.y = angle (local → world)
    toWorld: (lx, lz) => [lx * c + lz * s, -lx * s + lz * c],
    toLocal: (x, z) => [x * c - z * s, x * s + z * c],
  };
}

// Zone extraction knobs for the oriented path: in the rotated frame the walls
// are near-axis-aligned, so greedy rectangles reach near-full coverage — allow
// many small zones and stop only when 3% of the raster remains.
const ORIENTED_OPTS: DecomposeOpts = { maxZones: 40, minSize: 4, stopCoverage: 0.03 };

export function decomposeOriented(footprint: number[][]): { zones: Zone[]; frame: PlanFrame } {
  const frame = makeFrame(dominantAngle(footprint));
  const localRing = footprint.map(([x, z]) => {
    const [lx, lz] = frame.toLocal(x, z);
    return [lx, lz];
  });
  return { zones: decomposeToZones(localRing, ORIENTED_OPTS), frame };
}
```

- [ ] **Step 4: Run tests**

Run: `npx vitest run tests/interior/zones-oriented.test.ts tests/interior/zones.test.ts`
Expected: PASS. If the KSW coverage assertion (>0.8) fails: lower `CELL` from `2` to `1.5` **as a `cell?: number` opt consumed only by the oriented path** (`ORIENTED_OPTS: { …, cell: 1.5 }`) rather than globally, re-run, and record the achieved coverage in the commit message. Do NOT lower the assertion below 0.75 without flagging it in the PR description.

- [ ] **Step 5: Full suite + commit**

Run: `npx vitest run && npm run typecheck`

```bash
git add src/diorama/ksw/interior/zones.ts tests/interior/zones-oriented.test.ts
git commit -m "feat(interior): oriented zone decomposition — dominant wall angle frame, near-full footprint coverage"
```

---

### Task 3: Multi-storey plan — vertical hospital zoning

**Files:**
- Modify: `src/diorama/ksw/interior/generatePlan.ts`
- Create: `tests/interior/buildingPlan.test.ts`

**Interfaces:**
- Consumes: `Zone` (Task 2 shape, unchanged), `MainDoor`, existing `generateInteriorPlan`, `FloorPlan` from `../floorPlan`, `storeyLayout` (Task 1).
- Produces (used by Tasks 4, 6):

```ts
export type BuildingPlan = {
  storeyCount: number;
  storeyH: number;
  storeys: FloorPlan[]; // index 0 = EG; each is a normal single-floor FloorPlan in plan-local coords
};
export function generateBuildingPlan(zones: Zone[], mainDoor: MainDoor, eaveH: number): BuildingPlan;
```

**Vertical zoning (spec §5 `clinic`):** level 0 = Empfang/Notfall + Radiologie/CT/MRI + Apotheke/Cafeteria; middle levels = OP/IPS/Labor/Endoskopie/Kardiologie/Geburt/Neo; upper levels = Bettenstationen/Physio/Onko/Dialyse/Kinder; topmost level (only when `storeyCount ≥ 4`) = Technik. People (`layPeople`) are placed on level 0 ONLY — the crowd/nav system is 2D (Phase A constraint); upper storeys get props + signage but empty `people` arrays.

- [ ] **Step 1: Write the failing tests** — create `tests/interior/buildingPlan.test.ts`:

```ts
// Multi-storey building plan (Phase A): one FloorPlan per storey, vertically
// zoned like a real hospital. All geometry invariants of the single-floor
// generator hold per storey.
import { describe, expect, it } from 'vitest';
import { generateBuildingPlan } from '../../src/diorama/ksw/interior/generatePlan';
import type { Zone } from '../../src/diorama/ksw/interior/zones';

const zones: Zone[] = [
  { id: 'z0', x: 0, z: 0, w: 60, d: 30 },
  { id: 'z1', x: 50, z: 0, w: 30, d: 24 },
];
const door = { x: -10, z: 15, yaw: 0 };

describe('generateBuildingPlan', () => {
  it('derives the storey count from the eave height', () => {
    expect(generateBuildingPlan(zones, door, 14).storeyCount).toBe(4);
    expect(generateBuildingPlan(zones, door, 3).storeyCount).toBe(1);
    expect(generateBuildingPlan(zones, door, 14).storeys).toHaveLength(4);
  });

  it('level 0 leads with Empfang + Notfall; imaging stays on the ground floor', () => {
    const bp = generateBuildingPlan(zones, door, 14);
    const labels0 = bp.storeys[0].rooms.map((r) => r.label).join('|');
    expect(labels0).toContain('Empfang');
    expect(labels0).toContain('Notfall');
    expect(labels0).toContain('Radiologie');
  });

  it('a middle level carries OP/IPS, an upper level carries Bettenstationen', () => {
    const bp = generateBuildingPlan(zones, door, 17); // 5 storeys
    const mid = bp.storeys[1].rooms.map((r) => r.label).join('|');
    expect(mid).toMatch(/OP|Intensiv/);
    const upper = bp.storeys[3].rooms.map((r) => r.label).join('|');
    expect(upper).toContain('Bettenstation');
  });

  it('the top level of a ≥4-storey building is Technik', () => {
    const bp = generateBuildingPlan(zones, door, 17);
    const top = bp.storeys[bp.storeyCount - 1].rooms.map((r) => r.label).join('|');
    expect(top).toContain('Technik');
  });

  it('people exist ONLY on level 0 (nav is 2D in Phase A)', () => {
    const bp = generateBuildingPlan(zones, door, 14);
    expect(bp.storeys[0].rooms.some((r) => r.people.length > 0)).toBe(true);
    for (let k = 1; k < bp.storeyCount; k++) {
      for (const room of bp.storeys[k].rooms) expect(room.people).toHaveLength(0);
    }
  });

  it('every storey keeps the single-floor invariants: rooms inside the zone set, no room overlap', () => {
    const bp = generateBuildingPlan(zones, door, 14);
    const insideSomeZone = (x: number, z: number): boolean =>
      zones.some((zn) => x >= zn.x - zn.w / 2 - 1e-6 && x <= zn.x + zn.w / 2 + 1e-6 && z >= zn.z - zn.d / 2 - 1e-6 && z <= zn.z + zn.d / 2 + 1e-6);
    for (const plan of bp.storeys) {
      for (const room of plan.rooms) {
        expect(insideSomeZone(room.rect.x - room.rect.w / 2, room.rect.z - room.rect.d / 2)).toBe(true);
        expect(insideSomeZone(room.rect.x + room.rect.w / 2, room.rect.z + room.rect.d / 2)).toBe(true);
      }
      for (let i = 0; i < plan.rooms.length; i++) {
        for (let j = i + 1; j < plan.rooms.length; j++) {
          const a = plan.rooms[i].rect;
          const b = plan.rooms[j].rect;
          const e = 1e-6;
          const overlap =
            a.x - a.w / 2 < b.x + b.w / 2 - e && b.x - b.w / 2 < a.x + a.w / 2 - e &&
            a.z - a.d / 2 < b.z + b.d / 2 - e && b.z - b.d / 2 < a.z + a.d / 2 - e;
          expect(overlap).toBe(false);
        }
      }
    }
  });

  it('is deterministic', () => {
    expect(generateBuildingPlan(zones, door, 14)).toEqual(generateBuildingPlan(zones, door, 14));
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run tests/interior/buildingPlan.test.ts`
Expected: FAIL — `generateBuildingPlan` not exported.

- [ ] **Step 3: Implement in `src/diorama/ksw/interior/generatePlan.ts`**

Refactor with minimal churn: the existing `generateInteriorPlan(zones, mainDoor)` keeps its exact signature and behavior (existing tests depend on it). Internally, extract its body into a parameterized worker and add the storey layer:

```ts
import { storeyLayout } from './cutaway';

// ── Vertical hospital zoning (Phase A, spec §5 'clinic') ────────────────────
// Which department families fill which storey. Level 0 keeps the door-zone
// logic (Empfang + Notfall lead); imaging is heavy machinery → ground floor.
type LevelBand = 'ground' | 'treatment' | 'ward' | 'technik';

export function levelBand(level: number, storeyCount: number): LevelBand {
  if (level === 0) return 'ground';
  if (storeyCount >= 4 && level === storeyCount - 1) return 'technik';
  const lastTreatment = Math.max(1, Math.floor((storeyCount - 1) / 2));
  return level <= lastTreatment ? 'treatment' : 'ward';
}

function bandDepts(band: LevelBand): Dept[] {
  switch (band) {
    case 'ground':
      return [
        deptFrom('xray', 'Radiologie'),
        deptFrom('ct', 'Computertomographie'),
        deptFrom('mri', 'MRI'),
        deptFrom('apotheke', 'Spitalapotheke'),
        deptFrom('cafeteria', 'Cafeteria'),
        deptFrom('admin', 'Verwaltung'),
      ];
    case 'treatment':
      return [
        deptFrom('op1', 'Zentral-OP'),
        deptFrom('ips', 'Intensivstation IPS'),
        deptFrom('lab', 'Zentrallabor'),
        deptFrom('endo', 'Endoskopie'),
        deptFrom('cardio', 'Kardiologie'),
        deptFrom('geburt', 'Gebärsaal'),
        deptFrom('neo', 'Neonatologie'),
      ];
    case 'ward':
      return [
        deptFrom('wardChirurgie', 'Bettenstation Chirurgie'),
        deptFrom('wardMedizin', 'Bettenstation Medizin'),
        deptFrom('physio', 'Physiotherapie'),
        deptFrom('onko', 'Onkologie Tagesklinik'),
        deptFrom('dialyse', 'Dialyse'),
        deptFrom('kinder', 'Kinderklinik'),
      ];
    case 'technik':
      return [deptFrom('admin', 'Technikgeschoss'), deptFrom('lab', 'Gebäudetechnik')];
  }
}
```

Then restructure the plan assembly. Inside the current `generateInteriorPlan`, the per-zone loop calls `zoneDeptQueue(rank, isDoorZone, isLargest)`. Extract the whole body into:

```ts
function generatePlanWithQueues(
  zones: Zone[],
  mainDoor: MainDoor,
  queueFor: (rank: number, isDoorZone: boolean, isLargest: boolean) => Dept[],
  withPeople: boolean,
): FloorPlan
```

— identical body to today's `generateInteriorPlan` except (a) `const depts = queueFor(rank, isDoorZone, isLargest);` and (b) in `buildZoneLadder`, thread `withPeople` through and emit `people: withPeople ? layPeople(rect, dept.roles) : []`. (`buildZoneLadder` gains a fourth parameter `withPeople: boolean`.)

Now the two public entry points:

```ts
export function generateInteriorPlan(zones: Zone[], mainDoor: MainDoor): FloorPlan {
  return generatePlanWithQueues(zones, mainDoor, zoneDeptQueue, true);
}

export type BuildingPlan = { storeyCount: number; storeyH: number; storeys: FloorPlan[] };

export function generateBuildingPlan(zones: Zone[], mainDoor: MainDoor, eaveH: number): BuildingPlan {
  const { storeyCount, storeyH } = storeyLayout(eaveH);
  const storeys: FloorPlan[] = [];
  for (let level = 0; level < storeyCount; level++) {
    const band = levelBand(level, storeyCount);
    if (band === 'ground') {
      // level 0 keeps the authored door behavior: Empfang+Notfall lead the
      // door zone, then the ground-floor families (imaging, Apotheke, …).
      const groundQueue = (rank: number, isDoorZone: boolean, isLargest: boolean): Dept[] => {
        const base = bandDepts('ground');
        if (isDoorZone) return [...doorDepts(), ...base];
        const offset = (isLargest ? 0 : rank + 1) % base.length;
        return [...base.slice(offset), ...base.slice(0, offset)];
      };
      storeys.push(generatePlanWithQueues(zones, mainDoor, groundQueue, true));
    } else {
      const base = bandDepts(band);
      const levelQueue = (rank: number, _isDoorZone: boolean, isLargest: boolean): Dept[] => {
        const offset = ((isLargest ? 0 : rank + 1) + level) % base.length;
        return [...base.slice(offset), ...base.slice(0, offset)];
      };
      storeys.push(generatePlanWithQueues(zones, mainDoor, levelQueue, false));
    }
  }
  return { storeyCount, storeyH, storeys };
}
```

- [ ] **Step 4: Run tests**

Run: `npx vitest run tests/interior/buildingPlan.test.ts tests/interior/nav-zones.test.ts tests/diorama/floorPlan.test.ts`
Expected: PASS — new tests green AND the existing single-floor tests untouched-green.

- [ ] **Step 5: Full suite + commit**

Run: `npx vitest run && npm run typecheck`

```bash
git add src/diorama/ksw/interior/generatePlan.ts tests/interior/buildingPlan.test.ts
git commit -m "feat(interior): multi-storey building plan — vertical hospital zoning (EG Empfang/Notfall, OP mid, wards up, Technik top)"
```

---

### Task 4: Per-storey interior builder with coordinated fades

**Files:**
- Modify: `src/diorama/ksw/interior/buildInterior.ts`
- Create: `tests/interior/buildingInterior.test.ts`

**Interfaces:**
- Consumes: `BuildingPlan` (Task 3), `PlanFrame` (Task 2), existing `buildInterior(plan)`.
- Produces (used by Task 6):

```ts
export type BuildingInteriorControl = {
  group: THREE.Group;                      // rotation.y = frame.angle already applied
  setStoreyFades(fades: number[]): void;   // fades[k] drives level k opacity+visibility
};
export function buildBuildingInterior(bp: BuildingPlan, frame: { angle: number }): BuildingInteriorControl;
```

Mechanics: one child group per storey (`name: 'storey-<k>'`, `position.y = k * bp.storeyH`, `userData.level = k`). All mesh materials inside a storey are CLONED once at build time (per-storey `Map<orig, clone>` so sharing within a storey survives), with `transparent = true`, `depthWrite = true`. `setStoreyFades` writes `clone.opacity = fade` and `storeyGroup.visible = fade > 0.02`.

- [ ] **Step 1: Write the failing tests** — create `tests/interior/buildingInterior.test.ts`:

```ts
// buildBuildingInterior (Phase A): per-storey groups stacked at k·storeyH,
// per-storey material clones so setStoreyFades drives opacity independently.
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildBuildingInterior } from '../../src/diorama/ksw/interior/buildInterior';
import { generateBuildingPlan } from '../../src/diorama/ksw/interior/generatePlan';
import type { Zone } from '../../src/diorama/ksw/interior/zones';

const zones: Zone[] = [{ id: 'z0', x: 0, z: 0, w: 40, d: 24 }];
const bp = generateBuildingPlan(zones, { x: 0, z: 12, yaw: 0 }, 10.2); // 3 storeys, H=3.4

function storeyGroups(group: THREE.Group): THREE.Group[] {
  return group.children.filter((c): c is THREE.Group => c.name.startsWith('storey-'));
}

describe('buildBuildingInterior', () => {
  it('builds one group per storey at level·storeyH with userData.level', () => {
    const { group } = buildBuildingInterior(bp, { angle: 0.4 });
    const storeys = storeyGroups(group);
    expect(storeys).toHaveLength(3);
    storeys.forEach((s, k) => {
      expect(s.userData.level).toBe(k);
      expect(s.position.y).toBeCloseTo(k * bp.storeyH, 6);
    });
    expect(group.rotation.y).toBeCloseTo(0.4, 9);
  });

  it('materials are NOT shared across storeys (fades must be independent)', () => {
    const { group } = buildBuildingInterior(bp, { angle: 0 });
    const [s0, s1] = storeyGroups(group);
    const mats = (g: THREE.Object3D): Set<THREE.Material> => {
      const out = new Set<THREE.Material>();
      g.traverse((o) => {
        const m = (o as THREE.Mesh).material;
        if (m) (Array.isArray(m) ? m : [m]).forEach((mm) => out.add(mm as THREE.Material));
      });
      return out;
    };
    const m0 = mats(s0);
    for (const m of mats(s1)) expect(m0.has(m)).toBe(false);
  });

  it('setStoreyFades drives per-storey opacity and visibility', () => {
    const { group, setStoreyFades } = buildBuildingInterior(bp, { angle: 0 });
    setStoreyFades([1, 0.5, 0]);
    const [s0, s1, s2] = storeyGroups(group);
    expect(s0.visible).toBe(true);
    expect(s1.visible).toBe(true);
    expect(s2.visible).toBe(false);
    let sawHalf = false;
    s1.traverse((o) => {
      const m = (o as THREE.Mesh).material as THREE.Material | undefined;
      if (m && !Array.isArray(m)) {
        expect((m as THREE.MeshStandardMaterial).opacity).toBeCloseTo(0.5, 6);
        sawHalf = true;
      }
    });
    expect(sawHalf).toBe(true);
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run tests/interior/buildingInterior.test.ts`
Expected: FAIL — `buildBuildingInterior` not exported. (If `three/webgpu` import fails in the vitest node environment, check how `tests/diorama/floorPlan.test.ts` handles THREE imports and mirror it; the module constructs only Groups/Meshes/Materials, no renderer.)

- [ ] **Step 3: Implement** — append to `src/diorama/ksw/interior/buildInterior.ts`:

```ts
import type { BuildingPlan } from './generatePlan';

export type BuildingInteriorControl = {
  group: THREE.Group;
  setStoreyFades(fades: number[]): void;
};

// Stack one buildInterior() group per storey and give every storey its own
// material clones so the peel can fade levels independently. Clones are made
// ONCE at build (Map keyed by the original, so materials shared within a
// storey stay shared); setStoreyFades only writes opacity + visibility.
export function buildBuildingInterior(bp: BuildingPlan, frame: { angle: number }): BuildingInteriorControl {
  const group = new THREE.Group();
  group.name = 'kswInterior';
  group.rotation.y = frame.angle;

  const storeyMats: THREE.Material[][] = [];
  bp.storeys.forEach((plan, level) => {
    const storey = buildInterior(plan);
    storey.name = `storey-${level}`;
    storey.userData.level = level;
    storey.position.y = level * bp.storeyH;

    const clones = new Map<THREE.Material, THREE.Material>();
    storey.traverse((o) => {
      const mesh = o as THREE.Mesh;
      if (!mesh.isMesh) return;
      const swap = (m: THREE.Material): THREE.Material => {
        let c = clones.get(m);
        if (!c) {
          c = m.clone();
          c.transparent = true;
          c.depthWrite = true;
          clones.set(m, c);
        }
        return c;
      };
      mesh.material = Array.isArray(mesh.material) ? mesh.material.map(swap) : swap(mesh.material);
    });
    storeyMats.push([...clones.values()]);
    group.add(storey);
  });

  const setStoreyFades = (fades: number[]): void => {
    bp.storeys.forEach((_, level) => {
      const storey = group.getObjectByName(`storey-${level}`);
      if (!storey) return;
      const fade = fades[level] ?? 0;
      storey.visible = fade > 0.02;
      for (const m of storeyMats[level]) m.opacity = fade;
    });
  };

  return { group, setStoreyFades };
}
```

- [ ] **Step 4: Run tests**

Run: `npx vitest run tests/interior/buildingInterior.test.ts`
Expected: PASS.

- [ ] **Step 5: Full suite + commit**

Run: `npx vitest run && npm run typecheck`

```bash
git add src/diorama/ksw/interior/buildInterior.ts tests/interior/buildingInterior.test.ts
git commit -m "feat(interior): per-storey interior builder with independent fade control"
```

---

### Task 5: Facade shader v2 — dissolve band + `setCutaway` v2

**Files:**
- Modify: `src/diorama/ksw/geo/cityMassing.ts:240-360` (the `CutawayFacadeMaterial` type + `facadeMaterial`)
- Modify: `src/diorama/ksw/geo/kswCampus.ts` (export `largestBuilding`, `setCutaway` v2)

**Interfaces:**
- Consumes: `PeelState` fields `{ discardAbove, bandLo, bandFade, roofFade }` (Task 1).
- Produces (used by Task 6):

```ts
// cityMassing.ts
export type CutawayFacadeMaterial = FacadeMaterial & {
  discardAbove: ReturnType<typeof uniform>; // was cutH
  bandLo: ReturnType<typeof uniform>;
  bandFade: ReturnType<typeof uniform>;
};
// kswCampus.ts
export type CutawayUniforms = { discardAbove: number; bandLo: number; bandFade: number; roofFade: number };
export function largestBuilding(buildings: BakedBuilding[]): BakedBuilding; // now exported
// group.userData.setCutaway: (u: CutawayUniforms) => void
```

TSL shaders can't be unit-tested in vitest — this task's verification is typecheck + the Task 7 browser smoke. Keep the change minimal and symmetric to the existing code.

- [ ] **Step 1: Extend the shader in `facadeMaterial`** (`cityMassing.ts` ~line 322-341). Replace the cutaway block:

```ts
  // Storey-peel cutaway (Phase A): three uniforms. Fragments above
  // `discardAbove` are gone (hard cut — everything above the currently
  // dissolving storey). Fragments between `bandLo` and `discardAbove` are the
  // dissolving storey's shell: a deterministic world-position hash against
  // `bandFade` gives a stable screen-door dissolve (no transparency sorting).
  // A warm seam band caps the remaining solid shell just below `bandLo`.
  // At rest (discardAbove = 1e6, bandFade = 0) every node is a no-op — the
  // closed building renders byte-identical to the plain facade material.
  const discardAbove = uniform(1e6);
  const bandLo = uniform(1e6);
  const bandFade = uniform(0);
  const upperFade = uniform(1); // kept: roof/eave opacity driver (roofFade)
  if (opts.cutaway) {
    const seam = new THREE.Color(kswS3.seamColor);
    const seamTone = vec3(seam.r, seam.g, seam.b);
    m.colorNode = Fn(() => {
      const wy = positionWorld.y;
      wy.greaterThan(discardAbove).discard();
      // stable per-fragment hash from world position (same trick as the
      // night-window hash below — sin-fract, deterministic, no RNG)
      const n = positionWorld.x.mul(12.9898).add(wy.mul(78.233)).add(positionWorld.z.mul(37.719)).sin().mul(43758.5453);
      const h = n.sub(n.floor());
      wy.greaterThan(bandLo).and(h.lessThan(bandFade)).discard();
      const seamTop = bandLo.add(float(kswS3.cutSeam).mul(bandFade)); // seam grows in as the band dissolves
      const inSeam = wy.greaterThan(bandLo.sub(float(kswS3.cutSeam))).and(wy.lessThanEqual(seamTop)).and(bandFade.greaterThan(float(0.05)));
      return mix(facadeColor, seamTone, inSeam.select(float(1), float(0)));
    })();
  } else {
    m.colorNode = facadeColor;
  }
```

Update the type + the `Object.assign` return: `Object.assign(m, { facadeDetail, discardAbove, bandLo, bandFade, upperFade })` and the `CutawayFacadeMaterial` type accordingly (drop `cutH`).

- [ ] **Step 2: Update `kswCampus.ts`**

1. Add `export` to the existing `largestBuilding` function (line 42) and to `footprintArea` (line 32).
2. Replace `CutawayUniforms` and the driver:

```ts
export type CutawayUniforms = { discardAbove: number; bandLo: number; bandFade: number; roofFade: number };
```

```ts
  group.userData.setCutaway = (u: CutawayUniforms): void => {
    mainWallMat.discardAbove.value = u.discardAbove;
    mainWallMat.bandLo.value = u.bandLo;
    mainWallMat.bandFade.value = u.bandFade;
    mainRoofMat.opacity = u.roofFade;
    mainRoof.visible = u.roofFade > 0.001;
    mainEaveMat.opacity = u.roofFade;
    mainEave.visible = u.roofFade > 0.001;
  };
```

- [ ] **Step 3: Typecheck**

Run: `npm run typecheck`
Expected: errors ONLY in `src/diorama/ksw/main.ts` (still calling the old `setCutaway`/`cutawayState` shapes) — that is Task 6. If errors appear elsewhere, fix them here.

- [ ] **Step 4: Commit**

```bash
git add src/diorama/ksw/geo/cityMassing.ts src/diorama/ksw/geo/kswCampus.ts
git commit -m "feat(interior): facade dissolve band shader + storey-peel setCutaway, export largestBuilding"
```

(If `npm run typecheck` is red because of main.ts, note it in the commit body: `typecheck red pending main.ts rewiring (next commit)` — and do Tasks 5+6 as ONE push unit; never push with red typecheck.)

---

### Task 6: Wire it in `main.ts` — one main-building source, frame, per-frame peel

**Files:**
- Modify: `src/diorama/ksw/main.ts` (imports ~line 51-55; plan block ~line 220-243; scene block ~line 538-591; heroRect ~line 647-652; boot-apply ~line 1262-1268; animate ~line 1341-1361; `__KSW` snapshot)

**Interfaces:**
- Consumes: `peelState/closedPeel/PeelCfg` + `storeyLayout` (T1), `decomposeOriented` (T2), `generateBuildingPlan/departmentCenter` (T3), `buildBuildingInterior` (T4), `largestBuilding` + `setCutaway` v2 (T5), `kswPeel` token.
- Produces: `window.__KSW.peel = { p, storeyCount, storeyH }` (used by Task 7 smoke).

- [ ] **Step 1: Replace the plan block (~lines 220-243)**

Delete the inline `mainBuildingFp` reduce (lines 223-234). New code:

```ts
  // ── generated interior plan (Phase A): ONE source for the main building
  // (kswCampus.largestBuilding — the same call buildKswCampus makes), zones
  // decomposed in the footprint's dominant-wall-angle frame, one FloorPlan per
  // real storey (eaveH-derived). All plan geometry lives in the plan-local
  // frame; frame.toWorld/group.rotation.y map it back onto the world footprint.
  const mainBuildingFp = largestBuilding(kswBuildings);
  const { zones: interiorZones, frame: planFrame } = decomposeOriented(mainBuildingFp.footprint);
  const mainDoorWorld = mainBuildingFp.door ?? (() => {
    const [wx, wz] = planFrame.toWorld(interiorZones[0]?.x ?? 0, interiorZones[0]?.z ?? 0);
    return { x: wx, z: wz, yaw: 0 };
  })();
  const [doorLx, doorLz] = planFrame.toLocal(mainDoorWorld.x, mainDoorWorld.z);
  const mainDoor = { x: doorLx, z: doorLz, yaw: mainDoorWorld.yaw };
  const buildingPlan = generateBuildingPlan(interiorZones, mainDoor, mainBuildingFp.eaveH);
  const interiorPlan = buildingPlan.storeys[0]; // EG — nav/agents/plaza anchor (2D systems)
  // Re-aim er/ops onto the real department centers — departmentCenter returns
  // plan-local coords, transform to world for the camera targets.
  const [erLx, erLz] = departmentCenter(interiorPlan, 'Notfall');
  const [opLx, opLz] = departmentCenter(buildingPlan.storeys[Math.min(1, buildingPlan.storeyCount - 1)], 'OP');
  const [erX, erZ] = planFrame.toWorld(erLx, erLz);
  const [opX, opZ] = planFrame.toWorld(opLx, opLz);
```

Update imports at the top: from `./interior/zones` import `decomposeOriented` (drop `decomposeToZones` if now unused — check `Zone` type usage stays); from `./interior/generatePlan` import `generateBuildingPlan`; from `./interior/buildInterior` import `buildBuildingInterior`; from `./interior/cutaway` import `peelState, closedPeel, type PeelCfg` (drop `cutawayState` AND delete the temporary adapter in cutaway.ts now); from `./geo/kswCampus` import `largestBuilding`; from `../designTokens` add `kswPeel`.

- [ ] **Step 2: Replace the interior scene block (~lines 542-548)**

```ts
  // ── the generated multi-storey interior (Phase A): per-storey groups whose
  // fades the peel drives every frame. Plan coords are frame-local; the group
  // rotation maps them onto the world footprint.
  const interiorCtl = buildBuildingInterior(buildingPlan, planFrame);
  const interior = interiorCtl.group;
  interior.visible = false; // closed at boot (overview) — the peel opens it
  scene.add(interior);
```

- [ ] **Step 3: Fix the frame-dependent consumers**

1. **erZone / plaza (~line 554-563):** `interiorZones` are plan-local; `buildPlaza` works in world space. Compute the door-zone in local space exactly as today (the reduce over `interiorZones` with local `mainDoor`), then hand `buildPlaza` a world-frame pseudo-zone:

```ts
  const [ezWx, ezWz] = planFrame.toWorld(erZoneLocal.x, erZoneLocal.z);
  const erZone: Zone = { ...erZoneLocal, x: ezWx, z: ezWz };
  const plaza = buildPlaza(mainDoorWorld, erZone, cityRoads);
```

(rename today's reduce result to `erZoneLocal`; it must use local `mainDoor` for the distance test. Plaza's ambulance sits at the zone edge — with the rotated frame this is approximate by one rotation; acceptable: the plaza anchors on `mainDoorWorld` which is exact.)

2. **heroRect (~line 647-652):** `interiorPlan.building` is now local — a rotated rect's local bbox is NOT its world bbox. Use the world footprint instead:

```ts
  const fpB = (() => {
    let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
    for (const [x, z] of mainBuildingFp.footprint) {
      if (x < minX) minX = x; if (x > maxX) maxX = x;
      if (z < minZ) minZ = z; if (z > maxZ) maxZ = z;
    }
    return { minX, maxX, minZ, maxZ };
  })();
  const heroRect = { x: (fpB.minX + fpB.maxX) / 2, z: (fpB.minZ + fpB.maxZ) / 2, w: fpB.maxX - fpB.minX, d: fpB.maxZ - fpB.minZ };
```

(`mbBounds` at ~line 573-586 duplicates this loop — replace its body with `fpB` + the ±6 slack.)

3. **nav/agents (~line 870-912):** nav consumes `interiorPlan` (EG, local frame) — positions stay local, and the agent meshes are added to `interior` (rotated group), so world placement is automatic. No change needed EXCEPT verify the spawn `insideZone` check still uses the LOCAL `interiorZones` (it does — leave as is). Agents fade with the EG storey automatically only if their meshes hang UNDER `storey-0`; change the add to:

```ts
  const egStorey = interior.getObjectByName('storey-0') ?? interior;
  for (const m of agentInstances.meshes) egStorey.add(m);
```

- [ ] **Step 4: Replace the boot-apply + animate cutaway blocks**

Boot block (~lines 1262-1268):

```ts
  const peelCfg: PeelCfg = {
    storeyCount: buildingPlan.storeyCount,
    storeyH: buildingPlan.storeyH,
    baseY: 0, // KSW sits at the world anchor; B-phases feed real ground elevations here
    startR: kswPeel.startR,
    endR: kswPeel.endR,
  };
  const computePeel = () => (targetOverMain() ? peelState(rig.radius, peelCfg) : closedPeel(peelCfg));
  let peel = computePeel();
  const applyPeel = (s: PeelState): void => {
    setCutaway({ discardAbove: s.discardAbove, bandLo: s.bandLo, bandFade: s.bandFade, roofFade: s.roofFade });
    setHelipadFade(s.roofFade);
    interior.visible = s.p > 0.02;
    interiorCtl.setStoreyFades(s.storeyFades);
  };
  applyPeel(peel);
```

(import `type PeelState` too.) Animate block (~lines 1341-1361):

```ts
    // ── storey peel: drive shell dissolve + per-storey interior fades off the
    // zoom radius. GI + the cached shadow map refresh when the peel crosses a
    // storey boundary or settles (fully closed / fully open) — same policy as
    // the roof fade above.
    const nextPeel = computePeel();
    if (nextPeel.p !== peel.p) applyPeel(nextPeel);
    const stepChanged = Math.floor(nextPeel.p) !== Math.floor(peel.p);
    const settled = (nextPeel.p === 0 || nextPeel.p === peelCfg.storeyCount) && nextPeel.p !== peel.p;
    if (stepChanged || settled) {
      giScheduler.markDirty();
      if (shadowCached) sun.shadow.needsUpdate = true;
    }
    peel = nextPeel;
```

- [ ] **Step 5: Expose the smoke surface** — find where `kswSnapshot`/`window.__KSW` is populated (grep `kswSnapshot.radius`, ~line 1373) and add alongside:

```ts
    kswSnapshot.peel = { p: peel.p, storeyCount: peelCfg.storeyCount, storeyH: peelCfg.storeyH };
```

and extend the snapshot type/init wherever `kswSnapshot` is declared (grep `kswSnapshot =`). Remove the temporary `cutawayState` adapter from `cutaway.ts` now, plus any leftover `kswS3.fadeStartR`-family references (grep).

- [ ] **Step 6: Verify — full frontend gate**

Run: `npm run typecheck && npx vitest run && npm run build`
Expected: all PASS, zero references left to `cutawayState`/`cutH` (grep to confirm: `grep -rn "cutawayState\|\.cutH" src/ tests/` → empty).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(interior): wire storey peel — single main-building source, oriented frame, per-frame storey fades"
```

---

### Task 7: Browser smoke — the mandatory proof

**Files:**
- Create: `scripts/smoke-ksw-peel.mjs` (start from `scripts/smoke-ksw.mjs` as the template — same launch/`__KSW` polling scaffolding, same dev-stack assumptions)

**Preconditions (from memory/CLAUDE.md):** a fresh worktree needs the world bake before `ksw.html` boots: `npm run generate:proto`, then geo fetch/bake OR the existing `public/winterthur-world` symlink → verify `ls public/winterthur-world/` first; if missing, run `npm run geo:fetch && npm run geo:bake-world` (slow — run in background) or symlink `data/winterthur/world`. `__LOOK_READY` hangs without it.

- [ ] **Step 1: Write the smoke script.** Structure (adapt the template's browser/launch boilerplate — the checks are what matters):

```js
// scripts/smoke-ksw-peel.mjs — Phase A proof: the storey peel actually drives
// the DOM/GPU scene. Checks (against ?cam=overview boot):
//  1. __KSW.peel exists; storeyCount ≥ 2 (real KSW eave gives multiple storeys)
//  2. boot (overview, radius 520): peel.p === 0
//  3. drive the camera over the main building + zoom to radius ~75
//     (mid-window): 0 < peel.p < storeyCount  → screenshot peel-mid.png
//  4. zoom to radius ≤ 40: peel.p === storeyCount (fully open, EG visible)
//     → screenshot peel-open.png
//  5. zoom back out past 110: peel.p === 0 again (reversible, no stuck state)
//  6. zero page errors / console errors during the whole run
// Screenshots land in scratch/ (gitignored) — attach to the PR.
```

Implementation notes: reuse the template's `state()` = `page.evaluate(() => window.__KSW)`; move the camera with the same wheel/drag emulation the template uses; to center over the main building use the `?cam=er` preset boot (radius 40, already over Notfall — peel MUST be fully open there at boot: assert `peel.p === peel.storeyCount` immediately, then wheel OUT and assert p decreases through the mid state to 0).

- [ ] **Step 2: Run it**

```bash
npm run dev &            # or the template's own server handling — copy it
node scripts/smoke-ksw-peel.mjs
```

Expected output: every `check(...)` line PASS, screenshots written. If a check fails: use systematic-debugging — read the failing state values, inspect `peelState` inputs (radius, cfg), the shader uniforms (`page.evaluate` into the scene), fix production code, re-run. Do NOT weaken a check to make it pass.

- [ ] **Step 3: Visual pass** — open the two screenshots. Acceptance: (a) mid-peel shows the dissolving storey as a speckled band, storeys below intact, NO floating rooms outside the shell, rooms reach the facade; (b) open state shows the EG rooms filling the whole footprint. If rooms visibly stop short of the walls or poke through them, the oriented decomposition or frame transform is wrong — debug before proceeding.

- [ ] **Step 4: Commit**

```bash
git add scripts/smoke-ksw-peel.mjs
git commit -m "test(interior): browser smoke for the storey peel — boot states, reversibility, screenshots"
```

---

### Task 8: Gate, PR

- [ ] **Step 1: Full frontend CI gate**

Run: `npm run typecheck && npx vitest run && npm run build && node scripts/smoke-ksw-peel.mjs`
Expected: all green. (Playwright e2e: run `npx playwright test` if the repo's suite covers ksw.html — check `playwright.config.ts` testDir first.)

- [ ] **Step 2: Push + PR against origin/main**

```bash
git push -u origin feat/interior-generator
gh pr create --title "Interior Phase A: KSW multi-storey dollhouse — oriented full-coverage zones + seamless storey peel" --body "<summary of spec §12 Phase A, link the spec, attach the two smoke screenshots>"
```

- [ ] **Step 3: Wait for CI green (ALL checks pass, not just "not red") before merging.** On local-green/CI-red: check for stale branch base / rustfmt skew first (memory).

---

## Self-Review Notes (already applied)

- Spec coverage: §1 bug 1 → Tasks 2+3+6; §1 bug 2 → Tasks 1+4+5+6; §6 step 1-2 (core/corridor typology) is Phase B scope — Phase A keeps the ladder recipe per storey (spec §12 wording: "mehrstöckige Klinik-Zonierung + Dollhouse-Peel v2 + Voll-Footprint-Zonen + einmalige Hauptgebäude-Wahl + Terrain-Basis"); Terrain-Basis: `baseY` is threaded through `PeelCfg` (KSW = 0 at the anchor; real elevations arrive in B2).
- Type consistency: `PeelState.storeyFades` ↔ `setStoreyFades(fades)`; `CutawayUniforms` v2 shape identical in Task 5 producer and Task 6 consumer; `PlanFrame.toWorld` convention pinned to `group.rotation.y` in both Task 2 (definition) and Task 4 (application).
- Known risk: the vitest `three/webgpu` import in Task 4 — mirrored check against the existing diorama tests included in the step.
