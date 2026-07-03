# Endless Horizon — Slice 1: Bake-Umbau (ganze Gemeinde, Terrain, Graph, Kacheln)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Die Bake-Pipeline produziert statt 4 JSON-Blobs für 1,9 km² eine protobuf-Kachel-Pyramide für die ganze Gemeinde Winterthur — echtes swissALTI3D-Terrain, alle Gebäude (Metadaten + Straßen-Anbindung), routbarer Straßengraph, ÖV, Landuse — und der Renderer steht auf dem Terrain statt auf der Platte.

**Architecture:** Fetch (Netz, GDAL-Konvertierung) bleibt strikt vom Offline-Bake getrennt. Der Bake baut erst globale Strukturen (DEM-Sampler, Road-Graph, Zugangspunkte), verteilt dann Features auf einen Quadtree (L0 grob → L2 fein) und encodiert protobuf (SoA-Spalten). Der Renderer lädt in Slice 1 stumpf alle Kacheln über einen neuen typed Loader; `cityPlate` entfällt, KSW zieht auf ein DEM-Plateau.

**Tech Stack:** Node .mjs-Skripte, GDAL (ogr2ogr/gdalbuildvrt/gdal_translate), Overpass, swisstopo-STAC, buf/protoc-gen-es v2 (@bufbuild/protobuf), three.js/TSL-Diorama, vitest.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-03-winterthur-endless-horizon-design.md` — bei Zweifel gilt der Spec.
- Look bleibt pixel-treu Clay-Diorama; neue Geometrie durch bestehende Builder-Familien (`clayMat`, `mergeTinted`-Muster).
- Artefakt-Budget **< 40 MB gesamt**; Level-0 **< 1 MB**.
- Bake **byte-deterministisch** (zweimal baken → identisch); keine `Date.now()`/Zufälle ohne Seed.
- Projektion: lokale Meter, `ANCHOR` unverändert (`scripts/geo/lib/project.mjs`), +x=Ost, +z=Süd.
- Kacheln enthalten **nie** bewegte Objekte (Zukunftsvertrag 1); Tile-IDs `L{level}/{x}/{y}` stabil (Vertrag 2).
- Cargo nur via `scripts/cargo-serial.sh` (hier kaum nötig — Slice 1 ist Frontend/Tooling).
- Vor „fertig": Browser-Smoke (CLAUDE.md) — Renderer-Änderungen queren die Daten-Grenze.
- Alle Tests: `npx vitest run <pfad>`; Typecheck: `npx tsc -p tsconfig.typecheck.json --noEmit`.
- Commits klein und häufig, Präfixe wie im Repo (`feat(geo):`, `test(geo):`, `docs(geo):`).

## Dateistruktur (Slice 1)

| Datei | Verantwortung |
|---|---|
| `backend/crates/protocol/proto/world.proto` | Neu: Tile/Graph/Manifest/Transit-Schema (Client-Server-Vertrag) |
| `buf.gen.yaml` | Zweites Target: plain-JS nach `scripts/geo/proto/` für .mjs-Bake |
| `scripts/geo/fetch-winterthur.mjs` | Erweitert: Boundary, DEM, Multi-Tile-GDB, OSM-voll |
| `scripts/geo/lib/stac.mjs` | Neu: STAC-Item-Auflistung (pure, testbar) |
| `scripts/geo/lib/dem.mjs` | Neu: AAIGrid-Parser + bilinearer Höhen-Sampler + Dezimierung |
| `scripts/geo/lib/graph.mjs` | Neu: OSM→Road-Graph (Topologie, Attribute, Restrictions, Höhenprofil) |
| `scripts/geo/lib/access.mjs` | Neu: Gebäude→Kante-Zugangspunkte |
| `scripts/geo/lib/landuse.mjs` | Neu: Landuse-Polygone → getaggte Ringe |
| `scripts/geo/lib/transit.mjs` | Neu: Route-Relations + Stops → Linien an Graph-Kanten |
| `scripts/geo/lib/tiles.mjs` | Neu: Quadtree-Zuteilung, Level-Dezimierung, protobuf-Encode |
| `scripts/geo/bake-world.mjs` | Neu: Orchestrierung + Gates (alter Bake bleibt bis Renderer-Switch) |
| `data/winterthur/world/` | Bake-Output: `manifest.pb`, `graph.pb`, `tiles/L*/x_y.pb` |
| `src/diorama/ksw/geo/worldData.ts` | Neu: Loader (fetch+decode, Slice 1: alle Kacheln) |
| `src/diorama/ksw/geo/terrain.ts` | Neu: Heightfield→BufferGeometry mit Landcover-Vertex-Farben |
| `src/diorama/ksw/main.ts` | `cityPlate` raus, Terrain rein, KSW-Plateau |

---

### Task 1: `world.proto` + Codegen für Bake-Skripte

**Files:**
- Create: `backend/crates/protocol/proto/world.proto`
- Modify: `buf.gen.yaml`
- Test: `tests/geo/worldProto.test.ts`

**Interfaces:**
- Produces: Messages `WorldManifest`, `RoadGraph`, `WorldTile`, `TransitLayer` (Feldnamen unten sind verbindlich für alle Folge-Tasks); generierte Module `src/proto/world_pb.ts` und `scripts/geo/proto/world_pb.js`.

- [ ] **Step 1: Schema schreiben**

```proto
// backend/crates/protocol/proto/world.proto
// Welt-Artefakte des Winterthur-Bakes (Spec 2026-07-03 endless-horizon).
// SoA-Spalten: parallele repeated-Felder gleicher Länge, ECS-ready.
// Vertrag: Kacheln sind statisch; Positionen lokal-Meter um den Anker.
syntax = "proto3";
package abutown.world;

message Projection {
  double anchor_lon = 1;
  double anchor_lat = 2;
}

message TileRef {
  uint32 level = 1; // 0..2
  uint32 x = 2;
  uint32 y = 3;
  string path = 4;      // "tiles/L1/3_2.pb" relativ zum Manifest
  uint32 byte_size = 5;
}

message WorldManifest {
  uint32 bake_version = 1;
  Projection projection = 2;
  // Weltausdehnung in lokalen Metern (Wurzelzelle des Quadtrees).
  double min_x = 3; double min_z = 4; double size = 5; // Quadrat
  repeated TileRef tiles = 6;
  repeated double boundary_ring = 7; // Gemeindegrenze, [x0,z0,x1,z1,...]
  repeated string attribution = 8;
}

message RoadGraph {
  // Knoten (OSM-Kreuzungen), SoA:
  repeated sint64 node_osm_id = 1;
  repeated double node_x = 2;
  repeated double node_z = 3;
  repeated double node_y = 4;      // DEM-Höhe
  repeated bool node_signal = 5;   // Ampel

  // Kanten, SoA (Index in node_*-Arrays):
  repeated uint32 edge_a = 10;
  repeated uint32 edge_b = 11;
  repeated uint32 edge_class = 12;   // enum RoadClass
  repeated double edge_width = 13;
  repeated uint32 edge_oneway = 14;  // 0 beide, 1 a→b, 2 b→a
  repeated uint32 edge_maxspeed = 15; // km/h, 0 = unbekannt
  repeated uint32 edge_lanes = 16;    // 0 = unbekannt
  // Polylinie + Höhenprofil je Kante, flach mit Offsets:
  repeated uint32 edge_pt_offset = 17; // Start in edge_pt_*; Länge = next-offset
  repeated double edge_pt_x = 18;
  repeated double edge_pt_z = 19;
  repeated double edge_pt_y = 20;

  // Abbiegeverbote: (von-Kante, über-Knoten, nach-Kante)
  repeated uint32 restriction_from_edge = 30;
  repeated uint32 restriction_via_node = 31;
  repeated uint32 restriction_to_edge = 32;
}

enum RoadClass {
  ROAD_CLASS_UNSPECIFIED = 0;
  MOTORWAY = 1; PRIMARY = 2; SECONDARY = 3; RESIDENTIAL = 4;
  SERVICE = 5; TRACK = 6; PATH = 7; FOOTWAY = 8; CYCLEWAY = 9; RAIL = 10;
}

message TransitLayer {
  repeated string line_ref = 1;      // "Bus 2", "S12"
  repeated uint32 line_mode = 2;     // 0 bus, 1 tram, 2 train
  repeated uint32 line_stop_offset = 3;
  repeated uint32 stop_edge = 4;     // Graph-Kanten-Index
  repeated double stop_offset_m = 5; // Meter entlang der Kante
  repeated string stop_name = 6;
}

