// scripts/geo/bake-sim-world.mjs
// Compact sim-world artifact for world-core: buildings only (id, usage,
// centroid, area, height, road-graph access point) — NO tile pyramid, NO
// meshes. Reads the SAME scratch/geo intermediates as bake-world.mjs and
// builds the SAME road graph (graph.pb semantics), so `access_edge` indexes
// the graph.pb edge list 1:1. Output is byte-deterministic: buildings sorted
// by id, all numbers rounded to 2 decimals, stable field order.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { ANCHOR, makeProjector } from './lib/project.mjs';
import { parseAAIGrid, makeDemSampler } from './lib/dem.mjs';
import { buildRoadGraph } from './lib/graph.mjs';
import { accessPoints, buildSegmentGrid } from './lib/access.mjs';
import { transformBuildings } from './lib/transform.mjs';
import { pointInRing } from './lib/join.mjs';

const SCRATCH = 'scratch/geo';
const OUT = 'data/winterthur/simworld.json';
const BAKE_VERSION = 1;
const CLIP_PAD_M = 4000;
const MIN_H = 2.0;
const MAX_H = 300; // see bake-world.mjs — real 212m industrial mast exists
const ACCESS_NONE = 0xffffffff;

function fail(msg) {
  console.error(`bake-sim-world: ${msg}`);
  process.exit(1);
}

function loadJson(path) {
  if (!existsSync(path)) fail(`missing input ${path} — run \`npm run geo:fetch\` first`);
  return JSON.parse(readFileSync(path, 'utf8'));
}

const r2 = (v) => Math.round(v * 100) / 100;

// Shoelace centroid + absolute area of a local-meter ring (plan Task 1).
function centroidAndArea(ring) {
  let a = 0, cx = 0, cz = 0;
  for (let i = 0; i < ring.length; i++) {
    const [x1, z1] = ring[i], [x2, z2] = ring[(i + 1) % ring.length];
    const w = x1 * z2 - x2 * z1;
    a += w; cx += (x1 + x2) * w; cz += (z1 + z2) * w;
  }
  a /= 2;
  return { x: cx / (6 * a), z: cz / (6 * a), area: Math.abs(a) };
}

// ---- Step 1: boundary (identical to bake-world.mjs) -----------------------
const boundaryFc = loadJson(`${SCRATCH}/boundary-winterthur.geojson`);
const boundaryFeature = boundaryFc.features?.[0];
if (!boundaryFeature?.geometry) fail('boundary geojson has no geometry');
const boundaryGeom = boundaryFeature.geometry;
const outerRings = boundaryGeom.type === 'MultiPolygon'
  ? boundaryGeom.coordinates.map((poly) => poly[0])
  : [boundaryGeom.coordinates[0]];
const outerLonLat = outerRings.reduce((best, r) => (r && r.length > (best?.length ?? 0) ? r : best), null);
if (!outerLonLat || outerLonLat.length < 100)
  fail(`boundary outer ring has ${outerLonLat?.length ?? 0} points — need >= 100`);

const projector = makeProjector(ANCHOR);
const boundaryRing = outerLonLat.map(([lon, lat]) => projector.toLocal(lon, lat));
console.log(`boundary: ${boundaryRing.length} ring points`);

// ---- Step 2: DEM (needed so the road graph matches graph.pb byte-for-byte) -
console.log('parsing DEM (210 MB AAIGrid)...');
const demText = readFileSync(`${SCRATCH}/dem/dem.asc`, 'utf8');
const grid = parseAAIGrid(demText);
const dem = makeDemSampler(grid, projector);
const originH = dem.heightAt(0, 0);
console.log(`DEM: ${grid.ncols}x${grid.nrows} cells, heightAt(0,0)=${originH.toFixed(1)}m`);
if (!(originH >= 430 && originH <= 470))
  fail(`heightAt(0,0)=${originH} outside [430,470] — DEM/projection mismatch`);

// ---- Step 3: buildings from 30 GDBs (same clip + transform as bake-world) --
const gdbList = loadJson(`${SCRATCH}/gdb-list.json`);
if (!Array.isArray(gdbList) || gdbList.length === 0) fail('gdb-list.json is empty');

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
  const file = `${tmpDir}/tmp-${process.pid}-${tag}-${layer.toLowerCase()}.geojson`;
  rmSync(file, { force: true });
  const res = spawnSync('ogr2ogr', ['-f', 'GeoJSON', file, gdb, layer, ...spat, ...surf], { stdio: 'ignore' });
  if (!existsSync(file)) return { type: 'FeatureCollection', features: [] };
  const text = readFileSync(file, 'utf8');
  let fc;
  try {
    fc = JSON.parse(text);
  } catch (err) {
    throw new Error(`bake-sim-world: ogr2ogr output for ${gdb} layer ${layer} is not valid JSON `
      + `(${text.length} bytes, ogr2ogr exit ${res.status}): ${err.message}`);
  }
  fc.features = (fc.features ?? []).filter((f) => f.geometry);
  rmSync(file, { force: true });
  return fc;
}

