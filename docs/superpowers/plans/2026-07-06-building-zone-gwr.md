# Building Zone + GWR Enrichment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every baked Winterthur building carries its ÖREB Bauzone (allowed) + GWR category (is), persisted authoritatively in Supabase, and shown in an ultra-minimal hover card in the diorama.

**Architecture:** A new deterministic bake step (`geo:bake-attributes`) joins two pinned open-data sources against the already-baked building footprints in local plate metres, enriching `data/winterthur/buildings.json` in place and emitting `building-attributes.json`. The Rust sim-server ingests that artifact into a new Supabase table (source of truth) and serves it read-only. The client reads the baked fields through a single accessor (one-file pivot to the API later) and resolves hover picks via a per-vertex `buildingIdx` attribute + BVH-accelerated raycast on the merged city meshes.

**Tech Stack:** Node ESM scripts (no new script deps), vitest, Rust (axum + sqlx), Supabase Postgres, three.js r185 (`three/webgpu` + TSL), `three-mesh-bvh` (new dep), playwright smoke.

**Spec:** `docs/superpowers/specs/2026-07-06-building-zone-gwr-design.md`

## Global Constraints

- **Pinned data sources — NO fallback paths, fail loudly** (user directive "kein Fallback niemals Fallback"):
  - Bauzonen: WFS `https://maps.zh.ch/wfs/OGDZHWFS`, typename `ms:ogd-0156_arv_basis_np_gn_zonenflaeche_f` (ÖREB Nutzungsplanung Grundnutzung, rechtskräftig), `SRSNAME=urn:ogc:def:crs:EPSG::4326` (verified: returns CRS84 GeoJSON), filter client-side `Number(typ_bfsnr) === 230 && rechtsstatus === 'inKraft'` (230 = BFS-Nr Winterthur).
  - GWR: `https://public.madd.bfs.admin.ch/zh.zip` (verified HTTP 200), tab-separated `gebaeude_batiment_edificio.csv`, filter `GGDENR === '230' && GSTAT === '1004'`.
- **Every cargo invocation goes through `scripts/cargo-serial.sh`** (CLAUDE.md). Never two cargo at once. `pgrep -f cargo` before launching.
- **Coordinates:** all joins in local plate metres. Attribute data is projected with `makeProjector(ANCHOR).toLocal(lon, lat)` from `scripts/geo/lib/project.mjs` — never invert the plate transform, never join in degrees.
- **Supabase:** `DATABASE_URL` must use the `:5432` session pooler (never `:6543`). Writes only via the Rust backend; no service-role key exists or is added.
- **`tsconfig.json` does not type-check tests** — vitest runs them loose. Watch signatures manually when tests call production code.
- **Browser smoke is mandatory before claiming complete** (CLAUDE.md) — this feature adds a render-side picking path.
- Worktree `feat/building-zone-gwr` off `origin/main`; deliver via PR; never touch local `main`.
- Determinism: no `Date.now()`/randomness in bake outputs; stable tie-breaks (lowest EGID).

---

### Task 0: Preflight — dev stack readiness

**Files:** none created (environment check only)

**Interfaces:**
- Produces: a working dev stack where `ksw.html` reaches `__LOOK_READY` (needed by Tasks 8–10), and `scratch/geo/` exists for Task 2 outputs.

- [ ] **Step 1: Verify worktree + install**

```bash
cd .claude/worktrees/building-zone-gwr   # repo-relative; adjust to absolute path
git log --oneline -1                      # expect a387b96 or later on feat/building-zone-gwr
npm ci
npx playwright install chromium
```

- [ ] **Step 2: Ensure the world bake + symlink exist** (memory: fresh worktree boot hangs at `__LOOK_READY` without it)

```bash
ls data/winterthur/world/*.pb >/dev/null 2>&1 || { npm run geo:fetch && npm run geo:bake-world; }
[ -e public/winterthur-world ] || ln -s ../data/winterthur/world public/winterthur-world
mkdir -p scratch/geo
```

Note: `geo:fetch` is a large one-time download (swisstopo tiles + Overpass). If `data/winterthur/world/` already exists (long-lived checkout), skip.

- [ ] **Step 3: Baseline green**

```bash
npm run typecheck && npm test
```
Expected: PASS (this is `origin/main` + spec commits only).

---

### Task 1: Pure enrichment library (TDD)

**Files:**
- Create: `scripts/geo/lib/enrich.mjs`
- Test: `tests/geo/enrich.test.ts`

**Interfaces:**
- Consumes: nothing (pure functions).
- Produces (used by Tasks 2–3):
  - `lv95ToWgs84(e: number, n: number): { lon: number, lat: number }`
  - `pointInPolygon(pt: [number, number], ring: [number, number][]): boolean`
  - `centroid(ring: [number, number][]): [number, number]`
  - `joinBauzone(footprint, zones): { bauzone: string, bauzoneCode: string, zhCode: string } | null` — `zones: { ring: [number,number][], bauzone, bauzoneCode, zhCode }[]`, all in plate metres.
  - `joinGwr(footprint, points): { egid: number, gwrCategory: string, gwrClass: string | null, egids: number[] } | null` — `points: { x, z, egid, gkat, gklas }[]` in plate metres.
  - `GKAT_LABELS: Record<string, string>`

- [ ] **Step 1: Write the failing tests**

```ts
// tests/geo/enrich.test.ts
import { describe, expect, it } from 'vitest';
import {
  GKAT_LABELS, centroid, joinBauzone, joinGwr, lv95ToWgs84, pointInPolygon,
} from '../../scripts/geo/lib/enrich.mjs';

const square = (cx: number, cz: number, r: number): [number, number][] => [
  [cx - r, cz - r], [cx + r, cz - r], [cx + r, cz + r], [cx - r, cz + r], [cx - r, cz - r],
];

describe('lv95ToWgs84', () => {
  it('hits Winterthur HB within ~1 m', () => {
    // Reference: swisstopo NAVREF, Winterthur Hauptbahnhof
    const { lon, lat } = lv95ToWgs84(2696688.0, 1261945.0);
    expect(lon).toBeCloseTo(8.72385, 3);
    expect(lat).toBeCloseTo(47.50035, 3);
  });
});

describe('pointInPolygon', () => {
  const ring = square(0, 0, 10);
  it('inside / outside / concave-safe', () => {
    expect(pointInPolygon([0, 0], ring)).toBe(true);
    expect(pointInPolygon([11, 0], ring)).toBe(false);
    const lShape: [number, number][] = [[0,0],[10,0],[10,4],[4,4],[4,10],[0,10],[0,0]];
    expect(pointInPolygon([2, 8], lShape)).toBe(true);   // in the vertical arm
    expect(pointInPolygon([8, 8], lShape)).toBe(false);  // in the notch
  });
});

describe('centroid', () => {
  it('area centroid of an offset square', () => {
    expect(centroid(square(5, -3, 2))).toEqual([5, -3]);
  });
});

describe('joinBauzone', () => {
  const zones = [
    { ring: square(0, 0, 50), bauzone: 'Wohnzone W3', bauzoneCode: 'W3', zhCode: 'C1103' },
    { ring: square(200, 0, 50), bauzone: 'Gewerbezone 5.0', bauzoneCode: 'G5', zhCode: 'C1202' },
  ];
  it('centroid picks the containing zone', () => {
    expect(joinBauzone(square(10, 10, 5), zones)?.bauzoneCode).toBe('W3');
    expect(joinBauzone(square(210, 0, 5), zones)?.bauzoneCode).toBe('G5');
  });
  it('no containing zone → null', () => {
    expect(joinBauzone(square(1000, 1000, 5), zones)).toBeNull();
  });
});

describe('joinGwr', () => {
  const fp = square(0, 0, 10);
  it('single point inside', () => {
    const r = joinGwr(fp, [{ x: 1, z: 1, egid: 42, gkat: '1020', gklas: '1110' }]);
    expect(r).toEqual({ egid: 42, gwrCategory: GKAT_LABELS['1020'], gwrClass: '1110', egids: [42] });
  });
  it('dominant GKAT wins; tie → lowest EGID (deterministic)', () => {
    const r = joinGwr(fp, [
      { x: -1, z: 0, egid: 7, gkat: '1020', gklas: null },
      { x: 1, z: 0, egid: 5, gkat: '1020', gklas: null },
      { x: 0, z: 1, egid: 9, gkat: '1060', gklas: null },
    ]);
    expect(r?.gwrCategory).toBe(GKAT_LABELS['1020']); // 2× 1020 beats 1× 1060
    expect(r?.egid).toBe(5);                          // lowest EGID of the dominant class
    expect(r?.egids).toEqual([5, 7, 9]);              // sorted, all matches kept
  });
  it('no point inside → null', () => {
    expect(joinGwr(fp, [{ x: 100, z: 100, egid: 1, gkat: '1020', gklas: null }])).toBeNull();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
npx vitest run tests/geo/enrich.test.ts
```
Expected: FAIL — cannot resolve `scripts/geo/lib/enrich.mjs`.