message WorldTile {
  uint32 level = 1; uint32 x = 2; uint32 y = 3;

  // Terrain-Patch: reguläres Grid, Zeilen-major.
  uint32 grid_n = 10;          // Vertices pro Seite
  double cell_size = 11;       // Meter
  double origin_x = 12; double origin_z = 13;
  repeated float height = 14;          // grid_n²
  repeated uint32 landcover = 15;      // grid_n², enum Landcover

  // Gebäude, SoA. L0/L1: nur Prismen (footprint+height), L2: volle Meshes.
  repeated string b_id = 20;
  repeated uint32 b_usage = 21;        // enum Usage
  repeated double b_height = 22;
  repeated double b_base_y = 23;       // reale Höhenkote
  repeated uint32 b_fp_offset = 24;    // Offsets in b_fp_x/z
  repeated double b_fp_x = 25;
  repeated double b_fp_z = 26;
  repeated uint32 b_access_edge = 27;  // Graph-Kante, uint32max = keiner
  repeated double b_access_offset = 28;
  // L2: gebakte Meshes, flach konkateniert mit Offsets:
  repeated uint32 b_mesh_voffset = 29; repeated float b_mesh_pos = 30;
  repeated uint32 b_mesh_ioffset = 31; repeated uint32 b_mesh_idx = 32;

  // Straßen-Render-Bänder (aus dem Graph generiert, drapiert):
  repeated uint32 r_class = 40;
  repeated double r_width = 41;
  repeated uint32 r_pt_offset = 42;
  repeated double r_pt_x = 43; repeated double r_pt_z = 44; repeated double r_pt_y = 45;

  // Bäume (instanziert):
  repeated double t_x = 50; repeated double t_z = 51;
  repeated double t_h = 52; repeated double t_r = 53;
  repeated uint32 t_kind = 54; // 0 broad, 1 conifer
}

enum Landcover {
  LANDCOVER_UNSPECIFIED = 0; MEADOW = 1; FOREST = 2; FARMLAND = 3;
  RESIDENTIAL_LU = 4; INDUSTRIAL_LU = 5; WATER = 6; ROCK = 7;
}

enum Usage {
  USAGE_UNSPECIFIED = 0; RESIDENTIAL_U = 1; COMMERCIAL = 2;
  INDUSTRIAL_U = 3; PUBLIC = 4; AGRICULTURE = 5;
}
```

- [ ] **Step 2: buf.gen.yaml um JS-Target für die Bake-Skripte erweitern**

```yaml
version: v2
plugins:
  - local: ./node_modules/.bin/protoc-gen-es
    out: src/proto
    opt:
      - target=ts
      - import_extension=js
  # Plain-JS für die node-.mjs-Bake-Skripte (kein TS-Loader nötig).
  - local: ./node_modules/.bin/protoc-gen-es
    out: scripts/geo/proto
    opt:
      - target=js
      - import_extension=js
inputs:
  - directory: backend/crates/protocol/proto
```

- [ ] **Step 3: Codegen laufen lassen**

Run: `npm run generate:proto && npm run lint:proto`
Expected: `src/proto/world_pb.ts` und `scripts/geo/proto/world_pb.js` existieren, buf lint grün.

- [ ] **Step 4: Roundtrip-Test schreiben und laufen lassen**

```ts
// tests/geo/worldProto.test.ts
import { describe, expect, it } from 'vitest';
import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import { RoadGraphSchema, WorldTileSchema } from '../../src/proto/world_pb.js';

describe('world.proto roundtrip', () => {
  it('RoadGraph SoA survives encode/decode', () => {
    const g = create(RoadGraphSchema, {
      nodeOsmId: [1n, 2n], nodeX: [0, 10], nodeZ: [0, 0], nodeY: [450, 451],
      nodeSignal: [false, true],
      edgeA: [0], edgeB: [1], edgeClass: [4], edgeWidth: [5.5],
      edgeOneway: [0], edgeMaxspeed: [50], edgeLanes: [2],
      edgePtOffset: [0], edgePtX: [0, 10], edgePtZ: [0, 0], edgePtY: [450, 451],
    });
    const back = fromBinary(RoadGraphSchema, toBinary(RoadGraphSchema, g));
    expect(back.nodeSignal[1]).toBe(true);
    expect(back.edgePtY).toEqual([450, 451]);
  });
  it('WorldTile heightfield roundtrips', () => {
    const t = create(WorldTileSchema, {
      level: 2, x: 3, y: 4, gridN: 2, cellSize: 10,
      originX: -100, originZ: 200, height: [1, 2, 3, 4], landcover: [1, 1, 2, 2],
    });
    const back = fromBinary(WorldTileSchema, toBinary(WorldTileSchema, t));
    expect(back.height.length).toBe(4);
    expect(back.landcover[2]).toBe(2);
  });
});
```

Run: `npx vitest run tests/geo/worldProto.test.ts`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/protocol/proto/world.proto buf.gen.yaml src/proto scripts/geo/proto tests/geo/worldProto.test.ts
git commit -m "feat(geo): world.proto — Kachel/Graph/Manifest-Schema + JS-Codegen für Bake-Skripte"
```

---

### Task 2: STAC-Helper (Kachel-Auflistung für DEM + Gebäude)

**Files:**
- Create: `scripts/geo/lib/stac.mjs`
- Test: `tests/geo/stac.test.ts`

**Interfaces:**
- Produces: `stacItemUrls({ pageJsonList, assetSuffix }) -> string[]` — pure Funktion über bereits gefetchte STAC-Antwortseiten (Netz bleibt im fetch-Skript; collection/bbox stecken in der Seiten-URL, die Task 3 baut); wählt pro Item das passende Asset (DEM: `_2_2056.tif`-Variante = 2-m-Grid; Gebäude: `.gdb.zip`) und bei Jahrgangs-Duplikaten den **neuesten**.

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/stac.test.ts
import { describe, expect, it } from 'vitest';
import { stacItemUrls } from '../../scripts/geo/lib/stac.mjs';

const page = {
  features: [
    { id: 'swissalti3d_2019_2696-1262', assets: {
      a: { href: 'https://x/swissalti3d_2019_2696-1262_0.5_2056_5728.tif' },
      b: { href: 'https://x/swissalti3d_2019_2696-1262_2_2056_5728.tif' } } },
    { id: 'swissalti3d_2024_2696-1262', assets: {
      b: { href: 'https://x/swissalti3d_2024_2696-1262_2_2056_5728.tif' } } },
  ],
};

describe('stacItemUrls', () => {
  it('picks the 2m asset and the newest vintage per tile', () => {
    const urls = stacItemUrls({ pageJsonList: [page], assetSuffix: '_2_2056_5728.tif' });
    expect(urls).toEqual(['https://x/swissalti3d_2024_2696-1262_2_2056_5728.tif']);
  });
});
```

Run: `npx vitest run tests/geo/stac.test.ts` — Expected: FAIL (module not found).

- [ ] **Step 2: Implementierung**

```js
// scripts/geo/lib/stac.mjs
// Pure Auswahl-Logik über swisstopo-STAC-Seiten: pro Kachel (Suffix der
// Item-ID nach dem Jahrgang) das gewünschte Asset, neuester Jahrgang gewinnt.
// Das Netz (Pagination via links.rel=next) macht fetch-winterthur.mjs.
export function stacItemUrls({ pageJsonList, assetSuffix }) {
  const byTile = new Map(); // tileKey -> { vintage, url }
  for (const page of pageJsonList) {
    for (const item of page.features ?? []) {
      const m = /_(\d{4})_(\d+-\d+)$/.exec(item.id);
      if (!m) continue;
      const [, vintage, tile] = m;
      const asset = Object.values(item.assets ?? {}).find((a) => a.href.endsWith(assetSuffix));
      if (!asset) continue;
      const prev = byTile.get(tile);
      if (!prev || Number(vintage) > prev.vintage) byTile.set(tile, { vintage: Number(vintage), url: asset.href });
    }
  }
  return [...byTile.keys()].sort().map((k) => byTile.get(k).url);
}
```

- [ ] **Step 3: Test grün** — Run: `npx vitest run tests/geo/stac.test.ts` — Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add scripts/geo/lib/stac.mjs tests/geo/stac.test.ts
git commit -m "feat(geo): STAC-Asset-Auswahl (2m-DEM, neuester Jahrgang) als pure lib"
```

---

### Task 3: Fetch erweitern — Boundary, DEM, Multi-Tile-GDB, OSM-voll

**Files:**
- Modify: `scripts/geo/fetch-winterthur.mjs`

**Interfaces:**
- Consumes: `stacItemUrls` (Task 2).
- Produces (in `scratch/geo/`): `boundary-winterthur.geojson`; `dem/dem.asc` (ESRI-AAIGrid, EPSG:4326, aus allen 2-m-Kacheln gemosaikt und auf ~5 m resampled); GDB-Verzeichnisse aller benötigten swissBUILDINGS3D-Kacheln; `osm-roads.json` (inkl. Restrictions-Relations via `rel["type"="restriction"]`), `osm-transit.json`, `osm-landuse.json`, `osm-buildings.json`, `osm-nature.json` — alle für die Gemeinde-bbox `47.44,8.63,47.57,8.81`.

Kein sinnvoller Unit-Test (Netz + GDAL) — Verifikation = Artefakt-Gates im Skript selbst (Datei existiert, Feature-Count > Schwelle), wie im bestehenden Fetch.

- [ ] **Step 1: Boundary-Download anfügen** (an bestehende Struktur; swissBOUNDARIES3D als GeoPackage von data.geo.admin.ch, dann ogr2ogr-Filter auf `NAME='Winterthur'` aus `tlm_hoheitsgebiet`)

