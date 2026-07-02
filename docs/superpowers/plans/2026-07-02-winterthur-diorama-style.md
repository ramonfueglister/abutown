# Winterthur Diorama-Stil Implementation Plan (Fassaden, Dächer, Strassen, Licht, Perf)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Die geodätische Winterthur-Stadt bekommt den Stil, das Licht und die Ruhe des Original-Hero-Dioramas — plus Behebung von Ruckeln, Schwebe-Geometrie, Dach- und Strassen-Mängeln.

**Architecture:** Bake-seitig (`scripts/geo/lib/style.mjs`) werden Hüllen-Gate, Dachdicke/Giebel-Skirts, Baum-Spezies/Waldfüllung, Strassenbreiten aus Tags und Tür-Platzierung berechnet und in `data/winterthur/*.json` geschrieben. Runtime-seitig (`src/diorama/ksw/geo/`) entstehen Sockel/Trim, Fenster-/Tür-/Laternen-Instanzen, Strassen-Miter, Original-Baumform, ein 3-Ring-LOD-Manager und die Licht-Erweiterungen (kamerafolgendes Schatten-Frustum, 2-Layer-Wolken, Stadt-Mist). Spec: `docs/superpowers/specs/2026-07-02-winterthur-diorama-style-design.md`.

**Tech Stack:** Node .mjs Bake-Libs (getestet via vitest), three.js WebGPU (`three/webgpu`), TSL nur wo das Original es schon nutzt, InstancedMesh, vorhandene Clay-Builder.

## Global Constraints

- **Hero pixel-treu:** bestehende Werte in `designTokens.ts`, `look.ts`, `staticBatch.ts`, `clayNodes.ts` NICHT ändern — nur additive Token-Einträge. Für `radius ≤ 120` müssen Schatten/Wolken/Mist exakt heutiges Verhalten zeigen.
- **Geodäsie:** Stil-Schicht = deterministische Funktion echter Form (Footprint/Höhe/Dachfläche/Polylines/OSM-Tags). Deklarierte Defaults nur wie in der Spec (Baum ±15%, Klassen-Strassenbreite, Waldfüllung 1/60 m² innerhalb echter Polygone).
- **Screenshot-Gate pro Task:** `node scripts/capture-ksw.mjs <name> <preset> <cam>` für `overview`+`city` (Stil-Tasks zusätzlich `bahnhof`; Licht-/Lampen-Tasks zusätzlich `night`-Preset). Hero-`overview`/`er` gegen `artifacts/ksw/before-*.png` vergleichen — jede Hero-Abweichung ist ein Stopper. Screenshots mit Read ansehen und ehrlich bewerten, bevor der Task committet wird.
- Alle Kommandos vom Worktree-Root (`pwd` prüfen); `ogr2ogr` via `export PATH="/opt/anaconda3/bin:$PATH"`. Tests: `npx vitest run <file>`. Kein cargo.
- Bake-Größen-Budget: `data/winterthur/` gesamt ≤ 8 MB (Gate existiert im Bake).
- Overpass-Fetch nur wenn nötig (Task 4 braucht KEINEN neuen Fetch — Tags liegen schon in `scratch/geo/osm-*.json`; falls `scratch/` fehlt: `npm run geo:fetch`).

## Dateistruktur

| Datei | Verantwortung |
|---|---|
| `scripts/geo/lib/style.mjs` (neu) | pure Bake-Stil-Ableitungen: Hüllen-Gate, Dach-Fallback, Dach-Slab+Skirts, Baum-Spec, Waldfüllung, Strassenbreite aus Tags, Tür-Platzierung |
| `scripts/geo/lib/transform.mjs` | konsumiert style.mjs (Gate/Fallback/Dach), erweitert Schemas |
| `scripts/geo/bake-winterthur.mjs` | ruft die neuen Ableitungen, schreibt erweiterte Artefakte |
| `src/diorama/ksw/geo/facade.ts` (neu) | pure Fenster-/Tür-Raster-Ableitung aus Footprint+Höhe |
| `src/diorama/ksw/geo/windows.ts` (neu) | Instanz-Meshes: Rahmen, Glas, Night-Glow, Türen |
| `src/diorama/ksw/geo/lamps.ts` (neu) | Laternen-Platzierung entlang echter Strassen + Instanz-Meshes |
| `src/diorama/ksw/geo/cityMassing.ts` | + Sockel/Traufband, Tint gezähmt |
| `src/diorama/ksw/geo/roads.ts` | Miter-Joints, Klassen-Ebenen/-Farben |
| `src/diorama/ksw/geo/nature.ts` | Bäume v2 (Originalform, Nadel-Variante, Impostor) |
| `src/diorama/ksw/geo/lod.ts` (neu) | 3-Ring-Manager (Radius→Sichtbarkeit/castShadow) |
| `src/diorama/ksw/main.ts` | Perf-Fixes, Schatten-Follow, 2-Layer-Wolken, Stadt-Mist, LOD-Wiring |
| `src/diorama/designTokens.ts` | NUR anfügen: `kswCityStyle`-Block |

---

### Task 1: Perf messen, dann fixen (Ruckel-Fix)

**Files:**
- Modify: `src/diorama/ksw/main.ts` (GI-Probe-Ausschluss, Baum-Schatten-Politik)
- Modify: `src/diorama/ksw/geo/nature.ts` (castShadow-Flag parametrisieren)

**Interfaces:**
- Consumes: `window.__KSW_INFO()` (existiert: `{drawCalls, triangles, cpu:{frame,agents,render}}`), `GiProbeScheduler`/`renderProbeFace` in main.ts, `scene`-Gruppen `cityMassing`/`cityRoads`/`cityNature`.
- Produces: benannte Gruppe `cityRoot` (THREE.Group, enthält Stadt-Platte, Massing, Roads, Nature) — spätere Tasks hängen ihre Objekte ebenfalls unter `cityRoot`.

- [ ] **Step 1: Baseline messen (origin/main vs. Branch)**

```bash
export PATH="/opt/anaconda3/bin:$PATH"
node scripts/capture-ksw.mjs perf-probe morning overview   # startet Stack; danach:
```
Miss im Browser-Smoke-Stil per Playwright-Einzeiler (Datei `scratch/perf-measure.mjs`, nicht committen):

```js
// scratch/perf-measure.mjs — misst __KSW_INFO nach 8s Laufzeit für eine cam
import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
const cam = process.argv[2] ?? 'overview';
const dev = spawn('npm', ['run', 'dev', '--', '--port', '5186', '--strictPort'], { stdio: 'ignore', detached: true });
await new Promise((r) => setTimeout(r, 3000));
const browser = await chromium.launch({ args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan'] });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
await page.goto(`http://127.0.0.1:5186/ksw.html?cam=${cam}`);
await page.waitForFunction(() => window.__LOOK_READY, null, { timeout: 60000 });
await page.waitForTimeout(8000);
console.log(cam, JSON.stringify(await page.evaluate(() => window.__KSW_INFO())));
await browser.close();
process.kill(-dev.pid);
```

Run: `node scratch/perf-measure.mjs overview && node scratch/perf-measure.mjs city`
Notiere cpu.frame/render + drawCalls für beide Cams. Danach dasselbe auf einem Wegwerf-Checkout von `origin/main` (`git worktree add`+`npm ci` NICHT nötig — origin/main hat keine Stadt; die Referenzzahl steht in progress.md/#111: Hero läuft 100–120 fps ⇒ cpu.frame ≲ 8 ms). Regression = city/overview deutlich darüber.

- [ ] **Step 2: Stadt unter `cityRoot` bündeln + aus der GI-Probe ausschliessen**

In main.ts die Stadt-Adds ersetzen durch:

```ts
  const cityRoot = new THREE.Group();
  cityRoot.name = 'cityRoot';
  cityRoot.add(cityPlate);
  cityRoot.add(buildCityMassing(cityBuildings));
  cityRoot.add(buildRoads(cityRoads, cityRails));
  cityRoot.add(buildNature(cityNature, { excludeRect: { x: 0, z: 0, w: kswPlan.plate.w, d: kswPlan.plate.d } }));
  scene.add(cityRoot);
```

Beim GI-Probe-Face-Render (Suche nach `renderProbeFace` in main.ts) die Stadt ausblenden — die Hero-GI war ohne Stadt getunt, und die Probe rendert sonst 88k+ Stadt-Tris pro Face:

```ts
      cityRoot.visible = false;
      renderProbeFace(/* bestehende Argumente unverändert */);
      cityRoot.visible = true;
