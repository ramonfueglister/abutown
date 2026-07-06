// scripts/geo/bake-winterthur.mjs
// Offline bake: scratch/geo raw data → data/winterthur/*.json. Runs
// ogr2ogr (GDAL) to pull the three LoD2 layers out of the Esri GDB,
// clipped to the bbox, then hands everything to the pure transform libs.
// Hard-fails on empty extractions or a bloated output — no silent skips.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { ANCHOR, BBOX, makeProjector } from './lib/project.mjs';
import { transformBuildings, transformNature, transformRoads } from './lib/transform.mjs';
import { doorForBuilding } from './lib/style.mjs';

const SCRATCH = 'scratch/geo';
const GDB = `${SCRATCH}/swissBUILDINGS3D_3-0_1072-14.gdb`;
const OUT = 'data/winterthur';
// Raised 8 -> 16 MB 2026-07-06: the legitimate committed payload grew to
// ~11.2 MB with the Gemeinde-wide roads.json (#133) and the family-attributed
// nature.json (#136); 8 MB predates both. Still a guard against runaway blobs.
const MAX_TOTAL_BYTES = 16 * 1024 * 1024;

if (!existsSync(GDB)) throw new Error('GDB tile missing — run `npm run geo:fetch` first');

const spat = ['-spat', String(BBOX.lonMin), String(BBOX.latMin), String(BBOX.lonMax), String(BBOX.latMax),
  '-spat_srs', 'EPSG:4326', '-t_srs', 'EPSG:4326'];

// ogr2ogr exits non-zero whenever it skips a feature GeoJSON can't hold (the
// LoD2 tile mixes a few 3D solids into the surface layers), yet still writes
// every convertible feature. So: ignore the exit code, validate by feature
// count. `extra` lets the footprint pass flatten Building_solid to 2D.
function extractLayer(layer, extra = []) {
  const file = `${SCRATCH}/${layer.toLowerCase()}.geojson`;
  rmSync(file, { force: true });
  spawnSync('ogr2ogr', ['-f', 'GeoJSON', file, GDB, layer, ...spat, ...extra], { stdio: 'ignore' });
  if (!existsSync(file)) throw new Error(`bake: ogr2ogr produced no output for layer ${layer}`);
  const fc = JSON.parse(readFileSync(file, 'utf8'));
  fc.features = fc.features.filter((f) => f.geometry);
  if (fc.features.length === 0) throw new Error(`bake: layer ${layer} extracted 0 features`);
  console.log(`${layer}: ${fc.features.length} surfaces`);
  return fc;
}

const projector = makeProjector(ANCHOR);
// The LoD2 surfaces are PolyhedralSurface, which GeoJSON drops to null. Only
// `-explodecollections -dim XYZ` reliably yields every facet as a 3D Polygon
// (a bare -nlt MULTIPOLYGON leaves ~20% flattened to 2D → bogus zero heights).
// Exploded facets keep their parent UUID, so the transform still groups them
// per building. The footprint is traced from the wall bases (Building_solid
// flattened to 2D would just overlay all faces into a single stray facet).
const surf = ['-nlt', 'MULTIPOLYGON', '-explodecollections', '-dim', 'XYZ'];
const walls = extractLayer('Wall', surf);
const roofs = extractLayer('Roof', surf);

// OSM building polygons → local rings for the name join
const osmRaw = JSON.parse(readFileSync(`${SCRATCH}/osm-buildings.json`, 'utf8'));
const osmBuildings = [];
for (const el of osmRaw.elements ?? []) {
  const geom = el.type === 'way' ? el.geometry : el.members?.find((m) => m.role === 'outer')?.geometry;
  if (!geom || geom.length < 3 || !el.tags) continue;
  osmBuildings.push({ ring: geom.map(({ lon, lat }) => projector.toLocal(lon, lat)), tags: el.tags });
}
console.log(`OSM building polygons: ${osmBuildings.length}`);

const footprintStats = {
  traced: 0, fallback: 0, wallFallback: 0, roofFacetsTotal: 0, roofFacetsCovered: 0, floatingBuildings: 0, partHulls: 0,
};
const buildings = transformBuildings({ walls, roofs, osmBuildings, projector, stats: footprintStats });
console.log(`footprints: ${footprintStats.traced} traced, ${footprintStats.fallback} fallback`);
if (footprintStats.fallback / buildings.length > 0.25)
  throw new Error(
    `bake: ${footprintStats.fallback}/${buildings.length} buildings needed a footprint fallback — trace is broken`,
  );