- [ ] **Step 3: Implement `scripts/geo/lib/enrich.mjs`**

```js
// scripts/geo/lib/enrich.mjs
// Pure enrichment geometry for the building-attributes bake: LV95→WGS84,
// point-in-polygon, area centroid, and the two deterministic joins
// (ÖREB Bauzone via footprint centroid, GWR via building points inside the
// footprint). All join inputs are in LOCAL PLATE METRES — the caller projects
// attribute data with makeProjector().toLocal, never the other way around.

// swisstopo approximation formulas (EN→WGS84), accurate to ~1 m — far below
// parcel-polygon accuracy. Source: swisstopo "Näherungslösungen für die
// direkte Transformation LV95 ⇄ WGS84".
export function lv95ToWgs84(e, n) {
  const y = (e - 2600000) / 1000000;
  const x = (n - 1200000) / 1000000;
  const lonSec =
    2.6779094 + 4.728982 * y + 0.791484 * y * x + 0.1306 * y * x * x - 0.0436 * y * y * y;
  const latSec =
    16.9023892 + 3.238272 * x - 0.270978 * y * y - 0.002528 * x * x - 0.0447 * y * y * x -
    0.014 * x * x * x;
  return { lon: (lonSec * 100) / 36, lat: (latSec * 100) / 36 };
}

// Ray-casting point-in-polygon. Ring may or may not repeat its first vertex.
export function pointInPolygon([px, pz], ring) {
  let inside = false;
  const n = ring.length;
  for (let i = 0, j = n - 1; i < n; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    if (zi > pz !== zj > pz && px < ((xj - xi) * (pz - zi)) / (zj - zi) + xi) inside = !inside;
  }
  return inside;
}

// Area-weighted centroid (shoelace). Degenerate ring → vertex average.
export function centroid(ring) {
  let a = 0;
  let cx = 0;
  let cz = 0;
  const n = ring.length;
  for (let i = 0, j = n - 1; i < n; j = i++) {
    const cross = ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
    a += cross;
    cx += (ring[j][0] + ring[i][0]) * cross;
    cz += (ring[j][1] + ring[i][1]) * cross;
  }
  if (Math.abs(a) < 1e-9) {
    let sx = 0;
    let sz = 0;
    for (const [x, z] of ring) {
      sx += x;
      sz += z;
    }
    return [sx / n, sz / n];
  }
  return [cx / (3 * a), cz / (3 * a)];
}

// GWR Gebäudekategorie (GKAT) labels, Merkmalskatalog 4.2.
export const GKAT_LABELS = {
  1010: 'Provisorische Unterkunft',
  1020: 'Gebäude mit ausschliesslicher Wohnnutzung',
  1030: 'Wohngebäude mit Nebennutzung',
  1040: 'Gebäude mit teilweiser Wohnnutzung',
  1060: 'Gebäude ohne Wohnnutzung',
  1080: 'Sonderbau',
};

// Bauzone via footprint centroid: the centroid's containing zone wins
// (spec: building spanning >1 zone → centroid decides). No zone → null.
export function joinBauzone(footprint, zones) {
  const c = centroid(footprint);
  for (const z of zones) {
    if (pointInPolygon(c, z.ring)) {
      return { bauzone: z.bauzone, bauzoneCode: z.bauzoneCode, zhCode: z.zhCode };
    }
  }
  return null;
}

// GWR: every point inside the footprint counts; dominant GKAT wins, ties
// break to the LOWEST EGID (determinism). No point inside → null.
export function joinGwr(footprint, points) {
  const hits = points.filter((p) => pointInPolygon([p.x, p.z], footprint));
  if (hits.length === 0) return null;
  const byKat = new Map();
  for (const h of hits) {
    if (!byKat.has(h.gkat)) byKat.set(h.gkat, []);
    byKat.get(h.gkat).push(h);
  }
  let best = null;
  for (const [gkat, group] of byKat) {
    group.sort((a, b) => a.egid - b.egid);
    if (!best || group.length > best.group.length ||
        (group.length === best.group.length && group[0].egid < best.group[0].egid)) {
      best = { gkat, group };
    }
  }
  const primary = best.group[0];
  return {
    egid: primary.egid,
    gwrCategory: GKAT_LABELS[best.gkat] ?? `GKAT ${best.gkat}`,
    gwrClass: primary.gklas ?? null,
    egids: hits.map((h) => h.egid).sort((a, b) => a - b),
  };
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
npx vitest run tests/geo/enrich.test.ts
```
Expected: PASS (all suites).

- [ ] **Step 5: Commit**

```bash
git add scripts/geo/lib/enrich.mjs tests/geo/enrich.test.ts
git commit -m "feat(geo): pure enrichment lib — LV95→WGS84, PIP joins for Bauzone + GWR"
```

---

### Task 2: `geo:fetch-attributes` — pinned sources, fail loudly

**Files:**
- Create: `scripts/geo/fetch-attributes.mjs`
- Modify: `package.json` (scripts block: add `"geo:fetch-attributes": "node scripts/geo/fetch-attributes.mjs"`)

**Interfaces:**
- Consumes: `lv95ToWgs84` from `scripts/geo/lib/enrich.mjs`.
- Produces (read by Task 3):
  - `scratch/geo/bauzonen.geojson` — GeoJSON FeatureCollection, WGS84, Winterthur only, `properties: { bauzone, bauzoneCode, zhCode }`.
  - `scratch/geo/gwr-buildings.json` — `{ buildings: [{ egid: number, lon: number, lat: number, gkat: string, gklas: string|null }] }`.

- [ ] **Step 1: Write the script**

```js
// scripts/geo/fetch-attributes.mjs
// Network-only step (like fetch-winterthur.mjs): pulls the two PINNED
// attribute sources for the Gemeinde Winterthur (BFS-Nr 230) into
// scratch/geo/. NO fallback sources — a broken pin fails loudly.
//   1. ÖREB Nutzungsplanung Grundnutzung (rechtskräftig), Kanton ZH OGD WFS
//   2. GWR Gebäudedaten, BFS open data (public.madd)
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { lv95ToWgs84 } from './lib/enrich.mjs';

const OUT = 'scratch/geo';
mkdirSync(OUT, { recursive: true });
const BFS_NR_WINTERTHUR = 230;

// ---- 1. Bauzonen: WFS, paginated, WGS84 output, client-side Gemeinde filter
const WFS = 'https://maps.zh.ch/wfs/OGDZHWFS';
const TYPENAME = 'ms:ogd-0156_arv_basis_np_gn_zonenflaeche_f';
// Gemeinde bbox (WGS84, same extent as fetch-winterthur GEMEINDE_BBOX);
// urn:...:EPSG::4326 axis order is lat,lon.
const BBOX = '47.44,8.63,47.57,8.81,urn:ogc:def:crs:EPSG::4326';
const PAGE = 2000;

async function fetchZones() {
  const features = [];
  for (let start = 0; ; start += PAGE) {
    const url =
      `${WFS}?SERVICE=WFS&VERSION=2.0.0&REQUEST=GetFeature&TYPENAMES=${TYPENAME}` +
      `&SRSNAME=urn:ogc:def:crs:EPSG::4326&BBOX=${BBOX}` +
      `&COUNT=${PAGE}&STARTINDEX=${start}` +
      `&OUTPUTFORMAT=${encodeURIComponent('application/json; subtype=geojson')}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`bauzonen WFS ${res.status} at STARTINDEX=${start}`);
    const fc = await res.json();
    const page = fc.features ?? [];
    features.push(...page);
    if (page.length < PAGE) break;
  }
  const zones = features.filter(
    (f) =>
      Number(f.properties.typ_bfsnr) === BFS_NR_WINTERTHUR &&
      f.properties.rechtsstatus === 'inKraft' &&
      f.geometry,
  );
  if (zones.length < 50)
    throw new Error(`bauzonen: only ${zones.length} inKraft zones for BFS ${BFS_NR_WINTERTHUR} — pin broken?`);
  const out = {
    type: 'FeatureCollection',
    features: zones.map((f) => ({
      type: 'Feature',
      geometry: f.geometry, // Polygon, WGS84 lon/lat
      properties: {
        bauzone: f.properties.typ_gde_bezeichnung,
        bauzoneCode: f.properties.typ_gde_abkuerzung,
        zhCode: f.properties.typ_zh_code,
      },
    })),
  };
  writeFileSync(`${OUT}/bauzonen.geojson`, JSON.stringify(out));
  console.log(`bauzonen: ${zones.length} zones (inKraft, Winterthur)`);
}