```
(Exakt an ALLEN `renderProbeFace`-Aufrufstellen inkl. Boot-Warm-up/CubeCamera.update.)

- [ ] **Step 3: Baum-Schatten-Politik**

In `nature.ts` `buildNature` um Option erweitern und Canopy-Schatten standardmäßig aus (Task 10 schaltet sie im Nah-Ring wieder ein):

```ts
export type NatureOptions = {
  excludeRect?: { x: number; z: number; w: number; d: number };
  treeShadows?: boolean; // default false — LOD ring re-enables near the camera
};
```
und `canopy.castShadow = opts.treeShadows ?? false;`

- [ ] **Step 4: Re-messen + bewerten**

Run: `node scratch/perf-measure.mjs overview && node scratch/perf-measure.mjs city`
Gate: `cpu.frame(city) ≤ 12 ms` und `cpu.frame(overview)` ≈ Baseline. Wenn NICHT erreicht: die verbleibende Differenz gehört fast sicher dem Per-Frame-Shadow-Render (shadowCached ist bei 72 Agenten false, main.ts:413). Dann zusätzlich: Stadt-Meshes (`cityRoot.traverse`) `castShadow=false` setzen, wenn `rig.radius ≤ 120` (Hero-Frustum erreicht sie ohnehin nicht — Extent 46) und Task 11 verwaltet castShadow im Follow-Modus. Erneut messen, Zahlen im Commit-Text dokumentieren.

- [ ] **Step 5: Tests + Smoke + Screenshot-Gate**

```bash
npx vitest run && npm run typecheck
node scripts/smoke-ksw.mjs
node scripts/capture-ksw.mjs t1-overview morning overview && node scripts/capture-ksw.mjs t1-city morning city
```
`t1-overview` mit Read gegen `before-overview.png` prüfen (pixel-treu; Stadt darf im Hintergrund minimal anders schattiert sein, Hospital identisch).

- [ ] **Step 6: Commit**

```bash
git add src/diorama/ksw/main.ts src/diorama/ksw/geo/nature.ts tests/geo/nature-render.test.ts
git commit -m "perf(city): exclude city from GI probe, tree-shadow policy, cityRoot group — <gemessene Zahlen>"
```

---

### Task 2: Bake-Härtung — Hüllen-Gate + Dach-Umriss-Fallback

**Files:**
- Create: `scripts/geo/lib/style.mjs`
- Modify: `scripts/geo/lib/transform.mjs`
- Test: `tests/geo/style.test.ts`

**Interfaces:**
- Produces (style.mjs): `footprintValid(footprint, roofRings) → boolean` (Bbox-Umschluss +1.5 m UND Fläche ≥ 0.5× Dachprojektion), `roofOutlineFootprint(roofRings) → [[x,z],…]` (konvexe Hülle aller Dach-XZ-Punkte, ≥ 3 Punkte, CCW).
- transform.mjs nutzt beide: wenn Trace fehlt ODER `!footprintValid` → Fallback-Hülle; zählt `stats = {traced, fallback}` und gibt sie als zweites Element zurück: `transformBuildings(...)` → **unverändert** `BakedBuilding[]`, aber neues Export `lastTransformStats()` vermeiden — stattdessen Rückgabe erweitern? NEIN: Signatur bleibt, Stats via `options.stats`-Objekt das befüllt wird: `transformBuildings({ …, stats })`.

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/style.test.ts
import { describe, expect, it } from 'vitest';
import { footprintValid, roofOutlineFootprint } from '../../scripts/geo/lib/style.mjs';

const roof = [
  [[0, 10, 0], [20, 10, 0], [20, 12, 15], [0, 12, 15]], // one 20x15 sloped plane
];

describe('footprintValid', () => {
  it('accepts a footprint that encloses the roof', () => {
    expect(footprintValid([[-1, -1], [21, -1], [21, 16], [-1, 16]], roof)).toBe(true);
  });
  it('rejects a stray-facet footprint (tiny vs roof)', () => {
    expect(footprintValid([[0, 0], [2, 0], [1, 2]], roof)).toBe(false);
  });
});

describe('roofOutlineFootprint', () => {
  it('returns the convex hull of the roof XZ points', () => {
    const hull = roofOutlineFootprint(roof);
    expect(hull.length).toBeGreaterThanOrEqual(4);
    const xs = hull.map((p: number[]) => p[0]);
    const zs = hull.map((p: number[]) => p[1]);
    expect(Math.min(...xs)).toBe(0);
    expect(Math.max(...xs)).toBe(20);
    expect(Math.max(...zs)).toBe(15);
  });
});
```

- [ ] **Step 2: rot verifizieren** — `npx vitest run tests/geo/style.test.ts` → FAIL (Modul fehlt).

- [ ] **Step 3: Implementation**

```js
// scripts/geo/lib/style.mjs
// Pure bake-side style/robustness derivations for the diorama-style slice.
// Everything here is a deterministic function of real geometry.

function bboxOf(pts) {
  let x0 = Infinity, x1 = -Infinity, z0 = Infinity, z1 = -Infinity;
  for (const p of pts) {
    x0 = Math.min(x0, p[0]); x1 = Math.max(x1, p[0]);
    z0 = Math.min(z0, p[1]); z1 = Math.max(z1, p[1]);
  }
  return { x0, x1, z0, z1 };
}

function areaOf(ring) {
  let a = 0;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++)
    a += ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
  return Math.abs(a / 2);
}

const roofXZ = (roofRings) => roofRings.flatMap((r) => r.map(([x, , z]) => [x, z]));

// A footprint is trustworthy when it encloses the roof (±1.5 m) and is not a
// stray facet (≥ half the roof's projected area).
export function footprintValid(footprint, roofRings) {
  if (!footprint || footprint.length < 3) return false;
  if (roofRings.length === 0) return true; // nothing to validate against
  const fb = bboxOf(footprint);
  const rb = bboxOf(roofXZ(roofRings));
  const M = 1.5;
  if (rb.x0 < fb.x0 - M || rb.x1 > fb.x1 + M || rb.z0 < fb.z0 - M || rb.z1 > fb.z1 + M) return false;
  const roofArea = (rb.x1 - rb.x0) * (rb.z1 - rb.z0);
  return areaOf(footprint) >= 0.5 * Math.min(roofArea, areaOf(convexHull(roofXZ(roofRings))));
}

// Andrew's monotone chain — the roof IS swisstopo geometry, its projected
// hull is an honest footprint fallback.
export function convexHull(pts) {
  const p = [...pts].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  if (p.length < 3) return p;
  const cross = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  const lower = [];
  for (const pt of p) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], pt) <= 0) lower.pop();
    lower.push(pt);
  }
  const upper = [];
  for (const pt of p.reverse()) {
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], pt) <= 0) upper.pop();
    upper.push(pt);
  }
  return lower.slice(0, -1).concat(upper.slice(0, -1));
}

export function roofOutlineFootprint(roofRings) {
  return convexHull(roofXZ(roofRings));
}
```

- [ ] **Step 4: transform.mjs verdrahten**

Im Footprint-Block von `transformBuildings` (nach dem Trace):

```js
    // harden: a footprint that doesn't carry its roof is worse than the
    // roof's own projected outline (real swisstopo geometry either way)
    if (!footprintValid(footprint, b.roofs)) {
      const hull = roofOutlineFootprint(b.roofs.length ? b.roofs : b.walls);
      if (hull.length >= 3) {
        footprint = hull.map(([x, z]) => [Math.round(x * 100) / 100, Math.round(z * 100) / 100]);
        stats.fallback += 1;
      }
    } else {
      stats.traced += 1;
    }
```
mit Import `import { footprintValid, roofOutlineFootprint } from './style.mjs';`, Signatur `transformBuildings({ …, footprints = null, stats = { traced: 0, fallback: 0 } })`. Bake loggt `stats` und gated: `fallback / buildings ≤ 0.25`, sonst throw.

- [ ] **Step 5: grün + alle Transform-Tests** — `npx vitest run tests/geo/style.test.ts tests/geo/transform.test.ts` → PASS.

- [ ] **Step 6: Re-bake + Schwebe-Sichtprüfung**

```bash
export PATH="/opt/anaconda3/bin:$PATH" && npm run geo:bake
node scripts/capture-ksw.mjs t2-city morning city
```
`t2-city` mit Read prüfen: keine schwebenden Dächer mehr (vorher sichtbar im Nordteil).

- [ ] **Step 7: Commit** — `git add scripts/geo tests/geo/style.test.ts data/winterthur && git commit -m "geo(bake): footprint gate + roof-outline fallback — no building without a hull"`

---

### Task 3: Dachdicke + Giebel-Skirts (Bake)

**Files:**
- Modify: `scripts/geo/lib/style.mjs`, `scripts/geo/lib/transform.mjs`
- Test: `tests/geo/style.test.ts` (erweitern)

**Interfaces:**
- Produces (style.mjs): `roofSkirts(roofRings, eaveY) → ring[]` — pro dedupliziertem Boundary-Edge der Dachflächen ein vertikales Quad (als 4-Punkt-Ring `[[x,y,z]×4]`) von der Kante hinunter auf `max(eaveY, edgeMinY − 0.02)`; überspringt Kanten, die flach auf Traufhöhe liegen. `roofUnderside(roofRings, drop=0.22) → ring[]` — jede Dachfläche um `drop` nach unten kopiert, Winding invertiert.
- transform.mjs: Dach-Mesh = Oberseite + Underside; Skirts werden dem WALL-Mesh (Wandfarbe) hinzugefügt → geschlossene Giebel, sichtbare Dachkante.

- [ ] **Step 1: Failing Tests (an tests/geo/style.test.ts anhängen)**

```ts
import { roofSkirts, roofUnderside } from '../../scripts/geo/lib/style.mjs';

describe('roofSkirts', () => {
  // gabled roof: two planes meeting at a ridge y=6, eaves at y=4
  const planes = [
    [[0, 4, 0], [10, 4, 0], [10, 6, 5], [0, 6, 5]],
    [[0, 6, 5], [10, 6, 5], [10, 4, 10], [0, 4, 10]],
  ];
  it('emits vertical skirts only for rising boundary edges, ridge deduped', () => {
    const skirts = roofSkirts(planes, 4);
    // boundary edges above eave: the two gable sides x=0 and x=10 (2 edges each
    // rising to the ridge) → 4 skirts; eave edges (y=4) skipped; ridge shared → deduped
    expect(skirts.length).toBe(4);
    for (const ring of skirts) {
      expect(ring.length).toBe(4);
      const ys = ring.map((p: number[]) => p[1]);
      expect(Math.min(...ys)).toBeCloseTo(4, 5);
    }
  });
});

describe('roofUnderside', () => {
  it('copies each plane 0.22 lower with flipped winding', () => {
    const planes = [[[0, 5, 0], [4, 5, 0], [4, 5, 4], [0, 5, 4]]];
    const under = roofUnderside(planes);
    expect(under.length).toBe(1);
    expect(under[0][0][1]).toBeCloseTo(4.78, 5);
    expect(under[0][0][0]).toBe(planes[0][planes[0].length - 1][0]); // reversed order
  });
});
```

- [ ] **Step 2: rot** — `npx vitest run tests/geo/style.test.ts` → neue Tests FAIL.

- [ ] **Step 3: Implementation (style.mjs anhängen)**

