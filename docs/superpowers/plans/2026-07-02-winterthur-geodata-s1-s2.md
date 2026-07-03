# Winterthur-Geodaten S1+S2 Implementation Plan (Bake-Pipeline + Stadt-Massing)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Echte Winterthur-Stadt (swisstopo LoD2 + OSM) als Clay-Massing um das bestehende KSW-Diorama, KSW-Anker als Ursprung — Look pixel-treu.

## Execution-Briefing (für eine frische Session, kein Vorkontext nötig)

- **Branch:** `geo/winterthur-map` (Basis `origin/main` @ df8d9d4). Worktree:
  `.claude/worktrees/kind-elgamal-746d1f`. Nach Unterbrechungen `pwd` prüfen.
- **Reihenfolge:** Tasks strikt 1→10; jeder Task ist in sich abgeschlossen
  (Test → rot → Code → grün → Commit). Kein Task braucht Wissen ausserhalb
  seines eigenen Blocks + „Global Constraints“.
- **Alle Fakten sind verifiziert** (URLs, Layer-Namen, Attribut-Falle
  `GESAMTHOEHE` leer, 844 Gebäude in der Bbox, `ogr2ogr` unter
  `/opt/anaconda3/bin`) — nicht neu recherchieren, nicht anzweifeln; bei
  echtem Widerspruch (z. B. 404) stoppen und melden statt improvisieren.
- **Nichts umdesignen.** Wenn ein Test aus dem Plan scheitert, zuerst die
  eigene Umsetzung gegen den Plan-Code diffen — der Plan-Code ist gegen die
  echten Daten entworfen.
- **Look-Regel (hart):** an bestehenden Werten in `designTokens.ts`, an
  `look.ts`, `staticBatch.ts`, `clayNodes.ts` NICHTS ändern; nur anfügen.
- **Gates:** `npx vitest run <testfile>` pro Task; am Ende (Task 10)
  `npx tsc --noEmit && npx vitest run && npm run build`, dann
  `node scripts/smoke-ksw.mjs` + `node scripts/capture-ksw.mjs`
  (Browser-Smoke ist Pflicht, CLAUDE.md). Kein cargo involviert.
- **Vorher-Referenz fürs Look-Gate:** vor Task 10 einmal auf `origin/main`
  (`git stash` unnötig — eigener Worktree, einfach `git worktree` nicht
  wechseln, sondern die Captures VOR dem main.ts-Edit ziehen).

**Architecture:** Offline-Bake (`scripts/geo/`) lädt die swisstopo-Kachel + OSM, projiziert auf lokale Meter (Ursprung = KSW 47.5069/8.7285) und schreibt kompakte JSONs nach `data/winterthur/`. Die Runtime (`src/diorama/ksw/geo/`) baut daraus wenige gemergte Meshes (Wände/Dächer/Strassen) mit den **bestehenden** Clay-Materialien. Spec: `docs/superpowers/specs/2026-07-02-winterthur-geodata-design.md`.

**Tech Stack:** Node .mjs-Skripte, `ogr2ogr` (lokal vorhanden, `/opt/anaconda3/bin`), three.js `ShapeUtils` (Triangulation), Vitest, bestehende Diorama-Builder.

## Global Constraints

- **Look pixel-treu:** bestehende Werte in `src/diorama/designTokens.ts` NICHT ändern — nur neue Token-Blöcke/Einträge ANFÜGEN. `look.ts`, Post-Stack, `staticBatch.ts`, `clayNodes.ts` unangetastet.
- Ursprung (0,0,0) = KSW-Anker `{lon: 8.7285, lat: 47.5069}`; +x = Ost, +z = Süd, y = Höhe.
- Bbox: lon 8.7150–8.7300, lat 47.4955–47.5085.
- Baked-Artefakte werden committet; Rohdaten (GDB, Overpass-Dumps) nach `scratch/geo/` (gitignored). `data/winterthur/` gesamt ≤ 8 MB, sonst bricht der Bake ab.
- Kein `cargo` involviert. Node-Tests: `npx vitest run <file>`.
- Fehler im Bake = harter Abbruch mit Report, kein Silent-Skip.
- Browser-Smoke (CLAUDE.md) ist Pflicht vor „fertig“ — `ksw.html` crosses frontend wiring.

## Dateistruktur

| Datei | Verantwortung |
|---|---|
| `scripts/geo/fetch-winterthur.mjs` | Netz: STAC-GDB + Overpass → `scratch/geo/` |
| `scripts/geo/lib/project.mjs` | WGS84 → lokale ENU-Meter (pur, getestet) |
| `scripts/geo/lib/triangulate.mjs` | planare 3D-Polygone → Dreiecke (pur, getestet) |
| `scripts/geo/lib/join.mjs` | OSM-Namens-Join, Strassen-Klassifikation (pur, getestet) |
| `scripts/geo/lib/transform.mjs` | GeoJSON-Features → Baked-Schema (pur, getestet) |
| `scripts/geo/bake-winterthur.mjs` | Orchestrierung: ogr2ogr, transform, write, Report |
| `data/winterthur/{meta,buildings,roads}.json` | committete Artefakte |
| `src/diorama/ksw/geo/geoData.ts` | typisierter Loader + abgeleitete Platte/Landmarken |
| `src/diorama/ksw/geo/cityMassing.ts` | Stadt als 2 gemergte Clay-Meshes (Wände, Dächer) |
| `src/diorama/ksw/geo/roads.ts` | Strassen/Gleise als gemergte Bänder |
| `src/diorama/ksw/main.ts` | Wiring: Stadtgruppe, grosse Platte/Mist, Kamera |
| `src/diorama/designTokens.ts` | NUR anfügen: `kswCity`-Block |

---

### Task 1: Fetch-Skript + Scratch-Hygiene

**Files:**
- Create: `scripts/geo/fetch-winterthur.mjs`
- Modify: `.gitignore` (Zeile anfügen), `package.json` (scripts)

**Interfaces:**
- Produces: `scratch/geo/swissBUILDINGS3D_3-0_1072-14.gdb/` (Verzeichnis), `scratch/geo/osm-buildings.json`, `scratch/geo/osm-roads.json` (rohe Overpass-Antworten)

- [ ] **Step 1: `.gitignore` + package.json**

`.gitignore` anfügen:
```
scratch/
```
`package.json` unter `"scripts"` anfügen:
```json
"geo:fetch": "node scripts/geo/fetch-winterthur.mjs",
"geo:bake": "node scripts/geo/bake-winterthur.mjs"
```

- [ ] **Step 2: Fetch-Skript schreiben**

```js
// scripts/geo/fetch-winterthur.mjs
// Downloads the raw geodata for the Winterthur bake into scratch/geo/:
// the swissBUILDINGS3D 3.0 tile 1072-14 (Esri GDB) and the OSM overlay
// (building names/usage, roads, rails) via Overpass. Network-only step —
// the bake itself (bake-winterthur.mjs) then runs offline.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';

const OUT = 'scratch/geo';
const GDB_URL =
  'https://data.geo.admin.ch/ch.swisstopo.swissbuildings3d_3_0/swissbuildings3d_3_0_2019_1072-14/swissbuildings3d_3_0_2019_1072-14_2056_5728.gdb.zip';
const GDB_DIR = `${OUT}/swissBUILDINGS3D_3-0_1072-14.gdb`;
// bbox: lon 8.7150–8.7300, lat 47.4955–47.5085 (Overpass order: S,W,N,E)
const BBOX = '47.4955,8.7150,47.5085,8.7300';
const MIRRORS = [
  'https://overpass-api.de/api/interpreter',
  'https://overpass.kumi.systems/api/interpreter',
];

async function overpass(query, outfile) {
  for (const url of MIRRORS) {
    const res = await fetch(url, { method: 'POST', body: 'data=' + encodeURIComponent(query) });
    if (!res.ok) continue;
    const text = await res.text();
    try {
      JSON.parse(text);
    } catch {
      continue; // mirror returned an HTML error page — try the next one
    }
    writeFileSync(outfile, text);
    console.log(`wrote ${outfile} (${(text.length / 1024).toFixed(0)} KB)`);
    return;
  }
  throw new Error(`all Overpass mirrors failed for ${outfile}`);
}

mkdirSync(OUT, { recursive: true });

if (!existsSync(GDB_DIR)) {
  console.log('downloading swissBUILDINGS3D tile (38 MB)…');
  execFileSync('curl', ['-sf', '-o', `${OUT}/b3d.gdb.zip`, GDB_URL], { stdio: 'inherit' });
  execFileSync('unzip', ['-oq', `${OUT}/b3d.gdb.zip`, '-d', OUT], { stdio: 'inherit' });
} else {
  console.log('GDB tile already present, skipping download');
}

// out geom: full geometry for the polygon join (names sit on building areas)
await overpass(
  `[out:json][timeout:60];(way["building"](${BBOX});relation["building"](${BBOX}););out tags geom;`,
  `${OUT}/osm-buildings.json`,
);
await overpass(
  `[out:json][timeout:60];(way["highway"](${BBOX});way["railway"~"^(rail|tram)$"](${BBOX}););out tags geom;`,
  `${OUT}/osm-roads.json`,
);
console.log('fetch complete');
```

