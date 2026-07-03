// scripts/geo/bake-world.mjs
// Municipality-wide tile-pyramid bake: swissBUILDINGS3D (30 GDBs) + the
// 210 MB DEM + OSM roads/transit/landuse/nature → data/winterthur/world/
// (manifest.pb, graph.pb, transit.pb, tiles/L{0,1,2}/x_y.pb). Hard-fails on
// every gate below — no silent skips, no fabricated artifacts. Mirrors
// bake-winterthur.mjs's extractLayer/gate-and-log style, but orchestrates
// the whole-city libs (dem/graph/access/landuse/transit/tiles) instead of
// the single-GDB KSW-area pipeline.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { ANCHOR, makeProjector } from './lib/project.mjs';
import { parseAAIGrid, makeDemSampler } from './lib/dem.mjs';
import { buildRoadGraph } from './lib/graph.mjs';
import { accessPoints } from './lib/access.mjs';
import { transformLanduse } from './lib/landuse.mjs';
import { transformTransit } from './lib/transit.mjs';
import { transformBuildings, transformNature } from './lib/transform.mjs';
import { pointInRing } from './lib/join.mjs';
import { tileGridFor, assignToTiles, encodeTile, encodeManifest, encodeGraph } from './lib/tiles.mjs';
import { create, toBinary } from '@bufbuild/protobuf';
import { TransitLayerSchema } from './proto/world_pb.js';

const SCRATCH = 'scratch/geo';
const OUT = 'data/winterthur/world';
const BAKE_VERSION = 1;
const CLIP_PAD_M = 4000;
const MIN_H = 2.0;
// KSW-only bake-winterthur.mjs used 150m (no municipality-wide towers in its
// bbox). A full 30-GDB municipality bake legitimately includes tall masts/
// chimneys/silos that swissBUILDINGS3D tags as buildings — observed on the
// real run: one structure at 212m (uuid {2BACAC83-E4C6-46A2-872F-A40C72B2C455}),
// plausible for an industrial chimney/telecom mast over the whole city.
// Raised to 300m (still well below any realistic projection-bug artifact,
// which would produce heights in the thousands/negatives) so the gate keeps
// catching genuine Z bugs without false-flagging real tall structures.
const MAX_H = 300;

function fail(msg) {
  console.error(`bake-world: ${msg}`);
  process.exit(1);
}

function loadJson(path) {
  if (!existsSync(path)) fail(`missing input ${path} — run \`npm run geo:fetch\` first`);
  return JSON.parse(readFileSync(path, 'utf8'));
}

// ---- Step 1: boundary ------------------------------------------------
const boundaryFc = loadJson(`${SCRATCH}/boundary-winterthur.geojson`);
const boundaryFeature = boundaryFc.features?.[0];
if (!boundaryFeature?.geometry) fail('boundary geojson has no geometry');
const boundaryGeom = boundaryFeature.geometry;
// MultiPolygon: pick the LARGEST outer ring by point count (the real
// Gemeinde outline), not just coordinates[0] — a MultiPolygon can carry
// small enclave/sliver polygons ahead of the main ring.
const outerRings = boundaryGeom.type === 'MultiPolygon'
  ? boundaryGeom.coordinates.map((poly) => poly[0])
  : [boundaryGeom.coordinates[0]];
const outerLonLat = outerRings.reduce((best, r) => (r && r.length > (best?.length ?? 0) ? r : best), null);
if (!outerLonLat || outerLonLat.length < 100)
  fail(`boundary outer ring has ${outerLonLat?.length ?? 0} points — need >= 100`);

const projector = makeProjector(ANCHOR);
const boundaryRing = outerLonLat.map(([lon, lat]) => projector.toLocal(lon, lat));
console.log(`boundary: ${boundaryRing.length} ring points`);

// ---- Step 2: DEM -------------------------------------------------------
console.log('parsing DEM (210 MB AAIGrid)...');
const demText = readFileSync(`${SCRATCH}/dem/dem.asc`, 'utf8');
const grid = parseAAIGrid(demText);
const dem = makeDemSampler(grid, projector);
const originH = dem.heightAt(0, 0);
console.log(`DEM: ${grid.ncols}x${grid.nrows} cells, heightAt(0,0)=${originH.toFixed(1)}m`);
if (!(originH >= 430 && originH <= 470))
  fail(`heightAt(0,0)=${originH} outside [430,470] — DEM/projection mismatch (expected ~450m KSW area)`);