```js
const EDGE_EPS = 0.05;
const ekey = (a, b) => {
  const ka = `${Math.round(a[0] * 50)},${Math.round(a[1] * 50)},${Math.round(a[2] * 50)}`;
  const kb = `${Math.round(b[0] * 50)},${Math.round(b[1] * 50)},${Math.round(b[2] * 50)}`;
  return ka < kb ? `${ka}|${kb}` : `${kb}|${ka}`;
};

// Vertical fill from each rising roof boundary edge down to the eave —
// closes the open gable triangles left by prism walls. Shared (ridge/valley)
// edges appear in two planes and are deduped; edges lying at eave level need
// no skirt.
export function roofSkirts(roofRings, eaveY) {
  const seen = new Map(); // ekey -> count
  const edges = [];
  for (const ring of roofRings) {
    for (let i = 0; i < ring.length; i++) {
      const a = ring[i];
      const b = ring[(i + 1) % ring.length];
      const k = ekey(a, b);
      seen.set(k, (seen.get(k) ?? 0) + 1);
      edges.push({ a, b, k });
    }
  }
  const out = [];
  for (const { a, b, k } of edges) {
    if (seen.get(k) > 1) continue; // interior edge (ridge/valley)
    if (a[1] <= eaveY + EDGE_EPS && b[1] <= eaveY + EDGE_EPS) continue; // flat at eave
    out.push([
      [a[0], a[1], a[2]],
      [b[0], b[1], b[2]],
      [b[0], eaveY, b[2]],
      [a[0], eaveY, a[2]],
    ]);
  }
  return out;
}

export function roofUnderside(roofRings, drop = 0.22) {
  return roofRings.map((ring) => [...ring].reverse().map(([x, y, z]) => [x, y - drop, z]));
}
```

- [ ] **Step 4: transform.mjs verdrahten**

```js
import { footprintValid, roofOutlineFootprint, roofSkirts, roofUnderside } from './style.mjs';
```
und beim Mesh-Bau:

```js
    const skirts = roofSkirts(b.roofs, eaveY);
    const wall = extrudeWalls(footprint, eaveH);
    appendRings(wall, skirts, groundY); // helper unten
    const roof = meshFromRings([...b.roofs, ...roofUnderside(b.roofs)], groundY);
```
mit Helper (in transform.mjs, neben meshFromRings):

```js
function appendRings(mesh, rings, groundY) {
  const extra = meshFromRings(rings, groundY);
  const base = mesh.pos.length / 3;
  mesh.pos.push(...extra.pos);
  for (const i of extra.idx) mesh.idx.push(base + i);
}
```

- [ ] **Step 5: grün + Bake + Größe** — `npx vitest run tests/geo && export PATH="/opt/anaconda3/bin:$PATH" && npm run geo:bake` (Budget-Gate hält; Tris steigen ~1.5×).

- [ ] **Step 6: Screenshot-Gate** — `node scripts/capture-ksw.mjs t3-city morning city && node scripts/capture-ksw.mjs t3-bahnhof morning bahnhof`; Read: geschlossene Giebel, sichtbare Dachkanten, keine Papier-Planes von der Seite.

- [ ] **Step 7: Commit** — `git add scripts/geo tests/geo data/winterthur && git commit -m "geo(bake): roof slabs + gable skirts — closed volumes, visible clay roof edges"`

---

### Task 4: Bake-Daten v2 — Bäume (Tags/Wald), Strassenbreiten, Türen

**Files:**
- Modify: `scripts/geo/lib/style.mjs`, `scripts/geo/lib/transform.mjs`, `scripts/geo/bake-winterthur.mjs`
- Test: `tests/geo/style.test.ts` (erweitern), `tests/geo/transform.test.ts` (Roads-Breite)

**Interfaces (Produces):**
- `treeSpec(tags, x, z) → { x, z, h, r, kind }` — kind `'broad'|'conifer'`; Priorität height/diameter_crown-Tags → leaf_type-Default (broad 9/3, conifer 14/2 — h/Kronenradius) → broad-Default; ±15% deterministische Varianz via Hash(x,z) auf Defaults (NICHT auf echte Tag-Werte).
- `forestFill(ring, existingTrees, density=1/60) → Array<{x,z,h,r,kind:'broad'}>` — Hash-Gitter (Zellgröße √60 m), Zellzentrum + deterministischer Jitter, nur `pointInRing`, kein Punkt < 4 m an einem existierenden Baum.
- `roadWidthFromTags(tags, fallbackWidth) → number` — `parseFloat(tags.width)` wenn > 0, sonst `lanes × 3.2`, sonst fallback.
- `doorForBuilding(footprint, roadPts) → { x, z, yaw } | null` — Fassadensegment mit minimaler Distanz Segment-Mittelpunkt↔nächster Strassenpunkt; Tür am Segment-Mittelpunkt, yaw = Normale Richtung Strasse.
- Schema: `nature.json.trees` wird `Array<{x,z,h,r,kind}>`; `buildings.json` Gebäude bekommen `door?: {x,z,yaw}`; roads unverändert (Breite schon im Bake aufgelöst).

- [ ] **Step 1: Failing Tests (style.test.ts anhängen)**

```ts
import { doorForBuilding, forestFill, roadWidthFromTags, treeSpec } from '../../scripts/geo/lib/style.mjs';

describe('treeSpec', () => {
  it('real tags win, untouched by variance', () => {
    const t = treeSpec({ height: '17', diameter_crown: '8' }, 1, 2);
    expect(t.h).toBe(17);
    expect(t.r).toBe(4);
  });
  it('leaf_type default with deterministic ±15% variance', () => {
    const a = treeSpec({ leaf_type: 'needleleaved' }, 5, 5);
    const b = treeSpec({ leaf_type: 'needleleaved' }, 5, 5);
    expect(a).toEqual(b);
    expect(a.kind).toBe('conifer');
    expect(a.h).toBeGreaterThan(14 * 0.84);
    expect(a.h).toBeLessThan(14 * 1.16);
  });
});

describe('forestFill', () => {
  const ring = [[0, 0], [120, 0], [120, 60], [0, 60]]; // 7200 m² → ~120 trees
  it('fills the polygon at ~1/60 m² density, deterministic', () => {
    const a = forestFill(ring, []);
    expect(a.length).toBeGreaterThan(70);
    expect(a.length).toBeLessThan(170);
    expect(a).toEqual(forestFill(ring, []));
    for (const t of a) {
      expect(t.x).toBeGreaterThanOrEqual(0);
      expect(t.x).toBeLessThanOrEqual(120);
    }
  });
  it('respects mapped trees (4 m clearance)', () => {
    const a = forestFill(ring, [{ x: 60, z: 30 }]);
    for (const t of a) expect(Math.hypot(t.x - 60, t.z - 30)).toBeGreaterThan(4);
  });
});

describe('roadWidthFromTags', () => {
  it('width tag > lanes > fallback', () => {
    expect(roadWidthFromTags({ width: '7.5' }, 5.5)).toBe(7.5);
    expect(roadWidthFromTags({ lanes: '3' }, 5.5)).toBeCloseTo(9.6);
    expect(roadWidthFromTags({}, 5.5)).toBe(5.5);
  });
});

describe('doorForBuilding', () => {
  it('places the door on the road-facing facade', () => {
    const fp = [[0, 0], [10, 0], [10, 10], [0, 10]];
    const road = [[5, -6], [5, -20]]; // south of the building (−z side)
    const d = doorForBuilding(fp, road)!;
    expect(d.z).toBeCloseTo(0); // south edge
    expect(d.x).toBeCloseTo(5);
  });
});
```

- [ ] **Step 2: rot** — `npx vitest run tests/geo/style.test.ts` → FAIL.

- [ ] **Step 3: Implementation (style.mjs anhängen)**

```js
import { pointInRing } from './join.mjs';

const h01 = (x, z) => {
  const s = Math.sin(x * 127.1 + z * 311.7) * 43758.5453;
  return s - Math.floor(s);
};
const vary = (v, x, z) => v * (0.85 + 0.3 * h01(x, z));

const TREE_DEFAULTS = { broad: { h: 9, r: 3 }, conifer: { h: 14, r: 2 } };

export function treeSpec(tags, x, z) {
  const kind = tags.leaf_type === 'needleleaved' ? 'conifer' : 'broad';
  const d = TREE_DEFAULTS[kind];
  const tagH = Number.parseFloat(tags.height ?? '');
  const tagCrown = Number.parseFloat(tags.diameter_crown ?? '');
  return {
    x, z, kind,
    h: tagH > 0 ? tagH : Math.round(vary(d.h, x, z) * 10) / 10,
    r: tagCrown > 0 ? tagCrown / 2 : Math.round(vary(d.r, x + 31, z - 17) * 10) / 10,
  };
}

export function forestFill(ring, existingTrees, density = 1 / 60) {
  const cell = Math.sqrt(1 / density);
  let x0 = Infinity, x1 = -Infinity, z0 = Infinity, z1 = -Infinity;
  for (const [x, z] of ring) {
    x0 = Math.min(x0, x); x1 = Math.max(x1, x);
    z0 = Math.min(z0, z); z1 = Math.max(z1, z);
  }
  const out = [];
  for (let gx = Math.floor(x0 / cell); gx * cell < x1; gx++) {
    for (let gz = Math.floor(z0 / cell); gz * cell < z1; gz++) {
      const jx = (h01(gx * 13.7, gz * 71.3) - 0.5) * cell * 0.8;
      const jz = (h01(gx * 91.7, gz * 23.1) - 0.5) * cell * 0.8;
      const x = (gx + 0.5) * cell + jx;
      const z = (gz + 0.5) * cell + jz;
      if (!pointInRing(x, z, ring)) continue;
      if (existingTrees.some((t) => Math.hypot(t.x - x, t.z - z) < 4)) continue;
      out.push({ x: Math.round(x * 100) / 100, z: Math.round(z * 100) / 100, kind: 'broad',
        h: Math.round(vary(TREE_DEFAULTS.broad.h, x, z) * 10) / 10,
        r: Math.round(vary(TREE_DEFAULTS.broad.r, x + 31, z - 17) * 10) / 10 });
    }
  }
  return out;
}

export function roadWidthFromTags(tags, fallbackWidth) {
  const w = Number.parseFloat(tags.width ?? '');
  if (w > 0) return w;
  const lanes = Number.parseInt(tags.lanes ?? '', 10);
  if (lanes > 0) return lanes * 3.2;
  return fallbackWidth;
}

export function doorForBuilding(footprint, roadPts) {
  if (!roadPts.length) return null;
  let best = null;
  for (let i = 0; i < footprint.length; i++) {
    const [ax, az] = footprint[i];
    const [bx, bz] = footprint[(i + 1) % footprint.length];
    if (Math.hypot(bx - ax, bz - az) < 2.2) continue; // too short for a door
    const mx = (ax + bx) / 2;
    const mz = (az + bz) / 2;
    for (const [rx, rz] of roadPts) {
      const dist = Math.hypot(rx - mx, rz - mz);
      if (!best || dist < best.dist) best = { dist, mx, mz, ax, az, bx, bz, rx, rz };
    }
  }
  if (!best) return null;
  // outward normal of the edge, flipped toward the road
  let nx = -(best.bz - best.az);
  let nz = best.bx - best.ax;
  const len = Math.hypot(nx, nz) || 1;
  nx /= len; nz /= len;
  if (nx * (best.rx - best.mx) + nz * (best.rz - best.mz) < 0) { nx = -nx; nz = -nz; }
  return { x: Math.round(best.mx * 100) / 100, z: Math.round(best.mz * 100) / 100, yaw: Math.round(Math.atan2(nx, nz) * 1000) / 1000 };
}
```