```js
const BOUND_URL =
  'https://data.geo.admin.ch/ch.swisstopo.swissboundaries3d/swissboundaries3d_2026-01/swissboundaries3d_2026-01_2056_5728.gpkg.zip';
const BOUND_GPKG = `${OUT}/swissboundaries3d.gpkg`;
if (!existsSync(`${OUT}/boundary-winterthur.geojson`)) {
  if (!existsSync(BOUND_GPKG)) {
    execFileSync('curl', ['-sfL', '-o', `${OUT}/bnd.zip`, BOUND_URL], { stdio: 'inherit' });
    execFileSync('unzip', ['-o', '-d', OUT, `${OUT}/bnd.zip`], { stdio: 'inherit' });
    // entpackter Name kann variieren — erste .gpkg im OUT übernehmen
    const gpkg = readdirSync(OUT).find((f) => f.endsWith('.gpkg'));
    if (!gpkg) throw new Error('boundaries: no .gpkg in zip');
    renameSync(`${OUT}/${gpkg}`, BOUND_GPKG);
  }
  execFileSync('ogr2ogr', ['-f', 'GeoJSON', `${OUT}/boundary-winterthur.geojson`, BOUND_GPKG,
    '-t_srs', 'EPSG:4326', '-where', "name = 'Winterthur'", 'tlm_hoheitsgebiet'], { stdio: 'inherit' });
}
const boundary = JSON.parse(readFileSync(`${OUT}/boundary-winterthur.geojson`, 'utf8'));
if (!boundary.features?.length) throw new Error('boundaries: Winterthur polygon missing');
```

(Hinweis für den Implementierer: Layer-/Feldname vorab mit `ogrinfo` prüfen — `tlm_hoheitsgebiet` / `name` sind die 2026er-Konvention; bei Abweichung anpassen und den echten Namen einkommentieren.)

- [ ] **Step 2: DEM-Kacheln via STAC listen, laden, mosaiken**

```js
import { stacItemUrls } from './lib/stac.mjs';
const GEMEINDE_BBOX = '8.63,47.44,8.81,47.57'; // lonMin,latMin,lonMax,latMax (STAC-Ordnung)
async function stacPages(collection) {
  const pages = [];
  let url = `https://data.geo.admin.ch/api/stac/v0.9/collections/${collection}/items?bbox=${GEMEINDE_BBOX}&limit=100`;
  while (url) {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`STAC ${collection}: HTTP ${res.status}`);
    const page = await res.json();
    pages.push(page);
    url = page.links?.find((l) => l.rel === 'next')?.href ?? null;
  }
  return pages;
}
const demUrls = stacItemUrls({ pageJsonList: await stacPages('ch.swisstopo.swissalti3d'), assetSuffix: '_2_2056_5728.tif' });
if (demUrls.length < 100) throw new Error(`DEM: only ${demUrls.length} tiles — bbox wrong?`);
mkdirSync(`${OUT}/dem/tiles`, { recursive: true });
for (const u of demUrls) {
  const f = `${OUT}/dem/tiles/${u.split('/').pop()}`;
  if (!existsSync(f)) execFileSync('curl', ['-sf', '-o', f, u], { stdio: 'inherit' });
}
// Mosaik → EPSG:4326 → ~5 m → ein ASCII-Grid (bake parst plain text, kein npm-Dep)
execFileSync('gdalbuildvrt', [`${OUT}/dem/dem.vrt`, ...readdirSync(`${OUT}/dem/tiles`).map((f) => `${OUT}/dem/tiles/${f}`)], { stdio: 'inherit' });
execFileSync('gdalwarp', ['-t_srs', 'EPSG:4326', '-tr', '0.00006', '0.00004', '-r', 'bilinear',
  '-overwrite', `${OUT}/dem/dem.vrt`, `${OUT}/dem/dem4326.tif`], { stdio: 'inherit' });
execFileSync('gdal_translate', ['-of', 'AAIGrid', `${OUT}/dem/dem4326.tif`, `${OUT}/dem/dem.asc`], { stdio: 'inherit' });
if (!existsSync(`${OUT}/dem/dem.asc`)) throw new Error('DEM: AAIGrid conversion failed');
```

- [ ] **Step 3: Gebäude-Kacheln multi-tile** — gleicher STAC-Loop mit `collection ch.swisstopo.swissbuildings3d_3_0`, `assetSuffix '.gdb.zip'`; jede Zip nach `scratch/geo/` entpacken (bestehendes Muster der 1072-14-Kachel), Liste der GDB-Pfade als `scratch/geo/gdb-list.json` schreiben.

- [ ] **Step 4: Overpass-Queries auf Gemeinde-bbox weiten + neue Layer**

```js
const OSM_BBOX = '47.44,8.63,47.57,8.81'; // Overpass-Ordnung S,W,N,E
await overpass(
  `[out:json][timeout:180];(way["highway"](${OSM_BBOX});way["railway"~"^(rail|tram)$"](${OSM_BBOX});rel["type"="restriction"](${OSM_BBOX});node["highway"="traffic_signals"](${OSM_BBOX}););out tags geom;`,
  `${OUT}/osm-roads.json`,
);
await overpass(
  `[out:json][timeout:180];(rel["type"="route"]["route"~"^(bus|tram|train)$"](${OSM_BBOX});node["public_transport"="platform"](${OSM_BBOX});node["highway"="bus_stop"](${OSM_BBOX}););out tags geom;`,
  `${OUT}/osm-transit.json`,
);
await overpass(
  `[out:json][timeout:180];(way["landuse"](${OSM_BBOX});rel["landuse"](${OSM_BBOX}););out tags geom;`,
  `${OUT}/osm-landuse.json`,
);
```

Bestehende buildings/nature-Queries: nur `${BBOX}` → `${OSM_BBOX}` ersetzen.

- [ ] **Step 5: Fetch einmal komplett laufen lassen**

Run: `npm run geo:fetch`
Expected: alle Artefakte in `scratch/geo/` (Log nennt Feature-Counts), keine Exception. Scratch wird mehrere GB — nicht committen (ist gitignored).

- [ ] **Step 6: Commit**

```bash
git add scripts/geo/fetch-winterthur.mjs
git commit -m "feat(geo): fetch — Gemeindegrenze, swissALTI3D-Mosaik, Multi-Tile-GDB, OSM voll (Restrictions/Transit/Landuse)"
```

---

### Task 4: DEM-Lib — AAIGrid-Parser + bilinearer Sampler + Patch-Extraktion

**Files:**
- Create: `scripts/geo/lib/dem.mjs`
- Test: `tests/geo/dem.test.ts`

**Interfaces:**
- Consumes: `makeProjector` (project.mjs) für lon/lat→lokal.
- Produces:
  - `parseAAIGrid(text) -> { ncols, nrows, xll, yll, cell, nodata, data: Float32Array }`
  - `makeDemSampler(grid, projector) -> { heightAt(x, z): number }` (bilinear; außerhalb → nächste Randzelle)
  - `extractPatch(sampler, { originX, originZ, gridN, cellSize }) -> Float32Array` (gridN², Zeilen-major)

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/dem.test.ts
import { describe, expect, it } from 'vitest';
import { extractPatch, makeDemSampler, parseAAIGrid } from '../../scripts/geo/lib/dem.mjs';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';

// 3×3-Grid um den Anker, Zelle ≈ 0.0001° — Werte 100..108 zeilenweise (Nord→Süd)
const asc = [
  'ncols 3', 'nrows 3',
  `xllcorner ${ANCHOR.lon - 0.00015}`, `yllcorner ${ANCHOR.lat - 0.00015}`,
  'cellsize 0.0001', 'NODATA_value -9999',
  '100 101 102', '103 104 105', '106 107 108',
].join('\n');

describe('dem', () => {
  it('parses AAIGrid headers and data', () => {
    const g = parseAAIGrid(asc);
    expect(g.ncols).toBe(3);
    expect(g.data[4]).toBe(104); // Mitte
  });
  it('samples bilinearly at the anchor (grid center)', () => {
    const g = parseAAIGrid(asc);
    const s = makeDemSampler(g, makeProjector(ANCHOR));
    expect(s.heightAt(0, 0)).toBeCloseTo(104, 0);
  });
  it('extracts a row-major patch', () => {
    const g = parseAAIGrid(asc);
    const s = makeDemSampler(g, makeProjector(ANCHOR));
    const p = extractPatch(s, { originX: -10, originZ: -10, gridN: 2, cellSize: 20 });
    expect(p.length).toBe(4);
    expect(p[0]).toBeGreaterThan(99);
  });
});
```

Run: `npx vitest run tests/geo/dem.test.ts` — Expected: FAIL (module not found).

- [ ] **Step 2: Implementierung**