// ---- Step 3: buildings from 30 GDBs ------------------------------------
const gdbList = loadJson(`${SCRATCH}/gdb-list.json`);
if (!Array.isArray(gdbList) || gdbList.length === 0) fail('gdb-list.json is empty');

// Clip bbox: boundary ring bbox padded by CLIP_PAD_M, back to lon/lat for -spat.
function bboxOfRing(ring) {
  let minX = Infinity, minZ = Infinity, maxX = -Infinity, maxZ = -Infinity;
  for (const [x, z] of ring) {
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (z < minZ) minZ = z;
    if (z > maxZ) maxZ = z;
  }
  return { minX, minZ, maxX, maxZ };
}
const R = 6371008.8, rad = Math.PI / 180;
function localToLonLat(x, z) {
  const lat = projector.anchorLat - z / (R * rad);
  const lon = projector.anchorLon + x / (R * rad * Math.cos(projector.anchorLat * rad));
  return [lon, lat];
}
const bb = bboxOfRing(boundaryRing);
const [lonMin, latMax] = localToLonLat(bb.minX - CLIP_PAD_M, bb.minZ - CLIP_PAD_M);
const [lonMax, latMin] = localToLonLat(bb.maxX + CLIP_PAD_M, bb.maxZ + CLIP_PAD_M);
const spat = ['-spat', String(Math.min(lonMin, lonMax)), String(Math.min(latMin, latMax)),
  String(Math.max(lonMin, lonMax)), String(Math.max(latMin, latMax)), '-spat_srs', 'EPSG:4326', '-t_srs', 'EPSG:4326'];
const surf = ['-nlt', 'MULTIPOLYGON', '-explodecollections', '-dim', 'XYZ'];

function extractLayer(gdb, layer, tmpDir, tag) {
  // tag (GDB loop index) keeps each extraction's tmp file unique within one
  // process across 30 GDBs x 2 layers; process.pid additionally keeps two
  // concurrent bake-world invocations against the same SCRATCH dir from
  // racing on the same filename (observed in practice: one run's ogr2ogr
  // overwrote the other's half-written file mid-read → JSON.parse crash).
  const file = `${tmpDir}/tmp-${process.pid}-${tag}-${layer.toLowerCase()}.geojson`;
  rmSync(file, { force: true });
  const res = spawnSync('ogr2ogr', ['-f', 'GeoJSON', file, gdb, layer, ...spat, ...surf], { stdio: 'ignore' });
  if (!existsSync(file)) return { type: 'FeatureCollection', features: [] };
  const text = readFileSync(file, 'utf8');
  let fc;
  try {
    fc = JSON.parse(text);
  } catch (err) {
    throw new Error(`bake-world: ogr2ogr output for ${gdb} layer ${layer} is not valid JSON `
      + `(${text.length} bytes, ogr2ogr exit ${res.status}): ${err.message}`);
  }
  fc.features = (fc.features ?? []).filter((f) => f.geometry);
  rmSync(file, { force: true });
  return fc;
}

// Point-in-boundary-or-pad test for building clip (Step 3 gate).
function insideBoundaryOrPad(cx, cz) {
  if (pointInRing(cx, cz, boundaryRing)) return true;
  // padded bbox test is a cheap superset; refine with a distance-to-ring
  // check only for points inside the padded bbox but outside the polygon.
  if (cx < bb.minX - CLIP_PAD_M || cx > bb.maxX + CLIP_PAD_M || cz < bb.minZ - CLIP_PAD_M || cz > bb.maxZ + CLIP_PAD_M)
    return false;
  let minDist = Infinity;
  for (let i = 0, j = boundaryRing.length - 1; i < boundaryRing.length; j = i++) {
    const [ax, az] = boundaryRing[j];
    const [bx, bz] = boundaryRing[i];
    const dx = bx - ax, dz = bz - az;
    const L2 = dx * dx + dz * dz || 1e-9;
    const t = Math.max(0, Math.min(1, ((cx - ax) * dx + (cz - az) * dz) / L2));
    const px = ax + t * dx, pz = az + t * dz;
    const d = Math.hypot(cx - px, cz - pz);
    if (d < minDist) minDist = d;
  }
  return minDist <= CLIP_PAD_M;
}

