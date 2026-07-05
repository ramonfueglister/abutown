// scripts/geo/fetch-demand-data.mjs
// Downloads the public census inputs for the traffic demand bake into
// scratch/demand/ (gitignored):
//   statpop.csv       — BFS STATPOP hectare grid (E_KOORD,N_KOORD,BBTOT),
//                       filtered to hectares inside the Winterthur Gemeinde
//                       boundary polygon (bbox alone over-counts: it sums to
//                       ~179 k because it includes Seuzach/Wiesendangen/…;
//                       the polygon sum is ~120 k ≈ Winterthur's population).
//   pendlermatrix.csv — BFS Pendlermobilität Gemeindematrix (commune→commune
//                       worker flows), rows where origin or destination is
//                       BFS-Nr 230 (Winterthur). No mode split published →
//                       total workers.
//   communes.csv      — BFS-Nr, name, centroid lon/lat for all Swiss
//                       communes, extracted from swissBOUNDARIES3D
//                       (layer tlm_hoheitsgebiet) via ogr2ogr.
// Idempotent: existing outputs are kept unless --force is passed.
import { execFileSync } from 'node:child_process';
import { createReadStream, existsSync, mkdirSync, readFileSync, renameSync, readdirSync, writeFileSync } from 'node:fs';
import { createInterface } from 'node:readline';

const OUT = 'scratch/demand';
const RAW = `${OUT}/raw`;
const GEO = 'scratch/geo';
const FORCE = process.argv.includes('--force');

// Winterthur Gemeinde bbox (lonMin,latMin,lonMax,latMax) — cheap prefilter
// before the exact point-in-polygon test against the Gemeinde boundary.
const BBOX = { lonMin: 8.63, latMin: 47.44, lonMax: 8.81, latMax: 47.57 };
const WINTERTHUR_BFS = 230;

// ---------------------------------------------------------------------------
// Pinned BFS DAM assets (download pattern:
// https://dam-api.bfs.admin.ch/hub/api/dam/assets/<id>/master).
// Resolved 2026-07-05 from the BFS statistics pages — re-resolve there if a
// download 404s (BFS re-publishes each vintage under a NEW asset id):
//   STATPOP Geodaten: https://www.bfs.admin.ch/bfs/de/home/statistiken/bevoelkerung/erhebungen/statpop.html
//   Pendlermatrix:    https://www.bfs.admin.ch/bfs/de/home/statistiken/mobilitaet-verkehr/personenverkehr/pendlermobilitaet.html
// ---------------------------------------------------------------------------
// "Statistik der Bevölkerung und Haushalte (STATPOP), Geodaten 2024"
// (order nr ag-b-00.03-vz2024statpop, published 2025). ZIP containing
// STATPOP2024.csv — semicolon-separated hectare grid, LV95 SW-corner coords
// E_KOORD/N_KOORD, total residents column BBTOT (the 2024 vintage dropped
// the year infix; older vintages named it B23BTOT etc.).
const STATPOP_ASSET = 36079999;
// "Pendlermobilität: Gemeindematrix 2020" (published 2023-05, the newest
// matrix release as of 2026-07 — the 2022 Pendler publication has no
// commune matrix). Asset 27885394 is the CSV variant (27885387 is the same
// data as XLSX): columns PERSPECTIVE (R = residence-side estimate, W =
// work-side), REF_YEAR (2014/2018/2020), GEO_CANT_RESID, GEO_COMM_RESID,
// GEO_CANT_WORK, GEO_COMM_WORK, VALUE. GEO_COMM_* are national BFS numbers.
const PENDLER_ASSET = 27885394;
const PENDLER_YEAR = '2020';
const PENDLER_PERSPECTIVE = 'R'; // residence-side estimate (matches STATPOP homes)
// swissBOUNDARIES3D — same release fetch-winterthur.mjs uses.
const BOUND_URL =
  'https://data.geo.admin.ch/ch.swisstopo.swissboundaries3d/swissboundaries3d_2026-01/swissboundaries3d_2026-01_2056_5728.gpkg.zip';

const damUrl = (id) => `https://dam-api.bfs.admin.ch/hub/api/dam/assets/${id}/master`;