function insideBoundaryOrPad(cx, cz) {
  if (pointInRing(cx, cz, boundaryRing)) return true;
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
    // Same boundary+4km-pad clip as bake-world.mjs: the sim world must cover
    // exactly the buildings the render world shows (a visible house that no
    // citizen can live in would be a lie), and the plan's 20k–40k gate is
    // calibrated on that inventory (observed 29,450).
    if (!insideBoundaryOrPad(cx, cz)) continue;
    // Keep only the lean fields — same heap-discipline as bake-world.mjs.
    allBuildings.push({ id: b.id, footprint: b.footprint, height: b.height, usage: b.usage });
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

// ---- Step 3b: usage classification (identical regex + 100m bucketing) ------
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
    if (use === 0) continue;
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

// ---- Step 4: road graph (same construction as graph.pb) --------------------
console.log('building road graph...');
const osmRoads = loadJson(`${SCRATCH}/osm-roads.json`);
const graph = buildRoadGraph({ osmRoads, projector, dem });
console.log(`graph: ${graph.nodeOsmId.length} nodes, ${graph.edgeA.length} edges`);
if (graph.edgeA.length < 20000) fail(`only ${graph.edgeA.length} edges — expected >= 20000`);

// ---- Step 5: access points --------------------------------------------------
console.log('computing access points...');
const access = accessPoints({ graph, footprints: allBuildings.map((b) => b.footprint) });
for (let i = 0; i < allBuildings.length; i++) allBuildings[i].access = access[i];

// Wide fallback pass: accessPoints caps its search at 80 m — a render-era
// heuristic (a driveway longer than 80 m isn't drawn). For the SIM, a field
// barn 200 m from the lane still has that lane as its access; on the real
// inventory ~14% of buildings (pad farmland/forest sheds) sit past 80 m,
// which is what the ≥90% gate is about. Bind the leftovers to the nearest
// edge within 500 m (cellRadius 10 × 50 m grid), same two-tier
// drivable-beats-footway ranking and identical offset semantics.
{
  const { candidatesAt, project, arcTo } = buildSegmentGrid(graph);
  let rebound = 0;
  for (const b of allBuildings) {
    if (b.access.edge !== ACCESS_NONE) continue;
    const fp = b.footprint;
    const cx = fp.reduce((s, [x]) => s + x, 0) / fp.length;
    const cz = fp.reduce((s, [, z]) => s + z, 0) / fp.length;
    let best = null;
    for (const { e, i } of candidatesAt(cx, cz, 10)) {
      const p = project(cx, cz, e, i);
      if (p.d > 500) continue;
      const rank = graph.edgeClass[e] <= 6 ? 0 : 1;
      if (!best || rank < best.rank || (rank === best.rank && p.d < best.d))
        best = { rank, d: p.d, edge: e, offsetM: arcTo(e, i, p.t) };
    }
    if (best) {
      b.access = { edge: best.edge, offsetM: Math.round(best.offsetM * 10) / 10 };
      rebound++;
    }
  }
  console.log(`wide-fallback access: bound ${rebound} additional buildings (<=500m)`);
}

// ---- Step 6: emit deterministic JSON ----------------------------------------
// Sort by id; dedupe (a building straddling a GDB tile border can appear in
// two GDBs) keeping the larger-area instance so BuildingId (dense index by
// UUID sort, world-core Task 2) stays unambiguous.
let degenerate = 0;
const rows = allBuildings.flatMap((b) => {
  const { x, z, area } = centroidAndArea(b.footprint);
  // Degenerate footprint (zero shoelace area — collinear/duplicate vertices):
  // the centroid divides by zero -> NaN, and JSON.stringify(NaN) emits null,
  // which SimWorld::load rejects ("expected f32"). Such a building has no
  // area to house anyone anyway — drop it (Task 15 smoke finding: 21 of
  // 29450 buildings in the first committed bake were null).
  if (!Number.isFinite(x) || !Number.isFinite(z)) {
    degenerate++;
    return [];
  }
  return [{
    id: b.id,
    usage: b.usage,
    x: r2(x),
    z: r2(z),
    area_m2: r2(area),
    height_m: r2(b.height),
    access_edge: b.access.edge === ACCESS_NONE ? -1 : b.access.edge,
    access_offset: r2(b.access.offsetM),
  }];
});
if (degenerate > 0) console.log(`dropped ${degenerate} degenerate (zero-area) footprints`);
rows.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : a.area_m2 > b.area_m2 ? -1 : a.area_m2 < b.area_m2 ? 1 : 0));
const buildings = [];
let deduped = 0;
for (const row of rows) {
  if (buildings.length > 0 && buildings[buildings.length - 1].id === row.id) { deduped++; continue; }
  buildings.push(row);
}
if (deduped > 0) console.log(`deduped ${deduped} duplicate ids (GDB tile-border overlaps)`);

// Gates
if (buildings.length < 20000 || buildings.length > 40000)
  fail(`${buildings.length} buildings outside [20000, 40000] gate`);
const withAccess = buildings.filter((b) => b.access_edge >= 0).length;
const accessRate = withAccess / buildings.length;
console.log(`access: ${withAccess}/${buildings.length} bound (${(accessRate * 100).toFixed(1)}%)`);
if (accessRate < 0.9) fail(`only ${(accessRate * 100).toFixed(1)}% of buildings have access_edge — expected >= 90%`);

const out = {
  meta: { anchor: { lon: ANCHOR.lon, lat: ANCHOR.lat }, bake_version: BAKE_VERSION, source: 'bake-world inputs' },
  buildings,
};
mkdirSync('data/winterthur', { recursive: true });
// One building per line: stable bytes AND git-diffable.
const json = `{"meta":${JSON.stringify(out.meta)},"buildings":[\n${buildings.map((b) => JSON.stringify(b)).join(',\n')}\n]}\n`;
writeFileSync(OUT, json);

console.log('');
console.log('=== bake-sim-world OK ===');
console.log(`buildings: ${buildings.length}`);
console.log(`access rate: ${(accessRate * 100).toFixed(1)}%`);
console.log(`output: ${OUT} (${(statSync(OUT).size / 1e6).toFixed(2)} MB)`);