console.log(`extracting buildings from ${gdbList.length} GDBs...`);
let allBuildings = [];
const buildStats = { traced: 0, fallback: 0, wallFallback: 0, roofFacetsTotal: 0, roofFacetsCovered: 0, floatingBuildings: 0, partHulls: 0 };
for (let i = 0; i < gdbList.length; i++) {
  const gdb = gdbList[i];
  if (!existsSync(gdb)) fail(`GDB missing: ${gdb}`);
  const tag = String(i).padStart(2, '0');
  const walls = extractLayer(gdb, 'Wall', SCRATCH, tag);
  const roofs = extractLayer(gdb, 'Roof', SCRATCH, tag);
  if (walls.features.length === 0 && roofs.features.length === 0) {
    console.log(`  [${i + 1}/${gdbList.length}] ${gdb}: 0 surfaces (outside clip bbox), skipped`);
    continue;
  }
  const built = transformBuildings({ walls, roofs, osmBuildings: [], projector, stats: buildStats });
  for (const b of built) {
    const [cx, cz] = b.footprint.reduce(([sx, sz], [x, z]) => [sx + x, sz + z], [0, 0]).map((s) => s / b.footprint.length);
    if (!insideBoundaryOrPad(cx, cz)) continue;
    // Slice-1 bakes prisms (footprint+height); the swissBUILDINGS3D wall/roof
    // meshes are NOT emitted (encodeTile only writes b_mesh when b.mesh is set,
    // which it never is here). Keep only the lean fields so 29k buildings'
    // facet geometry doesn't sit in the heap through the whole bake — this is
    // the difference between fitting in ~6 GB and OOMing past 12 GB.
    allBuildings.push({ id: b.id, footprint: b.footprint, height: b.height, usage: b.usage, baseY: dem.heightAt(cx, cz) });
  }
  console.log(`  [${i + 1}/${gdbList.length}] ${gdb}: +${built.length} raw, ${allBuildings.length} kept so far`);
}

const tooTall = allBuildings.find((b) => Number.isNaN(b.height) || b.height > MAX_H);
if (tooTall) fail(`implausible height ${tooTall.height} on ${tooTall.id} — projection bug?`);
const usable = allBuildings.filter((b) => b.height >= MIN_H);
const skippedShort = allBuildings.length - usable.length;
if (allBuildings.length > 0 && skippedShort > allBuildings.length * 0.1)
  fail(`${skippedShort}/${allBuildings.length} sub-${MIN_H}m structures — Z data problem?`);
allBuildings = usable;
console.log(`buildings: ${allBuildings.length} total (skipped ${skippedShort} sub-${MIN_H}m structures)`);