// ---- 2. GWR: BFS madd open data, canton ZH ZIP, tab-separated CSV
const GWR_URL = 'https://public.madd.bfs.admin.ch/zh.zip';

async function fetchGwr() {
  const zipPath = `${OUT}/gwr-zh.zip`;
  if (!existsSync(zipPath)) {
    const res = await fetch(GWR_URL);
    if (!res.ok) throw new Error(`GWR download ${res.status} — pin broken?`);
    writeFileSync(zipPath, Buffer.from(await res.arrayBuffer()));
  }
  execFileSync('unzip', ['-o', zipPath, 'gebaeude_batiment_edificio.csv', '-d', OUT]);
  const csv = readFileSync(`${OUT}/gebaeude_batiment_edificio.csv`, 'utf8');
  const lines = csv.split('\n');
  const header = lines[0].replace(/\r$/, '').split('\t');
  const col = Object.fromEntries(header.map((h, i) => [h, i]));
  for (const required of ['EGID', 'GGDENR', 'GSTAT', 'GKAT', 'GKLAS', 'GKODE', 'GKODN'])
    if (!(required in col)) throw new Error(`GWR CSV missing column ${required} — format changed?`);
  const buildings = [];
  for (let i = 1; i < lines.length; i++) {
    const f = lines[i].replace(/\r$/, '').split('\t');
    if (f.length < header.length) continue; // trailing blank line
    if (f[col.GGDENR] !== String(BFS_NR_WINTERTHUR)) continue;
    if (f[col.GSTAT] !== '1004') continue; // 1004 = bestehend
    const e = Number(f[col.GKODE]);
    const n = Number(f[col.GKODN]);
    if (!Number.isFinite(e) || !Number.isFinite(n) || e === 0) continue; // no coordinate → unjoinable
    const { lon, lat } = lv95ToWgs84(e, n);
    buildings.push({
      egid: Number(f[col.EGID]),
      lon,
      lat,
      gkat: f[col.GKAT],
      gklas: f[col.GKLAS] || null,
    });
  }
  if (buildings.length < 5000)
    throw new Error(`GWR: only ${buildings.length} existing buildings for Winterthur — pin/filter broken?`);
  writeFileSync(`${OUT}/gwr-buildings.json`, JSON.stringify({ buildings }));
  console.log(`gwr: ${buildings.length} bestehende Gebäude (Winterthur)`);
}

await fetchZones();
await fetchGwr();
```

- [ ] **Step 2: Add the npm script**

In `package.json` scripts, after `"geo:fetch-demand"`:
```json
"geo:fetch-attributes": "node scripts/geo/fetch-attributes.mjs"
```

- [ ] **Step 3: Run it (network) and eyeball the outputs**

```bash
npm run geo:fetch-attributes
python3 -c "
import json
z = json.load(open('scratch/geo/bauzonen.geojson')); g = json.load(open('scratch/geo/gwr-buildings.json'))
print('zones:', len(z['features']), '| sample:', z['features'][0]['properties'])
print('gwr:', len(g['buildings']), '| sample:', g['buildings'][0])
"
```
Expected: `zones:` a few hundred (Winterthur has ~400–900 zone polygons); `gwr:` > 10'000; samples show German zone labels and plausible lon/lat around 8.7/47.5. If either gate throws, STOP and report — do not add a fallback.

- [ ] **Step 4: Commit** (script only — `scratch/` is gitignored)

```bash
git add scripts/geo/fetch-attributes.mjs package.json
git commit -m "feat(geo): fetch-attributes — pinned ÖREB Grundnutzung WFS + BFS GWR pulls for Winterthur"
```

---

### Task 3: `geo:bake-attributes` — deterministic enrichment bake

**Files:**
- Create: `scripts/geo/bake-attributes.mjs`
- Modify: `package.json` (add `"geo:bake-attributes": "node scripts/geo/bake-attributes.mjs"`)
- Modify (data): `data/winterthur/buildings.json` (4 new fields per building), create `data/winterthur/building-attributes.json`

**Interfaces:**
- Consumes: `scratch/geo/bauzonen.geojson`, `scratch/geo/gwr-buildings.json` (Task 2); `makeProjector, ANCHOR` from `scripts/geo/lib/project.mjs`; joins from `scripts/geo/lib/enrich.mjs`.
- Produces (read by Tasks 5 and 7):
  - Each building in `data/winterthur/buildings.json` gains `egid: number|null, gwrCategory: string|null, bauzone: string|null, bauzoneCode: string|null`.
  - `data/winterthur/building-attributes.json`:
    ```json
    { "worldId": "winterthur", "buildings": [
      { "id": "{UUID}", "egid": 150404, "gwrCategory": "Gebäude mit ausschliesslicher Wohnnutzung",
        "gwrClass": "1110", "bauzone": "Wohnzone W3", "bauzoneCode": "W3",
        "raw": { "egids": [150404], "zhCode": "C1103" } }
    ] }
    ```

- [ ] **Step 1: Write the script**

```js
// scripts/geo/bake-attributes.mjs
// Deterministic enrichment bake: joins the fetched attribute data
// (scratch/geo/{bauzonen.geojson,gwr-buildings.json}) against the ALREADY
// BAKED footprints in data/winterthur/buildings.json — in local plate metres,
// via the same projector as every other bake. Idempotent: re-running
// overwrites the four attribute fields and building-attributes.json.
import { readFileSync, writeFileSync } from 'node:fs';
import { ANCHOR, makeProjector } from './lib/project.mjs';
import { joinBauzone, joinGwr } from './lib/enrich.mjs';

const projector = makeProjector(ANCHOR);

const buildingsPath = 'data/winterthur/buildings.json';
const doc = JSON.parse(readFileSync(buildingsPath, 'utf8'));

// Zones: outer ring only (Grundnutzung holes are inner courtyards of the SAME
// zone's complement — the centroid test tolerates this; a centroid inside a
// hole belongs to whichever zone the hole cuts to, and those cases are logged).
const zonesFc = JSON.parse(readFileSync('scratch/geo/bauzonen.geojson', 'utf8'));
// Polygon AND MultiPolygon (the WFS mostly emits Polygon, but not always):
// every outer ring becomes one zone entry carrying the same properties.
const zones = zonesFc.features.flatMap((f) => {
  const polys =
    f.geometry.type === 'MultiPolygon' ? f.geometry.coordinates
    : f.geometry.type === 'Polygon' ? [f.geometry.coordinates]
    : [];
  if (polys.length === 0) throw new Error(`bauzonen: unexpected geometry ${f.geometry.type}`);
  return polys.map((rings) => ({
    ring: rings[0].map(([lon, lat]) => projector.toLocal(lon, lat)),
    bauzone: f.properties.bauzone,
    bauzoneCode: f.properties.bauzoneCode,
    zhCode: f.properties.zhCode,
  }));
});

const gwrDoc = JSON.parse(readFileSync('scratch/geo/gwr-buildings.json', 'utf8'));
const gwrPoints = gwrDoc.buildings.map((b) => {
  const [x, z] = projector.toLocal(b.lon, b.lat);
  return { x, z, egid: b.egid, gkat: b.gkat, gklas: b.gklas };
});

let zoned = 0;
let gwred = 0;
const attributes = [];
for (const b of doc.buildings) {
  const zone = joinBauzone(b.footprint, zones);
  const gwr = joinGwr(b.footprint, gwrPoints);
  b.bauzone = zone?.bauzone ?? null;
  b.bauzoneCode = zone?.bauzoneCode ?? null;
  b.egid = gwr?.egid ?? null;
  b.gwrCategory = gwr?.gwrCategory ?? null;
  if (zone) zoned++;
  if (gwr) gwred++;
  attributes.push({
    id: b.id,
    egid: gwr?.egid ?? null,
    gwrCategory: gwr?.gwrCategory ?? null,
    gwrClass: gwr?.gwrClass ?? null,
    bauzone: zone?.bauzone ?? null,
    bauzoneCode: zone?.bauzoneCode ?? null,
    raw: { egids: gwr?.egids ?? [], zhCode: zone?.zhCode ?? null },
  });
}