function download(url, dest, what) {
  console.log(`downloading ${what}…`);
  try {
    execFileSync('curl', ['-sfL', '-o', dest, url], { stdio: 'inherit' });
  } catch {
    throw new Error(
      `${what}: download failed (${url}). BFS re-publishes each vintage under a new DAM asset id — ` +
        `re-resolve the id from the BFS page named in the header comment and update the pinned constant.`,
    );
  }
}

// ---------------------------------------------------------------------------
// LV95 → WGS84, swisstopo's published approximation formulas
// ("Näherungslösungen für die direkte Transformation CH1903 ⇔ WGS84").
// Accurate to ~1 m — plenty for hectare cells and commune centroids.
// ---------------------------------------------------------------------------
function lv95ToWgs84(E, N) {
  const y = (E - 2600000) / 1e6;
  const x = (N - 1200000) / 1e6;
  const lambda = 2.6779094 + 4.728982 * y + 0.791484 * y * x + 0.1306 * y * x * x - 0.0436 * y * y * y;
  const phi =
    16.9023892 + 3.238272 * x - 0.270978 * y * y - 0.002528 * x * x - 0.0447 * y * y * x - 0.014 * x * x * x;
  return [(lambda * 100) / 36, (phi * 100) / 36]; // [lon, lat]
}

// Unit assertion near Winterthur: LV95 2696000/1261500 ≈ 8.72°E / 47.50°N.
{
  const [lon, lat] = lv95ToWgs84(2696000, 1261500);
  if (Math.abs(lon - 8.7127) > 0.002 || Math.abs(lat - 47.4972) > 0.002) {
    throw new Error(`lv95ToWgs84 self-check failed: got ${lon}, ${lat} (expected ≈8.7127, 47.4972)`);
  }
}

mkdirSync(RAW, { recursive: true });