if (footprintStats.wallFallback > 0)
  console.log(`wall facets: ${footprintStats.wallFallback} buildings had 0 wall facets — fell back to footprint prism`);
if (footprintStats.partHulls > 0)
  console.log(
    `per-facet closures: ${footprintStats.partHulls} roof facets had no rendered wall nearby — closed with a ` +
    `ground→eave prism from that facet's own outline`,
  );

// Coverage gate (data proof for the floating-roof root-fix, computed inside
// transformBuildings from the raw per-facet rings — see the comment there):
// wall/roof coverage must clear 90% (target > 95%, vs. the proven-broken
// 659/846 footprint-containment metric this replaces).
const wallRoofQuote = footprintStats.roofFacetsTotal > 0
  ? footprintStats.roofFacetsCovered / footprintStats.roofFacetsTotal
  : 1;
console.log(
  `wall/roof coverage: ${(wallRoofQuote * 100).toFixed(1)}% of roof facets have a wall within 6 m ` +
  `(${footprintStats.floatingBuildings}/${buildings.length} buildings floating)`,
);
if (wallRoofQuote < 0.9)
  throw new Error(`bake: wall/roof coverage only ${(wallRoofQuote * 100).toFixed(1)}% — floating-roof regression`);
const { roads, rails } = transformRoads({
  osmRoads: JSON.parse(readFileSync(`${SCRATCH}/osm-roads.json`, 'utf8')),
  projector,
});

// sanity gates. An absurd height means a broken projection/ground-normalize —
// hard fail. Sub-2 m "buildings" are canopies, ground slabs and degenerate
// solids; drop them with a logged count (not a silent skip), and fail only if
// suspiciously many vanish (that would signal a systemic Z problem).
const MIN_H = 2.0;
const MAX_H = 150;
const tooTall = buildings.find((b) => b.height > MAX_H);
if (tooTall) throw new Error(`bake: implausible height ${tooTall.height} on ${tooTall.id} — projection bug?`);
const buildingsAll = buildings;
const buildingsUsable = buildingsAll.filter((b) => b.height >= MIN_H);
const skipped = buildingsAll.length - buildingsUsable.length;
if (skipped > buildingsAll.length * 0.1)
  throw new Error(`bake: ${skipped}/${buildingsAll.length} sub-${MIN_H}m structures — Z data problem?`);
console.log(`skipped ${skipped} sub-${MIN_H}m structures (canopies/degenerate)`);

const buildingsOut = buildingsUsable;
if (buildingsOut.length < 500) throw new Error(`bake: only ${buildingsOut.length} buildings — bbox/clip broken?`);
const named = buildingsOut.filter((b) => b.name);
const ksw = buildingsOut.filter((b) => b.zone === 'ksw');
if (ksw.length === 0) throw new Error('bake: no buildings in the ksw zone');

// scratch/geo/osm-nature.json is fetched Gemeinde-wide (fetch-winterthur.mjs,
// #119) — this plate bake only covers the small KSW BBOX, so pre-clip
// elements to it (same scope walls/roofs already get via ogr2ogr `-spat`).
// Unlike roads.json, nature.json has no separate Gemeinde-wide re-bake step
// (rebake-roads-gemeinde.mjs's own header: "buildings.json / nature.json
// bleiben unangetastet"), so this plate stays the only source of trees/greens.
function elementInBbox(el, bbox) {
  const pts = el.type === 'node' ? [el] : el.geometry ?? [];
  return pts.some((p) => p.lon >= bbox.lonMin && p.lon <= bbox.lonMax && p.lat >= bbox.latMin && p.lat <= bbox.latMax);
}
const osmNatureRaw = JSON.parse(readFileSync(`${SCRATCH}/osm-nature.json`, 'utf8'));
const NATURE_PAD = 0.001; // ~100 m lon/lat pad so plate-edge greens/rivers keep their full ring
const natureBbox = {
  lonMin: BBOX.lonMin - NATURE_PAD, lonMax: BBOX.lonMax + NATURE_PAD,
  latMin: BBOX.latMin - NATURE_PAD, latMax: BBOX.latMax + NATURE_PAD,
};
const osmNature = { elements: osmNatureRaw.elements.filter((el) => elementInBbox(el, natureBbox)) };
console.log(`nature: ${osmNature.elements.length}/${osmNatureRaw.elements.length} OSM elements in plate bbox`);

