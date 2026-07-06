# Cities-Skylines-Style Cars Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the clay box cars with procedural Cities-Skylines-style low-poly cars — 6 silhouette variants, glossy clearcoat paint, bright reflective glass, baked light/grille/plate zones, and real cylinder wheels that roll with vehicle speed and steer at the front.

**Architecture:** `carModels.ts` gains a loft-based geometry kernel (trapezoid cross-sections swept along the car's z axis) producing per-variant body+glass geometries plus a shared wheel cylinder and a per-variant wheel layout. A new pure module `wheelSpin.ts` accumulates roll angle and filtered steer per vehicle. `carLayer.ts` renders body/glass/wheel InstancedMeshes (13 draws) and exposes a debug hook for the mandatory browser smoke.

**Tech Stack:** TypeScript, three.js (`three/webgpu`), vitest, Playwright smoke via `scripts/lib/traffic-stack.mjs`.

**Spec:** `docs/superpowers/specs/2026-07-06-cs-style-cars-design.md` (user-approved; reference screenshots in `/Users/ramonfuglister/Coding/abutown/research/`).

## Global Constraints

- Frontend-only change: NO server/protocol edits, NO external 3D assets, NO textures — geometry + vertex colours + materials only.
- `buildCarGeometry(): THREE.BufferGeometry` in `carLayer.ts` must keep its exact signature (consumed by `flowLayer.ts:29,202`); `flowLayer.ts` itself is untouched.
- Keep the stable id-hash selection API unchanged: `hashId`, `carColorForId`, `carVariantForId`, `CAR_PALETTE`, `CAR_VARIANTS` (table grows 3 → 6 entries).
- No per-frame allocation in `carLayer.update()` (scratch objects only, as today); `frustumCulled = false` on every instanced mesh; `DynamicDrawUsage` on instance matrices.
- `tsc --noEmit` does NOT type-check `tests/` (tsconfig includes `src` only) — keep test/production signatures in sync manually.
- Browser smoke is MANDATORY before claiming completion (CLAUDE.md); fresh worktree needs the world bake first (`npm run geo:fetch` → `npm run geo:bake-world` → `public/winterthur-world` symlink — see memory note "Diorama-Smoke braucht World-Bake"; check whether `public/winterthur-world` already resolves before rebaking).
- Run the full frontend CI gate before push: `npx tsc --noEmit`, `npx vitest run`, `npm run build`.
- Working branch: `cs-style-cars` (already created from `origin/main`). Commit after every green task.

## File Structure

- Rewrite: `src/diorama/traffic/carModels.ts` — palette, selection hashes (unchanged), loft kernel, 6 variant descriptors (body/glass builders + wheel layout), shared wheel geometry.
- Create: `src/diorama/traffic/wheelSpin.ts` — pure per-vehicle roll/steer state machine.
- Rewrite: `src/diorama/traffic/carLayer.ts` — body/glass/wheel instancing, paint/glass materials, spin integration, debug surface.
- Modify: `src/diorama/ksw/main.ts` — extend the `window.__traffic` debug hook with car-layer counters (types at `:104`, assignment at `:710`).
- Modify: `tests/traffic/carModels.test.ts` — extend for 6 variants + layouts + geometry invariants.
- Create: `tests/traffic/wheelSpin.test.ts`.
- Create: `scripts/smoke-cs-cars.mjs` — focused browser smoke (adapted from `scripts/smoke-traffic.mjs` harness).

---

### Task 1: carModels — variant descriptors, wheel layouts, palette

**Files:**
- Modify: `src/diorama/traffic/carModels.ts`
- Test: `tests/traffic/carModels.test.ts`

**Interfaces:**
- Consumes: existing `hashId`, `carColorForId`, `carVariantForId` (keep byte-identical behaviour, modulo the variant count change 3→6).
- Produces (used by Tasks 2/4):

```ts
export interface WheelLayout {
  wheelbase: number; // m, distance between axle centres
  track: number;     // m, distance between left/right wheel centres
  radius: number;    // m, wheel radius
  width: number;     // m, wheel width across the car
}
export interface CarVariant {
  name: string;
  length: number;                                  // m, overall
  wheels: WheelLayout;
  buildBody: (boxGeo: BoxGeo) => THREE.BufferGeometry;
  buildGlass: () => THREE.BufferGeometry;
}
export const CAR_VARIANTS: readonly CarVariant[]; // 6 entries, order: sedan, hatchback, wagon, suv, van, pickup
export function wheelOffsets(l: WheelLayout): [number, number, number][]; // 4 × [x, y, z] local offsets, y = radius
```

- [ ] **Step 1: Write the failing tests** — replace the header block of `tests/traffic/carModels.test.ts` imports and APPEND a new describe (keep every existing test unchanged — they must all still pass):

```ts
import {
  carColorForId,
  carVariantForId,
  CAR_PALETTE,
  CAR_VARIANTS,
  hashId,
  wheelOffsets,
} from '../../src/diorama/traffic/carModels';

describe('CS variant table', () => {
  it('has the 6 spec variants in stable order', () => {
    expect(CAR_VARIANTS.map((v) => v.name)).toEqual([
      'sedan', 'hatchback', 'wagon', 'suv', 'van', 'pickup',
    ]);
  });

  it('variant lengths match the spec table', () => {
    const byName = Object.fromEntries(CAR_VARIANTS.map((v) => [v.name, v.length]));
    expect(byName).toEqual({
      sedan: 4.5, hatchback: 3.9, wagon: 4.6, suv: 4.6, van: 5.2, pickup: 5.0,
    });
  });

  it('every wheel layout is physically sane', () => {
    for (const v of CAR_VARIANTS) {
      expect(v.wheels.wheelbase).toBeGreaterThan(1.5);
      expect(v.wheels.wheelbase).toBeLessThan(v.length); // axles inside the body
      expect(v.wheels.track).toBeGreaterThan(1.0);
      expect(v.wheels.radius).toBeGreaterThanOrEqual(0.28);
      expect(v.wheels.radius).toBeLessThanOrEqual(0.42);
    }
  });

  it('wheelOffsets puts 4 wheels at ±track/2, ±wheelbase/2, y=radius', () => {
    const l = CAR_VARIANTS[0].wheels;
    const offs = wheelOffsets(l);
    expect(offs).toHaveLength(4);
    const xs = offs.map((o) => o[0]).sort((a, b) => a - b);
    const zs = offs.map((o) => o[2]).sort((a, b) => a - b);
    expect(xs[0]).toBeCloseTo(-l.track / 2);
    expect(xs[3]).toBeCloseTo(l.track / 2);
    expect(zs[0]).toBeCloseTo(-l.wheelbase / 2);
    expect(zs[3]).toBeCloseTo(l.wheelbase / 2);
    for (const o of offs) expect(o[1]).toBeCloseTo(l.radius);
    // front pair (+z) must be listed FIRST (indices 0,1) — carLayer steers them
    expect(offs[0][2]).toBeGreaterThan(0);
    expect(offs[1][2]).toBeGreaterThan(0);
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run tests/traffic/carModels.test.ts`
Expected: FAIL — `wheelOffsets` not exported, variant table has 3 entries.

- [ ] **Step 3: Implement the data layer** in `src/diorama/traffic/carModels.ts`. Keep `hashId`, `carColorForId`, `carVariantForId`, `mergeParts`, `BoxGeo`, `Part` as-is. Replace the palette with the screenshot-tuned one, add the types and layouts, and (for THIS task only) register the 6 variants with the OLD box builders as stand-ins so tests pass before Task 2 replaces the geometry (wagon/suv/pickup temporarily alias buildSedan/buildVan shapes):

```ts
export const CAR_PALETTE: readonly number[] = [
  0xf2f0ea, // bright white
  0xd9d6cd, // pearl beige-white
  0xb9c0c6, // silver
  0x6f767d, // gunmetal
  0x24272b, // black
  0x8f2432, // maroon (screenshot dark red)
  0xc03a2b, // red
  0xb06a2c, // ochre/brown-orange (screenshot brown wagon)
  0xe0a91f, // taxi yellow (screenshot yellow hatch)
  0x2c4f8a, // dark blue (screenshot sedans)
  0x4a7ec2, // mid blue
  0x3f7d46, // green (screenshot compact)
  0x6b4a86, // purple (screenshot van)
  0xa8a08c, // beige (screenshot SUV)
] as const;

export interface WheelLayout {
  wheelbase: number;
  track: number;
  radius: number;
  width: number;
}

export function wheelOffsets(l: WheelLayout): [number, number, number][] {
  const x = l.track / 2;
  const z = l.wheelbase / 2;
  // FRONT pair first — carLayer applies steer to indices 0 and 1.
  return [
    [x, l.radius, z], [-x, l.radius, z],
    [x, l.radius, -z], [-x, l.radius, -z],
  ];
}

export interface CarVariant {
  name: string;
  length: number;
  wheels: WheelLayout;
  buildBody: (boxGeo: BoxGeo) => THREE.BufferGeometry;
  buildGlass: () => THREE.BufferGeometry;
}

const W = 1.82; // body width (kept < 1.9 so vans/pickups can go wider)

export const CAR_VARIANTS: readonly CarVariant[] = [
  { name: 'sedan',     length: 4.5, wheels: { wheelbase: 2.7, track: 1.56, radius: 0.31, width: 0.24 }, buildBody: buildSedan,     buildGlass: stubGlass },
  { name: 'hatchback', length: 3.9, wheels: { wheelbase: 2.5, track: 1.52, radius: 0.30, width: 0.22 }, buildBody: buildHatchback, buildGlass: stubGlass },
  { name: 'wagon',     length: 4.6, wheels: { wheelbase: 2.8, track: 1.56, radius: 0.31, width: 0.24 }, buildBody: buildSedan,     buildGlass: stubGlass },
  { name: 'suv',       length: 4.6, wheels: { wheelbase: 2.8, track: 1.62, radius: 0.38, width: 0.28 }, buildBody: buildVan,       buildGlass: stubGlass },
  { name: 'van',       length: 5.2, wheels: { wheelbase: 3.3, track: 1.66, radius: 0.36, width: 0.26 }, buildBody: buildVan,       buildGlass: stubGlass },
  { name: 'pickup',    length: 5.0, wheels: { wheelbase: 3.1, track: 1.62, radius: 0.38, width: 0.28 }, buildBody: buildVan,       buildGlass: stubGlass },
] as const;

function stubGlass(): THREE.BufferGeometry {
  return new THREE.BoxGeometry(1, 0.3, 1.5); // replaced in Task 2
}
```

Note: the old `CAR_VARIANTS` entries had `{ name, build }`; the table is now `CarVariant` — update the two call sites in `carLayer.ts` mechanically (`variant.build(boxGeo)` → `variant.buildBody(boxGeo)`) so typecheck stays green; the real carLayer rewrite is Task 4.

- [ ] **Step 4: Run the full test file + typecheck**

Run: `npx vitest run tests/traffic/carModels.test.ts && npx tsc --noEmit`
Expected: PASS (all old selection tests still green — 6 variants keep the hash valid) and clean typecheck.

- [ ] **Step 5: Commit**

```bash
git add src/diorama/traffic/carModels.ts src/diorama/traffic/carLayer.ts tests/traffic/carModels.test.ts
git commit -m "feat(traffic): 6 CS car variants — data layer, wheel layouts, screenshot palette"
```

---

### Task 2: carModels — loft geometry kernel + real CS silhouettes

**Files:**
- Modify: `src/diorama/traffic/carModels.ts`
- Test: `tests/traffic/carModels.test.ts`

**Interfaces:**
- Consumes: `mergeParts`, `Part`, `BoxGeo`, colours from Task 1.
- Produces: real `buildBody`/`buildGlass` per variant; `buildWheelGeometry(): THREE.BufferGeometry` (unit-ish wheel, radius `WHEEL_GEO_RADIUS = 0.3`, axis along x, vertex-coloured tire/rim — Task 4 scales instances by `layout.radius / WHEEL_GEO_RADIUS`); `export const WHEEL_GEO_RADIUS = 0.3`.

**Geometry kernel.** A car body is a **loft**: an ordered list of cross-sections swept along z. Each section is a trapezoid `{ z, yBot, yTop, wBot, wTop }` (bottom edge at `yBot` width `wBot`, top edge at `yTop` width `wTop`). Consecutive sections are stitched with two triangles per face strip (left, right, top, bottom), plus end caps. Non-indexed triangles + `computeVertexNormals()` → crisp flat-shaded low-poly panels (the CS look). Every loft face gets a single uniform vertex colour passed per call.

- [ ] **Step 1: Write the failing tests** — append to `tests/traffic/carModels.test.ts`:

```ts
import { buildWheelGeometry, WHEEL_GEO_RADIUS } from '../../src/diorama/traffic/carModels';
import { boxGeo } from '../../src/diorama/ksw/geometryCache';

describe('CS geometry builders', () => {
  it('every variant builds non-empty body + glass with colour attributes', () => {
    for (const v of CAR_VARIANTS) {
      const body = v.buildBody(boxGeo);
      const glass = v.buildGlass();
      for (const g of [body, glass]) {
        expect(g.attributes.position.count).toBeGreaterThan(24); // more than one box
        expect(g.attributes.color).toBeDefined();
        expect(g.boundingSphere).not.toBeNull();
      }
      // body spans the declared length along z (±3% tolerance)
      body.computeBoundingBox();
      const bb = body.boundingBox!;
      expect(bb.max.z - bb.min.z).toBeGreaterThan(v.length * 0.97);
      expect(bb.max.z - bb.min.z).toBeLessThan(v.length * 1.03);
      // body underside clears the ground (wheels live below it): min y ≥ 0.25
      expect(bb.min.y).toBeGreaterThanOrEqual(0.25);
      // glass sits above the beltline, inside the body footprint
      glass.computeBoundingBox();
      expect(glass.boundingBox!.min.y).toBeGreaterThan(0.8);
    }
  });

  it('body bakes non-white detail zones (grille/lights/plates present)', () => {
    const body = CAR_VARIANTS[0].buildBody(boxGeo);
    const col = body.attributes.color;
    let nonWhite = 0;
    for (let i = 0; i < col.count; i++) {
      if (col.getX(i) < 0.95 || col.getY(i) < 0.95 || col.getZ(i) < 0.95) nonWhite++;
    }
    expect(nonWhite).toBeGreaterThan(20); // grille + 2 headlights + 2 taillights + 2 plates
    expect(nonWhite).toBeLessThan(col.count / 2); // …but the body is mostly tintable white
  });

  it('wheel geometry: cylinder about the x axis at the shared geo radius', () => {
    const wheel = buildWheelGeometry();
    wheel.computeBoundingBox();
    const bb = wheel.boundingBox!;
    expect(bb.max.y).toBeCloseTo(WHEEL_GEO_RADIUS, 2);
    expect(bb.min.y).toBeCloseTo(-WHEEL_GEO_RADIUS, 2);
    expect(bb.max.z).toBeCloseTo(WHEEL_GEO_RADIUS, 2);
    expect(bb.max.x).toBeLessThan(WHEEL_GEO_RADIUS); // width < diameter → axis is x
    expect(wheel.attributes.color).toBeDefined();
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run tests/traffic/carModels.test.ts`
Expected: FAIL — `buildWheelGeometry` not exported; stub glass has no colour attribute.

- [ ] **Step 3: Implement the kernel + variants.** Add to `carModels.ts` (new colour constants replace the old `GLASS` value — CS glass is BRIGHT):

```ts
/** CS glass: bright reflective sky-blue (baked into the glass geometry AND
 * multiplied by the glass material colour — see carLayer). */
const GLASS = new THREE.Color(0xbfe0f2);
const RUBBER = new THREE.Color(0x17191c);
const RIM = new THREE.Color(0xb7bcc2);
const GRILLE = new THREE.Color(0x1d2126);
const HEADLIGHT = new THREE.Color(0xfff4d0);
const TAILLIGHT = new THREE.Color(0xb01a1a);
const PLATE = new THREE.Color(0xf5f5f0);

interface Section { z: number; yBot: number; yTop: number; wBot: number; wTop: number }

/** Sweep trapezoid cross-sections along z into a flat-shaded, non-indexed,
 * uniformly vertex-coloured hull (left/right/top/bottom strips + end caps). */
function loft(sections: Section[], color: THREE.Color): THREE.BufferGeometry {
  const pos: number[] = [];
  const quad = (a: number[], b: number[], c: number[], d: number[]) =>
    pos.push(...a, ...b, ...c, ...a, ...c, ...d);
  const corners = (s: Section) => ({
    bl: [-s.wBot / 2, s.yBot, s.z], br: [s.wBot / 2, s.yBot, s.z],
    tl: [-s.wTop / 2, s.yTop, s.z], tr: [s.wTop / 2, s.yTop, s.z],
  });
  for (let i = 0; i < sections.length - 1; i++) {
    const a = corners(sections[i]);
    const b = corners(sections[i + 1]);
    quad(a.br, b.br, b.tr, a.tr); // right (+x)
    quad(b.bl, a.bl, a.tl, b.tl); // left (−x)
    quad(a.tr, b.tr, b.tl, a.tl); // top
    quad(a.bl, b.bl, b.br, a.br); // bottom
  }
  const first = corners(sections[0]);
  const last = corners(sections[sections.length - 1]);
  quad(first.bl, first.br, first.tr, first.tl); // front cap (−z end listed first)
  quad(last.br, last.bl, last.tl, last.tr);     // rear cap
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pos), 3));
  const n = g.attributes.position.count;
  const colors = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) colors.set([color.r, color.g, color.b], i * 3);
  g.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  g.computeVertexNormals();
  g.computeBoundingSphere();
  return g;
}
```

Body recipe (shared helper — each variant differs only in its profile numbers). `+z` is FORWARD (matches the old models / pose yaw). Underside at `UNDERBODY = 0.30` (bodies float; wheels fill the gap). A body =

1. `loft(hullSections, BODY)` — nose → windshield base → roof start … tail, beltline ~`0.95`, hood/trunk heights per variant, bumper sections slightly narrower (`wBot − 0.12`) for the rounded look;
2. detail `Part[]` boxes merged via the existing `mergeParts` on top: grille `{w: W*0.55, h: 0.16, d: 0.06}` at the nose, headlights 2× `{w: 0.28, h: 0.12, d: 0.06}` (HEADLIGHT), taillights 2× (TAILLIGHT), plates front+rear `{w: 0.5, h: 0.13, d: 0.04}` (PLATE) — all protruding 0.03 beyond the cap face so they read at distance;
3. `mergeGeometries([hull, detailBoxes…])`.

Glass = `loft(greenhouseSections, GLASS)` — windshield base to rear-glass base, `wTop = wBot − 0.28` (pillar taper), raised `0.015` above the roofline sections and inset `0.02` narrower than the body so it never z-fights; front section raked (windshield slope) by giving the first glass section a larger `z` at `yTop` — express the rake by consecutive sections (`z` shifts between sections do this naturally).

Complete sedan implementation (the other five follow the same recipe with the numbers below):

```ts
const UNDERBODY = 0.30;

function buildSedan(boxGeo: BoxGeo): THREE.BufferGeometry {
  const L = 4.5, half = L / 2, belt = 0.95, roof = 1.38;
  const hull = loft([
    { z:  half,        yBot: UNDERBODY + 0.10, yTop: UNDERBODY + 0.42, wBot: W - 0.30, wTop: W - 0.34 }, // bumper lip
    { z:  half - 0.28, yBot: UNDERBODY,        yTop: belt - 0.12,      wBot: W - 0.06, wTop: W - 0.10 }, // nose
    { z:  half - 1.30, yBot: UNDERBODY,        yTop: belt,             wBot: W,        wTop: W - 0.06 }, // hood end / windshield base
    { z:  half - 2.05, yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // windshield top / roof front
    { z: -half + 1.55, yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // roof rear
    { z: -half + 0.95, yBot: UNDERBODY,        yTop: belt + 0.06,      wBot: W,        wTop: W - 0.10 }, // trunk lid
    { z: -half + 0.26, yBot: UNDERBODY,        yTop: belt - 0.10,      wBot: W - 0.06, wTop: W - 0.12 }, // tail
    { z: -half,        yBot: UNDERBODY + 0.10, yTop: UNDERBODY + 0.40, wBot: W - 0.30, wTop: W - 0.36 }, // rear bumper lip
  ], BODY);
  return finishMerged(hull, detailBoxes(boxGeo, L));
}
```

with the shared merge helper:

```ts
function finishMerged(...parts: THREE.BufferGeometry[]): THREE.BufferGeometry {
  const prepared = parts.map((p) => (p.index ? p.toNonIndexed() : p));
  const merged = mergeGeometries(prepared, false);
  if (!merged) throw new Error('carModels: hull merge failed');
  merged.computeVertexNormals();
  merged.computeBoundingSphere();
  return merged;
}
```

(`mergeGeometries` requires all-indexed or all-non-indexed inputs; the loft is non-indexed, `mergeParts` output is indexed — normalise via `toNonIndexed()`.) `detailBoxes(boxGeo, length)` builds the grille/light/plate `Part[]` with `mergeParts` at `±length/2` faces, heights centred at `UNDERBODY + 0.28` (plates/grille) and `belt − 0.18` (lights).

Glass (sedan):

```ts
function sedanGlass(): THREE.BufferGeometry {
  const half = 4.5 / 2, belt = 0.95, roof = 1.38;
  return loft([
    { z:  half - 1.32, yBot: belt, yTop: belt + 0.02, wBot: W - 0.10, wTop: W - 0.36 }, // windshield base
    { z:  half - 2.03, yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // windshield top
    { z: -half + 1.57, yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // rear roof
    { z: -half + 0.97, yBot: belt, yTop: belt + 0.02,  wBot: W - 0.10, wTop: W - 0.38 }, // rear glass base
  ], GLASS);
}
```

Variant profile numbers (same section pattern; only the values change):

| variant | belt | roof | character deltas |
|---|---|---|---|
| sedan | 0.95 | 1.38 | as above |
| hatchback | 0.95 | 1.42 | no trunk sections — rear glass slopes straight to the tail (drop the "trunk lid" section; tail section `yTop: 1.05`) |
| wagon | 0.95 | 1.42 | roof-rear section at `z: -half + 0.55` (long roof), short steep tailgate |
| suv | 1.10 | 1.68 | `W + 0.06` wide, `UNDERBODY = 0.42`, upright nose (hood end `yTop = belt` at `half − 1.05`) |
| van | 1.15 | 2.05 | `W + 0.10` wide, `UNDERBODY = 0.34`, short nose (`half − 0.85` windshield base), roof runs to `z: -half + 0.30`, near-vertical tail; glass = windshield + front-door band only (glass loft ends at `z ≈ half − 2.6`) |
| pickup | 1.05 | 1.62 | `UNDERBODY = 0.42`; cab like suv but roof ends at `z ≈ 0.2`; open bed = body loft continues at `yTop = belt` with a second inner loft (RUBBER colour, `wBot = W − 0.3`) from `z ≈ 0` to the tail giving the bed cavity look |

Wheel geometry:

```ts
export const WHEEL_GEO_RADIUS = 0.3;

export function buildWheelGeometry(): THREE.BufferGeometry {
  const r = WHEEL_GEO_RADIUS;
  const tire = new THREE.CylinderGeometry(r, r, 0.22, 12);
  tire.rotateZ(Math.PI / 2); // cylinder axis y → x (axle across the car)
  const rim = new THREE.CylinderGeometry(r * 0.55, r * 0.55, 0.235, 8);
  rim.rotateZ(Math.PI / 2);
  const paint = (g: THREE.BufferGeometry, c: THREE.Color) => {
    const n = g.attributes.position.count;
    const colors = new Float32Array(n * 3);
    for (let i = 0; i < n; i++) colors.set([c.r, c.g, c.b], i * 3);
    g.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    return g;
  };
  return finishMerged(paint(tire, RUBBER), paint(rim, RIM));
}
```

Delete the now-unused old builders (`buildSedan` box version, `buildHatchback`, `buildVan`, `wheels()`, `ROOF`, `WHEEL_LINE`, `BODY_W`) and `stubGlass`. Keep `mergeParts` (detail boxes use it).

- [ ] **Step 4: Run tests + typecheck**

Run: `npx vitest run tests/traffic/carModels.test.ts && npx tsc --noEmit`
Expected: PASS. If a bounding assertion fails, adjust the profile numbers — the tests encode the spec (length ±3 %, underside ≥ 0.25, glass above beltline), not the other way round.

- [ ] **Step 5: Commit**

```bash
git add src/diorama/traffic/carModels.ts tests/traffic/carModels.test.ts
git commit -m "feat(traffic): loft-based CS silhouettes — 6 bodies, glass shells, cylinder wheel"
```

---

### Task 3: wheelSpin — pure roll + steer state

**Files:**
- Create: `src/diorama/traffic/wheelSpin.ts`
- Test: `tests/traffic/wheelSpin.test.ts`

**Interfaces:**
- Consumes: `SIM_DT` from `./deadReckon`.
- Produces (used by Task 4):

```ts
export interface SpinState { theta: number; steer: number; lastTick: number; lastYaw: number }
export function initSpin(nowTick: number, yaw: number): SpinState;
/** Mutates and returns `st` (no allocation on the hot path). */
export function advanceSpin(st: SpinState, v: number, yaw: number, nowTick: number, wheelRadius: number): SpinState;
export const MAX_STEER = 0.45; // rad
```

- [ ] **Step 1: Write the failing tests** — `tests/traffic/wheelSpin.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { advanceSpin, initSpin, MAX_STEER } from '../../src/diorama/traffic/wheelSpin';
import { SIM_DT } from '../../src/diorama/traffic/deadReckon';

describe('wheelSpin', () => {
  it('rolls theta by v·dt/r', () => {
    const st = initSpin(100, 0);
    advanceSpin(st, 10, 0, 110, 0.31); // 10 ticks → dt = 1 s
    expect(st.theta).toBeCloseTo((10 * 10 * SIM_DT) / 0.31);
    expect(st.lastTick).toBe(110);
  });

  it('accumulates across calls and rolls backward for v < 0', () => {
    const st = initSpin(0, 0);
    advanceSpin(st, 5, 0, 10, 0.3);
    const t1 = st.theta;
    advanceSpin(st, -5, 0, 20, 0.3);
    expect(st.theta).toBeCloseTo(0);
    expect(t1).toBeGreaterThan(0);
  });

  it('ignores non-positive dt (tick replay) without NaN', () => {
    const st = initSpin(50, 0);
    advanceSpin(st, 10, 0, 50, 0.3); // dt = 0
    advanceSpin(st, 10, 0, 40, 0.3); // dt < 0
    expect(st.theta).toBe(0);
    expect(Number.isFinite(st.theta)).toBe(true);
  });

  it('steers toward yaw change, clamped to ±MAX_STEER, and decays straight', () => {
    const st = initSpin(0, 0);
    advanceSpin(st, 10, 0.5, 1, 0.3); // hard yaw step in one tick
    expect(st.steer).toBeGreaterThan(0);
    expect(st.steer).toBeLessThanOrEqual(MAX_STEER);
    for (let t = 2; t < 40; t++) advanceSpin(st, 10, 0.5, t, 0.3); // yaw now constant
    expect(Math.abs(st.steer)).toBeLessThan(0.02); // filtered back to straight
  });

  it('handles yaw wrap (π → −π is a small left turn, not a full spin)', () => {
    const st = initSpin(0, Math.PI - 0.01);
    advanceSpin(st, 10, -Math.PI + 0.01, 1, 0.3);
    expect(Math.abs(st.steer)).toBeLessThanOrEqual(MAX_STEER);
    expect(Math.abs(st.steer)).toBeLessThan(0.2); // Δyaw was only 0.02 rad
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run tests/traffic/wheelSpin.test.ts`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement** `src/diorama/traffic/wheelSpin.ts`:

```ts
// Pure per-vehicle wheel state: roll angle from dead-reckoned speed and a
// low-pass-filtered front steer angle from the yaw rate. carLayer keeps one
// SpinState per vehicle id and mutates it in place each frame (no allocation).

import { SIM_DT } from './deadReckon';

export interface SpinState { theta: number; steer: number; lastTick: number; lastYaw: number }

export const MAX_STEER = 0.45;
/** Steer gain: full-lock (MAX_STEER) at a yaw rate of ~1 rad/s. */
const STEER_GAIN = 0.45;
/** Low-pass rate (1/s): how fast steer chases its target. */
const STEER_LP = 8;

export function initSpin(nowTick: number, yaw: number): SpinState {
  return { theta: 0, steer: 0, lastTick: nowTick, lastYaw: yaw };
}

function wrapAngle(a: number): number {
  while (a > Math.PI) a -= 2 * Math.PI;
  while (a < -Math.PI) a += 2 * Math.PI;
  return a;
}

export function advanceSpin(st: SpinState, v: number, yaw: number, nowTick: number, wheelRadius: number): SpinState {
  const dt = (nowTick - st.lastTick) * SIM_DT;
  if (dt > 0) {
    st.theta += (v * dt) / wheelRadius;
    const yawRate = wrapAngle(yaw - st.lastYaw) / dt;
    const target = Math.max(-MAX_STEER, Math.min(MAX_STEER, yawRate * STEER_GAIN));
    st.steer += (target - st.steer) * Math.min(1, dt * STEER_LP);
    st.lastTick = nowTick;
    st.lastYaw = yaw;
  }
  return st;
}
```

- [ ] **Step 4: Run tests**

Run: `npx vitest run tests/traffic/wheelSpin.test.ts`
Expected: PASS (5/5).

- [ ] **Step 5: Commit**

```bash
git add src/diorama/traffic/wheelSpin.ts tests/traffic/wheelSpin.test.ts
git commit -m "feat(traffic): pure wheel roll + filtered front-steer state"
```

---

### Task 4: carLayer — body/glass/wheel instancing + materials + debug hook

**Files:**
- Rewrite: `src/diorama/traffic/carLayer.ts`
- Modify: `src/diorama/ksw/main.ts:104-120` (types) and `:710-722` (hook assignment)

**Interfaces:**
- Consumes: `CAR_VARIANTS` (`CarVariant` with `buildBody`/`buildGlass`/`wheels`), `wheelOffsets`, `buildWheelGeometry`, `WHEEL_GEO_RADIUS`, `carColorForId`, `carVariantForId` (Tasks 1-2); `initSpin`/`advanceSpin` (Task 3); `poseAtBlended`, `kswCity.roadYs.carriage`, `boxGeo` (existing).
- Produces: `CarLayer` interface gains `debug: { variantCounts(): number[]; wheelCount(): number; wheelMatrix(i: number): number[] }`. `buildCarGeometry()` unchanged signature (Task 5 reworks its content). `CAR_CAPACITY`/`CAR_PALETTE` re-exports unchanged.

Structure of the rewritten `createCarLayer(groundYAt?)` (keep the module banner style, update the FIX D2 text to describe the CS look):

- Materials:

```ts
function paintMaterial(): THREE.MeshPhysicalMaterial {
  const m = new THREE.MeshPhysicalMaterial({
    color: 0xffffff, vertexColors: true,
    roughness: 0.45, metalness: 0.1,
  });
  m.clearcoat = 1.0;
  m.clearcoatRoughness = 0.15;
  return m;
}
function glassMaterial(): THREE.MeshPhysicalMaterial {
  const m = new THREE.MeshPhysicalMaterial({
    color: 0xffffff, vertexColors: true, // baked light-blue vertex colour carries the tint
    roughness: 0.05, metalness: 0.4,
  });
  m.envMapIntensity = 1.6;
  return m;
}
```

- Meshes: per variant a body `InstancedMesh(variant.buildBody(boxGeo), paint, PER_VARIANT_CAPACITY)` (name `trafficCars_${name}`) AND a glass `InstancedMesh(variant.buildGlass(), glass, PER_VARIANT_CAPACITY)` (name `trafficCarsGlass_${name}`; no `instanceColor` — glass is uniform); ONE `wheelMesh = InstancedMesh(buildWheelGeometry(), paint, 4 * CAR_CAPACITY)` (name `trafficWheels`; instanceColor filled once with white). All: `DynamicDrawUsage`, `castShadow`/`receiveShadow` true (glass: `castShadow = false`), `frustumCulled = false`, `count = 0`.
- `PER_VARIANT_CAPACITY = 2048` stays (6 variants × 2048 ≥ any AOI); wheel capacity `4 * CAR_CAPACITY = 16384`.
- Update loop — per vehicle (existing variant/colour assignment maps unchanged):

```ts
const pose = poseAtBlended(net, veh, nowTick);
const groundY = groundYAt ? groundYAt(pose.x, pose.z) : 0;
pos.set(pose.x, groundY + surfaceOffset, pose.z);
quat.setFromAxisAngle(up, pose.yaw);
bodyMat.compose(pos, quat, scl);
bodyMeshes[variant].setMatrixAt(i, bodyMat);
glassMeshes[variant].setMatrixAt(i, bodyMat);
// body tint exactly as today (setColorAt on the body mesh only)

const layout = CAR_VARIANTS[variant].wheels;
let spin = spinOfId.get(id);
if (spin === undefined) { spin = initSpin(nowTick, pose.yaw); spinOfId.set(id, spin); }
advanceSpin(spin, veh.v, pose.yaw, nowTick, layout.radius);
const s = layout.radius / WHEEL_GEO_RADIUS;
for (let w = 0; w < 4; w++) {
  const off = offsets[variant][w]; // precomputed wheelOffsets per variant at layer build
  wpos.set(off[0], off[1], off[2]);
  weuler.set(spin.theta, w < 2 ? spin.steer : 0, 0, 'YXZ'); // steer about y THEN roll about x
  wquat.setFromEuler(weuler);
  wscl.setScalar(s);
  wheelMat.compose(wpos, wquat, wscl).premultiply(bodyMat);
  wheelMesh.setMatrixAt(wheelCursor++, wheelMat);
}
```

  Scratch objects (`bodyMat`, `wheelMat`, `wpos`, `wquat`, `wscl`, `weuler`, …) hoisted like today; `wheelCursor` reset per frame; after the loop `wheelMesh.count = wheelCursor`, body+glass counts from the per-variant cursors, `needsUpdate` on all instance matrices + body instanceColors.
- Extend the pruning sweep to `spinOfId`.
- `debug` on the returned `CarLayer`:

```ts
debug: {
  variantCounts: () => bodyMeshes.map((m) => m.count),
  wheelCount: () => wheelMesh.count,
  wheelMatrix: (i: number) => {
    const m = new THREE.Matrix4();
    wheelMesh.getMatrixAt(Math.max(0, Math.min(wheelMesh.count - 1, i)), m);
    return Array.from(m.elements); // JSON-safe for the CDP smoke
  },
},
```

- `src/diorama/ksw/main.ts`: inside the `window.__traffic = { … }` assignment add `cars: () => carLayer?.debug.variantCounts() ?? [],`, `wheels: () => carLayer?.debug.wheelCount() ?? 0,` and `wheelMatrix: (i: number) => carLayer?.debug.wheelMatrix(i) ?? null,`; mirror the three signatures in the `__traffic?:` type block at `:104`.

- [ ] **Step 1: Write the failing test** — append to `tests/traffic/carModels.test.ts` (carLayer itself needs a scene; test the pure wheel-matrix composition instead by exercising `createCarLayer` headlessly — three.js InstancedMesh works without a renderer):

```ts
import { createCarLayer, CAR_CAPACITY } from '../../src/diorama/traffic/carLayer';
import { buildLaneNet, type VehKinematics } from '../../src/diorama/traffic/deadReckon';

describe('carLayer instancing', () => {
  const net = buildLaneNet([
    { id: 0, edge: 0, index: 0, lengthM: 100, pts: [[0, 0], [100, 0]] },
  ]);
  const vehicles = new Map<number, VehKinematics>([
    [1, { lane: 0, s: 10, v: 10, tickAt: 0 }],
    [2, { lane: 0, s: 30, v: 0, tickAt: 0 }],
  ]);

  it('draws one body+glass pair per vehicle and 4 wheels each', () => {
    const layer = createCarLayer();
    layer.update(net, vehicles, 0);
    expect(layer.debug.variantCounts().reduce((a, b) => a + b, 0)).toBe(2);
    expect(layer.debug.wheelCount()).toBe(8);
  });

  it('rotates wheels of a MOVING vehicle between frames, keeps parked wheels still', () => {
    const layer = createCarLayer();
    layer.update(net, vehicles, 0);
    const m0 = layer.debug.wheelMatrix(0); // belongs to id 1 (v=10) — insertion order
    const p0 = layer.debug.wheelMatrix(4); // id 2 (v=0)
    layer.update(net, vehicles, 10); // +1 s
    const m1 = layer.debug.wheelMatrix(0);
    const p1 = layer.debug.wheelMatrix(4);
    // rotation part must change for the mover…
    const rotDelta = (a: number[], b: number[]) =>
      Math.abs(a[5] - b[5]) + Math.abs(a[6] - b[6]) + Math.abs(a[9] - b[9]) + Math.abs(a[10] - b[10]);
    expect(rotDelta(m0, m1)).toBeGreaterThan(0.05);
    // …its position advances along the lane…
    expect(m1[14] - m0[14]).toBeCloseTo(0, 1); // z stays (lane runs along +x here)
    expect(m1[12] - m0[12]).toBeCloseTo(10, 0); // x advances ~10 m
    // …and the parked car's wheels do not spin
    expect(rotDelta(p0, p1)).toBeLessThan(1e-6);
  });

  it('exposes capacity for the wheel mesh at 4× CAR_CAPACITY', () => {
    expect(CAR_CAPACITY).toBe(4096);
  });
});
```

Caveat for the test author: wheel slot order within a frame is insertion order of the `vehicles` Map × 4 wheels — id 1 owns slots 0-3, id 2 owns slots 4-7. If `carVariantForId(1) === carVariantForId(2)` the body cursors collide into one mesh; the wheel cursor is global and unaffected — the assertions above only rely on the global wheel order.

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run tests/traffic/carModels.test.ts`
Expected: FAIL — `layer.debug` undefined (old layer), wheel mesh absent.

- [ ] **Step 3: Rewrite `carLayer.ts`** per the structure above (keep `CAR_LIFT`, `surfaceOffset`, `GroundYAt`, capacity constants, the colour/variant maps, and the pruning sweep; `buildCarGeometry` keeps returning `CAR_VARIANTS[0].buildBody(boxGeo)` for now — Task 5 upgrades it). Then wire the three debug functions into `src/diorama/ksw/main.ts` (type block AND assignment).

- [ ] **Step 4: Run the full frontend gate**

Run: `npx vitest run && npx tsc --noEmit`
Expected: PASS everywhere (flowLayer still compiles against `buildCarGeometry`).

- [ ] **Step 5: Commit**

```bash
git add src/diorama/traffic/carLayer.ts src/diorama/ksw/main.ts tests/traffic/carModels.test.ts
git commit -m "feat(traffic): CS car layer — body/glass/wheel instancing, clearcoat paint, spinning+steering wheels"
```

---

### Task 5: far-LOD impostor geometry

**Files:**
- Modify: `src/diorama/traffic/carLayer.ts` (`buildCarGeometry` body only)
- Test: `tests/traffic/carModels.test.ts`

**Interfaces:**
- Consumes: `CAR_VARIANTS[0]` (sedan) builders, `buildWheelGeometry`, `wheelOffsets`, `WHEEL_GEO_RADIUS`.
- Produces: `buildCarGeometry(): THREE.BufferGeometry` — one merged static geometry (body + glass + 4 wheels) for `flowLayer.ts:202`.

- [ ] **Step 1: Write the failing test** — append:

```ts
import { buildCarGeometry } from '../../src/diorama/traffic/carLayer';

describe('far-LOD impostor geometry', () => {
  it('is a single merged geometry: sedan body + glass + 4 static wheels', () => {
    const g = buildCarGeometry();
    const sedan = CAR_VARIANTS[0];
    const bodyOnly = sedan.buildBody(boxGeo);
    expect(g.attributes.position.count).toBeGreaterThan(
      bodyOnly.attributes.position.count + 4 * 24, // strictly more than body + trivial wheels
    );
    g.computeBoundingBox();
    expect(g.boundingBox!.min.y).toBeLessThan(0.1); // wheels reach (near) the ground
    expect(g.attributes.color).toBeDefined();
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run tests/traffic/carModels.test.ts`
Expected: FAIL — current impostor is body-only (min y ≈ UNDERBODY = 0.30).

- [ ] **Step 3: Implement** — in `carLayer.ts`:

```ts
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
```

(import `mergeGeometries` from `three/addons/utils/BufferGeometryUtils.js` in carLayer, plus the new carModels exports).

- [ ] **Step 4: Run tests + typecheck**

Run: `npx vitest run && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/diorama/traffic/carLayer.ts tests/traffic/carModels.test.ts
git commit -m "feat(traffic): far-LOD impostor = merged sedan body+glass+wheels"
```

---

### Task 6: browser smoke — CS cars render, wheels turn

**Files:**
- Create: `scripts/smoke-cs-cars.mjs`
- Reference harness: `scripts/smoke-traffic.mjs`, `scripts/lib/traffic-stack.mjs`

**Interfaces:**
- Consumes: `window.__traffic.{count, cars, wheels, wheelMatrix, lookAt}` (Task 4), `window.__LOOK_READY` gate, `startTrafficStack`/`HOST` from `./lib/traffic-stack.mjs`.
- Produces: a pass/fail smoke + screenshot at `scratch/cs-cars-smoke.png` (gitignored path; verify `scratch/` is ignored, else use the session scratchpad).

- [ ] **Step 1: Pre-flight — world bake.** Check `ls -l public/winterthur-world` resolves; if missing run `npm run geo:fetch && npm run geo:bake-world` and re-link per the repo's geo README (see memory: fresh worktree boot hangs at `__LOOK_READY` without it).

- [ ] **Step 2: Write `scripts/smoke-cs-cars.mjs`.** Copy the launch/boot skeleton from `smoke-traffic.mjs` (chromium WebGPU flags, `startTrafficStack`, `ksw.html?traffic=1`, `__LOOK_READY` wait). Then assert:

```js
// (a) vehicles stream in
await page.waitForFunction(() => window.__traffic && window.__traffic.count() > 0, null, { timeout: 60_000 });

// frame a busy corridor (same coordinates smoke-traffic.mjs uses for its density scan)
await page.evaluate(() => window.__traffic.lookAt(/* corridor x, z from smoke-traffic.mjs */));
await page.waitForTimeout(2000);

// (b) CS meshes draw: ≥1 variant mesh has instances, wheels = 4 × bodies
const counts = await page.evaluate(() => window.__traffic.cars());
const wheels = await page.evaluate(() => window.__traffic.wheels());
const bodies = counts.reduce((a, b) => a + b, 0);
assert(bodies > 0, `no car bodies drawn (${JSON.stringify(counts)})`);
assert(wheels === 4 * bodies, `wheel count ${wheels} !== 4×${bodies}`);

// (c) variant diversity: a busy view shows ≥3 of the 6 silhouettes
assert(counts.filter((c) => c > 0).length >= 3, `too few variants live: ${JSON.stringify(counts)}`);

// (d) WHEELS ACTUALLY TURN: sample one wheel matrix twice 1 s apart; the
// rotation block (elements 5,6,9,10) must change while traffic flows.
const pick = await page.evaluate(() => {
  // find a wheel belonging to a moving vehicle: sample() gives poses; just use slot 0
  return { a: window.__traffic.wheelMatrix(0) };
});
await page.waitForTimeout(1000);
const b = await page.evaluate(() => window.__traffic.wheelMatrix(0));
const rotDelta = [5, 6, 9, 10].reduce((s, k) => s + Math.abs(pick.a[k] - b[k]), 0);
assert(rotDelta > 0.01, `wheel rotation did not change (Δ=${rotDelta})`);

// (e) screenshot for visual comparison with research/Download.jpeg
await page.screenshot({ path: 'scratch/cs-cars-smoke.png' });
```

Caveat: slot 0 can belong to a car that happens to be parked at a red light for the whole second; make (d) robust by sampling slots `0, 4, 8, …, 40` and requiring **at least one** to exceed the delta (the corridor is a through-road; a full simultaneous standstill of 10 sampled cars for 1 s does not happen at rush demand).

- [ ] **Step 3: Run the smoke**

Run: `node scripts/smoke-cs-cars.mjs`
Expected: all assertions pass; console prints the variant histogram; screenshot written.

- [ ] **Step 4: Look at the screenshot** (Read `scratch/cs-cars-smoke.png`) and compare against `research/Download.jpeg`: rounded silhouettes? bright glass? visible wheels? saturated palette? If the look is off (e.g. glass too dark, bodies too slab-sided), tune the profile numbers / materials in `carModels.ts`/`carLayer.ts` and re-run from Step 3. This visual gate is part of the task, not optional polish.

- [ ] **Step 5: Commit**

```bash
git add scripts/smoke-cs-cars.mjs
git commit -m "test(traffic): browser smoke — CS cars draw, all-variant histogram, wheels rotate"
```

---

### Task 7: full CI gate + finish

**Files:** none new.

- [ ] **Step 1: Full frontend gate**

Run: `npx tsc --noEmit && npx vitest run && npm run build`
Expected: all green (build uses `scripts/build.mjs` wrapper — do not bypass it).

- [ ] **Step 2: Re-run the browser smoke once more on the final tree**

Run: `node scripts/smoke-cs-cars.mjs`
Expected: PASS.

- [ ] **Step 3: Finish the branch** — invoke `superpowers:finishing-a-development-branch`; deliver via PR to `main` per repo convention (worktree → PR → origin; wait for ALL checks green before merge, never on UNSTABLE).