- [ ] **Step 4: transform + bake verdrahten**

- `transformNature`: Bäume via `treeSpec(t, …toLocal(el))`; nach der Element-Schleife pro `wood|forest`-Green `forestFill(g.ring, trees)` anhängen. Rückgabe-Feld `trees` = Objekt-Array (Schema-Wechsel).
- `transformRoads`: `roadStyle`-Breite durch `roadWidthFromTags(el.tags, style.width)` ersetzen.
- Bake: nach `transformRoads` ein `roadPtsFlat = roads.flatMap(r => r.pts)` einmal in ein Grid bucketen (Zellgröße 50 m) und pro Gebäude `doorForBuilding(footprint, nearbyRoadPts)` (nur Punkte der 9 Nachbarzellen — sonst O(n²)); `door` ans Gebäude. Gate: ≥ 80% der Gebäude haben eine Tür.
- `tests/geo/transform.test.ts`: Roads-Test ergänzen: `tags:{highway:'residential',width:'7.5'}` → width 7.5.
- `geoData.ts`: `CityNature.trees: Array<{x,z,h,r,kind:'broad'|'conifer'}>`, `BakedBuilding.door?: {x,z,yaw}`; `nature.ts`-Baumschleife auf `t.x/t.z` umstellen (Größen nutzt Task 8); `tests/geo/nature-render.test.ts`-Fixture auf Objekt-Bäume umstellen.

- [ ] **Step 5: grün + Re-bake** — `npx vitest run tests/geo && npm run typecheck && export PATH="/opt/anaconda3/bin:$PATH" && npm run geo:bake` — Report zeigt Waldbäume (>4127 gesamt), Tür-Quote, Budget hält (sonst Dichte auf 1/80 senken UND Spec-Abweichung im Commit notieren).

- [ ] **Step 6: Commit** — `git add scripts/geo tests/geo src/diorama/ksw/geo/geoData.ts src/diorama/ksw/geo/nature.ts data/winterthur && git commit -m "geo(bake): tree specs from OSM tags + declared forest fill, road widths from tags, road-facing doors"`

---

### Task 5: Sockel, Traufband, gezähmter Tint (Runtime)

**Files:**
- Modify: `src/diorama/ksw/geo/cityMassing.ts`, `src/diorama/designTokens.ts`
- Test: `tests/geo/cityMassing.test.ts` (erweitern)

**Interfaces:**
- designTokens NUR anfügen:

```ts
// Diorama-style layer for the geodetic city (style slice). Additive only.
export const kswCityStyle = {
  plinthH: 0.5, plinthOut: 0.12, plinthSink: 0.3, // sockel: height, outset, below-plate sink
  eaveBandH: 0.18, eaveBandOut: 0.08,
  tintL: 0.06, tintHue: 0.012, // tamed variation (was ±14% L)
  windowW: 1.3, windowH: 1.4, windowSpacing: 2.4, storeyH: 3.0, sillFrac: 0.32,
  doorW: 1.5, doorH: 2.2,
  lamp: { spacing: { primary: 25, secondary: 28, tertiary: 30, residential: 35, unclassified: 35, living_street: 35, service: 45, pedestrian: 30 } as Record<string, number>, sideOffset: 1.2 },
  lod: { nearR: 150, midR: 600, hysteresis: 0.1 },
  cloudSwap: { start: 300, end: 600 },
} as const;
```
- `buildCityMassing` produziert zusätzlich Kinder `cityPlinths` (weiss-creme Band-Mesh) und `cityEaves` (Traufband, roofTrim-Farbe), beide aus dem Footprint + `height`-Traufe abgeleitet; Wand-Extrusion beginnt bei `−plinthSink`.

- [ ] **Step 1: Failing Tests (cityMassing.test.ts anhängen)**

```ts
  it('adds plinth and eave band meshes', () => {
    const g2 = buildCityMassing([cube(0)]);
    expect(g2.getObjectByName('cityPlinths')).toBeTruthy();
    expect(g2.getObjectByName('cityEaves')).toBeTruthy();
    const plinth = g2.getObjectByName('cityPlinths') as THREE.Mesh;
    const pos = plinth.geometry.getAttribute('position');
    let minY = Infinity;
    for (let i = 0; i < pos.count; i++) minY = Math.min(minY, pos.getY(i));
    expect(minY).toBeLessThan(0); // sinks below the plate — nothing floats
  });
```
(Hinweis: der bestehende „exactly two meshes“-Test wird auf 4 Kinder angepasst.)

- [ ] **Step 2: rot.** — `npx vitest run tests/geo/cityMassing.test.ts`

- [ ] **Step 3: Implementation**

In `cityMassing.ts`: Ring-Band-Helfer + Einbau:

```ts
// horizontal band following a footprint ring: a short extruded wall strip,
// outset from the facade — the original's plinth/eave-trim language.
function ringBand(fp: number[][], y0: number, y1: number, out: number): { pos: number[]; idx: number[] } {
  const pos: number[] = [];
  const idx: number[] = [];
  const n = fp.length;
  for (let i = 0; i < n; i++) {
    const [ax, az] = fp[i];
    const [bx, bz] = fp[(i + 1) % n];
    const ex = bx - ax;
    const ez = bz - az;
    const len = Math.hypot(ex, ez);
    if (len < 0.05) continue;
    const ox = (-ez / len) * out;
    const oz = (ex / len) * out;
    const base = pos.length / 3;
    // both sides + top so the band reads as a solid rim from every angle
    pos.push(
      ax + ox, y0, az + oz, bx + ox, y0, bz + oz, bx + ox, y1, bz + oz, ax + ox, y1, az + oz,
      ax - ox, y0, az - oz, bx - ox, y0, bz - oz, bx - ox, y1, bz - oz, ax - ox, y1, az - oz,
      ax + ox, y1, az + oz, bx + ox, y1, bz + oz, bx - ox, y1, bz - oz, ax - ox, y1, az - oz,
    );
    idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
    idx.push(base + 5, base + 4, base + 7, base + 5, base + 7, base + 6);
    idx.push(base + 8, base + 9, base + 10, base + 8, base + 10, base + 11);
  }
  return { pos: pos.map((v) => Math.round(v * 100)), idx };
}
```
In `buildCityMassing`: nach walls/roofs zusätzlich

```ts
  const plinths = buildings.map((b) => ringBand(b.footprint, -kswCityStyle.plinthSink, kswCityStyle.plinthH, kswCityStyle.plinthOut));
  const eaves = buildings.map((b) => {
    const eave = Math.max(b.height - 2, kswCityStyle.plinthH + 0.5); // eave≈wall top; height is ridge — band sits just below
    return ringBand(b.footprint, eave - kswCityStyle.eaveBandH, eave, kswCityStyle.eaveBandOut);
  });
  make('cityPlinths', () => ({ pos: [], idx: [] }), palette.white); // replaced below
```
— konkret: `make` auf `(name, parts, base)`-Form zurückstellen, d.h. `mergeTinted` bekommt eine Overload für vorgefertigte Parts ohne Tint (einheitliche Farbe): einfachster Weg: `mergeBakedParts(parts: {pos,idx}[]): BufferGeometry` (die Task-8-freie Variante von `mergeBaked` aus S2 — wieder einführen) und

```ts
  const plinthMesh = new THREE.Mesh(mergeBakedParts(plinths), clayMat(palette.white));
  plinthMesh.name = 'cityPlinths';
  plinthMesh.castShadow = false; plinthMesh.receiveShadow = true;
  group.add(plinthMesh);
  const eaveMesh = new THREE.Mesh(mergeBakedParts(eaves), clayMat(kswPalette.roofTrim));
  eaveMesh.name = 'cityEaves';
  eaveMesh.castShadow = false; eaveMesh.receiveShadow = true;
  group.add(eaveMesh);
```
Wand-Extrusion tiefersetzen: in `mergeTinted` beim Wand-Pick y-Offset NICHT nötig — stattdessen in Task-3-Bake bereits ab y=0; hier: `plinthSink` regelt die Optik. Tint zähmen in `tintFor`:

```ts
  c.setHSL(hsl.h + (a - 0.5) * kswCityStyle.tintHue, hsl.s * (0.96 + 0.08 * b), hsl.l * (1 - kswCityStyle.tintL + 2 * kswCityStyle.tintL * a));
```

- [ ] **Step 4: grün + Typecheck** — `npx vitest run tests/geo/cityMassing.test.ts && npm run typecheck`

- [ ] **Step 5: Screenshot-Gate** — `node scripts/capture-ksw.mjs t5-city morning city && node scripts/capture-ksw.mjs t5-bahnhof morning bahnhof && node scripts/capture-ksw.mjs t5-overview morning overview`; Read: Häuser stehen satt (Sockel), Traufband liest sich als Original-Trim, Schattenseiten warm statt grau; Hero identisch.

- [ ] **Step 6: Commit** — `git add src/diorama tests/geo && git commit -m "city(style): plinths, eave trim bands, tamed clay tint"`

---

### Task 6: Fensterraster + Türen (facade.ts + windows.ts)

**Files:**
- Create: `src/diorama/ksw/geo/facade.ts`, `src/diorama/ksw/geo/windows.ts`
- Modify: `src/diorama/ksw/main.ts` (unter `cityRoot` einhängen)
- Test: `tests/geo/facade.test.ts`

**Interfaces:**
- facade.ts (pur, kein three):