- [ ] **Step 3: Laufen lassen und Outputs verifizieren**

Run: `npm run geo:fetch`
Expected: `scratch/geo/swissBUILDINGS3D_3-0_1072-14.gdb` existiert, beide `osm-*.json` > 100 KB. Bei Overpass-Timeout: einfach erneut laufen lassen (Mirrors rotieren).

- [ ] **Step 4: Commit**

```bash
git add .gitignore package.json scripts/geo/fetch-winterthur.mjs
git commit -m "geo: fetch script — swisstopo tile 1072-14 + OSM overlay to scratch/"
```

---

### Task 2: Projektion WGS84 → lokale Meter

**Files:**
- Create: `scripts/geo/lib/project.mjs`
- Test: `tests/geo/project.test.ts`

**Interfaces:**
- Produces: `makeProjector(anchor: {lon,lat}) → { toLocal(lon, lat): [x, z] }` — x = Ost-Meter, z = **Süd**-Meter (three.js-Konvention). `ANCHOR`, `BBOX` als benannte Konstanten.

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/project.test.ts
import { describe, expect, it } from 'vitest';
import { ANCHOR, BBOX, makeProjector } from '../../scripts/geo/lib/project.mjs';

function haversine(lon1: number, lat1: number, lon2: number, lat2: number): number {
  const R = 6371008.8;
  const p = Math.PI / 180;
  const a =
    Math.sin(((lat2 - lat1) * p) / 2) ** 2 +
    Math.cos(lat1 * p) * Math.cos(lat2 * p) * Math.sin(((lon2 - lon1) * p) / 2) ** 2;
  return 2 * R * Math.asin(Math.sqrt(a));
}

describe('makeProjector', () => {
  const proj = makeProjector(ANCHOR);

  it('maps the anchor itself to the origin', () => {
    const [x, z] = proj.toLocal(ANCHOR.lon, ANCHOR.lat);
    expect(Math.abs(x)).toBeLessThan(1e-9);
    expect(Math.abs(z)).toBeLessThan(1e-9);
  });

  it('matches haversine distance to <1 m across the whole bbox diagonal', () => {
    const [x, z] = proj.toLocal(BBOX.lonMin, BBOX.latMin);
    const d = Math.hypot(x, z);
    const ref = haversine(ANCHOR.lon, ANCHOR.lat, BBOX.lonMin, BBOX.latMin);
    expect(Math.abs(d - ref)).toBeLessThan(1.0);
  });

  it('Bahnhof lies south-west of the KSW anchor: x<0, z>0 (south positive)', () => {
    const [x, z] = proj.toLocal(8.724, 47.5003);
    expect(x).toBeLessThan(-300);
    expect(z).toBeGreaterThan(600);
  });
});
```

- [ ] **Step 2: Fehlschlag verifizieren**

Run: `npx vitest run tests/geo/project.test.ts`
Expected: FAIL — Modul nicht gefunden.

- [ ] **Step 3: Implementation**

```js
// scripts/geo/lib/project.mjs
// Local equirectangular projection around the KSW anchor. Over the ≤1.6 km
// bake bbox the distortion vs true geodesic distance is <0.1 m — verified
// against haversine in tests/geo/project.test.ts. +x = east, +z = SOUTH
// (three.js right-handed ground plane), y is height and untouched here.

export const ANCHOR = { lon: 8.7285, lat: 47.5069 }; // KSW Brauerstrasse 15
export const BBOX = { lonMin: 8.715, latMin: 47.4955, lonMax: 8.73, latMax: 47.5085 };

const R = 6371008.8; // mean earth radius, matches the haversine reference

export function makeProjector(anchor) {
  const rad = Math.PI / 180;
  const cos0 = Math.cos(anchor.lat * rad);
  return {
    toLocal(lon, lat) {
      const x = (lon - anchor.lon) * rad * R * cos0;
      const north = (lat - anchor.lat) * rad * R;
      return [x, -north];
    },
  };
}
```

- [ ] **Step 4: Test grün**

Run: `npx vitest run tests/geo/project.test.ts`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add scripts/geo/lib/project.mjs tests/geo/project.test.ts
git commit -m "geo: local ENU projection around the KSW anchor (<1m vs haversine)"
```

---

### Task 3: Triangulation planarer 3D-Polygone

**Files:**
- Create: `scripts/geo/lib/triangulate.mjs`
- Test: `tests/geo/triangulate.test.ts`

**Interfaces:**
- Consumes: nichts (pur).
- Produces: `triangulatePlanarPolygon(ring: number[][]) → { positions: number[], indices: number[] } | null` — `ring` = 3D-Aussenring `[[x,y,z],…]` (letzter Punkt ≠ erster; Duplikat wird entfernt). `null` für degenerierte Ringe (<3 Punkte oder ~0 Fläche). `polygonNormal(ring) → [nx,ny,nz]` (Newell, unnormiert).

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/triangulate.test.ts
import { describe, expect, it } from 'vitest';
import { polygonNormal, triangulatePlanarPolygon } from '../../scripts/geo/lib/triangulate.mjs';

describe('triangulatePlanarPolygon', () => {
  it('triangulates a horizontal unit square into 2 triangles', () => {
    const r = triangulatePlanarPolygon([[0, 5, 0], [1, 5, 0], [1, 5, 1], [0, 5, 1]]);
    expect(r).not.toBeNull();
    expect(r!.positions.length).toBe(12); // 4 vertices × xyz
    expect(r!.indices.length).toBe(6); // 2 triangles
  });

  it('triangulates a vertical wall quad (dominant x-normal)', () => {
    const r = triangulatePlanarPolygon([[2, 0, 0], [2, 0, 4], [2, 3, 4], [2, 3, 0]]);
    expect(r!.indices.length).toBe(6);
  });

  it('handles a gabled roof plane (tilted normal)', () => {
    const r = triangulatePlanarPolygon([[0, 4, 0], [10, 4, 0], [10, 6, 3], [0, 6, 3]]);
    expect(r!.indices.length).toBe(6);
    // area of the tilted quad = 10 × hypot(2,3)
    let area = 0;
    const p = r!.positions;
    for (let i = 0; i < r!.indices.length; i += 3) {
      const [a, b, c] = [r!.indices[i] * 3, r!.indices[i + 1] * 3, r!.indices[i + 2] * 3];
      const ab = [p[b] - p[a], p[b + 1] - p[a + 1], p[b + 2] - p[a + 2]];
      const ac = [p[c] - p[a], p[c + 1] - p[a + 1], p[c + 2] - p[a + 2]];
      const cr = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
      ];
      area += Math.hypot(...cr) / 2;
    }
    expect(area).toBeCloseTo(10 * Math.hypot(2, 3), 5);
  });

  it('returns null for degenerate rings', () => {
    expect(triangulatePlanarPolygon([[0, 0, 0], [1, 0, 0]])).toBeNull();
    expect(triangulatePlanarPolygon([[0, 0, 0], [1, 0, 0], [2, 0, 0]])).toBeNull(); // collinear
  });

  it('polygonNormal points up for a CCW-from-above horizontal ring', () => {
    const n = polygonNormal([[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]]);
    expect(n[1]).toBeGreaterThan(0);
  });
});
```

- [ ] **Step 2: Fehlschlag verifizieren**

Run: `npx vitest run tests/geo/triangulate.test.ts`
Expected: FAIL — Modul nicht gefunden.

- [ ] **Step 3: Implementation**

```js
// scripts/geo/lib/triangulate.mjs
// Triangulates the planar (walls/roofs are planar in swissBUILDINGS3D)
// 3D polygons of the LoD2 surfaces: Newell normal → drop the dominant
// axis → 2D ear-cut via three's ShapeUtils. Degenerate rings → null.
import { ShapeUtils } from 'three';