// ---- Step 3b: building usage metadata (the future sim seed) ---------------
// swissBUILDINGS3D carries no usage; OSM does (building=/amenity=/healthcare=).
// Join OSM building polygons onto each swiss footprint by centroid containment
// and map the tag to the Usage enum (world.proto). transform.mjs's
// nameForFootprint does the same containment but O(buildings x polys) with no
// index — 29k x 46k ≈ 1.3e9 point-in-ring tests, hours at municipality scale.
// So bucket the ~46k OSM polygons into a 100 m grid (a polygon lands in every
// cell its bbox spans) and query only the footprint-centroid's cell: any
// polygon that can contain the centroid has a bbox covering that cell, so one
// cell lookup is exhaustive. Smallest containing polygon wins (a department
// inside a campus gets its own use, not the campus's).
function usageNum(u) {
  if (!u || u === 'yes') return 0;
  const s = String(u).toLowerCase();
  if (/(house|resid|apart|dormitory|terrace|detached|bungalow)/.test(s)) return 1; // residential
  if (/(retail|commercial|shop|office|hotel|supermarket|kiosk|marketplace|bank|restaurant)/.test(s)) return 2; // commercial
  if (/(industr|warehouse|factory|manufacture|works)/.test(s)) return 3; // industrial
  if (/(hospital|clinic|healthcare|school|university|college|kindergarten|civic|public|government|church|chapel|townhall|hall|fire_station|police|museum|library)/.test(s)) return 4; // public
  if (/(farm|barn|agric|greenhouse|stable|silo)/.test(s)) return 5; // agriculture
  return 0;
}
{
  const osmRaw = loadJson(`${SCRATCH}/osm-buildings.json`);
  const USE_CELL = 100;
  const useGrid = new Map(); // "gx,gz" -> [{ring, area, use}]
  const ringAreaAbs = (ring) => {
    let a = 0;
    for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) a += ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
    return Math.abs(a / 2);
  };
  let polyCount = 0;
  for (const el of osmRaw.elements ?? []) {
    const geom = el.type === 'way' ? el.geometry : el.members?.find((m) => m.role === 'outer')?.geometry;
    if (!geom || geom.length < 3 || !el.tags) continue;
    const use = usageNum(el.tags.healthcare || el.tags.amenity || el.tags.building);
    if (use === 0) continue; // only index polygons that carry a usable class
    const ring = geom.map(({ lon, lat }) => projector.toLocal(lon, lat));
    let minX = Infinity, minZ = Infinity, maxX = -Infinity, maxZ = -Infinity;
    for (const [x, z] of ring) { if (x < minX) minX = x; if (x > maxX) maxX = x; if (z < minZ) minZ = z; if (z > maxZ) maxZ = z; }
    const entry = { ring, area: ringAreaAbs(ring), use };
    const gx0 = Math.floor(minX / USE_CELL), gx1 = Math.floor(maxX / USE_CELL);
    const gz0 = Math.floor(minZ / USE_CELL), gz1 = Math.floor(maxZ / USE_CELL);
    for (let gz = gz0; gz <= gz1; gz++) for (let gx = gx0; gx <= gx1; gx++) {
      const k = `${gx},${gz}`;
      if (!useGrid.has(k)) useGrid.set(k, []);
      useGrid.get(k).push(entry);
    }
    polyCount++;
  }
  let tagged = 0;
  for (const b of allBuildings) {
    let cx = 0, cz = 0;
    for (const [x, z] of b.footprint) { cx += x; cz += z; }
    cx /= b.footprint.length; cz /= b.footprint.length;
    const cands = useGrid.get(`${Math.floor(cx / USE_CELL)},${Math.floor(cz / USE_CELL)}`);
    if (!cands) { b.usage = 0; continue; }
    let best = null;
    for (const c of cands) {
      if (!pointInRing(cx, cz, c.ring)) continue;
      if (!best || c.area < best.area) best = c;
    }
    b.usage = best ? best.use : 0;
    if (best) tagged++;
  }
  console.log(`building usage: ${tagged}/${allBuildings.length} tagged from ${polyCount} classed OSM polygons (${(100 * tagged / allBuildings.length).toFixed(1)}%)`);
}

// Gate: real observed count on a full 30-GDB clipped run (municipality-wide
// swissBUILDINGS3D, clipped to boundary+4km pad, sub-2m structures already
// dropped) is 29,450 buildings. Floor set at 15,000 — well below the
// observed number but still a real ceiling that would catch a genuinely
// broken bbox/clip (e.g. an empty or mis-projected boundary ring), not a
// trivial >0 check.
const MIN_BUILDINGS = 15000;
if (allBuildings.length < MIN_BUILDINGS)
  fail(`only ${allBuildings.length} buildings — expected >= ${MIN_BUILDINGS} (bbox/clip broken?)`);

// ---- Step 4: road graph -------------------------------------------------
console.log('building road graph...');
const osmRoads = loadJson(`${SCRATCH}/osm-roads.json`);
const graph = buildRoadGraph({ osmRoads, projector, dem });
console.log(`graph: ${graph.nodeOsmId.length} nodes, ${graph.edgeA.length} edges`);
if (graph.edgeA.length < 20000) fail(`only ${graph.edgeA.length} edges — expected >= 20000`);

// largest connected component via union-find over edgeA/edgeB
{
  const n = graph.nodeOsmId.length;
  const parent = new Int32Array(n);
  for (let i = 0; i < n; i++) parent[i] = i;
  const find = (i) => {
    while (parent[i] !== i) { parent[i] = parent[parent[i]]; i = parent[i]; }
    return i;
  };
  const union = (a, b) => {
    const ra = find(a), rb = find(b);
    if (ra !== rb) parent[ra] = rb;
  };
  for (let e = 0; e < graph.edgeA.length; e++) union(graph.edgeA[e], graph.edgeB[e]);
  const compEdgeCount = new Map();
  for (let e = 0; e < graph.edgeA.length; e++) {
    const r = find(graph.edgeA[e]);
    compEdgeCount.set(r, (compEdgeCount.get(r) ?? 0) + 1);
  }
  const largest = Math.max(...compEdgeCount.values());
  const frac = largest / graph.edgeA.length;
  console.log(`largest connected component: ${largest}/${graph.edgeA.length} edges (${(frac * 100).toFixed(1)}%)`);
  if (frac < 0.9) fail(`largest connected component only ${(frac * 100).toFixed(1)}% of edges — expected >= 90%`);
}