```ts
export type WindowSlot = { x: number; y: number; z: number; yaw: number };
export type FacadeLayout = { windows: WindowSlot[]; door: WindowSlot | null };
export function facadeLayout(b: { footprint: number[][]; height: number; door?: { x: number; z: number; yaw: number } }): FacadeLayout;
```
Regeln: `floors = clamp(round(height/kswCityStyle.storeyH), 1, 24)`; pro Footprint-Kante `cols = floor((len − 0.8)/windowSpacing)`, zentriert; Fenster-y = `f*storeyH + storeyH*sillFrac + windowH/2`, nur solange `y + windowH/2 < height − 0.4`; yaw = Kanten-Normale (nach aussen). Tür ersetzt das nächste EG-Fenster der Tür-Kante (Abstand < spacing).
- windows.ts: `buildWindows(buildings: BakedBuilding[], opts: { lampGlow: boolean }): THREE.Group` — InstancedMeshes `cityWindowFrames` (weiss, Box 1.3×1.4×0.12, 0.07 vor Fassade), `cityWindowPanes` (glassMat-Klon), nachts `NIGHT_WINDOW_SHARE`-Anteil (via `nightWindowHash(x,z)`) als `cityWindowGlow` (MeshBasicMaterial warm 0xffd9a0), `cityDoors` (Rahmen weiss + Blatt `palette.woodSoft`, 1.5×2.2).

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/facade.test.ts
import { describe, expect, it } from 'vitest';
import { facadeLayout } from '../../src/diorama/ksw/geo/facade';

const b = { footprint: [[0, 0], [24, 0], [24, 10], [0, 10]], height: 9.5, door: { x: 12, z: 0, yaw: Math.PI } };

describe('facadeLayout', () => {
  const out = facadeLayout(b);
  it('derives storeys and columns from real size', () => {
    // floors = round(9.5/3)=3; 24m edge → floor((24-0.8)/2.4)=9 cols; 10m edge → 3 cols
    // top storey windows must stay 0.4m under the eave → still 3 rows here
    expect(out.windows.length).toBeGreaterThan((9 * 2 + 3 * 2) * 2); // ≥2 rows everywhere
    const ys = [...new Set(out.windows.map((w) => Math.round(w.y * 10) / 10))];
    expect(ys.length).toBe(3);
  });
  it('door replaces the nearest ground-floor window on its edge', () => {
    expect(out.door).not.toBeNull();
    const gfSouth = out.windows.filter((w) => w.z === 0 && w.y < 3);
    for (const w of gfSouth) expect(Math.abs(w.x - 12)).toBeGreaterThan(1.1);
  });
  it('is deterministic', () => {
    expect(facadeLayout(b)).toEqual(out);
  });
});
```

- [ ] **Step 2: rot.** `npx vitest run tests/geo/facade.test.ts`

- [ ] **Step 3: facade.ts implementieren**

```ts
// src/diorama/ksw/geo/facade.ts
// Pure derivation: window/door raster from the real footprint + real height.
// floors = height/3 m (documented), columns from real facade length. No RNG.
import { kswCityStyle } from '../../designTokens';

export type WindowSlot = { x: number; y: number; z: number; yaw: number };
export type FacadeLayout = { windows: WindowSlot[]; door: WindowSlot | null };

export function facadeLayout(b: { footprint: number[][]; height: number; door?: { x: number; z: number; yaw: number } }): FacadeLayout {
  const s = kswCityStyle;
  const floors = Math.min(24, Math.max(1, Math.round(b.height / s.storeyH)));
  const windows: WindowSlot[] = [];
  let door: WindowSlot | null = null;
  const fp = b.footprint;
  for (let i = 0; i < fp.length; i++) {
    const [ax, az] = fp[i];
    const [bx, bz] = fp[(i + 1) % fp.length];
    const ex = bx - ax;
    const ez = bz - az;
    const len = Math.hypot(ex, ez);
    const cols = Math.floor((len - 0.8) / s.windowSpacing);
    if (cols < 1) continue;
    const ux = ex / len;
    const uz = ez / len;
    const yaw = Math.atan2(-uz, ux) + Math.PI / 2; // outward normal heading
    const start = (len - (cols - 1) * s.windowSpacing) / 2;
    const isDoorEdge =
      b.door &&
      Math.abs((b.door.x - ax) * uz - (b.door.z - az) * ux) < 0.5 && // on the edge line
      (b.door.x - ax) * ux + (b.door.z - az) * uz > -0.5 &&
      (b.door.x - ax) * ux + (b.door.z - az) * uz < len + 0.5;
    for (let c = 0; c < cols; c++) {
      const t = start + c * s.windowSpacing;
      const x = ax + ux * t;
      const z = az + uz * t;
      for (let f = 0; f < floors; f++) {
        const y = f * s.storeyH + s.storeyH * s.sillFrac + s.windowH / 2;
        if (y + s.windowH / 2 > b.height - 0.4) break;
        if (f === 0 && isDoorEdge && b.door && Math.hypot(x - b.door.x, z - b.door.z) < s.windowSpacing / 2) {
          door = { x: b.door.x, y: s.doorH / 2, z: b.door.z, yaw: b.door.yaw };
          continue;
        }
        windows.push({ x, y, z, yaw });
      }
    }
  }
  if (!door && b.door) door = { x: b.door.x, y: kswCityStyle.doorH / 2, z: b.door.z, yaw: b.door.yaw };
  return { windows, door };
}
```

- [ ] **Step 4: grün.** Dann windows.ts:

```ts
// src/diorama/ksw/geo/windows.ts
// Instanced diorama windows/doors for the city: white frames + glass panes
// slightly proud of the real facades, warm night glow in the original share.
import * as THREE from 'three/webgpu';
import { kswCityStyle, palette } from '../../designTokens';
import { NIGHT_WINDOW_SHARE, nightWindowHash } from '../staticBatch';
import { clayMat, glassMat } from '../props';
import type { BakedBuilding } from './geoData';
import { facadeLayout, type WindowSlot } from './facade';

function fill(mesh: THREE.InstancedMesh, slots: WindowSlot[], out: number): void {
  const m = new THREE.Matrix4();
  const e = new THREE.Euler();
  for (let i = 0; i < slots.length; i++) {
    const s = slots[i];
    e.set(0, s.yaw, 0);
    m.makeRotationFromEuler(e);
    m.setPosition(s.x + Math.sin(s.yaw) * out, s.y, s.z + Math.cos(s.yaw) * out);
    mesh.setMatrixAt(i, m);
  }
  mesh.instanceMatrix.needsUpdate = true;
}

export function buildWindows(buildings: BakedBuilding[], opts: { lampGlow: boolean }): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityWindows';
  const s = kswCityStyle;
  const all: WindowSlot[] = [];
  const doors: WindowSlot[] = [];
  for (const b of buildings) {
    const layout = facadeLayout(b);
    all.push(...layout.windows);
    if (layout.door) doors.push(layout.door);
  }
  const glow: WindowSlot[] = [];
  const plain: WindowSlot[] = [];
  for (const w of all) (opts.lampGlow && nightWindowHash(w.x, w.z) < NIGHT_WINDOW_SHARE ? glow : plain).push(w);

  const frameGeo = new THREE.BoxGeometry(s.windowW + 0.16, s.windowH + 0.16, 0.1);
  const paneGeo = new THREE.BoxGeometry(s.windowW, s.windowH, 0.06);
  const frames = new THREE.InstancedMesh(frameGeo, clayMat(palette.white), all.length);
  frames.name = 'cityWindowFrames';
  fill(frames, all, 0.07);
  const panes = new THREE.InstancedMesh(paneGeo, glassMat().clone(), plain.length);
  panes.name = 'cityWindowPanes';
  fill(panes, plain, 0.1);
  const glowMesh = new THREE.InstancedMesh(paneGeo, new THREE.MeshBasicMaterial({ color: 0xffd9a0 }), glow.length);
  glowMesh.name = 'cityWindowGlow';
  fill(glowMesh, glow, 0.1);
  const doorGeo = new THREE.BoxGeometry(s.doorW, s.doorH, 0.14);
  const doorMesh = new THREE.InstancedMesh(doorGeo, clayMat(palette.woodSoft), doors.length);
  doorMesh.name = 'cityDoors';
  fill(doorMesh, doors, 0.08);
  for (const m of [frames, panes, glowMesh, doorMesh]) {
    m.castShadow = false;
    m.receiveShadow = false;
    group.add(m);
  }
  return group;
}
```
main.ts: `cityRoot.add(buildWindows(cityBuildings, { lampGlow: preset.lampOn }));` (Import ergänzen).

- [ ] **Step 5: Gates** — `npx vitest run && npm run typecheck && node scripts/smoke-ksw.mjs`, dann Captures `t6-bahnhof morning`, `t6-bahnhof-night` (`node scripts/capture-ksw.mjs t6-night night bahnhof`), `t6-overview morning overview`. Read: Fensterraster wie Original-Sprache, nachts warmes Glühen; Hero identisch; drawCalls +4.

- [ ] **Step 6: Commit** — `git add src/diorama tests/geo && git commit -m "city(style): instanced diorama windows/doors from real facades, night glow"`

---

### Task 7: Strassen v2 — Miter, Klassen-Ebenen, Ruhe

**Files:**
- Modify: `src/diorama/ksw/geo/roads.ts`, `src/diorama/designTokens.ts` (kswCity anfügen), Test: `tests/geo/roads.test.ts`

**Interfaces:**
- designTokens `kswCity` anfügen (additiv): `roadColors: { carriage: 0xcfc4b2, footway: 0xe5dcc8, rail: 0x8d949c, railBed: 0xb9b2a4 }`, `roadYs: { carriage: 0.04, footway: 0.045, railBed: 0.035, rail: 0.05 }`.
- `buildRoads(roads, rails)` liefert Group mit `carriageRibbons`, `footwayRibbons`, `railBeds`, `railRibbons` — Miter-Joins (Innen-/Aussenkante über Winkelhalbierende, Kappung bei Knick > 60° auf Segment-Normale), ein durchgehender Streifen pro Polyline (keine überlappenden Quads an Zwischenpunkten).

- [ ] **Step 1: Failing Tests (roads.test.ts ERSETZEN)**

```ts
// tests/geo/roads.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildRoads, miterStrip } from '../../src/diorama/ksw/geo/roads';