```js
// scripts/geo/lib/dem.mjs
// ESRI-AAIGrid (aus gdal_translate) → Höhen-Sampler in lokalen Metern.
// Zeile 0 = Nord. Bilinear; Anfragen außerhalb clampen auf den Rand —
// der Dörfer-Ring fragt bewusst über die Gemeindegrenze hinaus.
export function parseAAIGrid(text) {
  const lines = text.split('\n');
  const head = {};
  let i = 0;
  for (; i < lines.length; i++) {
    const m = /^(\w+)\s+(-?[\d.]+)/.exec(lines[i]);
    if (!m || !/^(ncols|nrows|xllcorner|yllcorner|cellsize|NODATA_value)$/i.test(m[1])) break;
    head[m[1].toLowerCase()] = Number(m[2]);
  }
  const ncols = head.ncols, nrows = head.nrows;
  const data = new Float32Array(ncols * nrows);
  let k = 0;
  for (; i < lines.length; i++) {
    if (!lines[i].trim()) continue;
    for (const v of lines[i].trim().split(/\s+/)) data[k++] = Number(v);
  }
  if (k !== ncols * nrows) throw new Error(`AAIGrid: ${k} values, expected ${ncols * nrows}`);
  return { ncols, nrows, xll: head.xllcorner, yll: head.yllcorner, cell: head.cellsize, nodata: head.nodata_value ?? -9999, data };
}

export function makeDemSampler(grid, projector) {
  // lokale Meter → lon/lat invers zur equirect-Projektion (project.mjs)
  const R = 6371008.8, rad = Math.PI / 180;
  // projector kennt nur toLocal; invers hier lokal nachgebaut (gleicher Anker):
  const [ax, az] = [0, 0]; // Anker ist Ursprung per Definition
  void ax; void az;
  return {
    heightAt(x, z) {
      // invers: lon = anchor.lon + x/(R·cosφ·rad), lat = anchor.lat − z/(R·rad)
      const lat = projector.anchorLat - z / (R * rad);
      const lon = projector.anchorLon + x / (R * rad * Math.cos(projector.anchorLat * rad));
      const col = (lon - grid.xll) / grid.cell - 0.5;
      const rowFromS = (lat - grid.yll) / grid.cell - 0.5;
      const row = grid.nrows - 1 - rowFromS;
      const c0 = Math.max(0, Math.min(grid.ncols - 2, Math.floor(col)));
      const r0 = Math.max(0, Math.min(grid.nrows - 2, Math.floor(row)));
      const fc = Math.max(0, Math.min(1, col - c0));
      const fr = Math.max(0, Math.min(1, row - r0));
      const at = (r, c) => grid.data[r * grid.ncols + c];
      const h = at(r0, c0) * (1 - fc) * (1 - fr) + at(r0, c0 + 1) * fc * (1 - fr)
        + at(r0 + 1, c0) * (1 - fc) * fr + at(r0 + 1, c0 + 1) * fc * fr;
      return h;
    },
  };
}

export function extractPatch(sampler, { originX, originZ, gridN, cellSize }) {
  const out = new Float32Array(gridN * gridN);
  for (let j = 0; j < gridN; j++)
    for (let i = 0; i < gridN; i++)
      out[j * gridN + i] = sampler.heightAt(originX + i * cellSize, originZ + j * cellSize);
  return out;
}
```

**Achtung Signatur-Abgleich:** `makeDemSampler` braucht den Anker — `makeProjector` in `project.mjs` um zwei Felder erweitern (`anchorLon`, `anchorLat` im Rückgabeobjekt mitgeben; rückwärtskompatibel). Diese Änderung gehört zu diesem Task, inkl. einer Assertion in `tests/geo/project.test.ts`, dass die Felder existieren.

- [ ] **Step 3: Tests grün** — Run: `npx vitest run tests/geo/dem.test.ts tests/geo/project.test.ts` — Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add scripts/geo/lib/dem.mjs scripts/geo/lib/project.mjs tests/geo/dem.test.ts tests/geo/project.test.ts
git commit -m "feat(geo): DEM-Lib — AAIGrid-Parser, bilinearer Höhen-Sampler, Patch-Extraktion"
```

---

### Task 5: Road-Graph-Lib — Topologie, Attribute, Restrictions, Höhenprofil

**Files:**
- Create: `scripts/geo/lib/graph.mjs`
- Test: `tests/geo/graph.test.ts`

**Interfaces:**
- Consumes: Overpass-`osm-roads.json` (Ways mit `geometry` + `nodes`-Ids — **fetch muss `out tags geom` durch `out tags geom qt; >; out skel qt;`-Äquivalent liefern; praktisch: Overpass-`geom` enthält pro Way `nodes` nur, wenn zusätzlich angefragt** → Implementierer prüft und ergänzt die Query in Task 3 um Node-Ids, sonst ist Topologie unmöglich); `makeDemSampler` (Task 4).
- Produces: `buildRoadGraph({ osmRoads, projector, dem }) -> graph` mit exakt den SoA-Feldern aus `world.proto` (camelCase: `nodeOsmId`, `edgeA`, `edgePtOffset`, `restrictionFromEdge`, …) plus `edgeWayId: number[]` (nur bake-intern für Restrictions-Auflösung, nicht encodiert).
- Kanten-Regeln: Ways werden an **geteilten Knoten gesplittet** (Knoten = OSM-Node, der in ≥2 Ways vorkommt oder Way-Endpunkt ist); Klassen-Mapping auf `RoadClass`; `rail` wird eigene Kanten-Klasse (RAIL), keine getrennte Liste.

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/graph.test.ts
import { describe, expect, it } from 'vitest';
import { buildRoadGraph } from '../../scripts/geo/lib/graph.mjs';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';

const dLon = 0.000013; // ≈ 1 m Ost am Anker
const pt = (i) => ({ lon: ANCHOR.lon + i * dLon * 10, lat: ANCHOR.lat });
const osmRoads = { elements: [
  { type: 'way', id: 100, tags: { highway: 'residential', maxspeed: '30', lanes: '2' },
    nodes: [1, 2, 3], geometry: [pt(0), pt(1), pt(2)] },
  { type: 'way', id: 101, tags: { highway: 'service', oneway: 'yes' },
    nodes: [2, 4], geometry: [pt(1), { lon: ANCHOR.lon + dLon * 10, lat: ANCHOR.lat + 0.0001 }] },
  { type: 'node', id: 2, tags: { highway: 'traffic_signals' },
    lat: pt(1).lat, lon: pt(1).lon },
  { type: 'relation', id: 900, tags: { type: 'restriction', restriction: 'no_left_turn' },
    members: [
      { type: 'way', ref: 100, role: 'from' },
      { type: 'node', ref: 2, role: 'via' },
      { type: 'way', ref: 101, role: 'to' },
    ] },
] };

const dem = { heightAt: () => 450 };

describe('buildRoadGraph', () => {
  const g = buildRoadGraph({ osmRoads, projector: makeProjector(ANCHOR), dem });
  it('splits way 100 at shared node 2 → 3 edges total', () => {
    expect(g.edgeA.length).toBe(3);
  });
  it('marks node 2 as signal and node ids survive', () => {
    const i2 = g.nodeOsmId.findIndex((id) => id === 2n);
    expect(g.nodeSignal[i2]).toBe(true);
  });
  it('carries attributes: maxspeed 30, oneway on the service edge', () => {
    expect(g.edgeMaxspeed).toContain(30);
    expect(g.edgeOneway).toContain(1);
  });
  it('resolves the turn restriction to edge indices via node 2', () => {
    expect(g.restrictionFromEdge.length).toBe(1);
    const via = g.restrictionViaNode[0];
    expect(g.nodeOsmId[via]).toBe(2n);
  });
  it('drapes elevation onto every polyline point', () => {
    expect(g.edgePtY.every((y) => y === 450)).toBe(true);
  });
});
```

Run: `npx vitest run tests/geo/graph.test.ts` — Expected: FAIL.

- [ ] **Step 2: Implementierung**