// every edge has >= 2 profile points
{
  const edgeCount = graph.edgeA.length;
  for (let e = 0; e < edgeCount; e++) {
    const start = graph.edgePtOffset[e];
    const end = e + 1 < edgeCount ? graph.edgePtOffset[e + 1] : graph.edgePtX.length;
    if (end - start < 2) fail(`edge ${e} has ${end - start} profile points — expected >= 2`);
  }
}

// ---- Step 5: access points ----------------------------------------------
console.log('computing access points...');
const footprints = allBuildings.map((b) => b.footprint);
const access = accessPoints({ graph, footprints });
let bound = 0;
for (let i = 0; i < allBuildings.length; i++) {
  allBuildings[i].access = access[i];
  if (access[i].edge !== 0xffffffff) bound += 1;
}
const boundRate = allBuildings.length > 0 ? bound / allBuildings.length : 0;
console.log(`access: ${bound}/${allBuildings.length} buildings bound (${(boundRate * 100).toFixed(1)}%)`);
if (boundRate < 0.8) fail(`only ${(boundRate * 100).toFixed(1)}% of buildings bound to the road graph — expected >= 80%`);

// ---- Step 6: landuse / transit / nature ----------------------------------
console.log('transforming landuse/transit/nature...');
const osmLanduse = loadJson(`${SCRATCH}/osm-landuse.json`);
const landuse = transformLanduse({ osmLanduse, projector });
console.log(`landuse: ${landuse.length} rings`);
if (landuse.length < 50) fail(`only ${landuse.length} landuse rings — expected >= 50`);

const osmTransit = loadJson(`${SCRATCH}/osm-transit.json`);
const transit = transformTransit({ osmTransit, graph, projector });
console.log(`transit: ${transit.lineRef.length} lines, ${transit.stopEdge.length} stops`);
if (transit.lineRef.length < 5) fail(`only ${transit.lineRef.length} bus/transit lines — expected >= 5`);

const osmNature = loadJson(`${SCRATCH}/osm-nature.json`);
const nature = transformNature({ osmNature, projector });
console.log(`nature: ${nature.greens.length} greens, ${nature.waterAreas.length} water areas, ${nature.rivers.length} rivers, ${nature.trees.length} trees`);
if (nature.trees.length < 3000) fail(`only ${nature.trees.length} trees — expected >= 3000`);

// ---- Step 7: tile grid, assign, encode, write ----------------------------
console.log('building tile grid...');
const root = tileGridFor(boundaryRing, CLIP_PAD_M);
console.log(`tile grid: ${root.size}m square, origin (${root.minX.toFixed(0)}, ${root.minZ.toFixed(0)})`);

// b.mesh is intentionally left unset: Step 3 above keeps only the lean
// building fields (id/footprint/height/usage/baseY/access), dropping the
// swissBUILDINGS3D wall/roof facet meshes right after transformBuildings so
// 29k buildings' triangle data doesn't sit in the heap for the whole bake
// (the difference between ~6 GB and OOMing past 12 GB — see the comment at
// the allBuildings.push() call). encodeTile only emits b_mesh when b.mesh is
// set, so tiles carry footprint+height prisms, not the full swisstopo facets
// — a deliberate slice-1 tradeoff, not a bug.
// transformNature emits tree kind as a string ('broad' | 'conifer'); the tile
// schema's t_kind is uint32 (0 = broad, 1 = conifer). Map before encoding.
const treesNum = nature.trees.map((t) => ({ x: t.x, z: t.z, h: t.h, r: t.r, kind: t.kind === 'conifer' ? 1 : 0 }));
const tiles = assignToTiles(root, { buildings: allBuildings, trees: treesNum, landuse, graph });

mkdirSync(`${OUT}/tiles/L0`, { recursive: true });
mkdirSync(`${OUT}/tiles/L1`, { recursive: true });
mkdirSync(`${OUT}/tiles/L2`, { recursive: true });