describe('miterStrip', () => {
  it('builds a continuous strip: 2 verts per point, no seams', () => {
    const g = miterStrip([[0, 0], [10, 0], [10, 10]], 6, 0.04);
    expect(g.positions.length / 3).toBe(6); // 3 pts × 2
    expect(g.indices.length).toBe(12); // 2 segments × 2 tris
  });
  it('miter joint bisects a right angle (outer corner further than half width)', () => {
    const g = miterStrip([[0, 0], [10, 0], [10, 10]], 6, 0);
    // corner verts are at index 2,3: bisector direction (±(1,-1)/√2) × 3√2
    const cx = [g.positions[6], g.positions[9]];
    for (const x of cx) expect(Math.abs(x - 10)).toBeCloseTo(3, 3);
  });
  it('caps extreme spikes', () => {
    const g = miterStrip([[0, 0], [10, 0], [0, 0.4]], 6, 0); // ~176° turn
    for (let i = 0; i < g.positions.length; i += 3) {
      expect(Math.abs(g.positions[i])).toBeLessThan(25); // no infinite miter spike
    }
  });
});

describe('buildRoads', () => {
  const group = buildRoads(
    [
      { class: 'residential', width: 6, pts: [[0, 0], [10, 0]] },
      { class: 'footway', width: 2.2, pts: [[0, 5], [10, 5]] },
    ],
    [{ class: 'rail', width: 3, pts: [[0, 9], [10, 9]] }],
  );
  it('splits carriage / footway / rail(+bed) into named layers', () => {
    for (const n of ['carriageRibbons', 'footwayRibbons', 'railBeds', 'railRibbons'])
      expect(group.getObjectByName(n)).toBeTruthy();
  });
  it('layers sit on distinct heights (no z-fight)', () => {
    const y = (n: string) => (group.getObjectByName(n) as THREE.Mesh).geometry.getAttribute('position').getY(0);
    expect(y('carriageRibbons')).not.toBeCloseTo(y('footwayRibbons'), 3);
  });
});
```

- [ ] **Step 2: rot.** — Test ersetzt alte Erwartungen; `npx vitest run tests/geo/roads.test.ts` → FAIL (miterStrip fehlt).

- [ ] **Step 3: roads.ts neu**

```ts
// src/diorama/ksw/geo/roads.ts
// OSM ways as flat clay ribbons v2: continuous miter-joined strips (no wedge
// gaps, no overlapping quads), one visual layer per class — carriageways,
// footpaths, rail on its ballast band — each on its own height so junctions
// never flicker.
import * as THREE from 'three/webgpu';
import { kswCity } from '../../designTokens';
import { clayMat } from '../props';
import type { RoadPath } from './geoData';

export function miterStrip(pts: number[][], width: number, y: number): { positions: number[]; indices: number[] } {
  const positions: number[] = [];
  const indices: number[] = [];
  const half = width / 2;
  const n = pts.length;
  if (n < 2) return { positions, indices };
  for (let i = 0; i < n; i++) {
    const [px, pz] = pts[Math.max(0, i - 1)];
    const [cx, cz] = pts[i];
    const [nx2, nz2] = pts[Math.min(n - 1, i + 1)];
    let dx0 = cx - px;
    let dz0 = cz - pz;
    let dx1 = nx2 - cx;
    let dz1 = nz2 - cz;
    const l0 = Math.hypot(dx0, dz0) || 1;
    const l1 = Math.hypot(dx1, dz1) || 1;
    dx0 /= l0; dz0 /= l0; dx1 /= l1; dz1 /= l1;
    // averaged tangent → miter normal; scale = 1/cos(θ/2), capped at 60° kink
    const tx = dx0 + dx1;
    const tz = dz0 + dz1;
    const tl = Math.hypot(tx, tz);
    let mx: number;
    let mz: number;
    let scale = 1;
    if (tl < 1e-6) {
      mx = -dz0; mz = dx0; // 180° hairpin: fall back to segment normal
    } else {
      mx = -tz / tl; mz = tx / tl;
      const cosHalf = Math.max(0.5, mx * -dz0 + mz * dx0); // cap: ≤ 2× width spike
      scale = 1 / cosHalf;
    }
    positions.push(cx + mx * half * scale, y, cz + mz * half * scale, cx - mx * half * scale, y, cz - mz * half * scale);
    if (i > 0) {
      const a = (i - 1) * 2;
      indices.push(a, a + 2, a + 1, a + 1, a + 2, a + 3);
    }
  }
  return { positions, indices };
}

function stripsMesh(name: string, paths: RoadPath[], widthOf: (p: RoadPath) => number, color: number, y: number): THREE.Mesh {
  const positions: number[] = [];
  const indices: number[] = [];
  for (const p of paths) {
    const s = miterStrip(p.pts, widthOf(p), y);
    const base = positions.length / 3;
    positions.push(...s.positions);
    for (const i of s.indices) indices.push(base + i);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3));
  geo.setIndex(positions.length / 3 > 65535 ? new THREE.BufferAttribute(new Uint32Array(indices), 1) : new THREE.BufferAttribute(new Uint16Array(indices), 1));
  geo.computeVertexNormals();
  const mesh = new THREE.Mesh(geo, clayMat(color));
  mesh.name = name;
  mesh.receiveShadow = true;
  mesh.castShadow = false;
  return mesh;
}

const FOOT = new Set(['footway', 'path', 'cycleway', 'steps', 'pedestrian', 'track']);

export function buildRoads(roads: RoadPath[], rails: RoadPath[]): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityRoads';
  const carriage = roads.filter((r) => !FOOT.has(r.class));
  const foot = roads.filter((r) => FOOT.has(r.class));
  group.add(stripsMesh('carriageRibbons', carriage, (p) => p.width, kswCity.roadColors.carriage, kswCity.roadYs.carriage));
  group.add(stripsMesh('footwayRibbons', foot, (p) => p.width, kswCity.roadColors.footway, kswCity.roadYs.footway));
  group.add(stripsMesh('railBeds', rails, (p) => p.width + 2.2, kswCity.roadColors.railBed, kswCity.roadYs.railBed));
  group.add(stripsMesh('railRibbons', rails, (p) => p.width, kswCity.roadColors.rail, kswCity.roadYs.rail));
  return group;
}
```
designTokens `kswCity` um `roadColors`/`roadYs` erweitern (Werte aus Interfaces oben; die alten Einträge `roadY`/`railY` bleiben stehen — additiv, nature.ts nutzt weiter `greenY`/`waterY`).

- [ ] **Step 4: grün + Gates** — `npx vitest run && npm run typecheck`, Captures `t7-city`, `t7-bahnhof` (Read: ruhige Strassenzüge, kein Flackern/Keile, Gleisfeld liest sich als Gleisfeld mit Schotterband).

- [ ] **Step 5: Commit** — `git add src/diorama tests/geo && git commit -m "city(roads v2): miter-joined strips, class layers/colors, calm junctions"`

---

### Task 8: Bäume v2 — Originalform, Nadel-Variante, Impostor

**Files:**
- Modify: `src/diorama/ksw/geo/nature.ts`, Test: `tests/geo/nature-render.test.ts`

**Interfaces:**
- `buildNature` nutzt `trees: Array<{x,z,h,r,kind}>` (Task 4). Produziert 5 benannte Instanz-Meshes: `treeTrunks`, `treeCanopies` (Laub, Original-4-Puff-Form als EINE gemergte Geometrie), `treeConifers` (Kegel, 2-stufig), `treeImpostors` (Lowpoly-Ico wie bisher, für den Fern-Ring), Trunks gemeinsam. `treeCanopies/treeConifers` starten `visible=true`, `treeImpostors.visible=false` — Task 10 schaltet.

- [ ] **Step 1: Failing Tests (nature-render.test.ts: Baum-Fixture & Assertions ersetzen)**

```ts
const nature: CityNature = {
  greens: [], waterAreas: [], rivers: [],
  trees: [
    { x: 10, z: 10, h: 9, r: 3, kind: 'broad' },
    { x: 20, z: 15, h: 14, r: 2, kind: 'conifer' },
  ],
};
// …
  it('splits broadleaf/conifer/impostor instances with real sizes', () => {
    const g = buildNature(nature, {});
    const broad = g.getObjectByName('treeCanopies') as THREE.InstancedMesh;
    const conif = g.getObjectByName('treeConifers') as THREE.InstancedMesh;
    const imp = g.getObjectByName('treeImpostors') as THREE.InstancedMesh;
    expect(broad.count).toBe(1);
    expect(conif.count).toBe(1);
    expect(imp.count).toBe(2);
    expect(imp.visible).toBe(false);
    const m = new THREE.Matrix4();
    broad.getMatrixAt(0, m);
    const s = new THREE.Vector3();
    m.decompose(new THREE.Vector3(), new THREE.Quaternion(), s);
    expect(s.x).toBeCloseTo(3 / 0.75, 1); // canopy geo authored at 0.75 puff radius → scale = r/0.75
  });
```

- [ ] **Step 2: rot.**

- [ ] **Step 3: Implementation (nature.ts Baumteil ersetzen)**

```ts
// Original tree form (props.ts `tree()`): trunk + 4 clay puffs — merged into
// one canopy geometry so thousands instance in a single draw call. Conifers
// are a two-cone stack in the same vocabulary.
function broadCanopyGeometry(): THREE.BufferGeometry {
  const puffs: Array<[number, number, number, number]> = [
    [0, 0.5, 0, 0.75], [0.45, 0.15, 0.2, 0.48], [-0.4, 0.25, -0.18, 0.52], [0.1, 0.1, -0.42, 0.42],
  ];
  const geos = puffs.map(([x, y, z, r]) => {
    const g = new THREE.IcosahedronGeometry(r, 2);
    g.translate(x, y, z);
    return g;
  });
  return mergeGeos(geos); // BufferGeometryUtils.mergeGeometries — import from 'three/addons/utils/BufferGeometryUtils.js'
}
function coniferGeometry(): THREE.BufferGeometry {
  const a = new THREE.ConeGeometry(0.75, 1.4, 8);
  a.translate(0, 0.5, 0);
  const b = new THREE.ConeGeometry(0.55, 1.1, 8);
  b.translate(0, 1.15, 0);
  return mergeGeos([a, b]);
}
```
Instanzierung: broad scale `r/0.75`, Position y = Stammhöhe (h − r·1.6, min 1) + Krone; conifer scale x/z `r/0.75`, y `(h−1)/2.5`; Trunks: Höhe `min(2.2, h·0.22)`. Impostor: bisheriges Ico-Set über ALLE Bäume, `visible=false`. `setColorAt`-Tints beibehalten (broad wie gehabt; conifer Richtung `kswCity.woodGreen`).
Import oben: `import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';` und `const mergeGeos = (g: THREE.BufferGeometry[]) => mergeGeometries(g)!;`