export function polygonNormal(ring) {
  let nx = 0, ny = 0, nz = 0;
  for (let i = 0; i < ring.length; i++) {
    const [ax, ay, az] = ring[i];
    const [bx, by, bz] = ring[(i + 1) % ring.length];
    nx += (ay - by) * (az + bz);
    ny += (az - bz) * (ax + bx);
    nz += (ax - bx) * (ay + by);
  }
  return [nx, ny, nz];
}

export function triangulatePlanarPolygon(ring) {
  // drop a duplicated closing point
  const pts =
    ring.length > 1 && ring[0].every((v, i) => Math.abs(v - ring[ring.length - 1][i]) < 1e-9)
      ? ring.slice(0, -1)
      : ring.slice();
  if (pts.length < 3) return null;

  const [nx, ny, nz] = polygonNormal(pts);
  const [ax, ay, az] = [Math.abs(nx), Math.abs(ny), Math.abs(nz)];
  if (ax + ay + az < 1e-6) return null; // zero area / collinear

  // project onto the plane's dominant axis pair, keeping winding intact
  let to2d;
  if (ay >= ax && ay >= az) to2d = ny >= 0 ? (p) => [p[0], p[2]] : (p) => [p[2], p[0]];
  else if (ax >= az) to2d = nx >= 0 ? (p) => [p[2], p[1]] : (p) => [p[1], p[2]];
  else to2d = nz >= 0 ? (p) => [p[1], p[0]] : (p) => [p[0], p[1]];

  const contour = pts.map((p) => {
    const [u, v] = to2d(p);
    return { x: u, y: v };
  });
  const tris = ShapeUtils.triangulateShape(contour, []);
  if (tris.length === 0) return null;

  return {
    positions: pts.flat(),
    indices: tris.flat(),
  };
}
```

- [ ] **Step 4: Test grün**

Run: `npx vitest run tests/geo/triangulate.test.ts`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add scripts/geo/lib/triangulate.mjs tests/geo/triangulate.test.ts
git commit -m "geo: planar-polygon triangulation (Newell + ShapeUtils ear-cut)"
```

---

### Task 4: OSM-Join + Strassen-Klassifikation

**Files:**
- Create: `scripts/geo/lib/join.mjs`
- Test: `tests/geo/join.test.ts`

**Interfaces:**
- Consumes: nichts (pur).
- Produces:
  - `pointInRing(x, z, ring: number[][]) → boolean` (ring = `[[x,z],…]`)
  - `ringCentroid(ring) → [x, z]`
  - `nameForFootprint(footprint: number[][], osmPolys: Array<{ring: number[][], tags: Record<string,string>}>) → {name?: string, usage?: string}` — OSM-Polygon, dessen Ring das Footprint-Zentroid enthält (erster Treffer mit `name`); `usage` aus `healthcare`/`amenity`/`building`.
  - `roadStyle(tags) → {class: string, width: number} | null` — null = ignorieren (z. B. `highway=proposed`).

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/join.test.ts
import { describe, expect, it } from 'vitest';
import { nameForFootprint, pointInRing, ringCentroid, roadStyle } from '../../scripts/geo/lib/join.mjs';

const square = [[0, 0], [10, 0], [10, 10], [0, 10]];

describe('geometry predicates', () => {
  it('pointInRing', () => {
    expect(pointInRing(5, 5, square)).toBe(true);
    expect(pointInRing(15, 5, square)).toBe(false);
  });
  it('ringCentroid of the square is its middle', () => {
    const [cx, cz] = ringCentroid(square);
    expect(cx).toBeCloseTo(5);
    expect(cz).toBeCloseTo(5);
  });
});

describe('nameForFootprint', () => {
  const osm = [
    { ring: [[-1, -1], [20, -1], [20, 20], [-1, 20]], tags: { building: 'hospital', name: 'Radio-Onkologie', healthcare: 'clinic' } },
  ];
  it('joins by centroid containment', () => {
    expect(nameForFootprint(square, osm)).toEqual({ name: 'Radio-Onkologie', usage: 'clinic' });
  });
  it('returns empty object when nothing contains the centroid', () => {
    expect(nameForFootprint([[100, 100], [110, 100], [110, 110], [100, 110]], osm)).toEqual({});
  });
});

describe('roadStyle', () => {
  it('classifies the hierarchy with descending widths', () => {
    const w = (t: Record<string, string>) => roadStyle(t)!.width;
    expect(w({ highway: 'primary' })).toBeGreaterThan(w({ highway: 'residential' }));
    expect(w({ highway: 'residential' })).toBeGreaterThan(w({ highway: 'footway' }));
  });
  it('classifies rails and rejects junk', () => {
    expect(roadStyle({ railway: 'rail' })!.class).toBe('rail');
    expect(roadStyle({ highway: 'proposed' })).toBeNull();
  });
});
```

- [ ] **Step 2: Fehlschlag verifizieren**

Run: `npx vitest run tests/geo/join.test.ts`
Expected: FAIL — Modul nicht gefunden.

- [ ] **Step 3: Implementation**

```js
// scripts/geo/lib/join.mjs
// swisstopo gives shape, OSM gives meaning: join OSM names/usage onto the
// swisstopo footprints via centroid containment (no shared id — EGID is not
// mapped area-wide in OSM), and classify OSM ways into renderable road
// ribbons with per-class widths (meters).

export function pointInRing(x, z, ring) {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    if (zi > z !== zj > z && x < ((xj - xi) * (z - zi)) / (zj - zi) + xi) inside = !inside;
  }
  return inside;
}

export function ringCentroid(ring) {
  let a = 0, cx = 0, cz = 0;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const f = ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
    a += f;
    cx += (ring[j][0] + ring[i][0]) * f;
    cz += (ring[j][1] + ring[i][1]) * f;
  }
  if (Math.abs(a) < 1e-9) return ring[0];
  return [cx / (3 * a), cz / (3 * a)];
}

export function nameForFootprint(footprint, osmPolys) {
  const [cx, cz] = ringCentroid(footprint);
  // smallest containing polygon wins: a department building inside the
  // campus polygon should get its own name, not the campus name
  let best = null;
  for (const p of osmPolys) {
    if (!p.tags.name || !pointInRing(cx, cz, p.ring)) continue;
    const size = ringArea(p.ring);
    if (!best || size < best.size) best = { p, size };
  }
  if (!best) return {};
  const t = best.p.tags;
  const out = { name: t.name };
  const usage = t.healthcare || t.amenity || t.building;
  if (usage && usage !== 'yes') out.usage = usage;
  return out;
}

function ringArea(ring) {
  let a = 0;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    a += ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
  }
  return Math.abs(a / 2);
}

const ROAD_WIDTHS = {
  motorway: 12, trunk: 11, primary: 9, secondary: 8, tertiary: 7,
  unclassified: 5.5, residential: 5.5, living_street: 5, service: 4,
  pedestrian: 4, track: 3, cycleway: 2.4, footway: 2.2, path: 2, steps: 2,
};