// ---------------------------------------------------------------------------
// Step 0: Winterthur boundary polygon + swissBOUNDARIES3D GPKG.
// The boundary usually already exists from geo:fetch; the GPKG is needed for
// communes.csv either way, and can regenerate the boundary if it's missing.
// ---------------------------------------------------------------------------
const BOUND_GPKG = `${GEO}/swissboundaries3d.gpkg`;
const BOUNDARY_GJ = `${GEO}/boundary-winterthur.geojson`;
const needGpkg = !existsSync(BOUND_GPKG) && (!existsSync(BOUNDARY_GJ) || !existsSync(`${OUT}/communes.csv`) || FORCE);
if (needGpkg) {
  mkdirSync(GEO, { recursive: true });
  download(BOUND_URL, `${GEO}/bnd.zip`, 'swissBOUNDARIES3D (37 MB)');
  execFileSync('unzip', ['-o', '-q', '-d', GEO, `${GEO}/bnd.zip`], { stdio: 'inherit' });
  const gpkg = readdirSync(GEO).find((f) => f.endsWith('.gpkg') && f !== 'swissboundaries3d.gpkg');
  if (!gpkg) throw new Error('boundaries: no .gpkg in zip');
  renameSync(`${GEO}/${gpkg}`, BOUND_GPKG);
}
if (!existsSync(BOUNDARY_GJ)) {
  execFileSync(
    'ogr2ogr',
    ['-f', 'GeoJSON', BOUNDARY_GJ, BOUND_GPKG, '-t_srs', 'EPSG:4326', '-where', "name = 'Winterthur'", 'tlm_hoheitsgebiet'],
    { stdio: 'inherit' },
  );
}
const boundaryDoc = JSON.parse(readFileSync(BOUNDARY_GJ, 'utf8'));
if (boundaryDoc.features?.length !== 1) {
  throw new Error(`boundary: expected exactly 1 Winterthur feature, got ${boundaryDoc.features?.length ?? 0}`);
}
const geom = boundaryDoc.features[0].geometry;
const polys = geom.type === 'MultiPolygon' ? geom.coordinates : [geom.coordinates];
function inRing(lon, lat, ring) {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i];
    const [xj, yj] = ring[j];
    if (yi > lat !== yj > lat && lon < ((xj - xi) * (lat - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}
const insideGemeinde = (lon, lat) =>
  polys.some((rings) => inRing(lon, lat, rings[0]) && rings.slice(1).every((hole) => !inRing(lon, lat, hole)));

// ---------------------------------------------------------------------------
// Step 1: statpop.csv — hectares inside the Gemeinde, E_KOORD,N_KOORD,BBTOT
// ---------------------------------------------------------------------------
let statpopRows = 0;
let statpopPop = 0;
async function buildStatpop() {
  const zip = `${RAW}/statpop-geodaten-2024.zip`;
  const rawCsv = `${RAW}/STATPOP2024.csv`;
  if (!existsSync(rawCsv) || FORCE) {
    if (!existsSync(zip) || FORCE) download(damUrl(STATPOP_ASSET), zip, `STATPOP Geodaten 2024 (asset ${STATPOP_ASSET}, ~11 MB)`);
    execFileSync('unzip', ['-o', '-q', zip, 'STATPOP2024.csv', '-d', RAW], { stdio: 'inherit' });
  }
  const rl = createInterface({ input: createReadStream(rawCsv) });
  const out = ['E_KOORD,N_KOORD,BBTOT'];
  let header = null;
  let iE, iN, iB;
  for await (const line of rl) {
    const cols = line.split(';');
    if (!header) {
      header = cols.map((c) => c.replaceAll('"', ''));
      iE = header.indexOf('E_KOORD');
      iN = header.indexOf('N_KOORD');
      iB = header.indexOf('BBTOT');
      if (iE < 0 || iN < 0 || iB < 0) {
        throw new Error(`STATPOP: expected columns E_KOORD/N_KOORD/BBTOT, got: ${header.slice(0, 8).join(', ')}… (new vintage renamed the total column?)`);
      }
      continue;
    }
    const E = Number(cols[iE]);
    const N = Number(cols[iN]);
    const pop = Number(cols[iB]);
    // hectare cell center (coords are the SW corner of the 100 m cell)
    const [lon, lat] = lv95ToWgs84(E + 50, N + 50);
    if (lon < BBOX.lonMin || lon > BBOX.lonMax || lat < BBOX.latMin || lat > BBOX.latMax) continue;
    if (!insideGemeinde(lon, lat)) continue;
    out.push(`${cols[iE]},${cols[iN]},${pop}`);
    statpopRows++;
    statpopPop += pop;
  }
  writeFileSync(`${OUT}/statpop.csv`, out.join('\n') + '\n');
}

// ---------------------------------------------------------------------------
// Step 2: pendlermatrix.csv — origin_bfs,dest_bfs,workers (Winterthur rows)
// ---------------------------------------------------------------------------
let pendlerRows = 0;
async function buildPendler() {
  const rawCsv = `${RAW}/pendler-gemeindematrix.csv`;
  if (!existsSync(rawCsv) || FORCE) download(damUrl(PENDLER_ASSET), rawCsv, `Pendlermobilität Gemeindematrix (asset ${PENDLER_ASSET}, ~17 MB)`);
  const rl = createInterface({ input: createReadStream(rawCsv) });
  const out = ['origin_bfs,dest_bfs,workers'];
  let header = null;
  let iP, iY, iO, iD, iV;
  for await (const rawLine of rl) {
    const line = rawLine.replace(/^﻿/, '');
    const cols = line.split(',').map((c) => c.replaceAll('"', ''));
    if (!header) {
      header = cols;
      iP = header.indexOf('PERSPECTIVE');
      iY = header.indexOf('REF_YEAR');
      iO = header.indexOf('GEO_COMM_RESID');
      iD = header.indexOf('GEO_COMM_WORK');
      iV = header.indexOf('VALUE');
      if ([iP, iY, iO, iD, iV].some((i) => i < 0)) {
        throw new Error(`Pendlermatrix: unexpected header: ${header.join(', ')}`);
      }
      continue;
    }
    if (cols[iP] !== PENDLER_PERSPECTIVE || cols[iY] !== PENDLER_YEAR) continue;
    const origin = Number(cols[iO]);
    const dest = Number(cols[iD]);
    if (origin !== WINTERTHUR_BFS && dest !== WINTERTHUR_BFS) continue;
    const workers = Number(cols[iV]);
    if (!Number.isFinite(workers)) throw new Error(`Pendlermatrix: non-numeric VALUE in: ${line}`);
    out.push(`${origin},${dest},${workers}`);
    pendlerRows++;
  }
  writeFileSync(`${OUT}/pendlermatrix.csv`, out.join('\n') + '\n');
}

// ---------------------------------------------------------------------------
// Step 3: communes.csv — bfs_nr,name,lon,lat for all Swiss communes
// ---------------------------------------------------------------------------
let communeRows = 0;
function buildCommunes() {
  // LV95 centroids straight from the GPKG (SQLite dialect for ST_Centroid);
  // WGS84 conversion via lv95ToWgs84 above, consistent with the STATPOP path.
  const csv = execFileSync(
    'ogr2ogr',
    [
      '-f', 'CSV', '/vsistdout/', BOUND_GPKG,
      '-dialect', 'sqlite',
      '-sql',
      // icc = 'CH' drops Liechtenstein + foreign enclaves; bfs_nummer is
      // duplicated for multi-part communes → dedupe below keeps the first.
      "SELECT bfs_nummer, name, ST_X(ST_Centroid(geom)) AS ce, ST_Y(ST_Centroid(geom)) AS cn FROM tlm_hoheitsgebiet WHERE icc = 'CH' ORDER BY bfs_nummer",
    ],
    { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  );
  const lines = csv.trim().split('\n');
  const out = ['bfs_nr,name,lon,lat'];
  const seen = new Set();
  for (const line of lines.slice(1)) {
    // name may contain commas (e.g. "Vals, Teil") → parse quoted CSV lightly
    const m = /^"?(\d+)"?,(?:"((?:[^"]|"")*)"|([^,]*)),([-\d.]+),([-\d.]+)$/.exec(line);
    if (!m) throw new Error(`communes: unparseable ogr2ogr CSV line: ${line}`);
    const bfs = Number(m[1]);
    if (seen.has(bfs)) continue;
    seen.add(bfs);
    const name = (m[2] ?? m[3]).replaceAll('""', '"');
    const [lon, lat] = lv95ToWgs84(Number(m[4]), Number(m[5]));
    out.push(`${bfs},"${name.replaceAll('"', '""')}",${lon.toFixed(6)},${lat.toFixed(6)}`);
    communeRows++;
  }
  writeFileSync(`${OUT}/communes.csv`, out.join('\n') + '\n');
}

// ---------------------------------------------------------------------------
// Run + sanity gates
// ---------------------------------------------------------------------------
if (existsSync(`${OUT}/statpop.csv`) && !FORCE) {
  console.log('statpop.csv already present, skipping (use --force to refetch)');
} else {
  await buildStatpop();
  console.log(`wrote ${OUT}/statpop.csv (${statpopRows} hectares, population ${statpopPop})`);
  if (statpopPop < 100000 || statpopPop > 140000) {
    throw new Error(`STATPOP sanity failed: Winterthur population sum ${statpopPop}, expected 100k–140k`);
  }
}
if (existsSync(`${OUT}/pendlermatrix.csv`) && !FORCE) {
  console.log('pendlermatrix.csv already present, skipping (use --force to refetch)');
} else {
  await buildPendler();
  console.log(`wrote ${OUT}/pendlermatrix.csv (${pendlerRows} Winterthur flows, ${PENDLER_YEAR}, perspective ${PENDLER_PERSPECTIVE})`);
  if (pendlerRows < 100) throw new Error(`Pendlermatrix sanity failed: only ${pendlerRows} Winterthur rows`);
}
if (existsSync(`${OUT}/communes.csv`) && !FORCE) {
  console.log('communes.csv already present, skipping (use --force to refetch)');
} else {
  buildCommunes();
  console.log(`wrote ${OUT}/communes.csv (${communeRows} communes)`);
  if (communeRows < 2100 || communeRows > 2300) {
    throw new Error(`communes sanity failed: ${communeRows} communes, expected 2100–2300`);
  }
}
console.log('demand data fetch complete');