// Coverage gates — fail loudly, never ship a silently-empty join.
const n = doc.buildings.length;
console.log(`bauzone: ${zoned}/${n} (${((100 * zoned) / n).toFixed(1)}%)`);
console.log(`gwr:     ${gwred}/${n} (${((100 * gwred) / n).toFixed(1)}%)`);
if (zoned / n < 0.85) throw new Error(`bake-attributes: bauzone coverage ${zoned}/${n} < 85% — projection or fetch broken`);
if (gwred / n < 0.5) throw new Error(`bake-attributes: GWR coverage ${gwred}/${n} < 50% — projection or fetch broken`);

writeFileSync(buildingsPath, JSON.stringify(doc));
writeFileSync(
  'data/winterthur/building-attributes.json',
  JSON.stringify({ worldId: 'winterthur', buildings: attributes }),
);
console.log(`wrote ${buildingsPath} + data/winterthur/building-attributes.json`);
```

- [ ] **Step 2: Add the npm script**

```json
"geo:bake-attributes": "node scripts/geo/bake-attributes.mjs"
```

- [ ] **Step 3: Run + sanity-check known buildings**

```bash
npm run geo:bake-attributes
python3 -c "
import json
doc = json.load(open('data/winterthur/buildings.json'))
named = [b for b in doc['buildings'] if b.get('name')]
for b in named[:8]:
    print(f\"{b['name'][:34]:34} | {str(b.get('gwrCategory'))[:40]:40} | {b.get('bauzone')}\")
"
```
Expected: coverage gates pass (≥85% / ≥50%); named buildings show plausible pairs (e.g. a school → `Gebäude ohne Wohnnutzung` + `Zone für öffentliche Bauten…`, Wohnhäuser → `…Wohnnutzung` + `Wohnzone…` / `Zentrumszone…`). If a spot-check looks systematically wrong (e.g. everything null on one side of town), STOP — that smells like an axis/projection bug, debug before committing.

- [ ] **Step 4: Verify determinism (idempotent re-run)**

```bash
shasum data/winterthur/building-attributes.json > /tmp/attr.sha
npm run geo:bake-attributes
shasum -c /tmp/attr.sha
```
Expected: `OK` — byte-identical on re-run.

- [ ] **Step 5: Run the full frontend test suite** (bake changed a committed artifact other tests import)

```bash
npm run typecheck && npm test
```
Expected: PASS — existing tests don't assert on unknown-field absence; if one does, fix the test expectation (the field addition is intended).

- [ ] **Step 6: Commit (script + enriched data)**

```bash
git add scripts/geo/bake-attributes.mjs package.json data/winterthur/buildings.json data/winterthur/building-attributes.json
git commit -m "feat(geo): bake-attributes — Bauzone + GWR joined onto every baked building"
```

---

### Task 4: Supabase table + Rust store (opt-in PG test)

**Files:**
- Create: `backend/crates/sim-server/migrations/202607060001_building_attributes.sql`
- Create: `backend/crates/sim-server/src/building_attributes.rs`
- Modify: `backend/crates/sim-server/src/lib.rs` (add `pub mod building_attributes;`)

**Interfaces:**
- Consumes: `connect_shared_pool` (`db.rs`), migration-in-module pattern (`card_hand.rs`).
- Produces (used by Tasks 5–6):
  ```rust
  pub struct BuildingAttributes { pub building_id: String, pub egid: Option<i64>,
      pub gwr_category: Option<String>, pub gwr_class: Option<String>,
      pub bauzone: Option<String>, pub bauzone_code: Option<String>, pub raw: serde_json::Value }
  impl BuildingAttributesStore {
      pub async fn with_pool(pool: PgPool) -> Result<Self, sqlx::Error>; // runs migration
      pub fn memory() -> Self;                                           // dev/no-DB mode
      pub async fn upsert_all(&self, world_id: &str, rows: &[BuildingAttributes]) -> Result<u64, sqlx::Error>;
      pub async fn list(&self, world_id: &str) -> Result<Vec<BuildingAttributes>, sqlx::Error>;
  }
  ```

- [ ] **Step 1: Write the migration**

```sql
-- backend/crates/sim-server/migrations/202607060001_building_attributes.sql
-- Authoritative per-building enrichment: ÖREB Bauzone (allowed) + GWR (is).
-- Seeded from data/winterthur/building-attributes.json via
-- `sim-server load-building-attributes` (writes only through DATABASE_URL).
CREATE TABLE IF NOT EXISTS building_attributes (
  world_id      TEXT        NOT NULL,
  building_id   TEXT        NOT NULL,   -- swissBUILDINGS3D UUID
  egid          BIGINT,
  gwr_category  TEXT,
  gwr_class     TEXT,
  bauzone       TEXT,
  bauzone_code  TEXT,
  raw           JSONB       NOT NULL DEFAULT '{}'::jsonb,
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (world_id, building_id)
);
-- Open data, safe to read publicly; writes only via the direct Postgres
-- connection (sqlx), never via PostgREST/anon.
ALTER TABLE building_attributes ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS building_attributes_public_read ON building_attributes;
CREATE POLICY building_attributes_public_read ON building_attributes FOR SELECT USING (true);
```

- [ ] **Step 2: Write the store with a failing-first opt-in PG test**

`backend/crates/sim-server/src/building_attributes.rs`:

```rust
//! Per-building enrichment store: ÖREB Bauzone (allowed) + GWR category (is).
//! Supabase is the source of truth; `data/winterthur/building-attributes.json`
//! is the deterministic bake artifact that seeds it. Mirrors the CardHandStore
//! shape: Postgres in production, in-memory in the no-DATABASE_URL dev mode.
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use sqlx::PgPool;

const BUILDING_ATTRIBUTES_MIGRATION: &str =
    include_str!("../migrations/202607060001_building_attributes.sql");

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildingAttributes {
    #[serde(rename = "id")]
    pub building_id: String,
    pub egid: Option<i64>,
    pub gwr_category: Option<String>,
    pub gwr_class: Option<String>,
    pub bauzone: Option<String>,
    pub bauzone_code: Option<String>,
    #[serde(default)]
    pub raw: serde_json::Value,
}

/// The bake artifact (data/winterthur/building-attributes.json).
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildingAttributesFile {
    pub world_id: String,
    pub buildings: Vec<BuildingAttributes>,
}

#[derive(Clone)]
pub struct BuildingAttributesStore(Inner);

#[derive(Clone)]
enum Inner {
    Postgres(PgPool),
    Memory(Arc<RwLock<HashMap<String, Vec<BuildingAttributes>>>>),
}

impl BuildingAttributesStore {
    /// Production: runs the migration, then serves reads/writes from Postgres.
    pub async fn with_pool(pool: PgPool) -> Result<Self, sqlx::Error> {
        for statement in BUILDING_ATTRIBUTES_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement).execute(&pool).await?;
        }
        Ok(Self(Inner::Postgres(pool)))
    }

    pub fn memory() -> Self {
        Self(Inner::Memory(Arc::new(RwLock::new(HashMap::new()))))
    }

    pub async fn upsert_all(
        &self,
        world_id: &str,
        rows: &[BuildingAttributes],
    ) -> Result<u64, sqlx::Error> {
        match &self.0 {
            Inner::Postgres(pool) => {
                // One round-trip per batch via UNNEST (SOTA bulk upsert, no per-row loop).
                let mut total = 0u64;
                for chunk in rows.chunks(1000) {
                    let (mut ids, mut egids, mut cats, mut classes, mut zones, mut codes, mut raws) =
                        (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
                    for r in chunk {
                        ids.push(r.building_id.clone());
                        egids.push(r.egid);
                        cats.push(r.gwr_category.clone());
                        classes.push(r.gwr_class.clone());
                        zones.push(r.bauzone.clone());
                        codes.push(r.bauzone_code.clone());
                        raws.push(r.raw.clone());
                    }
                    let done = sqlx::query(
                        r#"INSERT INTO building_attributes
                             (world_id, building_id, egid, gwr_category, gwr_class, bauzone, bauzone_code, raw, updated_at)
                           SELECT $1, u.building_id, u.egid, u.gwr_category, u.gwr_class, u.bauzone, u.bauzone_code, u.raw, now()
                           FROM UNNEST($2::text[], $3::int8[], $4::text[], $5::text[], $6::text[], $7::text[], $8::jsonb[])
                             AS u(building_id, egid, gwr_category, gwr_class, bauzone, bauzone_code, raw)
                           ON CONFLICT (world_id, building_id) DO UPDATE SET
                             egid = EXCLUDED.egid, gwr_category = EXCLUDED.gwr_category,
                             gwr_class = EXCLUDED.gwr_class, bauzone = EXCLUDED.bauzone,
                             bauzone_code = EXCLUDED.bauzone_code, raw = EXCLUDED.raw,
                             updated_at = now()"#,
                    )
                    .bind(world_id)
                    .bind(&ids)
                    .bind(&egids)
                    .bind(&cats)
                    .bind(&classes)
                    .bind(&zones)
                    .bind(&codes)
                    .bind(&raws)
                    .execute(pool)
                    .await?;
                    total += done.rows_affected();
                }
                Ok(total)
            }
            Inner::Memory(map) => {
                let mut guard = map.write().expect("building_attributes lock poisoned");
                let entry = guard.entry(world_id.to_string()).or_default();
                for r in rows {
                    entry.retain(|e| e.building_id != r.building_id);
                    entry.push(r.clone());
                }
                Ok(rows.len() as u64)
            }
        }
    }

    pub async fn list(&self, world_id: &str) -> Result<Vec<BuildingAttributes>, sqlx::Error> {
        match &self.0 {
            Inner::Postgres(pool) => {
                sqlx::query_as::<_, (String, Option<i64>, Option<String>, Option<String>, Option<String>, Option<String>, serde_json::Value)>(
                    "SELECT building_id, egid, gwr_category, gwr_class, bauzone, bauzone_code, raw
                     FROM building_attributes WHERE world_id = $1 ORDER BY building_id",
                )
                .bind(world_id)
                .fetch_all(pool)
                .await
                .map(|rows| {
                    rows.into_iter()
                        .map(|(building_id, egid, gwr_category, gwr_class, bauzone, bauzone_code, raw)| BuildingAttributes {
                            building_id, egid, gwr_category, gwr_class, bauzone, bauzone_code, raw,
                        })
                        .collect()
                })
            }
            Inner::Memory(map) => {
                let guard = map.read().expect("building_attributes lock poisoned");
                let mut rows = guard.get(world_id).cloned().unwrap_or_default();
                rows.sort_by(|a, b| a.building_id.cmp(&b.building_id));
                Ok(rows)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> BuildingAttributes {
        BuildingAttributes {
            building_id: id.to_string(),
            egid: Some(42),
            gwr_category: Some("Gebäude mit ausschliesslicher Wohnnutzung".into()),
            gwr_class: Some("1110".into()),
            bauzone: Some("Wohnzone W3".into()),
            bauzone_code: Some("W3".into()),
            raw: serde_json::json!({"egids": [42]}),
        }
    }

    #[tokio::test]
    async fn memory_upsert_and_list_roundtrip() {
        let store = BuildingAttributesStore::memory();
        store.upsert_all("winterthur", &[sample("{B}"), sample("{A}")]).await.unwrap();
        // idempotent overwrite
        store.upsert_all("winterthur", &[sample("{A}")]).await.unwrap();
        let rows = store.list("winterthur").await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].building_id, "{A}"); // sorted
        assert!(store.list("other").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn postgres_upsert_and_list_roundtrip() {
        // Opt-in (same pattern as db.rs): set ABUTOWN_TEST_DATABASE_URL to run.
        let Ok(url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else { return };
        let pool = crate::db::connect_shared_pool(&url).await.expect("connect");
        let store = BuildingAttributesStore::with_pool(pool).await.expect("migrate");
        let world = format!("test-{}", std::process::id());
        store.upsert_all(&world, &[sample("{A}"), sample("{B}")]).await.unwrap();
        store.upsert_all(&world, &[sample("{B}")]).await.unwrap(); // upsert path
        let rows = store.list(&world).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].bauzone_code.as_deref(), Some("W3"));
    }
}
```

Add to `backend/crates/sim-server/src/lib.rs` alongside the existing modules:
```rust
pub mod building_attributes;
```

- [ ] **Step 3: Run the scoped tests (serialized cargo)**

```bash
pgrep -f cargo || true   # clear orphans first per CLAUDE.md
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server building_attributes
```
Expected: `memory_upsert_and_list_roundtrip` PASS; the PG test silently returns (env unset). If `.env` has a reachable `DATABASE_URL`, optionally run once with `ABUTOWN_TEST_DATABASE_URL` set to it to exercise the real path.

- [ ] **Step 4: fmt + clippy**

```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-server -- -D warnings
```
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/migrations/202607060001_building_attributes.sql \
        backend/crates/sim-server/src/building_attributes.rs backend/crates/sim-server/src/lib.rs
git commit -m "feat(server): building_attributes table + store — Supabase source of truth, RLS public-read"
```

---

### Task 5: Ingest subcommand + `geo:load-attributes`

**Files:**
- Modify: `backend/crates/sim-server/src/main.rs` (arg branch at the top of `main`, right after `dotenvy::dotenv()`)
- Modify: `backend/crates/sim-server/src/building_attributes.rs` (add `load_from_file`)
- Modify: `package.json` (add `"geo:load-attributes"`)

**Interfaces:**
- Consumes: `BuildingAttributesStore::{with_pool, upsert_all}`, `BuildingAttributesFile` (Task 4); `connect_shared_pool` (db.rs); `data/winterthur/building-attributes.json` (Task 3).
- Produces: `sim-server load-building-attributes <path>` CLI; populated `building_attributes` rows in Supabase.

- [ ] **Step 1: Add `load_from_file` to `building_attributes.rs`**

```rust
/// One-shot ingest: parse the bake artifact and upsert it into Postgres.
/// Explicitly invoked (never on boot) so the load stays auditable.
/// Requires DATABASE_URL — this is a persistence command, no in-memory mode.
pub async fn load_from_file(path: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    let url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL is required for load-building-attributes")?;
    let text = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let file: BuildingAttributesFile = serde_json::from_str(&text).context("parse artifact")?;
    let pool = crate::db::connect_shared_pool(&url).await.context("connect")?;
    let store = BuildingAttributesStore::with_pool(pool).await.context("migrate")?;
    let n = store.upsert_all(&file.world_id, &file.buildings).await.context("upsert")?;
    println!(
        "building_attributes: upserted {n} rows for world '{}' from {path}",
        file.world_id
    );
    Ok(())
}
```

- [ ] **Step 2: Add the subcommand branch in `main.rs`**

Directly after `let _ = dotenvy::dotenv();` in `main()`:

```rust
// One-shot admin subcommands (no daemon boot). Unknown commands fail loudly.
let mut cli_args = std::env::args().skip(1);
if let Some(cmd) = cli_args.next() {
    return match cmd.as_str() {
        "load-building-attributes" => {
            let path = cli_args
                .next()
                .context("usage: sim-server load-building-attributes <path>")?;
            sim_server::building_attributes::load_from_file(&path).await
        }
        other => anyhow::bail!("unknown subcommand {other:?}"),
    };
}
```

- [ ] **Step 3: Add the npm script** (cargo through the serializer, per CLAUDE.md)

```json
"geo:load-attributes": "scripts/cargo-serial.sh run --manifest-path backend/Cargo.toml -p sim-server -- load-building-attributes data/winterthur/building-attributes.json"
```

- [ ] **Step 4: Build + run the ingest against the real Supabase** (`.env` provides `DATABASE_URL` on `:5432` + `PGSSLROOTCERT`)

```bash
pgrep -f cargo || true
npm run geo:load-attributes
```
Expected: `building_attributes: upserted 846 rows for world 'winterthur' from data/winterthur/building-attributes.json`.

- [ ] **Step 5: Verify rows in Supabase**

Spot-check with the same `DATABASE_URL` the ingest used (source it from `.env`):
```bash
psql "$DATABASE_URL" -c "SELECT count(*), count(bauzone), count(gwr_category) FROM building_attributes WHERE world_id='winterthur';"
```
Expected: `846 | ≥720 | ≥423` (mirrors the 85%/50% bake gates). If `psql` is not installed, run the opt-in PG test against the same URL instead:
```bash
ABUTOWN_TEST_DATABASE_URL="$DATABASE_URL" scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server building_attributes
```

- [ ] **Step 6: fmt + clippy + commit**

```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-server -- -D warnings
git add backend/crates/sim-server/src/main.rs backend/crates/sim-server/src/building_attributes.rs package.json
git commit -m "feat(server): load-building-attributes subcommand + geo:load-attributes alias"
```

---

### Task 6: Read endpoint `GET /building-attributes`

**Files:**
- Modify: `backend/crates/sim-server/src/app/mod.rs` (AppState + route + handler)
- Modify: `backend/crates/sim-server/src/main.rs` (pass the store into app construction — follow how `CardHandStore` flows)
- Test: inline `#[cfg(test)]` in `app/mod.rs` or the crate's existing app-test location (mirror where `/cards` is tested)

**Interfaces:**
- Consumes: `BuildingAttributesStore::{with_pool, memory, list}` (Task 4).
- Produces: `GET /building-attributes?world_id=winterthur` → `200 application/json`, body `[{ "id": "...", "egid": 150404, "gwrCategory": "...", "gwrClass": "...", "bauzone": "...", "bauzoneCode": "...", "raw": {...} }]` (serde camelCase from Task 4's struct). Unauthenticated (public open data), CORS-guarded like the rest.

- [ ] **Step 1: Extend AppState + router**

In `app/mod.rs`:
- Add field `building_attributes: BuildingAttributesStore` to `AppState`; extend `AppState::new(card_hands, auth, health, building_attributes)`.
- `build_app_with_shared_pool`: `let building_attributes = BuildingAttributesStore::with_pool(pool.clone()).await?;` (clone the pool BEFORE `CardHandStore::with_pool(pool)` consumes it) and pass through.
- `build_app_with_card_hands` (test/dev wiring): use `BuildingAttributesStore::memory()`.
- Route: `.route("/building-attributes", get(building_attributes))`.
- Handler:

```rust
#[derive(serde::Deserialize)]
struct BuildingAttributesQuery {
    world_id: String,
}

async fn building_attributes(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<BuildingAttributesQuery>,
) -> Result<Json<Vec<crate::building_attributes::BuildingAttributes>>, axum::http::StatusCode> {
    state
        .building_attributes
        .list(&q.world_id)
        .await
        .map(Json)
        .map_err(|err| {
            tracing::error!("building_attributes list failed: {err}");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })
}
```

- [ ] **Step 2: Write the failing test first** (find where `/cards` or `/health` is integration-tested — same file/pattern; if inline `#[cfg(test)]` in app/mod.rs, put it there)

```rust
#[tokio::test]
async fn building_attributes_endpoint_lists_world_rows() {
    use tower::ServiceExt; // mirror the crate's existing router-test imports
    let app = build_app(); // in-memory wiring
    // Seeding: build_app's memory store starts empty → expect 200 + []
    let res = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/building-attributes?world_id=winterthur")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], b"[]");
}
```
Adapt imports to the crate's existing router-test conventions (check how existing route tests do `oneshot`; reuse their helper if one exists). If a seeded-store variant is easy (construct `AppState` with a memory store pre-filled via `upsert_all`), add a second assertion that a seeded row round-trips with camelCase keys (`"gwrCategory"`, `"bauzoneCode"`).

- [ ] **Step 3: Run tests — fail, implement Step 1, run again — pass**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server
```
Expected: full sim-server suite PASS (AppState signature change touches other constructors — fix all call sites, the compiler will list them).

- [ ] **Step 4: fmt + clippy + commit**

```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-server -- -D warnings
git add backend/crates/sim-server/src/app/mod.rs backend/crates/sim-server/src/main.rs
git commit -m "feat(server): GET /building-attributes — public read of the enrichment table"
```

---

### Task 7: Client types + accessor

**Files:**
- Modify: `src/diorama/ksw/geo/geoData.ts` (extend `BakedBuilding`)
- Create: `src/diorama/ksw/geo/buildingAttributes.ts`
- Test: `tests/geo/buildingAttributes.test.ts`

**Interfaces:**
- Consumes: enriched `data/winterthur/buildings.json` (Task 3), `cityBuildings, kswBuildings` from `geoData.ts`.
- Produces (used by Task 9):
  ```ts
  export type BuildingHoverInfo = {
    name?: string;
    gwrCategory: string | null;
    bauzone: string | null;
    bauzoneCode: string | null;
  };
  export function getBuildingHoverInfo(id: string): BuildingHoverInfo | undefined;
  ```

- [ ] **Step 1: Extend `BakedBuilding` in `geoData.ts`**

```ts
export type BakedBuilding = {
  id: string;
  name?: string;
  usage?: string;
  zone: 'ksw' | 'city';
  // Enrichment (geo:bake-attributes): ÖREB Grundnutzung + GWR. Null = no join
  // hit (e.g. shed without EGID, footprint outside a Bauzone) — shown as such,
  // never guessed.
  egid?: number | null;
  gwrCategory?: string | null;
  bauzone?: string | null;
  bauzoneCode?: string | null;
  footprint: number[][];
  height: number;
  // Real eave height (m, 1 decimal) — the facade shader clamps windows below it.
  eaveH: number;
  wall: BakedWallMesh;
  roof: BakedMesh;
  door?: BakedDoor;
};
```

- [ ] **Step 2: Write the failing test**

```ts
// tests/geo/buildingAttributes.test.ts
import { describe, expect, it } from 'vitest';
import { getBuildingHoverInfo } from '../../src/diorama/ksw/geo/buildingAttributes';
import { cityBuildings, kswBuildings } from '../../src/diorama/ksw/geo/geoData';

describe('getBuildingHoverInfo', () => {
  it('resolves every baked building id', () => {
    for (const b of [...cityBuildings, ...kswBuildings]) {
      expect(getBuildingHoverInfo(b.id)).toBeDefined();
    }
  });
  it('carries the enrichment through (coverage gates re-asserted client-side)', () => {
    const all = [...cityBuildings, ...kswBuildings];
    const zoned = all.filter((b) => getBuildingHoverInfo(b.id)?.bauzone).length;
    const gwred = all.filter((b) => getBuildingHoverInfo(b.id)?.gwrCategory).length;
    expect(zoned / all.length).toBeGreaterThan(0.85);
    expect(gwred / all.length).toBeGreaterThan(0.5);
  });
  it('unknown id → undefined', () => {
    expect(getBuildingHoverInfo('{NOPE}')).toBeUndefined();
  });
});
```

- [ ] **Step 3: Run — fail; implement; run — pass**

```ts
// src/diorama/ksw/geo/buildingAttributes.ts
// Single accessor for per-building enrichment (Bauzone erlaubt / GWR ist).
// TODAY: resolves from the baked static artifact already in the bundle —
// zero new wire. LATER (Vercel+Fly cutover): swap this body to fetch+cache
// GET /building-attributes from VITE_ABUTOWN_BACKEND_URL; the return shape
// is identical, so callers never change.
import { type BakedBuilding, cityBuildings, kswBuildings } from './geoData';

export type BuildingHoverInfo = {
  name?: string;
  gwrCategory: string | null;
  bauzone: string | null;
  bauzoneCode: string | null;
};

const byId = new Map<string, BakedBuilding>();
for (const b of [...cityBuildings, ...kswBuildings]) byId.set(b.id, b);

export function getBuildingHoverInfo(id: string): BuildingHoverInfo | undefined {
  const b = byId.get(id);
  if (!b) return undefined;
  return {
    name: b.name,
    gwrCategory: b.gwrCategory ?? null,
    bauzone: b.bauzone ?? null,
    bauzoneCode: b.bauzoneCode ?? null,
  };
}
```

```bash
npx vitest run tests/geo/buildingAttributes.test.ts && npm run typecheck
```
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/diorama/ksw/geo/geoData.ts src/diorama/ksw/geo/buildingAttributes.ts tests/geo/buildingAttributes.test.ts
git commit -m "feat(client): BakedBuilding enrichment fields + getBuildingHoverInfo accessor"
```

---

### Task 8: Pickable merged city — `buildingIdx` attribute + BVH raycast

**Files:**
- Modify: `package.json` (dependency: `"three-mesh-bvh": "^0.9.1"` — latest 0.9.x, three r185-compatible)
- Modify: `src/diorama/ksw/geo/cityMassing.ts` (`mergeTinted` writes a `buildingIdx` vertex attribute)
- Create: `src/diorama/ksw/hoverPick.ts`
- Test: extend `tests/geo/cityMassing.test.ts` + create `tests/geo/hoverPick.test.ts`

**Interfaces:**
- Consumes: `mergeTinted(buildings, pick, base)` (existing export), `BakedBuilding`.
- Produces (used by Task 9):
  ```ts
  export function createHoverPicker(opts: {
    camera: THREE.Camera;
    meshes: THREE.Mesh[];       // merged wall + roof meshes, geometry carries buildingIdx
    buildings: BakedBuilding[]; // SAME array + order used for the merge
  }): { pick(ndcX: number, ndcY: number): BakedBuilding | null };
  ```
- **Critical invariant:** the `buildingIdx` written during merge indexes into the exact `buildings` array passed to `createHoverPicker`. `cityMassing` must build both meshes from the same `cityBuildings` array in the same order (it already does — verify, don't assume).

- [ ] **Step 1: Add the dependency**

```bash
npm install three-mesh-bvh@^0.9.1
```
(If 0.9.x peer-conflicts with three 0.185, use the newest version whose peer range includes it — check `npm view three-mesh-bvh peerDependencies`. Do NOT downgrade three.)

- [ ] **Step 2: Failing test — merged geometry carries `buildingIdx`**

Append to `tests/geo/cityMassing.test.ts` (reuse its existing fixture helpers for `BakedBuilding`s; if it has none, build two minimal buildings whose `wall`/`roof` are one triangle each):

```ts
it('mergeTinted writes a per-vertex buildingIdx attribute', () => {
  const tri = (o: number): BakedMesh => ({ pos: [o, 0, 0, o + 1, 0, 0, o, 1, 0], idx: [0, 1, 2] });
  const b = (id: string, o: number): BakedBuilding => ({
    id, zone: 'city', footprint: [[o, 0], [o + 1, 0], [o, 1]], height: 3, eaveH: 3,
    wall: { ...tri(o), fuv: [0, 0, 0, 0, 0, 0] }, roof: tri(o),
  });
  const geo = mergeTinted([b('{A}', 0), b('{B}', 10)], (x) => x.roof, 0xffffff);
  const idx = geo.getAttribute('buildingIdx');
  expect(idx).toBeDefined();
  expect(idx.getX(0)).toBe(0);  // first building's vertices
  expect(idx.getX(3)).toBe(1);  // second building's vertices
});
```

- [ ] **Step 3: Run — fail; implement in `mergeTinted`**

In `cityMassing.ts` `mergeTinted`, alongside the existing position/color fill loop, allocate `const buildingIdx = new Float32Array(vtx);` and, while copying building `i`'s vertices, fill its vertex range with `i`. After the existing `setAttribute` calls:

```ts
geometry.setAttribute('buildingIdx', new THREE.BufferAttribute(buildingIdx, 1));
```
(Match the file's existing attribute-set style exactly — read the function before editing; the vertex-offset bookkeeping already exists for positions.)

- [ ] **Step 4: Failing test — picker resolves a building through a merged mesh**

```ts
// tests/geo/hoverPick.test.ts
// Raycast is pure CPU math — runs headless in vitest. This test ALSO guards
// the three/webgpu ↔ three-mesh-bvh class-identity interop: if the BVH
// prototype patch missed the Mesh class the diorama uses, pick() returns null
// and this fails.
import * as THREE from 'three/webgpu';
import { describe, expect, it } from 'vitest';
import type { BakedBuilding, BakedMesh } from '../../src/diorama/ksw/geo/geoData';
import { mergeTinted } from '../../src/diorama/ksw/geo/cityMassing';
import { createHoverPicker } from '../../src/diorama/ksw/hoverPick';

const flatQuad = (cx: number, cz: number): BakedMesh => ({
  // 1×1 horizontal quad at y=5 centred on (cx, cz)
  pos: [cx - 0.5, 5, cz - 0.5, cx + 0.5, 5, cz - 0.5, cx + 0.5, 5, cz + 0.5, cx - 0.5, 5, cz + 0.5],
  idx: [0, 1, 2, 0, 2, 3],
});
const building = (id: string, cx: number, cz: number): BakedBuilding => ({
  id, zone: 'city', footprint: [[cx - 0.5, cz - 0.5], [cx + 0.5, cz - 0.5], [cx + 0.5, cz + 0.5], [cx - 0.5, cz + 0.5]],
  height: 5, eaveH: 5,
  wall: { ...flatQuad(cx, cz), fuv: new Array(8).fill(0) }, roof: flatQuad(cx, cz),
});

describe('createHoverPicker', () => {
  const buildings = [building('{A}', 0, 0), building('{B}', 20, 0)];
  const mesh = new THREE.Mesh(mergeTinted(buildings, (b) => b.roof, 0xffffff));
  const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 1000);
  camera.position.set(0, 100, 0);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  const picker = createHoverPicker({ camera, meshes: [mesh], buildings });

  it('straight down onto {A} at screen centre', () => {
    expect(picker.pick(0, 0)?.id).toBe('{A}');
  });
  it('empty sky → null', () => {
    expect(picker.pick(0.9, 0.9)).toBeNull();
  });
});
```

- [ ] **Step 5: Run — fail; implement `hoverPick.ts`**

```ts
// src/diorama/ksw/hoverPick.ts
// BVH-accelerated building picking on the MERGED city meshes (one wall mesh +
// one roof mesh for ~800 buildings — per-mesh userData can't identify a
// building, so the merge carries a per-vertex buildingIdx attribute instead;
// see cityMassing.mergeTinted). three-mesh-bvh makes raycasting the ~240k-tri
// merge O(log n): the boundsTree is built once per geometry, firstHitOnly.
import * as THREE from 'three/webgpu';
import { acceleratedRaycast, computeBoundsTree, disposeBoundsTree } from 'three-mesh-bvh';
import type { BakedBuilding } from './geo/geoData';

// Patch once at module load. three r185 shares core classes between 'three'
// and 'three/webgpu' (three.core), so the prototype patch reaches both; the
// hoverPick unit test guards this interop.
THREE.BufferGeometry.prototype.computeBoundsTree = computeBoundsTree;
THREE.BufferGeometry.prototype.disposeBoundsTree = disposeBoundsTree;
THREE.Mesh.prototype.raycast = acceleratedRaycast;

export function createHoverPicker({
  camera,
  meshes,
  buildings,
}: {
  camera: THREE.Camera;
  meshes: THREE.Mesh[];
  buildings: BakedBuilding[];
}): { pick(ndcX: number, ndcY: number): BakedBuilding | null } {
  for (const mesh of meshes) {
    if (!mesh.geometry.boundsTree) mesh.geometry.computeBoundsTree();
  }
  const raycaster = new THREE.Raycaster();
  raycaster.firstHitOnly = true;
  const ndc = new THREE.Vector2();
  return {
    pick(ndcX: number, ndcY: number): BakedBuilding | null {
      ndc.set(ndcX, ndcY);
      raycaster.setFromCamera(ndc, camera);
      const hits = raycaster.intersectObjects(meshes, false);
      const hit = hits[0];
      if (!hit || !hit.face) return null;
      const attr = (hit.object as THREE.Mesh).geometry.getAttribute('buildingIdx');
      if (!attr) return null;
      const idx = attr.getX(hit.face.a);
      return buildings[idx] ?? null;
    },
  };
}
```
Type note: `three-mesh-bvh` ships its own d.ts that augments `BufferGeometry`/`Raycaster` — if `tsc` complains about `boundsTree`/`firstHitOnly`, ensure the package's types are picked up via the import (they are, from the named imports); do NOT `as any` around it.

- [ ] **Step 6: Run all affected tests + typecheck**

```bash
npx vitest run tests/geo/cityMassing.test.ts tests/geo/hoverPick.test.ts && npm run typecheck
```
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add package.json package-lock.json src/diorama/ksw/geo/cityMassing.ts src/diorama/ksw/hoverPick.ts \
        tests/geo/cityMassing.test.ts tests/geo/hoverPick.test.ts
git commit -m "feat(render): per-vertex buildingIdx in the city merge + BVH hover picker"
```

---

### Task 9: Ultra-minimal hover card + main.ts wiring

**Files:**
- Create: `src/diorama/ksw/hoverCard.ts`
- Modify: `src/diorama/ksw/main.ts` (pointermove → per-frame pick → card)

**Interfaces:**
- Consumes: `createHoverPicker` (Task 8), `getBuildingHoverInfo` (Task 7), the `cityWalls`/roof mesh variables and `cityBuildings` already present in `main.ts` (grep `cityWalls` there; pass the SAME buildings array cityMassing merged).
- Produces: `createHoverCard(): { show(info: BuildingHoverInfo, clientX: number, clientY: number): void; hide(): void }`.

- [ ] **Step 1: Implement the card**

```ts
// src/diorama/ksw/hoverCard.ts
// Ultra-minimal building info card: GWR category (what it IS) over the ÖREB
// Bauzone (what's ALLOWED). Two lines + optional name. DOM-injected,
// pointer-events: none, follows the cursor with a small offset.
import type { BuildingHoverInfo } from './geo/buildingAttributes';

export function createHoverCard(): {
  show(info: BuildingHoverInfo, clientX: number, clientY: number): void;
  hide(): void;
} {
  const el = document.createElement('div');
  el.style.cssText = [
    'position:fixed', 'display:none', 'pointer-events:none', 'z-index:30',
    'padding:6px 9px', 'border-radius:3px',
    'background:rgba(24,26,30,0.82)', 'backdrop-filter:blur(2px)',
    'color:#e9ede1', 'font:11px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace',
    'letter-spacing:0.01em', 'white-space:nowrap',
  ].join(';');
  document.body.appendChild(el);
  return {
    show(info, clientX, clientY) {
      const name = info.name ? `<div style="font-weight:600">${escapeHtml(info.name)}</div>` : '';
      const ist = escapeHtml(info.gwrCategory ?? 'Nutzung unbekannt');
      const erlaubt = info.bauzone
        ? `${escapeHtml(info.bauzone)} · erlaubt`
        : 'keine Bauzone';
      el.innerHTML = `${name}<div>${ist}</div><div style="opacity:0.72">${erlaubt}</div>`;
      el.style.left = `${clientX + 14}px`;
      el.style.top = `${clientY + 14}px`;
      el.style.display = 'block';
    },
    hide() {
      el.style.display = 'none';
    },
  };
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}
```

- [ ] **Step 2: Wire into `main.ts`**

Read the surrounding code first (pointer handlers live around lines 374–408; the render loop and `cityWalls` exist — match local style). The shape:

```ts
import { createHoverCard } from './hoverCard';
import { createHoverPicker } from './hoverPick';
import { getBuildingHoverInfo } from './geo/buildingAttributes';

// after the city meshes exist:
const hoverPicker = createHoverPicker({
  camera,
  meshes: [cityWalls, cityRoofs],       // use the actual local variable names
  buildings: cityBuildings,             // SAME array the merge consumed
});
const hoverCard = createHoverCard();
let hoverEvent: PointerEvent | null = null;

renderer.domElement.addEventListener('pointermove', (e: PointerEvent) => {
  hoverEvent = e;
});
renderer.domElement.addEventListener('pointerleave', () => {
  hoverEvent = null;
  hoverCard.hide();
});

// in the render loop (once per frame, NOT per pointermove — raycast throttle):
if (hoverEvent) {
  const r = renderer.domElement.getBoundingClientRect();
  const ndcX = ((hoverEvent.clientX - r.left) / r.width) * 2 - 1;
  const ndcY = -(((hoverEvent.clientY - r.top) / r.height) * 2 - 1);
  const hit = isDragging ? null : hoverPicker.pick(ndcX, ndcY); // don't fight camera drag — use the file's actual drag flag
  const info = hit ? getBuildingHoverInfo(hit.id) : undefined;
  if (info) hoverCard.show(info, hoverEvent.clientX, hoverEvent.clientY);
  else hoverCard.hide();
  hoverEvent = null;
}
```
`isDragging`: `main.ts` tracks a drag state for the right-drag camera (find the actual variable in the pointerdown/up handlers and use it). Suppress hover during drag.

- [ ] **Step 3: Typecheck + full frontend tests**

```bash
npm run typecheck && npm test
```
Expected: PASS.

- [ ] **Step 4: Manual dev-server sighting** (pre-smoke sanity)

```bash
npm run dev
```
Open `https://127.0.0.1:5173/ksw.html` (whatever entry `main.ts` serves — check `vite.config`/`ksw.html`), hover a few buildings: card appears near cursor with two lines, disappears over sky/roads, doesn't flicker during camera drag. Then stop the server.

- [ ] **Step 5: Commit**

```bash
git add src/diorama/ksw/hoverCard.ts src/diorama/ksw/main.ts
git commit -m "feat(render): ultra-minimal ist/erlaubt hover card on building pick"
```

---

### Task 10: Browser smoke + full CI gate + PR

**Files:**
- Create: `scripts/smoke-hover.mjs`
- Modify: `docs/superpowers/specs/2026-07-06-building-zone-gwr-design.md` only if reality diverged (record deltas, don't rewrite history)

**Interfaces:**
- Consumes: the running dev stack (Task 0 world bake + symlink), the hover card DOM from Task 9.
- Produces: a repeatable smoke proving the feature end-to-end in a real browser.

- [ ] **Step 1: Write the smoke** (mirror `scripts/smoke-ksw.mjs` for stack boot + `__LOOK_READY` wait + WebGPU chromium flags — read it first and reuse its helpers/idioms)

```js
// scripts/smoke-hover.mjs
// Browser smoke (CLAUDE.md-mandatory): hover a building → the ist/erlaubt
// card appears with non-empty GWR + Bauzone lines; hover the sky → it hides.
// Scans a coarse grid from the canvas centre outward until a card shows —
// deterministic for a fixed camera boot pose.
import { chromium } from 'playwright';
// …boot the vite dev stack exactly like smoke-ksw.mjs does (reuse its
// start/stop helper from scripts/lib/ if one exists)…

const browser = await chromium.launch({
  headless: true,
  args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'],
});
const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, ignoreHTTPSErrors: true });
await page.goto(URL_KSW); // same URL constant/env the other smokes use
await page.waitForFunction(() => window.__LOOK_READY === true, null, { timeout: 120_000 });

const card = () => page.evaluate(() => {
  const els = [...document.querySelectorAll('div')].filter(
    (d) => d.style.position === 'fixed' && d.style.display === 'block' && d.textContent.includes('erlaubt'),
  );
  return els[0]?.textContent ?? null;
});

// scan a grid around centre until a building hover registers
let hit = null;
outer: for (let dy = 0; dy <= 200; dy += 40) {
  for (let dx = -300; dx <= 300; dx += 60) {
    await page.mouse.move(640 + dx, 400 + dy);
    await page.waitForTimeout(120); // give the rAF-throttled pick a frame
    hit = await card();
    if (hit) break outer;
  }
}
if (!hit) throw new Error('smoke-hover: no hover card appeared anywhere in the scan grid');
if (!/erlaubt|keine Bauzone/.test(hit)) throw new Error(`smoke-hover: card text unexpected: ${hit}`);
console.log(`smoke-hover: card OK → ${hit.slice(0, 120)}`);

await page.mouse.move(10, 10); // top-left sky
await page.waitForTimeout(200);
if (await card()) throw new Error('smoke-hover: card did not hide over empty sky');
console.log('smoke-hover: hides over sky OK');
await browser.close();
// …stop the dev stack…
process.exit(0);
```
Adapt the stack boot/teardown and URL to what `smoke-ksw.mjs` actually does — copy its working pattern, don't invent a new one.

- [ ] **Step 2: Run the smoke**

```bash
node scripts/smoke-hover.mjs
```
Expected output: `smoke-hover: card OK → …erlaubt…` and `hides over sky OK`. **A green vitest suite is NOT a substitute for this step** (CLAUDE.md).

- [ ] **Step 3: Full CI gate before push** (cargo serialized, background for the slow ones)

```bash
pgrep -f cargo || true
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace
npm run typecheck && npm test && npm run build
node scripts/smoke-hover.mjs
```
Expected: all green. Fix anything red before proceeding — report failures honestly.

- [ ] **Step 4: Commit smoke, push, open PR**

```bash
git add scripts/smoke-hover.mjs
git commit -m "test(smoke): browser hover smoke — card shows ist/erlaubt, hides over sky"
git push -u origin feat/building-zone-gwr
gh pr create --title "Building zone + GWR enrichment: Supabase-persisted, ist/erlaubt hover card" --body "$(cat <<'EOF'
## Summary
- `geo:fetch-attributes` + `geo:bake-attributes`: every baked Winterthur building gains its ÖREB Bauzone (erlaubt) + GWR category (ist), joined deterministically in plate metres from pinned open-data sources (no fallbacks)
- Supabase `building_attributes` table (RLS public-read) as source of truth, seeded via `sim-server load-building-attributes` / `npm run geo:load-attributes`; `GET /building-attributes` read endpoint
- BVH-accelerated hover picking on the merged city meshes (per-vertex `buildingIdx`) + ultra-minimal ist/erlaubt hover card
- Spec: docs/superpowers/specs/2026-07-06-building-zone-gwr-design.md

## Test plan
- [ ] vitest: enrich joins, accessor coverage gates, buildingIdx merge, headless raycast pick
- [ ] cargo: store round-trip (memory + opt-in PG), endpoint 200/shape
- [ ] browser smoke `scripts/smoke-hover.mjs`: card shows non-empty ist/erlaubt, hides over sky
- [ ] Supabase: 846 rows upserted, count(bauzone) ≥ 85%

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```
Then wait for CI to be **fully green** (not "not red" — no UNSTABLE) before merging, per memory. After merge: delete the branch + worktree.