export function roadStyle(tags) {
  if (tags.railway === 'rail') return { class: 'rail', width: 3.2 };
  if (tags.railway === 'tram') return { class: 'rail', width: 2.8 };
  const hw = tags.highway;
  if (!hw) return null;
  // "_link" ramps inherit their parent class width
  const base = hw.endsWith('_link') ? hw.slice(0, -5) : hw;
  const width = ROAD_WIDTHS[base];
  if (width === undefined) return null; // proposed, construction, corridor, …
  return { class: base, width };
}
```

- [ ] **Step 4: Test grün**

Run: `npx vitest run tests/geo/join.test.ts`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add scripts/geo/lib/join.mjs tests/geo/join.test.ts
git commit -m "geo: OSM name join (centroid containment) + road classification"
```

---

### Task 5: Transform GeoJSON → Baked-Schema

**Files:**
- Create: `scripts/geo/lib/transform.mjs`
- Test: `tests/geo/transform.test.ts`

**Interfaces:**
- Consumes: Task 2 `makeProjector`, Task 3 `triangulatePlanarPolygon`, Task 4 `nameForFootprint`/`roadStyle`.
- Produces:
  - `transformBuildings({floors, walls, roofs, osmBuildings, projector}) → BakedBuilding[]`, wobei GeoJSON-FeatureCollections (WGS84, Z in m ü. M.) reinkommen. `BakedBuilding = { id: string, name?: string, usage?: string, zone: 'ksw'|'city', footprint: number[][], height: number, wall: {pos: number[], idx: number[]}, roof: {pos: number[], idx: number[]} }` — `pos` = **Zentimeter-Ganzzahlen** (lokale Meter × 100, gerundet), y bodennormalisiert (min-Z des Gebäudes → 0).
  - `transformRoads({osmRoads, projector}) → { roads: Array<{class, width, pts: number[][]}>, rails: […] }` (pts = Meter, 2 Nachkommastellen).
  - `KSW_ZONE_RADIUS = 170` (m): Gebäude mit Footprint-Zentroid näher am Ursprung → `zone: 'ksw'`.
  - Gruppierung swisstopo-Flächen → Gebäude via Feature-Property `UUID`.

- [ ] **Step 1: Failing Test (synthetisches Mini-Fixture)**

```ts
// tests/geo/transform.test.ts
import { describe, expect, it } from 'vitest';
import { makeProjector, ANCHOR } from '../../scripts/geo/lib/project.mjs';
import { transformBuildings, transformRoads } from '../../scripts/geo/lib/transform.mjs';

// one synthetic flat-roof building ~50 m east of the anchor, ground at 450 m
const lonAt = (m: number) => ANCHOR.lon + m / (111320 * Math.cos((ANCHOR.lat * Math.PI) / 180));
const latAt = (m: number) => ANCHOR.lat + m / 111132;
const ringLL = (x0: number, x1: number, z0: number, z1: number, y: number) => [
  [lonAt(x0), latAt(-z0), y], [lonAt(x1), latAt(-z0), y], [lonAt(x1), latAt(-z1), y], [lonAt(x0), latAt(-z1), y], [lonAt(x0), latAt(-z0), y],
];
const feat = (uuid: string, ring: number[][]) => ({
  type: 'Feature', properties: { UUID: uuid }, geometry: { type: 'MultiPolygon', coordinates: [[ring]] },
});
const fc = (...features: unknown[]) => ({ type: 'FeatureCollection', features });

const floors = fc(feat('b1', ringLL(50, 60, 10, 20, 450)));
const roofs = fc(feat('b1', ringLL(50, 60, 10, 20, 458)));
const walls = fc(
  feat('b1', [
    [lonAt(50), latAt(-10), 450], [lonAt(60), latAt(-10), 450],
    [lonAt(60), latAt(-10), 458], [lonAt(50), latAt(-10), 458], [lonAt(50), latAt(-10), 450],
  ]),
);
const osmBuildings = [{ ring: [[45, 5], [65, 5], [65, 25], [45, 25]], tags: { name: 'Testbau', building: 'hospital' } }];

describe('transformBuildings', () => {
  const out = transformBuildings({ floors, walls, roofs, osmBuildings, projector: makeProjector(ANCHOR) });

  it('produces one building with ground-normalized cm-integer geometry', () => {
    expect(out.length).toBe(1);
    const b = out[0];
    expect(b.height).toBeCloseTo(8, 1); // 458 − 450
    expect(Number.isInteger(b.roof.pos[0])).toBe(true);
    const ys = b.roof.pos.filter((_, i) => i % 3 === 1);
    expect(Math.max(...ys)).toBe(800); // roof at 8 m = 800 cm
    const wys = b.wall.pos.filter((_, i) => i % 3 === 1);
    expect(Math.min(...wys)).toBe(0); // ground normalized
  });

  it('joins the OSM name and flags the ksw zone (centroid 55 m < 170 m)', () => {
    expect(out[0].name).toBe('Testbau');
    expect(out[0].zone).toBe('ksw');
  });
});

describe('transformRoads', () => {
  it('projects way geometry and keeps the classification', () => {
    const osmRoads = {
      elements: [{ type: 'way', tags: { highway: 'residential' }, geometry: [
        { lon: lonAt(0), lat: latAt(0) }, { lon: lonAt(100), lat: latAt(0) },
      ] }],
    };
    const { roads } = transformRoads({ osmRoads, projector: makeProjector(ANCHOR) });
    expect(roads.length).toBe(1);
    expect(roads[0].width).toBe(5.5);
    expect(roads[0].pts[1][0]).toBeCloseTo(100, 0);
  });
});
```

- [ ] **Step 2: Fehlschlag verifizieren**

Run: `npx vitest run tests/geo/transform.test.ts`
Expected: FAIL — Modul nicht gefunden.

- [ ] **Step 3: Implementation**