- [ ] **Step 4: grün + Gates** — `npx vitest run && npm run typecheck`; Captures `t8-city`, `t8-bahnhof`, plus Hero `t8-overview` (Read: Bäume = chunky Original-Silhouetten, Nadelbäume im Lindberg-Wald erkennbar).

- [ ] **Step 5: Commit** — `git add src/diorama tests/geo && git commit -m "city(trees v2): original clay form, conifer variant, impostor set for LOD"`

---

### Task 9: Strassenlaternen (lamps.ts)

**Files:**
- Create: `src/diorama/ksw/geo/lamps.ts`
- Modify: `src/diorama/ksw/main.ts`
- Test: `tests/geo/lamps.test.ts`

**Interfaces:**
- `lampSpots(roads: RoadPath[]) → Array<{x,z}>`: pro Fahrbahn-Polyline (Klasse in `kswCityStyle.lamp.spacing`) alle `spacing` Meter entlang des echten Laufs, seitlicher Versatz `width/2 + sideOffset`, Seite alternierend; deterministisch.
- `buildLamps(roads, opts: { lampGlow: boolean }) → THREE.Group` mit `lampPosts` (Instanz: Mast+Kopf, Original-`lamppost()`-Maße) und `lampBulbs` (Instanz; nachts MeshBasic warm wie `glowNight`-Farbwelt 0xffe3b0, tags `castShadow=false`).

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/lamps.test.ts
import { describe, expect, it } from 'vitest';
import { lampSpots } from '../../src/diorama/ksw/geo/lamps';

describe('lampSpots', () => {
  it('spaces lamps along a residential road, alternating sides', () => {
    const spots = lampSpots([{ class: 'residential', width: 6, pts: [[0, 0], [120, 0]] }]);
    // 120 m / 35 m spacing → 4 lamps (t=0,35,70,105)
    expect(spots.length).toBe(4);
    expect(spots[0].z).toBeCloseTo(-(3 + 1.2));
    expect(spots[1].z).toBeCloseTo(3 + 1.2);
  });
  it('skips footways entirely', () => {
    expect(lampSpots([{ class: 'footway', width: 2.2, pts: [[0, 0], [100, 0]] }]).length).toBe(0);
  });
  it('is deterministic', () => {
    const r = [{ class: 'primary', width: 9, pts: [[0, 0], [50, 10], [90, 40]] }];
    expect(lampSpots(r)).toEqual(lampSpots(r));
  });
});
```

- [ ] **Step 2: rot.** — `npx vitest run tests/geo/lamps.test.ts`

- [ ] **Step 3: Implementation**

```ts
// src/diorama/ksw/geo/lamps.ts
// Street lamps along the REAL road polylines: class-based spacing, alternating
// sides — deterministic, no scattering. Night: warm bulbs like the original.
import * as THREE from 'three/webgpu';
import { kswCityStyle, palette } from '../../designTokens';
import { clayMat } from '../props';
import type { RoadPath } from './geoData';

export function lampSpots(roads: RoadPath[]): Array<{ x: number; z: number }> {
  const out: Array<{ x: number; z: number }> = [];
  for (const r of roads) {
    const spacing = kswCityStyle.lamp.spacing[r.class];
    if (!spacing) continue;
    const off = r.width / 2 + kswCityStyle.lamp.sideOffset;
    let travelled = 0;
    let next = 0;
    let side = 1;
    for (let i = 0; i < r.pts.length - 1; i++) {
      const [ax, az] = r.pts[i];
      const [bx, bz] = r.pts[i + 1];
      const dx = bx - ax;
      const dz = bz - az;
      const len = Math.hypot(dx, dz);
      if (len < 0.01) continue;
      while (next <= travelled + len) {
        const t = (next - travelled) / len;
        const nx = (-dz / len) * off * side;
        const nz = (dx / len) * off * side;
        out.push({ x: ax + dx * t + nx, z: az + dz * t + nz });
        side = -side;
        next += spacing;
      }
      travelled += len;
    }
  }
  return out;
}

export function buildLamps(roads: RoadPath[], opts: { lampGlow: boolean }): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityLamps';
  const spots = lampSpots(roads);
  // original lamppost proportions (props.ts): 2.9 m pole + head + bulb
  const pole = new THREE.CylinderGeometry(0.07, 0.1, 2.9, 6);
  pole.translate(0, 1.45, 0);
  const head = new THREE.CylinderGeometry(0.26, 0.34, 0.22, 8);
  head.translate(0, 2.98, 0);
  const posts = new THREE.InstancedMesh(pole, clayMat(palette.metalMatt), spots.length);
  posts.name = 'lampPosts';
  const heads = new THREE.InstancedMesh(head, clayMat(palette.metalDark), spots.length);
  heads.name = 'lampHeads';
  const bulbGeo = new THREE.SphereGeometry(0.15, 8, 6);
  bulbGeo.translate(0, 2.86, 0);
  const bulbs = new THREE.InstancedMesh(
    bulbGeo,
    opts.lampGlow ? new THREE.MeshBasicMaterial({ color: 0xffe3b0 }) : clayMat(palette.white),
    spots.length,
  );
  bulbs.name = 'lampBulbs';
  const m = new THREE.Matrix4();
  spots.forEach((s, i) => {
    m.makeTranslation(s.x, 0, s.z);
    posts.setMatrixAt(i, m);
    heads.setMatrixAt(i, m);
    bulbs.setMatrixAt(i, m);
  });
  for (const mesh of [posts, heads, bulbs]) {
    mesh.castShadow = false;
    mesh.receiveShadow = false;
    group.add(mesh);
  }
  return group;
}
```
main.ts: `cityRoot.add(buildLamps(cityRoads, { lampGlow: preset.lampOn }));`

- [ ] **Step 4: Gates** — `npx vitest run && npm run typecheck && node scripts/capture-ksw.mjs t9-night night bahnhof && node scripts/capture-ksw.mjs t9-night-city night city && node scripts/capture-ksw.mjs t9-overview morning overview`. Read: nachts Laternenketten entlang ECHTER Strassenzüge + Glühfenster — Stadt liest sich wie das Original bei Nacht; Hero morgens identisch.

- [ ] **Step 5: Commit** — `git add src/diorama tests/geo && git commit -m "city(lamps): instanced street lamps along real roads, warm night glow"`

---

### Task 10: LOD-Ringe (lod.ts)

**Files:**
- Create: `src/diorama/ksw/geo/lod.ts`
- Modify: `src/diorama/ksw/main.ts` (animate-Wiring), Test: `tests/geo/lod.test.ts`

**Interfaces:**
- `cityLodState(radius, prev: 'near'|'mid'|'far') → 'near'|'mid'|'far'` — Grenzen `kswCityStyle.lod.nearR/midR` mit ±`hysteresis`-Band (kein Flattern).
- `applyCityLod(ring, refs)` mit `refs = { windows: Group, lamps: Group, footways: Object3D, treesFull: Object3D[], treeImpostors: Object3D, treeCanopyShadow: (on: boolean) => void }` — Tabelle der Spec §2c: far: windows/lamps/footways aus, Impostor an; mid: windows/footways/lampPosts an (Glow an), Voll-Bäume an, Impostor aus, Baumschatten aus; near: alles an + Baumschatten an.

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/lod.test.ts
import { describe, expect, it } from 'vitest';
import { cityLodState } from '../../src/diorama/ksw/geo/lod';

describe('cityLodState', () => {
  it('classifies rings', () => {
    expect(cityLodState(100, 'near')).toBe('near');
    expect(cityLodState(300, 'near')).toBe('mid');
    expect(cityLodState(900, 'mid')).toBe('far');
  });
  it('hysteresis: no flip inside the band', () => {
    expect(cityLodState(155, 'near')).toBe('near'); // 150×1.1=165 upper band
    expect(cityLodState(166, 'near')).toBe('mid');
    expect(cityLodState(145, 'mid')).toBe('mid'); // 150×0.9=135 lower band
    expect(cityLodState(130, 'mid')).toBe('near');
  });
});
```

- [ ] **Step 2: rot.**

- [ ] **Step 3: Implementation**

```ts
// src/diorama/ksw/geo/lod.ts
// Semantic 3-ring LOD for the city (spec §2c): visibility + shadow policy by
// camera radius, with hysteresis so orbiting at a boundary never flickers.
import type * as THREE from 'three/webgpu';
import { kswCityStyle } from '../../designTokens';

export type CityLodRing = 'near' | 'mid' | 'far';

export function cityLodState(radius: number, prev: CityLodRing): CityLodRing {
  const { nearR, midR, hysteresis } = kswCityStyle.lod;
  const up = 1 + hysteresis;
  const dn = 1 - hysteresis;
  if (prev === 'near') return radius > nearR * up ? cityLodState(radius, 'mid') : 'near';
  if (prev === 'mid') {
    if (radius < nearR * dn) return 'near';
    return radius > midR * up ? 'far' : 'mid';
  }
  return radius < midR * dn ? cityLodState(radius, 'mid') : 'far';
}

export type CityLodRefs = {
  windows: THREE.Object3D;
  lamps: THREE.Object3D;
  footways: THREE.Object3D;
  treesFull: THREE.Object3D[];
  treeImpostors: THREE.Object3D;
  setTreeShadows: (on: boolean) => void;
};

export function applyCityLod(ring: CityLodRing, r: CityLodRefs): void {
  const far = ring === 'far';
  r.windows.visible = !far;
  r.lamps.visible = !far;
  r.footways.visible = !far;
  for (const t of r.treesFull) t.visible = !far;
  r.treeImpostors.visible = far;
  r.setTreeShadows(ring === 'near');
}
```
main.ts-Wiring (nach dem Aufbau von cityRoot):