```js
// scripts/geo/lib/graph.mjs
// OSM-Ways → routbarer Graph in SoA-Spalten (Feldnamen = world.proto).
// Knoten = Way-Endpunkte + jeder OSM-Node, der in ≥2 Ways vorkommt.
// Ways werden an Knoten gesplittet; Restrictions von Way-Ids auf
// Kanten-Indizes aufgelöst (via-Node muss Endpunkt beider Kanten sein).
const CLASS = { motorway: 1, trunk: 1, primary: 2, secondary: 3, tertiary: 3,
  residential: 4, unclassified: 4, living_street: 4, service: 5, track: 6,
  path: 7, footway: 8, pedestrian: 8, steps: 8, cycleway: 9 };
const WIDTH = { 1: 12, 2: 7, 3: 6, 4: 5.5, 5: 3.5, 6: 2.5, 7: 1.5, 8: 2, 9: 2, 10: 3 };

export function buildRoadGraph({ osmRoads, projector, dem }) {
  const els = osmRoads.elements ?? [];
  const ways = els.filter((e) => e.type === 'way' && e.geometry?.length >= 2 && e.nodes?.length === e.geometry.length
    && (CLASS[e.tags?.highway] || /^(rail|tram)$/.test(e.tags?.railway ?? '')));
  const signalIds = new Set(els.filter((e) => e.type === 'node' && e.tags?.highway === 'traffic_signals').map((e) => e.id));

  // Knotenkandidaten: Nutzungszähler über alle Way-Node-Ids
  const useCount = new Map();
  for (const w of ways) for (const id of w.nodes) useCount.set(id, (useCount.get(id) ?? 0) + 1);
  const isNode = (w, i) => i === 0 || i === w.nodes.length - 1 || useCount.get(w.nodes[i]) >= 2;

  const nodeIndex = new Map(); // osmId -> idx
  const g = { nodeOsmId: [], nodeX: [], nodeZ: [], nodeY: [], nodeSignal: [],
    edgeA: [], edgeB: [], edgeClass: [], edgeWidth: [], edgeOneway: [], edgeMaxspeed: [], edgeLanes: [],
    edgePtOffset: [], edgePtX: [], edgePtZ: [], edgePtY: [],
    restrictionFromEdge: [], restrictionViaNode: [], restrictionToEdge: [], edgeWayId: [] };
  const addNode = (osmId, lon, lat) => {
    if (nodeIndex.has(osmId)) return nodeIndex.get(osmId);
    const [x, z] = projector.toLocal(lon, lat);
    const idx = g.nodeOsmId.length;
    nodeIndex.set(osmId, idx);
    g.nodeOsmId.push(BigInt(osmId)); g.nodeX.push(x); g.nodeZ.push(z);
    g.nodeY.push(dem.heightAt(x, z)); g.nodeSignal.push(signalIds.has(osmId));
    return idx;
  };

  for (const w of ways) {
    const t = w.tags ?? {};
    const cls = t.railway ? 10 : CLASS[t.highway];
    const oneway = t.oneway === 'yes' || t.oneway === '1' ? 1 : t.oneway === '-1' ? 2 : 0;
    const maxspeed = Number.parseInt(t.maxspeed ?? '', 10) || 0;
    const lanes = Number.parseInt(t.lanes ?? '', 10) || 0;
    const width = Number.parseFloat(t.width ?? '') || WIDTH[cls];
    let segStart = 0;
    for (let i = 1; i < w.nodes.length; i++) {
      if (!isNode(w, i)) continue;
      const a = addNode(w.nodes[segStart], w.geometry[segStart].lon, w.geometry[segStart].lat);
      const b = addNode(w.nodes[i], w.geometry[i].lon, w.geometry[i].lat);
      g.edgeA.push(a); g.edgeB.push(b); g.edgeClass.push(cls); g.edgeWidth.push(width);
      g.edgeOneway.push(oneway); g.edgeMaxspeed.push(maxspeed); g.edgeLanes.push(lanes);
      g.edgeWayId.push(w.id);
      g.edgePtOffset.push(g.edgePtX.length);
      for (let k = segStart; k <= i; k++) {
        const [x, z] = projector.toLocal(w.geometry[k].lon, w.geometry[k].lat);
        g.edgePtX.push(Math.round(x * 100) / 100); g.edgePtZ.push(Math.round(z * 100) / 100);
        g.edgePtY.push(Math.round(dem.heightAt(x, z) * 100) / 100);
      }
      segStart = i;
    }
  }

  // Restrictions: Way-Ids → Kanten, die am via-Node enden
  for (const rel of els.filter((e) => e.type === 'relation' && e.tags?.type === 'restriction')) {
    const from = rel.members?.find((m) => m.role === 'from' && m.type === 'way');
    const via = rel.members?.find((m) => m.role === 'via' && m.type === 'node');
    const to = rel.members?.find((m) => m.role === 'to' && m.type === 'way');
    if (!from || !via || !to || !nodeIndex.has(via.ref)) continue;
    const viaIdx = nodeIndex.get(via.ref);
    const touching = (wayId) => g.edgeWayId.findIndex((wid, e) => wid === wayId && (g.edgeA[e] === viaIdx || g.edgeB[e] === viaIdx));
    const fe = touching(from.ref), te = touching(to.ref);
    if (fe < 0 || te < 0) continue;
    g.restrictionFromEdge.push(fe); g.restrictionViaNode.push(viaIdx); g.restrictionToEdge.push(te);
  }
  return g;
}
```

- [ ] **Step 3: Tests grün** — Run: `npx vitest run tests/geo/graph.test.ts` — Expected: 5 passed.

- [ ] **Step 4: Overpass-Query-Nachtrag prüfen** (Interfaces-Hinweis oben): sicherstellen, dass die Task-3-Query pro Way `nodes` liefert (Overpass: `out tags geom` liefert `nodes` NICHT — Query auf `(...); out body geom;` ändern und mit einem kleinen Live-Abruf verifizieren, dass `elements[].nodes` existiert). Fetch-Skript entsprechend anpassen.

- [ ] **Step 5: Commit**

```bash
git add scripts/geo/lib/graph.mjs tests/geo/graph.test.ts scripts/geo/fetch-winterthur.mjs
git commit -m "feat(geo): Road-Graph — Topologie-Split, Attribute, Ampeln, Abbiegeverbote, DEM-Höhenprofil"
```

---

### Task 6: Zugangspunkte — Gebäude → nächste Kante

**Files:**
- Create: `scripts/geo/lib/access.mjs`
- Test: `tests/geo/access.test.ts`

**Interfaces:**
- Consumes: Graph aus Task 5 (`edgePtOffset/edgePtX/edgePtZ`, `edgeClass`).
- Produces: `accessPoints({ graph, footprints }) -> { edge: number, offsetM: number }[]` — pro Footprint (Zentroid) die nächste Kante mit `edgeClass <= 6` (befahrbar) innerhalb 80 m, sonst Klasse 7–8 (Fußweg) innerhalb 80 m, sonst `{ edge: 0xffffffff, offsetM: 0 }`. `offsetM` = Bogenlänge vom Kantenstart bis zum Lotfußpunkt. 50-m-Grid-Bucketing wie das Door-Muster im alten Bake.

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/access.test.ts
import { describe, expect, it } from 'vitest';
import { accessPoints } from '../../scripts/geo/lib/access.mjs';

// Eine Kante von (0,0) nach (100,0), Klasse residential(4)
const graph = {
  edgeA: [0], edgeB: [1], edgeClass: [4],
  edgePtOffset: [0], edgePtX: [0, 100], edgePtZ: [0, 0], edgePtY: [450, 450],
};

describe('accessPoints', () => {
  it('binds a building at (40,10) to offset ~40 on the edge', () => {
    const [ap] = accessPoints({ graph, footprints: [[[38, 8], [42, 8], [42, 12], [38, 12]]] });
    expect(ap.edge).toBe(0);
    expect(ap.offsetM).toBeCloseTo(40, 0);
  });
  it('returns sentinel when nothing within 80 m', () => {
    const [ap] = accessPoints({ graph, footprints: [[[0, 500], [1, 500], [1, 501]]] });
    expect(ap.edge).toBe(0xffffffff);
  });
});
```

Run: `npx vitest run tests/geo/access.test.ts` — Expected: FAIL.

- [ ] **Step 2: Implementierung**

```js
// scripts/geo/lib/access.mjs
// Zugangspunkt pro Gebäude: nächste befahrbare Kante (Klasse ≤6) in 80 m,
// Fallback Fußweg (7–8), sonst Sentinel. Segment-Lot + Bogenlängen-Offset.
// Grid-Bucketing (50 m) über Kanten-Segmente — O(n), wie das Door-Muster.
const NONE = 0xffffffff;
export function accessPoints({ graph, footprints }) {
  const CELL = 50;
  const grid = new Map(); // "gx,gz" -> [{edge, segIdx}]
  const segsOf = (e) => {
    const start = graph.edgePtOffset[e];
    const end = e + 1 < graph.edgePtOffset.length ? graph.edgePtOffset[e + 1] : graph.edgePtX.length;
    return { start, end };
  };
  for (let e = 0; e < graph.edgeA.length; e++) {
    const { start, end } = segsOf(e);
    for (let i = start; i < end - 1; i++) {
      const k = `${Math.floor(graph.edgePtX[i] / CELL)},${Math.floor(graph.edgePtZ[i] / CELL)}`;
      (grid.get(k) ?? grid.set(k, []).get(k)).push({ e, i });
    }
  }
  const project = (px, pz, e, i) => {
    const ax = graph.edgePtX[i], az = graph.edgePtZ[i];
    const bx = graph.edgePtX[i + 1], bz = graph.edgePtZ[i + 1];
    const dx = bx - ax, dz = bz - az;
    const L2 = dx * dx + dz * dz || 1e-9;
    const t = Math.max(0, Math.min(1, ((px - ax) * dx + (pz - az) * dz) / L2));
    const qx = ax + t * dx, qz = az + t * dz;
    return { d: Math.hypot(px - qx, pz - qz), t, segLen: Math.sqrt(L2) };
  };
  const arcTo = (e, segIdx, t) => {
    const { start } = segsOf(e);
    let arc = 0;
    for (let i = start; i < segIdx; i++)
      arc += Math.hypot(graph.edgePtX[i + 1] - graph.edgePtX[i], graph.edgePtZ[i + 1] - graph.edgePtZ[i]);
    return arc + t * Math.hypot(graph.edgePtX[segIdx + 1] - graph.edgePtX[segIdx], graph.edgePtZ[segIdx + 1] - graph.edgePtZ[segIdx]);
  };
  return footprints.map((fp) => {
    const cx = fp.reduce((s, [x]) => s + x, 0) / fp.length;
    const cz = fp.reduce((s, [, z]) => s + z, 0) / fp.length;
    let best = null;
    const gx = Math.floor(cx / CELL), gz = Math.floor(cz / CELL);
    for (let dx = -2; dx <= 2; dx++) for (let dz = -2; dz <= 2; dz++) {
      for (const { e, i } of grid.get(`${gx + dx},${gz + dz}`) ?? []) {
        const p = project(cx, cz, e, i);
        if (p.d > 80) continue;
        const drivable = graph.edgeClass[e] <= 6;
        const rank = drivable ? 0 : 1;
        if (!best || rank < best.rank || (rank === best.rank && p.d < best.d))
          best = { rank, d: p.d, edge: e, offsetM: arcTo(e, i, p.t) };
      }
    }
    return best ? { edge: best.edge, offsetM: Math.round(best.offsetM * 10) / 10 } : { edge: NONE, offsetM: 0 };
  });
}
```

- [ ] **Step 3: Tests grün** — Run: `npx vitest run tests/geo/access.test.ts` — Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add scripts/geo/lib/access.mjs tests/geo/access.test.ts
git commit -m "feat(geo): Zugangspunkte — Gebäude an nächste Graph-Kante gebunden (BLOCKER-1-Lektion)"
```