```js
// scripts/geo/lib/transform.mjs
// The heart of the bake: swisstopo LoD2 GeoJSON (WGS84 + real Z) plus the
// OSM overlay → the compact baked schema the diorama loads. Groups surfaces
// by swisstopo UUID, normalizes each building to its own ground (min Z → 0),
// projects to local meters around the KSW anchor and quantizes positions to
// integer centimeters (JSON size). Throws on buildings that end up with no
// triangulatable geometry — a bake must never silently drop shape.
import { triangulatePlanarPolygon } from './triangulate.mjs';
import { nameForFootprint, ringCentroid, roadStyle } from './join.mjs';

export const KSW_ZONE_RADIUS = 170; // m — hero exclusion zone around the anchor

function* ringsOf(geometry) {
  // MultiPolygon: [poly][ring][pt]; we take every outer ring (holes are
  // rare in LoD2 surfaces and negligible at clay scale)
  if (!geometry) return;
  if (geometry.type === 'MultiPolygon') for (const poly of geometry.coordinates) yield poly[0];
  else if (geometry.type === 'Polygon') yield geometry.coordinates[0];
}

function collectByUuid(fc, projector, into, key) {
  for (const f of fc.features) {
    const uuid = f.properties?.UUID;
    if (!uuid || !f.geometry) continue;
    const b = into.get(uuid) ?? { floors: [], walls: [], roofs: [] };
    for (const ring of ringsOf(f.geometry)) {
      // project each vertex: [lon, lat, Z] → [x, Z, z]
      b[key].push(ring.map(([lon, lat, y]) => {
        const [x, z] = projector.toLocal(lon, lat);
        return [x, y ?? 0, z];
      }));
    }
    into.set(uuid, b);
  }
}

function meshFromRings(rings, groundY) {
  const pos = [];
  const idx = [];
  for (const ring of rings) {
    const tri = triangulatePlanarPolygon(ring);
    if (!tri) continue; // degenerate sliver surface — fine to skip a face
    const base = pos.length / 3;
    for (let i = 0; i < tri.positions.length; i += 3) {
      pos.push(
        Math.round(tri.positions[i] * 100),
        Math.round((tri.positions[i + 1] - groundY) * 100),
        Math.round(tri.positions[i + 2] * 100),
      );
    }
    for (const t of tri.indices) idx.push(base + t);
  }
  return { pos, idx };
}

export function transformBuildings({ floors, walls, roofs, osmBuildings, projector }) {
  const byUuid = new Map();
  collectByUuid(floors, projector, byUuid, 'floors');
  collectByUuid(walls, projector, byUuid, 'walls');
  collectByUuid(roofs, projector, byUuid, 'roofs');

  const out = [];
  for (const [uuid, b] of byUuid) {
    if (b.roofs.length === 0 && b.walls.length === 0) continue; // floor-only stub
    let groundY = Infinity;
    for (const ring of [...b.floors, ...b.walls, ...b.roofs])
      for (const [, y] of ring) groundY = Math.min(groundY, y);
    let topY = -Infinity;
    for (const ring of b.roofs.length ? b.roofs : b.walls)
      for (const [, y] of ring) topY = Math.max(topY, y);

    // footprint: largest floor ring (fallback: lowest wall ring projected)
    const floorRings = b.floors.length ? b.floors : b.walls;
    const footprint3d = floorRings.reduce((best, r) => (r.length > best.length ? r : best), floorRings[0]);
    const footprint = footprint3d.map(([x, , z]) => [Math.round(x * 100) / 100, Math.round(z * 100) / 100]);

    const wall = meshFromRings(b.walls, groundY);
    const roof = meshFromRings(b.roofs, groundY);
    if (wall.idx.length === 0 && roof.idx.length === 0)
      throw new Error(`bake: building ${uuid} has surfaces but none triangulated`);

    const [cx, cz] = ringCentroid(footprint);
    const building = {
      id: uuid,
      zone: Math.hypot(cx, cz) < KSW_ZONE_RADIUS ? 'ksw' : 'city',
      footprint,
      height: Math.round((topY - groundY) * 100) / 100,
      wall,
      roof,
      ...nameForFootprint(footprint, osmBuildings),
    };
    out.push(building);
  }
  return out;
}

export function transformRoads({ osmRoads, projector }) {
  const roads = [];
  const rails = [];
  for (const el of osmRoads.elements ?? []) {
    if (el.type !== 'way' || !el.geometry || el.geometry.length < 2) continue;
    const style = roadStyle(el.tags ?? {});
    if (!style) continue;
    const pts = el.geometry.map(({ lon, lat }) => {
      const [x, z] = projector.toLocal(lon, lat);
      return [Math.round(x * 100) / 100, Math.round(z * 100) / 100];
    });
    (style.class === 'rail' ? rails : roads).push({ class: style.class, width: style.width, pts });
  }
  return { roads, rails };
}
```

- [ ] **Step 4: Test grün**

Run: `npx vitest run tests/geo/transform.test.ts`
Expected: 3 passed. Falls die Wand-y-Assertion scheitert: prüfen, ob das Fixture-Z als 3. Koordinate ankommt (GeoJSON `[lon, lat, z]`).

- [ ] **Step 5: Commit**

```bash
git add scripts/geo/lib/transform.mjs tests/geo/transform.test.ts
git commit -m "geo: transform — UUID grouping, ground-normalize, cm quantization, zones"
```

---

### Task 6: Bake-Orchestrierung + echte Artefakte

**Files:**
- Create: `scripts/geo/bake-winterthur.mjs`
- Create (generiert + committet): `data/winterthur/meta.json`, `data/winterthur/buildings.json`, `data/winterthur/roads.json`

**Interfaces:**
- Consumes: Tasks 2–5, `scratch/geo/*` aus Task 1, lokales `ogr2ogr`.
- Produces: die 3 committeten JSONs. `meta.json = { anchor, bbox, plate: {cx, cz, w, d}, landmarks: {bahnhof: [x,z], zagTurbinenstrasse: [x,z], zagKonradstrasse: [x,z]}, counts: {buildings, kswBuildings, roads, rails, triangles}, attribution: string[], sourceTile: string }`. Platte = Bbox-Rechteck + 30 m Rand.

- [ ] **Step 1: Bake-Skript schreiben**

```js
// scripts/geo/bake-winterthur.mjs
// Offline bake: scratch/geo raw data → data/winterthur/*.json. Runs
// ogr2ogr (GDAL) to pull the three LoD2 layers out of the Esri GDB,
// clipped to the bbox, then hands everything to the pure transform libs.
// Hard-fails on empty extractions or a bloated output — no silent skips.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { ANCHOR, BBOX, makeProjector } from './lib/project.mjs';
import { transformBuildings, transformRoads } from './lib/transform.mjs';

const SCRATCH = 'scratch/geo';
const GDB = `${SCRATCH}/swissBUILDINGS3D_3-0_1072-14.gdb`;
const OUT = 'data/winterthur';
const MAX_TOTAL_BYTES = 8 * 1024 * 1024;

if (!existsSync(GDB)) throw new Error('GDB tile missing — run `npm run geo:fetch` first');

const spat = [String(BBOX.lonMin), String(BBOX.latMin), String(BBOX.lonMax), String(BBOX.latMax)];
function extractLayer(layer) {
  const file = `${SCRATCH}/${layer.toLowerCase()}.geojson`;
  execFileSync('ogr2ogr', [
    '-f', 'GeoJSON', file, GDB, layer,
    '-spat', ...spat, '-spat_srs', 'EPSG:4326', '-t_srs', 'EPSG:4326',
  ]);
  const fc = JSON.parse(readFileSync(file, 'utf8'));
  fc.features = fc.features.filter((f) => f.geometry);
  if (fc.features.length === 0) throw new Error(`bake: layer ${layer} extracted 0 features`);
  console.log(`${layer}: ${fc.features.length} surfaces`);
  return fc;
}

const projector = makeProjector(ANCHOR);
const floors = extractLayer('Floor');
const walls = extractLayer('Wall');
const roofs = extractLayer('Roof');

// OSM building polygons → local rings for the name join
const osmRaw = JSON.parse(readFileSync(`${SCRATCH}/osm-buildings.json`, 'utf8'));
const osmBuildings = [];
for (const el of osmRaw.elements ?? []) {
  const geom = el.type === 'way' ? el.geometry : el.members?.find((m) => m.role === 'outer')?.geometry;
  if (!geom || geom.length < 3 || !el.tags) continue;
  osmBuildings.push({ ring: geom.map(({ lon, lat }) => projector.toLocal(lon, lat)), tags: el.tags });
}
console.log(`OSM building polygons: ${osmBuildings.length}`);

const buildings = transformBuildings({ floors, walls, roofs, osmBuildings, projector });
const { roads, rails } = transformRoads({
  osmRoads: JSON.parse(readFileSync(`${SCRATCH}/osm-roads.json`, 'utf8')),
  projector,
});

// sanity gates
if (buildings.length < 500) throw new Error(`bake: only ${buildings.length} buildings — bbox/clip broken?`);
const named = buildings.filter((b) => b.name);
const ksw = buildings.filter((b) => b.zone === 'ksw');
if (ksw.length === 0) throw new Error('bake: no buildings in the ksw zone');
for (const b of buildings)
  if (!(b.height > 0.5 && b.height < 120)) throw new Error(`bake: implausible height ${b.height} on ${b.id}`);

const triangles = buildings.reduce((n, b) => n + (b.wall.idx.length + b.roof.idx.length) / 3, 0);

// plate = bbox rect in local meters + 30 m apron
const [x0, z0] = projector.toLocal(BBOX.lonMin, BBOX.latMax); // NW corner
const [x1, z1] = projector.toLocal(BBOX.lonMax, BBOX.latMin); // SE corner
const pad = 30;
const meta = {
  anchor: ANCHOR,
  bbox: BBOX,
  plate: {
    cx: Math.round((x0 + x1) / 2),
    cz: Math.round((z0 + z1) / 2),
    w: Math.round(x1 - x0 + 2 * pad),
    d: Math.round(z1 - z0 + 2 * pad),
  },
  landmarks: {
    bahnhof: projector.toLocal(8.7240, 47.5003).map((v) => Math.round(v)),
    zagTurbinenstrasse: projector.toLocal(8.7182, 47.4973).map((v) => Math.round(v)),
    zagKonradstrasse: projector.toLocal(8.7219, 47.5022).map((v) => Math.round(v)),
  },
  counts: { buildings: buildings.length, kswBuildings: ksw.length, named: named.length, roads: roads.length, rails: rails.length, triangles },
  attribution: ['Gebäude: © swisstopo (swissBUILDINGS3D 3.0)', 'Karte: © OpenStreetMap-Mitwirkende (ODbL)'],
  sourceTile: 'swissbuildings3d_3_0_2019_1072-14',
};

mkdirSync(OUT, { recursive: true });
writeFileSync(`${OUT}/meta.json`, JSON.stringify(meta, null, 1));
writeFileSync(`${OUT}/buildings.json`, JSON.stringify({ buildings }));
writeFileSync(`${OUT}/roads.json`, JSON.stringify({ roads, rails }));

const total = ['meta', 'buildings', 'roads'].reduce((n, f) => n + statSync(`${OUT}/${f}.json`).size, 0);
if (total > MAX_TOTAL_BYTES) throw new Error(`bake: output ${(total / 1e6).toFixed(1)} MB > 8 MB budget`);
console.log(`bake OK: ${buildings.length} buildings (${ksw.length} ksw, ${named.length} named), ` +
  `${roads.length} roads, ${rails.length} rails, ${triangles} tris, ${(total / 1e6).toFixed(1)} MB`);
```