```ts
  const lodRefs = {
    windows: cityRoot.getObjectByName('cityWindows')!,
    lamps: cityRoot.getObjectByName('cityLamps')!,
    footways: cityRoot.getObjectByName('footwayRibbons')!,
    treesFull: ['treeCanopies', 'treeConifers'].map((n) => cityRoot.getObjectByName(n)!),
    treeImpostors: cityRoot.getObjectByName('treeImpostors')!,
    setTreeShadows: (on: boolean) => {
      (cityRoot.getObjectByName('treeCanopies') as THREE.Object3D).castShadow = on;
      (cityRoot.getObjectByName('treeConifers') as THREE.Object3D).castShadow = on;
    },
  };
  let cityRing = cityLodState(rig.radius, 'far');
  applyCityLod(cityRing, lodRefs);
```
und im animate (bei den anderen radius-abhängigen Updates):

```ts
    const nextRing = cityLodState(rig.radius, cityRing);
    if (nextRing !== cityRing) {
      cityRing = nextRing;
      applyCityLod(cityRing, lodRefs);
      if (shadowCached) sun.shadow.needsUpdate = true;
    }
```

- [ ] **Step 4: Gates** — `npx vitest run && npm run typecheck && node scripts/smoke-ksw.mjs && node scratch/perf-measure.mjs city` (cpu.frame ≤ 12 ms hält; drawCalls im Fern-Ring gesunken). Captures `t10-city`, `t10-bahnhof`.

- [ ] **Step 5: Commit** — `git add src/diorama tests/geo && git commit -m "city(lod): 3-ring semantic LOD with hysteresis — detail follows the camera"`

---

### Task 11: Licht-Finale — Schatten-Follow, 2-Layer-Wolken, Stadt-Mist + Voll-Gate

**Files:**
- Modify: `src/diorama/ksw/main.ts`

**Interfaces:**
- Consumes: `sun` (DirectionalLight, Setup main.ts ~306-320), `cloudMatDome`/`driftU`/`sunDirUniform`-Rezept (~157-182), Mist-Ring-Block (~442-…), `kswCityStyle.cloudSwap`, `cityMeta.plate`.
- Verhalten: `radius ≤ 120` → Schattenkamera exakt heutige Werte (Extent 46, far 220, Zentrum Origin), Hero-Wolkenkuppel voll, Stadt-Wolken 0, Stadt-Mist 0 → **pixel-treu**. Darüber: Extent `min(46 + (radius−120)*0.9, 900)`, `sun.target` = rig.target (an scene hängen!), far mitskaliert (`220 + extent*2`), Refresh throttled (nur wenn sich Extent um >10% oder Target um >20 m geändert hat → `sun.shadow.needsUpdate = true` falls shadowCached, sonst automatisch); Hero-Wolken faden 300→600 aus, Stadt-Dome (Radius `kswCity.domeRadius`, gleiches Material-Rezept mit eigenem `driftU`-Anteil und `scale × 3`) faded ein; Stadt-Mist-Ring um `cityMeta.plate` mit eigenem Material, Opazität 0 bei radius<300, `preset.mistOpacity*0.8` ab 600.

- [ ] **Step 1: Schatten-Follow implementieren**

Nach dem Sun-Setup:

```ts
  scene.add(sun.target);
  let shadowExtentNow = kswScene.shadowExtent;
  let shadowTargetNow = new THREE.Vector3(0, 0, 0);
  const updateShadowFrustum = (): void => {
    const wantExtent = rig.radius <= 120
      ? kswScene.shadowExtent
      : Math.min(kswScene.shadowExtent + (rig.radius - 120) * 0.9, 900);
    const wantTarget = rig.radius <= 120 ? new THREE.Vector3(0, 0, 0) : new THREE.Vector3(...rig.target);
    const extentJump = Math.abs(wantExtent - shadowExtentNow) > shadowExtentNow * 0.1;
    const targetJump = wantTarget.distanceTo(shadowTargetNow) > 20;
    if (!extentJump && !targetJump) return;
    shadowExtentNow = wantExtent;
    shadowTargetNow = wantTarget;
    sun.shadow.camera.left = -wantExtent;
    sun.shadow.camera.right = wantExtent;
    sun.shadow.camera.top = wantExtent;
    sun.shadow.camera.bottom = -wantExtent;
    sun.shadow.camera.far = 220 + wantExtent * 2;
    sun.shadow.camera.updateProjectionMatrix();
    sun.target.position.copy(wantTarget);
    sun.position.copy(wantTarget).addScaledVector(currentSunDir, kswScene.sunDistance + wantExtent);
    if (shadowCached) sun.shadow.needsUpdate = true;
  };
```
`currentSunDir`: in `applySunState` die zuletzt gesetzte `dir` in einer Modul-Variablen `currentSunDir` merken (Init: `initialSunDir.clone()`); der bestehende `sun.position.copy(dir…)`-Pfad in `applySunState` bleibt für den Hero-Fall — `updateShadowFrustum()` läuft im animate NACH `applyRig()` und übersteuert nur im Stadt-Fall. City-Meshes: `cityRoot.traverse → castShadow` bleibt wie gebaut (Massing wirft, Fenster/Lampen nicht, Bäume via LOD).

- [ ] **Step 2: 2-Layer-Wolken**

Nach dem bestehenden cloudDome-Block:

```ts
  // city cloud layer: same recipe, big dome, coarser noise — takes over as
  // the hero dome fades out on zoom-out (spec: two-layer clouds)
  const cityCloudOpacity = uniform(0);
  const heroCloudOpacity = uniform(1);
  cloudMatDome.opacityNode = (cloudMatDome.opacityNode as ReturnType<typeof float>).mul(heroCloudOpacity);
  const cloudMatCity = new THREE.MeshBasicNodeMaterial();
  cloudMatCity.transparent = true;
  cloudMatCity.side = THREE.BackSide;
  cloudMatCity.depthWrite = false;
  cloudMatCity.fog = false;
  {
    const dir = positionWorld.normalize();
    const p = vec3(dir.x.mul(float(cloudCfg.scale * 3)).add(driftU), dir.y.mul(float(cloudCfg.scale * 4.8)), dir.z.mul(float(cloudCfg.scale * 3)));
    const n = mx_fractal_noise_float(p, 4, 2.0, 0.55, 1.0);
    const coverage = float(cloudCfg.coverage[presetName]);
    const dens = smoothstep(float(0.06), float(0.34), n.add(coverage.sub(0.5)));
    const horizonFade = smoothstep(float(0.0), float(0.07), dir.y);
    cloudMatCity.opacityNode = dens.mul(horizonFade).mul(cityCloudOpacity);
    const facing = dot(dir, sunDirUniform).mul(0.5).add(0.5);
    type Vec3Node = ReturnType<typeof vec3>;
    cloudMatCity.colorNode = mix(cloudShadow as unknown as Vec3Node, (cloudLit as unknown as Vec3Node).mul(float(cloudCfg.litBoost)), facing.pow(2.0));
  }
  const cityCloudDome = new THREE.Mesh(new THREE.SphereGeometry(kswCity.domeRadius, 32, 24), cloudMatCity);
  scene.add(cityCloudDome);
```
und im animate:

```ts
    const swap = kswCityStyle.cloudSwap;
    const cloudMix = Math.min(1, Math.max(0, (rig.radius - swap.start) / (swap.end - swap.start)));
    heroCloudOpacity.value = 1 - cloudMix;
    cityCloudOpacity.value = cloudMix;
```

- [ ] **Step 3: Stadt-Mist-Ring**

Den bestehenden Mist-Puff-Loop in eine lokale Funktion heben `addMistRing(halfW: number, halfD: number, cx: number, cz: number, mat: THREE.MeshBasicMaterial)` (Körper = exakt der heutige Loop, nur `mx+cx`/`mz+cz` und `mat` parametrisiert; Hero-Aufruf unverändert mit `(rimX, rimZ, 0, 0, mistMat)`). Zusätzlich:

```ts
  const cityMistMat = mistMat.clone();
  cityMistMat.opacity = 0;
  addMistRing(cityMeta.plate.w / 2 + 2.2, cityMeta.plate.d / 2 + 2.2, cityMeta.plate.cx, cityMeta.plate.cz, cityMistMat);
```
animate: `cityMistMat.opacity = preset.mistOpacity * 0.8 * cloudMix;` (gleiche 300→600-Rampe). Hero-`mistMat`-Zeile unverändert lassen.

- [ ] **Step 4: Voll-Gate (alle Ebenen)**

```bash
npx vitest run && npm run typecheck && npm run build && node scripts/smoke-ksw.mjs
node scratch/perf-measure.mjs overview && node scratch/perf-measure.mjs city
for c in overview er city bahnhof zag; do node scripts/capture-ksw.mjs final-$c morning $c; done
node scripts/capture-ksw.mjs final-night-city night city
node scripts/capture-ksw.mjs final-night-bahnhof night bahnhof
node scripts/capture-ksw.mjs final-dusk-city dusk city
```
Read-Bewertung gegen Original: `final-overview`/`final-er` pixel-treu zu `before-*`; `final-city` = Sonnenschatten über der Stadt, Wolken über der ganzen Platte, Mist am Stadtrand, ruhige Strassen, Diorama-Fassaden; Nacht = Laternen + Glühfenster. Jede Abweichung → fixen, neu capturen, DANN erst weiter.

- [ ] **Step 5: Commit + Push + PR-Update**

```bash
git add src/diorama
git commit -m "city(light): camera-following sun shadows, two-layer clouds, city mist rim"
git push
gh pr comment 113 --body "Style-Slice komplett: Diorama-Fassaden/Dächer/Strassen/Bäume/Laternen + Stadt-Licht (Schatten-Follow, 2-Layer-Wolken, Mist). Screenshots in artifacts/ksw/final-*."
```

---

## Selbst-Review-Notizen (bereits eingearbeitet)

- Spec §1→Task 2, §2→5+6, §2b→9, §2c→10, §2d→4+8, §3→7, §4→1+11, §5→1(+Gates in 10/11), §6→jeder Task. Keine Lücke.
- Typkonsistenz: `BakedBuilding.door`, `CityNature.trees`-Objekte (Task 4) werden von facade.ts (Task 6) und nature.ts (Task 8) exakt so konsumiert; `kswCityStyle` (Task 5) liefert alle in 6/9/10/11 benutzten Schlüssel; `miterStrip` exportiert für den Test.
- Reihenfolge zwingend: 1 → 2 → 3 → 4 (Bake-Schema) → 5…9 (Runtime, konsumieren Schema) → 10 (braucht 6/8/9-Gruppen) → 11.