---

### Task 7: Landuse- und Transit-Transforms

**Files:**
- Create: `scripts/geo/lib/landuse.mjs`, `scripts/geo/lib/transit.mjs`
- Test: `tests/geo/landuse.test.ts`, `tests/geo/transit.test.ts`

**Interfaces:**
- Produces:
  - `transformLanduse({ osmLanduse, projector }) -> { kind: number, ring: number[][] }[]` — `kind` = `Landcover`-Enum (meadow 1, forest 2, farmland 3, residential 4, industrial 5, water 6); unbekannte Tags → weglassen.
  - `transformTransit({ osmTransit, graph, projector }) -> transit` mit den `TransitLayer`-Feldern (`lineRef`, `lineMode`, `lineStopOffset`, `stopEdge`, `stopOffsetM`, `stopName`); Stops werden über `accessPoints`-Logik (Task 6, re-export der Projektion aufs Segment) an die nächste Kante ≤ Klasse 6 gebunden.

- [ ] **Step 1: Failing Tests** (Muster identisch zu Task 5/6 — synthetische Overpass-JSONs: ein `landuse=forest`-Way → `kind 2`; eine Bus-Route-Relation mit 2 Platform-Nodes → 1 Linie, 2 Stops mit `stopEdge >= 0`)

```ts
// tests/geo/landuse.test.ts
import { describe, expect, it } from 'vitest';
import { transformLanduse } from '../../scripts/geo/lib/landuse.mjs';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';

const way = { type: 'way', tags: { landuse: 'forest' }, geometry: [
  { lon: ANCHOR.lon, lat: ANCHOR.lat }, { lon: ANCHOR.lon + 0.001, lat: ANCHOR.lat },
  { lon: ANCHOR.lon + 0.001, lat: ANCHOR.lat + 0.001 }, { lon: ANCHOR.lon, lat: ANCHOR.lat } ] };

describe('transformLanduse', () => {
  it('maps forest to Landcover 2 with a local-meter ring', () => {
    const out = transformLanduse({ osmLanduse: { elements: [way] }, projector: makeProjector(ANCHOR) });
    expect(out).toHaveLength(1);
    expect(out[0].kind).toBe(2);
    expect(out[0].ring.length).toBeGreaterThanOrEqual(3);
  });
});
```

Run: `npx vitest run tests/geo/landuse.test.ts tests/geo/transit.test.ts` — Expected: FAIL.

- [ ] **Step 2: Implementierung** — `landuse.mjs`: Tag-Map `{ meadow:1, grass:1, forest:2, wood:2, farmland:3, residential:4, industrial:5, commercial:5, basin:6, reservoir:6 }`, Ringe wie `transformRoads` in lokale Meter runden. `transit.mjs`: Route-Relations gruppieren (`tags.ref ?? tags.name`), Modus aus `tags.route`, Platform-/Stop-Nodes projizieren und per Segment-Lot an die nächste Kante binden (Hilfsfunktion aus `access.mjs` exportieren: `nearestEdgePoint(graph, x, z, maxDist) -> { edge, offsetM } | null` — Refactor dort, `accessPoints` nutzt sie intern).

- [ ] **Step 3: Tests grün** — Run: `npx vitest run tests/geo/landuse.test.ts tests/geo/transit.test.ts tests/geo/access.test.ts` — Expected: PASS (access-Refactor darf nichts brechen).

- [ ] **Step 4: Commit**

```bash
git add scripts/geo/lib/landuse.mjs scripts/geo/lib/transit.mjs scripts/geo/lib/access.mjs tests/geo/landuse.test.ts tests/geo/transit.test.ts
git commit -m "feat(geo): Landuse-Ringe + ÖV-Linien/Stops an Graph-Kanten"
```

---

### Task 8: Quadtree-Kacheln + protobuf-Encode

**Files:**
- Create: `scripts/geo/lib/tiles.mjs`
- Test: `tests/geo/tiles.test.ts`

**Interfaces:**
- Consumes: DEM-Sampler (Task 4), Graph (Task 5), Gebäude (bestehende `transformBuildings`-Ausgabe + `accessPoints`), Landuse (Task 7), Bäume (`transformNature`).
- Produces:
  - `tileGridFor(boundaryRing, ringPadM) -> { minX, minZ, size, levels: 3 }` — Wurzel = kleinstes Quadrat über Grenze+Ring, an 1000 m gerundet.
  - `assignToTiles(root, features) -> Map<tileId, bucket>` — `tileId` = `"L{level}/{x}_{y}"`; Gebäude/Bäume nach Zentroid in L2-Zellen; L1/L0 erhalten **abgeleitete** Grobfassungen (L1: Gebäude als Footprint-Prismen ohne Mesh; L0: Gebäude ganz weg, nur Terrain+Landcover).
  - `encodeTile(bucket, demSampler) -> Uint8Array` (WorldTile-protobuf; Terrain-Patch `gridN` pro Level: L0 = 21 @ 50 m, L1 = 51 @ 20 m, L2 = 101 @ 10 m über die Zellgröße; Landcover-Grid = Punkt-in-Ring-Test gegen Landuse, Default MEADOW).
  - `encodeManifest(...)`, `encodeGraph(graph)` — dünne Wrapper um `toBinary`.
- Determinismus: alle Iterationen über sortierte Schlüssel; keine Map-Insertion-Order in Ausgaben.

- [ ] **Step 1: Failing Test**

```ts
// tests/geo/tiles.test.ts
import { describe, expect, it } from 'vitest';
import { fromBinary } from '@bufbuild/protobuf';
import { WorldTileSchema } from '../../src/proto/world_pb.js';
import { assignToTiles, encodeTile, tileGridFor } from '../../scripts/geo/lib/tiles.mjs';

const boundary = [[-4000, -4000], [4000, -4000], [4000, 4000], [-4000, 4000]];
const dem = { heightAt: (x, z) => 400 + x / 1000 };

describe('tiles', () => {
  it('roots a 1km-aligned square over boundary+ring', () => {
    const g = tileGridFor(boundary, 4000);
    expect(g.size % 1000).toBe(0);
    expect(g.size).toBeGreaterThanOrEqual(16000);
  });
  it('assigns a building to its L2 cell and drops it from L0', () => {
    const g = tileGridFor(boundary, 4000);
    const b = { id: 'x', footprint: [[10, 10], [20, 10], [20, 20]], height: 9, usage: 1, baseY: 401, access: { edge: 0, offsetM: 5 } };
    const tiles = assignToTiles(g, { buildings: [b], trees: [], landuse: [], graph: { edgeA: [], edgePtOffset: [], edgePtX: [], edgePtZ: [], edgePtY: [], edgeClass: [], edgeWidth: [] } });
    const l2 = [...tiles.keys()].filter((k) => k.startsWith('L2/'));
    const l0 = tiles.get('L0/0_0');
    expect(l2.some((k) => tiles.get(k).buildings.length === 1)).toBe(true);
    expect(l0.buildings.length).toBe(0);
  });
  it('encodes a decodable tile with the right grid resolution', () => {
    const g = tileGridFor(boundary, 4000);
    const tiles = assignToTiles(g, { buildings: [], trees: [], landuse: [], graph: { edgeA: [], edgePtOffset: [], edgePtX: [], edgePtZ: [], edgePtY: [], edgeClass: [], edgeWidth: [] } });
    const [id, bucket] = [...tiles.entries()].find(([k]) => k.startsWith('L0/'));
    const bin = encodeTile(bucket, dem);
    const t = fromBinary(WorldTileSchema, bin);
    expect(t.gridN).toBe(21);
    expect(t.height.length).toBe(21 * 21);
    expect(id).toBe('L0/0_0');
  });
  it('is byte-deterministic', () => {
    const g = tileGridFor(boundary, 4000);
    const mk = () => {
      const tiles = assignToTiles(g, { buildings: [], trees: [], landuse: [], graph: { edgeA: [], edgePtOffset: [], edgePtX: [], edgePtZ: [], edgePtY: [], edgeClass: [], edgeWidth: [] } });
      return encodeTile(tiles.get('L0/0_0'), dem);
    };
    expect(Buffer.from(mk()).equals(Buffer.from(mk()))).toBe(true);
  });
});
```