- [ ] **Step 2: Bake laufen lassen**

Run: `npm run geo:bake`
Expected: `bake OK: 800±100 buildings (…ksw ≥ 10…), … tris, ≤ 8 MB`. Der Tri-Report ist das Budget-Gate der Spec. Bei „only N buildings“: Bbox in `project.mjs` gegen die Spec prüfen.

- [ ] **Step 3: Artefakt-Stichprobe**

Run: `node -e "const m=require('./data/winterthur/meta.json');console.log(m.plate,m.landmarks,m.counts)"`
Expected: plate ≈ `{cx≈-450, cz≈545, w≈1190, d≈1510}`; bahnhof ≈ `[-340, 730]`; zagTurbinenstrasse ≈ `[-775, 1065]`; zagKonradstrasse ≈ `[-495, 520]` (±15 m).

- [ ] **Step 4: Commit (inkl. Artefakte)**

```bash
git add scripts/geo/bake-winterthur.mjs data/winterthur/
git commit -m "geo: bake pipeline + committed Winterthur artifacts (swisstopo LoD2 + OSM)"
```

---

### Task 7: Runtime-Loader `geoData.ts`

**Files:**
- Create: `src/diorama/ksw/geo/geoData.ts`
- Test: `tests/geo/geoData.test.ts`
- Modify (nur falls nötig): `tsconfig.json` — `"resolveJsonModule": true` unter `compilerOptions`, falls nicht gesetzt.

**Interfaces:**
- Consumes: die committeten JSONs (statischer Vite/TS-Import — kein fetch).
- Produces:
```ts
export type BakedMesh = { pos: number[]; idx: number[] }; // pos = cm-Ganzzahlen
export type BakedBuilding = { id: string; name?: string; usage?: string; zone: 'ksw' | 'city'; footprint: number[][]; height: number; wall: BakedMesh; roof: BakedMesh };
export type RoadPath = { class: string; width: number; pts: number[][] };
export type CityMeta = { plate: { cx: number; cz: number; w: number; d: number }; landmarks: Record<string, number[]>; counts: Record<string, number>; attribution: string[] };
export const cityMeta: CityMeta;
export const cityBuildings: BakedBuilding[]; // nur zone==='city' — die ksw-Zone gehört dem Hero-Diorama
export const kswBuildings: BakedBuilding[];
export const cityRoads: RoadPath[];
export const cityRails: RoadPath[];
```

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/geoData.test.ts
import { describe, expect, it } from 'vitest';
import { cityBuildings, cityMeta, cityRoads, kswBuildings } from '../../src/diorama/ksw/geo/geoData';

describe('baked city data', () => {
  it('loads a real-sized city', () => {
    expect(cityBuildings.length).toBeGreaterThan(500);
    expect(kswBuildings.length).toBeGreaterThan(5);
    expect(cityRoads.length).toBeGreaterThan(50);
  });
  it('every building has positive height and non-empty geometry', () => {
    for (const b of [...cityBuildings, ...kswBuildings]) {
      expect(b.height).toBeGreaterThan(0);
      expect(b.wall.idx.length + b.roof.idx.length).toBeGreaterThan(0);
    }
  });
  it('plate covers all landmarks', () => {
    const { plate, landmarks } = cityMeta;
    for (const [x, z] of Object.values(landmarks)) {
      expect(Math.abs(x - plate.cx)).toBeLessThan(plate.w / 2);
      expect(Math.abs(z - plate.cz)).toBeLessThan(plate.d / 2);
    }
  });
  it('ksw zone contains the named departments', () => {
    const names = kswBuildings.map((b) => b.name).filter(Boolean).join(' ');
    expect(names.length).toBeGreaterThan(0);
  });
});
```

- [ ] **Step 2: Fehlschlag verifizieren**

Run: `npx vitest run tests/geo/geoData.test.ts`
Expected: FAIL — Modul nicht gefunden.

- [ ] **Step 3: Implementation**

```ts
// src/diorama/ksw/geo/geoData.ts
// Typed access to the baked Winterthur artifacts (data/winterthur/*.json,
// produced by scripts/geo/bake-winterthur.mjs). Static imports — the data
// ships with the bundle, no fetch, no fallback. The ksw zone is split out:
// those footprints belong to the hero diorama, the city renders the rest.
import buildingsJson from '../../../../data/winterthur/buildings.json';
import metaJson from '../../../../data/winterthur/meta.json';
import roadsJson from '../../../../data/winterthur/roads.json';

export type BakedMesh = { pos: number[]; idx: number[] };
export type BakedBuilding = {
  id: string;
  name?: string;
  usage?: string;
  zone: 'ksw' | 'city';
  footprint: number[][];
  height: number;
  wall: BakedMesh;
  roof: BakedMesh;
};
export type RoadPath = { class: string; width: number; pts: number[][] };
export type CityMeta = {
  plate: { cx: number; cz: number; w: number; d: number };
  landmarks: Record<string, number[]>;
  counts: Record<string, number>;
  attribution: string[];
};

const all = (buildingsJson as { buildings: BakedBuilding[] }).buildings;