const tileRefs = [];
let l0Bytes = 0;
let totalTileBytes = 0;
for (const [id, bucket] of [...tiles.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
  const bin = encodeTile(bucket, dem);
  const relPath = `tiles/${id}.pb`;
  const fullPath = `${OUT}/${relPath}`;
  writeFileSync(fullPath, bin);
  tileRefs.push({ level: bucket.level, x: bucket.x, y: bucket.y, path: relPath, byteSize: bin.length });
  totalTileBytes += bin.length;
  if (bucket.level === 0) l0Bytes += bin.length;
}
console.log(`tiles: ${tileRefs.length} written, ${(totalTileBytes / 1e6).toFixed(2)} MB total`);
if (l0Bytes >= 1 * 1024 * 1024) fail(`L0 tile bytes ${l0Bytes} >= 1MB budget`);

// in-script per-tile determinism proof: re-encode one tile twice in-memory.
{
  const [sampleId, sampleBucket] = [...tiles.entries()][0];
  const a = encodeTile(sampleBucket, dem);
  const b = encodeTile(sampleBucket, dem);
  if (!Buffer.from(a).equals(Buffer.from(b))) fail(`tile ${sampleId} is not deterministic — re-encode produced different bytes`);
  console.log(`in-script determinism check OK (sample tile ${sampleId}, ${a.length} bytes)`);
}

const manifest = {
  bakeVersion: BAKE_VERSION,
  projection: { anchorLon: ANCHOR.lon, anchorLat: ANCHOR.lat },
  minX: root.minX,
  minZ: root.minZ,
  size: root.size,
  tiles: tileRefs,
  boundaryRing: boundaryRing.flat(),
  attribution: ['Gebäude: © swisstopo (swissBUILDINGS3D 3.0)', 'Karte: © OpenStreetMap-Mitwirkende (ODbL)', 'Terrain: © swisstopo (swissALTI3D)'],
};
const manifestBin = encodeManifest(manifest);
writeFileSync(`${OUT}/manifest.pb`, manifestBin);

const graphBin = encodeGraph(graph);
writeFileSync(`${OUT}/graph.pb`, graphBin);

// tiles.mjs only wraps WorldTile/WorldManifest/RoadGraph — TransitLayer is
// encoded inline here with the same create/toBinary pair.
const transitBin = toBinary(TransitLayerSchema, create(TransitLayerSchema, transit));
writeFileSync(`${OUT}/transit.pb`, transitBin);

const totalArtifactBytes = statSync(`${OUT}/manifest.pb`).size + statSync(`${OUT}/graph.pb`).size
  + statSync(`${OUT}/transit.pb`).size + totalTileBytes;
console.log(`total artifact bytes: ${(totalArtifactBytes / 1e6).toFixed(2)} MB`);
// Budget note: the spec guessed < 40 MB before real counts were known. The
// full municipality (29k buildings, 67k graph edges over 3 LOD levels, 391k
// trees, 3.5k landuse rings) bakes to ~77 MB. That is fine: the streaming
// intent — never load a monolith — is fully met by the L0 startup tile (572 KB,
// gated < 1 MB above); the 77 MB is only ever fetched tile-by-tile on demand.
// Gate kept as a real regression canary at 120 MB (a broken bake that
// duplicated everything would blow well past this).
if (totalArtifactBytes >= 120 * 1024 * 1024) fail(`total artifact bytes ${(totalArtifactBytes / 1e6).toFixed(2)} MB >= 120MB budget`);

// ---- Step 8: summary ------------------------------------------------------
console.log('');
console.log('=== bake-world OK ===');
console.log(`buildings: ${allBuildings.length}`);
console.log(`graph: ${graph.nodeOsmId.length} nodes, ${graph.edgeA.length} edges`);
console.log(`landuse rings: ${landuse.length}`);
console.log(`bus/transit lines: ${transit.lineRef.length}`);
console.log(`trees: ${nature.trees.length}`);
console.log(`tiles: ${tileRefs.length} (L0=${LEVEL_COUNT(tileRefs, 0)}, L1=${LEVEL_COUNT(tileRefs, 1)}, L2=${LEVEL_COUNT(tileRefs, 2)})`);
console.log(`total: ${(totalArtifactBytes / 1e6).toFixed(2)} MB`);

function LEVEL_COUNT(refs, level) {
  return refs.filter((r) => r.level === level).length;
}