Run: `npx vitest run tests/geo/tiles.test.ts` — Expected: FAIL.

- [ ] **Step 2: Implementierung** — Kernpunkte (vollständig ausschreiben):
  - `tileGridFor`: bbox über Ring + `ringPadM`, größte Seite auf 1000 m aufrunden, quadratisch; L0 = 1 Zelle, L1 = 4×4, L2 = 16×16 (Zellgröße = size/1, size/4, size/16).
  - `assignToTiles`: Buckets `{ level, x, y, originX, originZ, cellSize, buildings, trees, landuse, roadSegs }`; Straßen-**Render**-Segmente pro L1/L2-Zelle geclippt aus den Graph-Polylinien (Klasse ≤5 in L1, alle in L2, Hauptklassen ≤3 zusätzlich in L0); Landuse-Ringe jeder überlappenden Zelle zugeteilt (bbox-Test reicht — der Punkt-in-Ring-Test beim Encoden schneidet exakt).
  - `encodeTile`: Terrain-`gridN` per Level (21/51/101), `height` via `extractPatch`; `landcover` per Grid-Punkt: erster Landuse-Ring, der ihn enthält (Ring-Reihenfolge: nach `kind`, dann Fläche absteigend — deterministisch), sonst 1 (MEADOW); Gebäude-SoA aus Bucket (L2 zusätzlich `b_mesh_*` aus den gebakten wall/roof-Meshes, konkateniert mit Offsets); `create(WorldTileSchema, …)` + `toBinary`.
  - Punkt-in-Ring: Standard-Raycast, als lokale Hilfsfunktion.