export const cityMeta = metaJson as unknown as CityMeta;
export const cityBuildings: BakedBuilding[] = all.filter((b) => b.zone === 'city');
export const kswBuildings: BakedBuilding[] = all.filter((b) => b.zone === 'ksw');
export const cityRoads: RoadPath[] = (roadsJson as { roads: RoadPath[] }).roads;
export const cityRails: RoadPath[] = (roadsJson as { rails: RoadPath[] }).rails;
```

- [ ] **Step 4: Test grün + Typecheck**

Run: `npx vitest run tests/geo/geoData.test.ts && npx tsc --noEmit`
Expected: 4 passed; tsc sauber (sonst `resolveJsonModule` ergänzen).

- [ ] **Step 5: Commit**

```bash
git add src/diorama/ksw/geo/geoData.ts tests/geo/geoData.test.ts tsconfig.json
git commit -m "geo: typed runtime loader for the baked Winterthur artifacts"
```

---

### Task 8: Stadt-Massing `cityMassing.ts`

**Files:**
- Create: `src/diorama/ksw/geo/cityMassing.ts`
- Test: `tests/geo/cityMassing.test.ts`
- Modify: `src/diorama/designTokens.ts` — NUR anfügen (siehe Step 3).

**Interfaces:**
- Consumes: `BakedBuilding[]` (Task 7), `clayMat` aus `../props`, `palette`/`kswPalette` aus `../../designTokens`.
- Produces: `buildCityMassing(buildings: BakedBuilding[]) → THREE.Group` — genau 2 Kinder: `walls`-Mesh (`palette.creamBase`) und `roofs`-Mesh (`kswPalette.roofClay`), je EINE gemergte indizierte BufferGeometry, `castShadow = true`, `receiveShadow = true`, `name` gesetzt.

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/cityMassing.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildCityMassing } from '../../src/diorama/ksw/geo/cityMassing';
import type { BakedBuilding } from '../../src/diorama/ksw/geo/geoData';

const cube = (x: number): BakedBuilding => ({
  id: `b${x}`, zone: 'city', footprint: [[x, 0], [x + 5, 0], [x + 5, 5], [x, 5]], height: 6,
  // one wall quad + one roof quad, cm ints
  wall: { pos: [x * 100, 0, 0, (x + 5) * 100, 0, 0, (x + 5) * 100, 600, 0, x * 100, 600, 0], idx: [0, 1, 2, 0, 2, 3] },
  roof: { pos: [x * 100, 600, 0, (x + 5) * 100, 600, 0, (x + 5) * 100, 600, 500, x * 100, 600, 500], idx: [0, 1, 2, 0, 2, 3] },
});

describe('buildCityMassing', () => {
  const group = buildCityMassing([cube(0), cube(20)]);
  const walls = group.getObjectByName('cityWalls') as THREE.Mesh;
  const roofs = group.getObjectByName('cityRoofs') as THREE.Mesh;

  it('merges everything into exactly two meshes', () => {
    expect(group.children.length).toBe(2);
    expect(walls.geometry.getAttribute('position').count).toBe(8); // 2 buildings × 4 verts
    expect(roofs.geometry.index!.count).toBe(12); // 2 buildings × 2 tris
  });
  it('converts cm ints back to meters and stays finite', () => {
    const pos = walls.geometry.getAttribute('position');
    let maxX = -Infinity;
    for (let i = 0; i < pos.count; i++) {
      expect(Number.isFinite(pos.getX(i))).toBe(true);
      maxX = Math.max(maxX, pos.getX(i));
    }
    expect(maxX).toBe(25); // 2500 cm
  });
  it('casts and receives shadows', () => {
    expect(walls.castShadow && walls.receiveShadow).toBe(true);
    expect(roofs.castShadow && roofs.receiveShadow).toBe(true);
  });
});
```

- [ ] **Step 2: Fehlschlag verifizieren**

Run: `npx vitest run tests/geo/cityMassing.test.ts`
Expected: FAIL — Modul nicht gefunden.

- [ ] **Step 3: designTokens NUR anfügen**

Ans Ende von `src/diorama/designTokens.ts`:

```ts
// Winterthur city context (geo slices S1/S2). Scale-extension values only —
// the hero look tokens above are untouched; these govern the big city plate.
export const kswCity = {
  radiusMax: 1500, // wheel dolly ceiling to frame the whole Bahnhof↔ZAG span
  domeRadius: 1800, // clouds/stars dome swallows the city plate
  skyScale: 4000,
  cameraFar: 12000,
  roadY: 0.04, // road ribbons float just above the plate (no z-fight)
  railY: 0.05,
} as const;
```

- [ ] **Step 4: Implementation**

```ts
// src/diorama/ksw/geo/cityMassing.ts
// Renders the baked swisstopo LoD2 city as clay massing: every wall surface
// of every building merged into ONE mesh, every roof surface into another —
// two draw calls for ~800 real buildings, real roof shapes included. Colors
// and material come from the existing clay tokens, so the city reads as the
// same handmade material as the hero hospital.
import * as THREE from 'three/webgpu';
import { kswPalette, palette } from '../../designTokens';
import { clayMat } from '../props';
import type { BakedBuilding, BakedMesh } from './geoData';

function mergeBaked(parts: BakedMesh[]): THREE.BufferGeometry {
  let vtx = 0;
  let tri = 0;
  for (const p of parts) {
    vtx += p.pos.length / 3;
    tri += p.idx.length;
  }
  const positions = new Float32Array(vtx * 3);
  const indices = vtx > 65535 ? new Uint32Array(tri) : new Uint16Array(tri);
  let vo = 0;
  let io = 0;
  for (const p of parts) {
    const base = vo / 3;
    for (let i = 0; i < p.pos.length; i++) positions[vo + i] = p.pos[i] / 100; // cm → m
    for (let i = 0; i < p.idx.length; i++) indices[io + i] = base + p.idx[i];
    vo += p.pos.length;
    io += p.idx.length;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setIndex(new THREE.BufferAttribute(indices, 1));
  geo.computeVertexNormals();
  return geo;
}

export function buildCityMassing(buildings: BakedBuilding[]): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityMassing';

  const make = (name: string, parts: BakedMesh[], color: number): THREE.Mesh => {
    const mesh = new THREE.Mesh(mergeBaked(parts), clayMat(color));
    mesh.name = name;
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    group.add(mesh);
    return mesh;
  };

  make('cityWalls', buildings.map((b) => b.wall), palette.creamBase);
  make('cityRoofs', buildings.map((b) => b.roof), kswPalette.roofClay);
  return group;
}
```

- [ ] **Step 5: Test grün**

Run: `npx vitest run tests/geo/cityMassing.test.ts`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add src/diorama/ksw/geo/cityMassing.ts tests/geo/cityMassing.test.ts src/diorama/designTokens.ts
git commit -m "geo: city massing — 800 real LoD2 buildings in two clay draw calls"
```

---

### Task 9: Strassen & Gleise `roads.ts`

**Files:**
- Create: `src/diorama/ksw/geo/roads.ts`
- Test: `tests/geo/roads.test.ts`

**Interfaces:**
- Consumes: `RoadPath[]` (Task 7), `kswCity` Tokens (Task 8), `clayMat`, `kswPalette`/`palette`.
- Produces: `buildRoads(roads: RoadPath[], rails: RoadPath[]) → THREE.Group` — 2 Meshes: `roadRibbons` (`kswPalette.plazaPath`, y = `kswCity.roadY`), `railRibbons` (`palette.metalMatt`, y = `kswCity.railY`); flache Bänder, `receiveShadow = true`, `castShadow = false`.

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/roads.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildRoads } from '../../src/diorama/ksw/geo/roads';

describe('buildRoads', () => {
  const group = buildRoads(
    [{ class: 'residential', width: 6, pts: [[0, 0], [10, 0], [10, 10]] }],
    [{ class: 'rail', width: 3, pts: [[0, 5], [20, 5]] }],
  );
  const roads = group.getObjectByName('roadRibbons') as THREE.Mesh;
  const rails = group.getObjectByName('railRibbons') as THREE.Mesh;

  it('builds one ribbon quad per segment', () => {
    // road: 2 segments × 4 verts, rail: 1 segment × 4 verts
    expect(roads.geometry.getAttribute('position').count).toBe(8);
    expect(rails.geometry.getAttribute('position').count).toBe(4);
  });
  it('ribbon width matches the class width', () => {
    const pos = roads.geometry.getAttribute('position');
    // first segment runs +x, so its first two verts differ by `width` in z
    expect(Math.abs(pos.getZ(0) - pos.getZ(1))).toBeCloseTo(6);
  });
  it('roads receive but never cast shadows', () => {
    expect(roads.receiveShadow).toBe(true);
    expect(roads.castShadow).toBe(false);
  });
});
```

- [ ] **Step 2: Fehlschlag verifizieren**

Run: `npx vitest run tests/geo/roads.test.ts`
Expected: FAIL — Modul nicht gefunden.

- [ ] **Step 3: Implementation**