const nature = transformNature({
  osmNature,
  projector,
  buildingFootprints: buildingsOut.map((b) => b.footprint),
});
if (nature.trees.length < 3000) throw new Error(`bake: only ${nature.trees.length} trees — nature fetch broken?`);

// doors: bucket every road point into a 50 m grid once, then per building
// query the 9 neighbor cells for nearby road points — O(n) instead of O(n²).
const DOOR_CELL = 50;
const cellKey = (x, z) => `${Math.floor(x / DOOR_CELL)},${Math.floor(z / DOOR_CELL)}`;
const roadGrid = new Map();
for (const r of roads) {
  for (const [x, z] of r.pts) {
    const k = cellKey(x, z);
    (roadGrid.get(k) ?? roadGrid.set(k, []).get(k)).push([x, z]);
  }
}
let withDoor = 0;
for (const b of buildingsOut) {
  const [cx, cz] = b.footprint.reduce(([sx, sz], [x, z]) => [sx + x, sz + z], [0, 0]).map((s) => s / b.footprint.length);
  const gx = Math.floor(cx / DOOR_CELL);
  const gz = Math.floor(cz / DOOR_CELL);
  const nearby = [];
  for (let dx = -1; dx <= 1; dx++) {
    for (let dz = -1; dz <= 1; dz++) {
      const pts = roadGrid.get(`${gx + dx},${gz + dz}`);
      if (pts) nearby.push(...pts);
    }
  }
  const door = doorForBuilding(b.footprint, nearby);
  if (door) {
    b.door = door;
    withDoor += 1;
  }
}
const doorRate = withDoor / buildingsOut.length;
console.log(`doors: ${withDoor}/${buildingsOut.length} buildings (${(doorRate * 100).toFixed(1)}%)`);
if (doorRate < 0.8) throw new Error(`bake: only ${(doorRate * 100).toFixed(1)}% of buildings got a door — door join broken?`);

const triangles = buildingsOut.reduce((n, b) => n + (b.wall.idx.length + b.roof.idx.length) / 3, 0);

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
  counts: {
    buildings: buildingsOut.length, kswBuildings: ksw.length, named: named.length,
    roads: roads.length, rails: rails.length, triangles,
    greens: nature.greens.length, waterAreas: nature.waterAreas.length,
    rivers: nature.rivers.length, trees: nature.trees.length,
  },
  attribution: ['Gebäude: © swisstopo (swissBUILDINGS3D 3.0)', 'Karte: © OpenStreetMap-Mitwirkende (ODbL)'],
  sourceTile: 'swissbuildings3d_3_0_2019_1072-14',
};

// Gate BEFORE writing: a busted budget must not leave half-updated committed
// data on disk (previously the throw came after writeFileSync, so every
// over-budget run still rewrote all four files).
const payloads = {
  meta: JSON.stringify(meta, null, 1),
  buildings: JSON.stringify({ buildings: buildingsOut }),
  roads: JSON.stringify({ roads, rails }),
  nature: JSON.stringify(nature),
};
const total = Object.values(payloads).reduce((n, s) => n + Buffer.byteLength(s), 0);
if (total > MAX_TOTAL_BYTES) {
  throw new Error(`bake: output ${(total / 1e6).toFixed(1)} MB > ${(MAX_TOTAL_BYTES / 1e6).toFixed(0)} MB budget — nothing written`);
}
mkdirSync(OUT, { recursive: true });
for (const [name, payload] of Object.entries(payloads)) writeFileSync(`${OUT}/${name}.json`, payload);
console.log(`bake OK: ${buildingsOut.length} buildings (${ksw.length} ksw, ${named.length} named), ` +
  `${roads.length} roads, ${rails.length} rails, ${triangles} tris, ${(total / 1e6).toFixed(1)} MB`);