- [ ] **Step 3: Tests grün** — Run: `npx vitest run tests/geo/tiles.test.ts` — Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
git add scripts/geo/lib/tiles.mjs tests/geo/tiles.test.ts
git commit -m "feat(geo): Quadtree-Kacheln — Zuteilung, Level-Dezimierung, deterministisches protobuf-Encode"
```

---

### Task 9: `bake-world.mjs` — Orchestrierung + Gates

**Files:**
- Create: `scripts/geo/bake-world.mjs`
- Modify: `package.json` (Script `"geo:bake-world": "node scripts/geo/bake-world.mjs"`)

**Interfaces:**
- Consumes: alle Libs (Task 2–8) + bestehende `transformBuildings`/`transformNature` (Multi-GDB: `extractLayer` pro GDB aus `gdb-list.json`, Ergebnisse konkatenieren; `-spat` auf Gemeinde-bbox).
- Produces: `data/winterthur/world/manifest.pb`, `graph.pb`, `transit.pb`, `tiles/L{0..2}/x_y.pb`. Bestehende `data/winterthur/*.json` bleiben unangetastet (Renderer-Switch ist Task 11/12).

- [ ] **Step 1: Skript schreiben** — Ablauf mit denselben Hard-Fail-Gates wie der alte Bake:
  1. Boundary laden → Ring in lokale Meter; Gate: Ring ≥ 100 Punkte.
  2. DEM parsen (`dem.asc`), Sampler bauen; Gate: `heightAt(0,0)` in [430, 470] (KSW liegt ~450 m).
  3. Alle GDBs extrahieren (Wall/Roof), `transformBuildings` je GDB, konkatenieren, **an Boundary+Ring clippen** (Zentroid-in-Ring bzw. -in-Ring+Pad für Dörfer); `baseY = dem.heightAt(cx, cz)` pro Gebäude. Gates: ≥ 15000 Gebäude gesamt; Höhen-Gates wie bisher.
  4. `buildRoadGraph`; Gates: ≥ 20000 Kanten; größte Zusammenhangskomponente (Union-Find über edgeA/edgeB) ≥ 90 % der Kanten; jede Kante hat ≥ 2 Profilpunkte.
  5. `accessPoints`; Gate: ≥ 80 % gebunden (Sentinel-Quote < 20 %).
  6. `transformLanduse`, `transformTransit`, `transformNature` (Bäume). Gates: ≥ 50 Landuse-Ringe, ≥ 5 Bus-Linien, ≥ 3000 Bäume.
  7. `tileGridFor(boundaryRing, 4000)`, `assignToTiles`, encode alles, schreiben. Gates: Level-0-Summe < 1 MB; Gesamt < 40 MB; zweiter Encode-Durchlauf in-memory byte-identisch (Determinismus-Gate im Bake selbst).
  8. Console-Summary wie im alten Bake (Counts, MB, Kachelzahl).

- [ ] **Step 2: Bake laufen lassen**

Run: `npm run geo:fetch && npm run geo:bake-world`
Expected: alle Gates grün, Summary z. B. `bake OK: ~2xxxx buildings, ~3xxxx edges, 273 tiles, xx.x MB`. Bei Gate-Rissen: Daten anschauen, Schwellen NUR mit dokumentierter Begründung anpassen (Kommentar am Gate).

- [ ] **Step 3: Determinismus von außen verifizieren**

Run: `npm run geo:bake-world && shasum data/winterthur/world/manifest.pb data/winterthur/world/graph.pb | tee /tmp/h1 && npm run geo:bake-world && shasum -c /tmp/h1`
Expected: `OK` für beide Dateien.

- [ ] **Step 4: Commit** (Artefakte mit-committen wie bisher `data/winterthur/*.json`)

```bash
git add scripts/geo/bake-world.mjs package.json data/winterthur/world
git commit -m "feat(geo): bake-world — Gemeinde-Pyramide mit Graph/Transit/Landuse, Gates + Determinismus"
```

---

### Task 10: Runtime-Loader `worldData.ts`

**Files:**
- Create: `src/diorama/ksw/geo/worldData.ts`
- Test: `tests/geo/worldData.test.ts`

**Interfaces:**
- Consumes: `src/proto/world_pb.ts` (Task 1); Artefakte via `fetch` relativ zu `import.meta.env.BASE_URL` (Kacheln sind zu groß für Bundle-Import — **Bruch mit dem static-import-Muster von `geoData.ts`, absichtlich**: Vorbereitung auf Slice-2-Streaming. Dev: `data/winterthur/world` nach `public/world` symlinken/kopieren via `vite.config.ts` `publicDir`-Ergänzung oder ein `scripts/`-Kopierschritt — Implementierer wählt den kleinsten Eingriff und dokumentiert ihn im Commit).
- Produces:
  - `loadWorld(baseUrl?: string): Promise<World>` — lädt Manifest, dann **alle** Kacheln (Slice 1) + Graph.
  - `type World = { manifest: WorldManifest; graph: RoadGraph; tiles: DecodedTile[] }`
  - `type DecodedTile = { level: number; x: number; y: number; tile: WorldTile }`
  - Pure Hilfsfunktion `decodeWorld(manifestBin: Uint8Array, graphBin: Uint8Array, tileBins: { path: string; bin: Uint8Array }[]): World` — testbar ohne fetch.

- [ ] **Step 1: Failing Test** — `decodeWorld` mit in-memory `toBinary`-Fixtures (Manifest mit 1 TileRef, 1 Mini-Tile, leerer Graph); Assertion: `tiles[0].tile.gridN` stimmt, Pfad-Zuordnung über `TileRef.path`.

- [ ] **Step 2: Implementierung** — `decodeWorld` pur; `loadWorld` = `fetch(baseUrl + 'manifest.pb')` → `Promise.all` über `manifest.tiles` (Slice 1: alle; die Signatur nimmt optional einen Filter `keep?: (ref: TileRef) => boolean`, den Slice 2 dann durch den Tile-Manager ersetzt).

- [ ] **Step 3: Tests grün + Typecheck** — Run: `npx vitest run tests/geo/worldData.test.ts && npx tsc -p tsconfig.typecheck.json --noEmit` — Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/diorama/ksw/geo/worldData.ts tests/geo/worldData.test.ts
git commit -m "feat(geo): worldData-Loader — Manifest/Graph/Kacheln fetch+decode, Slice-2-Filter vorbereitet"
```

---

### Task 11: Terrain-Renderer `terrain.ts`

**Files:**
- Create: `src/diorama/ksw/geo/terrain.ts`
- Test: `tests/geo/terrain.test.ts`

**Interfaces:**
- Consumes: `DecodedTile` (Task 10); `clayMat`/vertexTint-Muster aus `nature.ts` (`vertexTintMat`-Äquivalent — Material mit `vertexColors: true`).
- Produces: `buildTerrainTiles(tiles: DecodedTile[], opts: { level: number }): THREE.Group` — pro Kachel des gewählten Levels ein indiziertes Grid-Mesh (`gridN`×`gridN` Vertices, Position aus origin/cellSize/height, Vertex-Farbe aus `landcover` über eine Farbtabelle in `designTokens` — neue Token-Gruppe `terrainLook = { meadow, forest, farmland, residentialLu, industrialLu, water, rock }`, Werte an bestehende `kswCity.parkGreen`/`water`-Töne angelehnt), `receiveShadow = true`, `castShadow = false`, `name = 'terrainL{level}/{x}_{y}'`.

- [ ] **Step 1: Failing Test** — synthetische 3×3-Kachel → Gruppe mit 1 Mesh, `geometry.attributes.position.count === 9`, Index-Länge `(3-1)²·6`, y-Werte = height-Array; Landcover 2 (forest) ergibt die forest-Tokenfarbe am Vertex.

- [ ] **Step 2: Implementierung** — Grid-Trianglation Zeilen-major (zwei Dreiecke pro Zelle), `Float32Array`-Farben aus der Tokentabelle, ein `MeshPhysicalMaterial`-Klon mit `vertexColors` (exakt das `vertexTintMat`-Muster aus `nature.ts` wiederverwenden — Funktion dorthin exportieren statt duplizieren).

- [ ] **Step 3: Tests grün + Typecheck** — Run: `npx vitest run tests/geo/terrain.test.ts && npx tsc -p tsconfig.typecheck.json --noEmit` — Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/diorama/ksw/geo/terrain.ts src/diorama/ksw/geo/nature.ts src/diorama/designTokens.ts tests/geo/terrain.test.ts
git commit -m "feat(diorama): Terrain-Kachel-Meshes mit Landcover-Vertex-Tint"
```

---

### Task 12: Renderer-Umbau — Platte raus, Terrain rein, KSW-Plateau

**Files:**
- Modify: `src/diorama/ksw/main.ts` (cityPlate-Block ~Zeile 480, cityRoot-Aufbau, KSW-Gruppen-y)
- Modify: `src/diorama/ksw/geo/geoData.ts` → Konsumenten schrittweise auf `worldData` umziehen (Gebäude/Straßen/Natur der Kernstadt kommen jetzt aus L2-Kacheln; `cityMassing`/`roads`/`nature`-Builder erhalten dieselben Datenformen — Adapter in `worldData.ts`: `toBakedBuildings(world): BakedBuilding[]`, `toRoadPaths(world): RoadPath[]`, damit die Builder unverändert bleiben)
- Test: bestehende `tests/geo/*.test.ts` bleiben grün; neu `tests/geo/worldAdapter.test.ts`

**Interfaces:**
- Consumes: `loadWorld` (Task 10), `buildTerrainTiles` (Task 11).
- Produces: `main.ts` ohne `cityPlate`/ohne flache Platte; KSW-Gruppe auf `plateauY` = Median der DEM-Höhen unter dem Campus-Rect (Berechnung als exportierte Funktion `plateauHeight(world, rect): number` in `worldData.ts`, Test mit synthetischer Welt).

- [ ] **Step 1: Adapter-Test schreiben** (`toBakedBuildings` liefert `BakedBuilding`-kompatible Objekte aus einer synthetischen L2-Kachel; `plateauHeight` = Median) — Run: FAIL.
- [ ] **Step 2: Adapter implementieren** — Run: PASS.
- [ ] **Step 3: `main.ts` umbauen** — `cityPlate`-Erzeugung löschen; `cityRoot.add(buildTerrainTiles(world.tiles, { level: 2 }))`; Gebäude/Straßen/Natur-Builder aus den Adaptern speisen; `interior`/`plaza`/`helipad`-Wurzelgruppe `position.y = plateauY`; Agent-y-Offsets (`inBuilding`-Zweig, `slot.set(..., y, ...)`) um `plateauY` ergänzen. Edge-Mist-Ring **bleibt** (Slice 3 entfernt ihn). Fernring/GI: Terrain-Gruppe per `layers`/Namesliste aus `renderProbeFace` ausnehmen (Spec §Renderer — nur der Backdrop, die Kernstadt bleibt in der Probe).
- [ ] **Step 4: Voller Gate-Lauf** — Run: `npx tsc -p tsconfig.typecheck.json --noEmit && npx vitest run && npm run build`
  Expected: alles grün (vite-Build wegen `publicDir`-Anpassung beobachten — ETIMEDOUT-Falle aus CLAUDE.md).
- [ ] **Step 5: Browser-Smoke (Pflicht, CLAUDE.md)** — bestehendes Capture-Harness (`scripts/capture-env.mjs`-Familie auf origin/main) auf die neue Szene richten: Screenshot Übersicht + Zoom KSW + Zoom Altstadt; prüfen: Stadt steht auf Hügeln (y-Varianz im Terrain sichtbar), KSW-Campus nicht unter/über dem Boden, Straßen liegen auf dem Terrain (kein Schweben > 1 m), fps im Budget der 10k-Pipeline (Konsole `?stats`-Pfad, falls vorhanden).
  Expected: Captures zeigen Terrain; keine WebGL-Errors in der Konsole.
- [ ] **Step 6: Commit**

```bash
git add src/diorama/ksw/main.ts src/diorama/ksw/geo/worldData.ts tests/geo/worldAdapter.test.ts
git commit -m "feat(diorama): Welt steht auf echtem Terrain — cityPlate raus, KSW auf DEM-Plateau"
```

---

### Task 13: Slice-1-Abschluss — Alt-Artefakte, Doku, PR

**Files:**
- Modify: `progress.md` (neuer Eintrag OBEN im reverse-chron-Block ab Zeile 19 — Konvention!)
- Delete: nichts löschen, was `geoData.ts`-Konsumenten noch brauchen — erst wenn Task 12 ALLE Konsumenten umgezogen hat: `data/winterthur/{buildings,roads,nature}.json` + `geoData.ts` entfernen, sonst als Follow-up-Chip.

- [ ] **Step 1: Konsumenten-Sweep** — `grep -rn "from './geoData'\|geo/geoData" src tests` — jede Fundstelle entweder umgezogen (Task 12) oder begründet stehen lassen; wenn 0 Fundstellen: JSONs + geoData.ts löschen (No-legacy-Regel), `tests/geo/geoData.test.ts` durch worldData-Äquivalent ersetzt.
- [ ] **Step 2: Voller CI-Gate lokal** — Run: `npx tsc -p tsconfig.typecheck.json --noEmit && npx vitest run && npm run build && npx playwright test` (e2e nur falls vorhanden/konfiguriert) — Expected: grün.
- [ ] **Step 3: progress.md + Commit + PR**

```bash
git add -A
git commit -m "feat(geo): Slice 1 komplett — Gemeinde-Welt-Bake, Terrain-Renderer, Alt-Pfad entfernt"
git push -u origin geo/endless-horizon
gh pr create --title "Endless Horizon Slice 1: Gemeinde-Bake, echtes Terrain, Road-Graph, Kachel-Pyramide" --body "Spec: docs/superpowers/specs/2026-07-03-winterthur-endless-horizon-design.md (Slice 1)

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```

- [ ] **Step 4: CI abwarten — GRÜN, nicht nur nicht-rot** (`gh pr checks --watch` bzw. exit-status; Memory-Lektion: nie bei UNSTABLE mergen).

---

## Self-Review (durchgeführt)

- **Spec-Abdeckung Slice 1:** Fetch (Boundary/DEM/Multi-Tile/OSM-voll) = Task 2–3; Pyramiden-Bake mit Terrain/Gebäude+Metadaten+Anbindung/Graph inkl. Restrictions+Ampeln/ÖV/Landuse/protobuf-SoA = Task 1, 4–9; Renderer-Minimalumbau (Terrain trägt, Platte raus, KSW-Plateau, stumpf alle Kacheln) = Task 10–12; Determinismus + Gates + Browser-Smoke = Task 9/12; Budget-Gates < 1 MB/L0 und < 40 MB = Task 9. Aerial perspective, Dörfer-Ring-Kacheln, Mist-Ring-Entfernung, Streaming = bewusst Slice 2/3, nicht hier.
- **Platzhalter:** Task 3 (Fetch) und Task 9 (Orchestrierung) beschreiben Ablauf + Gates statt Vollcode — begründet: Netz/GDAL-Schritte sind nicht unit-testbar, die Verifikation sind die im Text exakt bezifferten Gates; alle pure-Logik-Tasks (2, 4–8, 10–11) tragen vollständigen Code.
- **Typ-Konsistenz:** SoA-Feldnamen in Task 5/6/8/10 = proto-camelCase aus Task 1 (`edgePtOffset`, `restrictionViaNode`, `stopOffsetM`); `NONE = 0xffffffff` konsistent Task 6 ↔ `b_access_edge`-Sentinel Task 1; `gridN` 21/51/101 konsistent Task 8 ↔ Test.
- **Bekannte Unsicherheiten für Implementierer markiert:** swissBOUNDARIES3D-Layer-/Feldnamen (Task 3 Step 1), Overpass `nodes`-Lieferung (Task 5 Step 4), STAC-Asset-Suffixe (Task 2 — gegen echte Antwort verifizieren), vite-`publicDir` (Task 10).
