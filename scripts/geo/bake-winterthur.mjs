// scripts/geo/bake-winterthur.mjs
// Offline bake: scratch/geo raw data → data/winterthur/*.json. Runs
// ogr2ogr (GDAL) to pull the three LoD2 layers out of the Esri GDB,
// clipped to the bbox, then hands everything to the pure transform libs.
// Hard-fails on empty extractions or a bloated output — no silent skips.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { ANCHOR, BBOX, makeProjector } from './lib/project.mjs';
import { transformBuildings, transformRoads } from './lib/transform.mjs';

const SCRATCH = 'scratch/geo';
const GDB = `${SCRATCH}/swissBUILDINGS3D_3-0_1072-14.gdb`;
const OUT = 'data/winterthur';
const MAX_TOTAL_BYTES = 8 * 1024 * 1024;

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
// per building. Building_solid flattened to 2D gives the clean footprint.
const surf = ['-nlt', 'MULTIPOLYGON', '-explodecollections', '-dim', 'XYZ'];
const walls = extractLayer('Wall', surf);
const roofs = extractLayer('Roof', surf);
const solids2d = extractLayer('Building_solid', ['-nlt', 'MULTIPOLYGON', '-dim', '2']);

// footprints: Map<uuid, ring[[x,z]]> in local meters (largest ring per uuid)
const footprints = new Map();
for (const f of solids2d.features) {
  const uuid = f.properties?.UUID;
  if (!uuid || !f.geometry) continue;
  const rings = [];
  if (f.geometry.type === 'MultiPolygon') for (const poly of f.geometry.coordinates) rings.push(poly[0]);
  else if (f.geometry.type === 'Polygon') rings.push(f.geometry.coordinates[0]);
  const largest = rings.reduce((best, r) => (!best || r.length > best.length ? r : best), null);
  if (largest) footprints.set(uuid, largest.map(([lon, lat]) => projector.toLocal(lon, lat)));
}
console.log(`footprints: ${footprints.size}`);

// OSM building polygons → local rings for the name join
const osmRaw = JSON.parse(readFileSync(`${SCRATCH}/osm-buildings.json`, 'utf8'));
const osmBuildings = [];
for (const el of osmRaw.elements ?? []) {
  const geom = el.type === 'way' ? el.geometry : el.members?.find((m) => m.role === 'outer')?.geometry;
  if (!geom || geom.length < 3 || !el.tags) continue;
  osmBuildings.push({ ring: geom.map(({ lon, lat }) => projector.toLocal(lon, lat)), tags: el.tags });
}
console.log(`OSM building polygons: ${osmBuildings.length}`);

const buildings = transformBuildings({ walls, roofs, osmBuildings, projector, footprints });
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
  counts: { buildings: buildingsOut.length, kswBuildings: ksw.length, named: named.length, roads: roads.length, rails: rails.length, triangles },
  attribution: ['Gebäude: © swisstopo (swissBUILDINGS3D 3.0)', 'Karte: © OpenStreetMap-Mitwirkende (ODbL)'],
  sourceTile: 'swissbuildings3d_3_0_2019_1072-14',
};

mkdirSync(OUT, { recursive: true });
writeFileSync(`${OUT}/meta.json`, JSON.stringify(meta, null, 1));
writeFileSync(`${OUT}/buildings.json`, JSON.stringify({ buildings: buildingsOut }));
writeFileSync(`${OUT}/roads.json`, JSON.stringify({ roads, rails }));

const total = ['meta', 'buildings', 'roads'].reduce((n, f) => n + statSync(`${OUT}/${f}.json`).size, 0);
if (total > MAX_TOTAL_BYTES) throw new Error(`bake: output ${(total / 1e6).toFixed(1)} MB > 8 MB budget`);
console.log(`bake OK: ${buildingsOut.length} buildings (${ksw.length} ksw, ${named.length} named), ` +
  `${roads.length} roads, ${rails.length} rails, ${triangles} tris, ${(total / 1e6).toFixed(1)} MB`);