```ts
// src/diorama/ksw/geo/roads.ts
// OSM ways as flat clay ribbons on the plate: one quad per polyline
// segment, width by road class, slightly lifted to avoid z-fighting the
// lawn. Deliberately no miter joins — at clay scale the tiny wedge gaps at
// bends read as handmade, and the merged geometry stays trivial.
import * as THREE from 'three/webgpu';
import { kswCity, kswPalette, palette } from '../../designTokens';
import { clayMat } from '../props';
import type { RoadPath } from './geoData';

function ribbonGeometry(paths: RoadPath[], y: number): THREE.BufferGeometry {
  const positions: number[] = [];
  const indices: number[] = [];
  for (const path of paths) {
    for (let i = 0; i < path.pts.length - 1; i++) {
      const [x0, z0] = path.pts[i];
      const [x1, z1] = path.pts[i + 1];
      const dx = x1 - x0;
      const dz = z1 - z0;
      const len = Math.hypot(dx, dz);
      if (len < 0.05) continue;
      const hx = (-dz / len) * (path.width / 2); // segment normal × half width
      const hz = (dx / len) * (path.width / 2);
      const base = positions.length / 3;
      positions.push(
        x0 + hx, y, z0 + hz, x0 - hx, y, z0 - hz,
        x1 + hx, y, z1 + hz, x1 - hx, y, z1 - hz,
      );
      indices.push(base, base + 2, base + 1, base + 1, base + 2, base + 3);
    }
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3));
  geo.setIndex(positions.length / 3 > 65535 ? new THREE.BufferAttribute(new Uint32Array(indices), 1) : new THREE.BufferAttribute(new Uint16Array(indices), 1));
  geo.computeVertexNormals();
  return geo;
}

export function buildRoads(roads: RoadPath[], rails: RoadPath[]): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityRoads';

  const make = (name: string, paths: RoadPath[], color: number, y: number): void => {
    const mesh = new THREE.Mesh(ribbonGeometry(paths, y), clayMat(color));
    mesh.name = name;
    mesh.receiveShadow = true;
    mesh.castShadow = false;
    group.add(mesh);
  };

  make('roadRibbons', roads, kswPalette.plazaPath, kswCity.roadY);
  make('railRibbons', rails, palette.metalMatt, kswCity.railY);
  return group;
}
```

- [ ] **Step 4: Test grün**

Run: `npx vitest run tests/geo/roads.test.ts`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/diorama/ksw/geo/roads.ts tests/geo/roads.test.ts
git commit -m "geo: OSM roads/rails as merged clay ribbons"
```

---

### Task 10: main.ts-Wiring — Stadt, grosse Platte, Kamera-Presets

**Files:**
- Modify: `src/diorama/ksw/main.ts` (Stadtgruppe + Platte + Mist-Rim + Kamera)

**Interfaces:**
- Consumes: `buildCityMassing`, `buildRoads`, `cityBuildings`, `cityRoads`, `cityRails`, `cityMeta`, `kswCity`.
- Produces: das laufende Diorama; neue Presets `bahnhof`, `zag` (URL `?cam=bahnhof` funktioniert wie die bestehenden).

- [ ] **Step 1: Stadt einhängen (nach `buildHospital`, ~Zeile 339)**

Imports oben ergänzen:
```ts
import { buildCityMassing } from './geo/cityMassing';
import { buildRoads } from './geo/roads';
import { cityBuildings, cityMeta, cityRails, cityRoads } from './geo/geoData';
import { kswCity } from '../designTokens';
```
Nach dem `scene.add(hospital)`-Äquivalent:
```ts
// Real Winterthur context (swisstopo LoD2 + OSM), clay-styled. The hero
// diorama keeps its authored plate; the city sits on its own bigger slab.
const cityPlate = new THREE.Mesh(
  boxGeo(cityMeta.plate.w, kswScene.plateThickness, cityMeta.plate.d),
  clayMat(palette.lawn),
);
cityPlate.position.set(cityMeta.plate.cx, -kswScene.plateThickness / 2 - 0.02, cityMeta.plate.cz);
cityPlate.receiveShadow = true;
scene.add(cityPlate);
scene.add(buildCityMassing(cityBuildings));
scene.add(buildRoads(cityRoads, cityRails));
```
(`boxGeo` aus `./geometryCache`, `clayMat` aus `./props`, `palette` ist schon importiert — Importe entsprechend ergänzen. Die Hero-Platte liegt 2 cm höher: kein Z-Fighting.)

- [ ] **Step 2: Mist-Rim auf die Stadt-Platte umstellen (~Zeile 406–416)**

Die beiden Rim-Halbmasse ersetzen:
```ts
const rimX = cityMeta.plate.w / 2;
const rimZ = cityMeta.plate.d / 2;
```
und beim Ablaufen des Rechtecks die Zentren `cityMeta.plate.cx/cz` addieren (der bestehende Perimeter-Walk läuft um `(0,0)` — jeden erzeugten Punkt um `cx/cz` verschieben).

- [ ] **Step 3: Kamera-Reichweite + Presets**

Wo `kswCamera.radiusMax` konsumiert wird (Zoom-Clamp in `cameraRig`-Aufrufen oder `main.ts`): den Clamp auf `kswCity.radiusMax` heben — ABER `kswCamera` selbst nicht editieren; an der Konsumstelle `Math.max(kswCamera.radiusMax, kswCity.radiusMax)` verwenden. Kamera-`far` auf `kswCity.cameraFar`; `domeRadius`/`skyScale`-Konsumstellen analog auf die `kswCity`-Werte anheben (nur Reichweite, keine Look-Parameter). `camPresets` ergänzen:
```ts
bahnhof: { target: [cityMeta.landmarks.bahnhof[0], 0.5, cityMeta.landmarks.bahnhof[1]], radius: 90, yaw: -0.6, pitch: 0.8 },
zag: { target: [cityMeta.landmarks.zagTurbinenstrasse[0], 0.5, cityMeta.landmarks.zagTurbinenstrasse[1]], radius: 90, yaw: 0.4, pitch: 0.8 },
```
`CamPresetName`-Union erweitern: `'overview' | 'er' | 'ops' | 'bahnhof' | 'zag'`.

- [ ] **Step 4: Voller Gate-Lauf**

Run: `npx tsc --noEmit && npx vitest run && npm run build`
Expected: alles grün. (`scripts/build.mjs`-Wrapper benutzen, nicht raw vite — CLAUDE.md.)

- [ ] **Step 5: Browser-Smoke + Look-Gate (PFLICHT)**

Run: `node scripts/smoke-ksw.mjs` — Expected: `__LOOK_READY`, keine Console-Errors, `__KSW_INFO().drawCalls` < 60 (Stadt = +5 Draw-Calls: Platte, Wände, Dächer, Strassen, Gleise).
Run: `node scripts/capture-ksw.mjs` — Screenshots aller Presets. **Hero-Presets (`overview`/`er`/`ops`) visuell gegen den Stand vor diesem Branch vergleichen** (einmal auf `origin/main` capturen): Clay-Look, Himmel, Licht identisch; NUR im Hintergrund erscheint Stadt. `?cam=bahnhof` und `?cam=zag` zeigen echte Stadtstruktur (Gleisfeld! Bahnhofsdach!).

- [ ] **Step 6: Commit**

```bash
git add src/diorama/ksw/main.ts
git commit -m "geo: wire the real Winterthur city into the KSW diorama (plate, mist, camera)"
```

---

## Danach: Beauty-Loop + Folgeplan

- Screenshots aus Task 10 an den User — **Beauty-Loop** (Iteration auf Schatten-Reichweite/fitted frustum, Farben, Fog über die grosse Platte) auf Basis echter Bilder.
- S3 (KSW-Hero aus realen Footprints — `kswBuildings` liegt dafür schon bereit) + S4 (Aussenweg-Walker, Labels, Attribution) bekommen einen eigenen Plan, sobald S1+S2 sichtbar stehen.
