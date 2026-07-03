// scripts/geo/fetch-winterthur.mjs
// Downloads the raw geodata for the Winterthur bake into scratch/geo/:
// the Gemeinde boundary (swissBOUNDARIES3D), the DEM (swissALTI3D 2 m,
// mosaicked + resampled to ~5 m), all swissBUILDINGS3D 3.0 tiles covering
// the Gemeinde, and the OSM overlay (buildings, roads incl. restrictions,
// transit, landuse, nature) via Overpass — all for the full Gemeinde bbox.
// Network-only step — the bake itself (bake-winterthur.mjs) then runs offline.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { stacItemUrls } from './lib/stac.mjs';

const OUT = 'scratch/geo';
// Gemeinde-bbox Winterthur (verified against swissBOUNDARIES3D extent below)
const GEMEINDE_BBOX = '8.63,47.44,8.81,47.57'; // lonMin,latMin,lonMax,latMax (STAC-Ordnung)
const OSM_BBOX = '47.44,8.63,47.57,8.81'; // Overpass-Ordnung S,W,N,E

const MIRRORS = ['https://overpass-api.de/api/interpreter', 'https://overpass.kumi.systems/api/interpreter'];

// Overpass is picky: it 406s without a form Content-Type and 429s without a
// meaningful User-Agent; individual mirrors also 504 transiently under load.
// So: real headers, and a few rounds over the mirrors before giving up.
const HEADERS = {
  'User-Agent': 'abutown-ksw-diorama/1.0 (winterthur geodata bake)',
  'Content-Type': 'application/x-www-form-urlencoded',
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function overpassRaw(query) {
  for (let attempt = 0; attempt < 4; attempt++) {
    for (const url of MIRRORS) {
      let res;
      try {
        res = await fetch(url, { method: 'POST', headers: HEADERS, body: 'data=' + encodeURIComponent(query) });
      } catch {
        continue; // network hiccup — next mirror
      }
      if (!res.ok) continue; // 429/504/… — next mirror
      const text = await res.text();
      let json;
      try {
        json = JSON.parse(text);
      } catch {
        continue; // mirror returned an HTML error page — try the next one
      }
      return json;
    }
    console.log(`  all mirrors busy, retrying in 8s (attempt ${attempt + 1}/4)…`);
    await sleep(8000);
  }
  return null; // all mirrors/attempts exhausted for this query
}

// Splits an S,W,N,E bbox string into two halves along the longer axis.
function splitBbox(bbox) {
  const [s, w, n, e] = bbox.split(',').map(Number);
  const dLat = n - s;
  const dLon = e - w;
  if (dLat >= dLon) {
    const mid = (s + n) / 2;
    return [`${s},${w},${mid},${e}`, `${mid},${w},${n},${e}`];
  }
  const mid = (w + e) / 2;
  return [`${s},${w},${n},${mid}`, `${s},${mid},${n},${e}`];
}

// Runs a query template (a function bbox -> query string) against `bbox`.
// On repeated total failure (504s on all mirrors), splits the bbox in half
// and merges the resulting `elements` arrays into one artifact — still one
// file per artifact, as required.
async function overpassSplit(queryFor, bbox, outfile, depth = 0) {
  const json = await overpassRaw(queryFor(bbox));
  if (json) {
    return json.elements ?? [];
  }
  if (depth >= 3) {
    throw new Error(`all Overpass mirrors failed for ${outfile} (bbox ${bbox}), even after splitting`);
  }
  console.log(`  query for ${outfile} over bbox ${bbox} failed on all mirrors — splitting bbox and retrying`);
  const [a, b] = splitBbox(bbox);
  const elsA = await overpassSplit(queryFor, a, outfile, depth + 1);
  const elsB = await overpassSplit(queryFor, b, outfile, depth + 1);
  // de-dupe by type+id since split halves can share boundary elements
  const seen = new Set();
  const merged = [];
  for (const el of [...elsA, ...elsB]) {
    const key = `${el.type}/${el.id}`;
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(el);
  }
  return merged;
}

async function overpass(queryFor, bbox, outfile, minElements) {
  const elements = await overpassSplit(queryFor, bbox, outfile);
  if (elements.length < minElements) {
    throw new Error(
      `${outfile}: only ${elements.length} elements, expected at least ${minElements} — query or bbox regression?`,
    );
  }
  const payload = JSON.stringify({ elements });
  writeFileSync(outfile, payload);
  console.log(`wrote ${outfile} (${(payload.length / 1024).toFixed(0)} KB, ${elements.length} elements)`);
}

mkdirSync(OUT, { recursive: true });
mkdirSync(`${OUT}/dem/tiles`, { recursive: true });

// ---------------------------------------------------------------------------
// Step 1: Gemeinde boundary (swissBOUNDARIES3D)
// Verified via ogrinfo on the downloaded GPKG (2026-07-03): the zip contains
// swissBOUNDARIES3D_1_5_LV95_LN02.gpkg with layer `tlm_hoheitsgebiet`
// (3D Multi Polygon) and string field `name` — filtering `name = 'Winterthur'`
// yields exactly 1 feature (bfs_nummer 230). Matches the brief's assumption.
// ---------------------------------------------------------------------------
const BOUND_URL =
  'https://data.geo.admin.ch/ch.swisstopo.swissboundaries3d/swissboundaries3d_2026-01/swissboundaries3d_2026-01_2056_5728.gpkg.zip';
const BOUND_GPKG = `${OUT}/swissboundaries3d.gpkg`;
if (!existsSync(`${OUT}/boundary-winterthur.geojson`)) {
  if (!existsSync(BOUND_GPKG)) {
    console.log('downloading swissBOUNDARIES3D (37 MB)…');
    execFileSync('curl', ['-sfL', '-o', `${OUT}/bnd.zip`, BOUND_URL], { stdio: 'inherit' });
    execFileSync('unzip', ['-o', '-d', OUT, `${OUT}/bnd.zip`], { stdio: 'inherit' });
    // entpackter Name kann variieren — erste .gpkg im OUT übernehmen
    const gpkg = readdirSync(OUT).find((f) => f.endsWith('.gpkg'));
    if (!gpkg) throw new Error('boundaries: no .gpkg in zip');
    renameSync(`${OUT}/${gpkg}`, BOUND_GPKG);
  }
  execFileSync(
    'ogr2ogr',
    [
      '-f',
      'GeoJSON',
      `${OUT}/boundary-winterthur.geojson`,
      BOUND_GPKG,
      '-t_srs',
      'EPSG:4326',
      '-where',
      "name = 'Winterthur'",
      'tlm_hoheitsgebiet',
    ],
    { stdio: 'inherit' },
  );
} else {
  console.log('boundary already present, skipping download');
}
const boundary = JSON.parse(readFileSync(`${OUT}/boundary-winterthur.geojson`, 'utf8'));
if (!boundary.features?.length) throw new Error('boundaries: Winterthur polygon missing');
if (boundary.features.length !== 1)
  throw new Error(`boundaries: expected exactly 1 Winterthur feature, got ${boundary.features.length}`);
console.log(`boundary: ${boundary.features.length} feature(s)`);

// ---------------------------------------------------------------------------
// Step 2: DEM tiles via STAC — list, download, mosaic, resample, AAIGrid
// Verified via curl on the STAC items endpoint (2026-07-03): item ids follow
// `swissalti3d_<year>_<tile>`, and the 2 m COG asset key is
// `..._2_2056_5728.tif` — matches the brief's assetSuffix exactly.
// ---------------------------------------------------------------------------
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

if (!existsSync(`${OUT}/dem/dem.asc`)) {
  const demUrls = stacItemUrls({
    pageJsonList: await stacPages('ch.swisstopo.swissalti3d'),
    assetSuffix: '_2_2056_5728.tif',
  });
  if (demUrls.length < 100) throw new Error(`DEM: only ${demUrls.length} tiles — bbox wrong?`);
  console.log(`DEM: ${demUrls.length} tiles to fetch`);
  let downloaded = 0;
  for (const u of demUrls) {
    const f = `${OUT}/dem/tiles/${u.split('/').pop()}`;
    if (!existsSync(f)) {
      execFileSync('curl', ['-sf', '-o', f, u], { stdio: 'inherit' });
    }
    downloaded++;
    if (downloaded % 25 === 0 || downloaded === demUrls.length) {
      console.log(`  DEM tiles: ${downloaded}/${demUrls.length}`);
    }
  }
  // Mosaik → EPSG:4326 → ~5 m → ein ASCII-Grid (bake parst plain text, kein npm-Dep)
  execFileSync(
    'gdalbuildvrt',
    [`${OUT}/dem/dem.vrt`, ...readdirSync(`${OUT}/dem/tiles`).map((f) => `${OUT}/dem/tiles/${f}`)],
    { stdio: 'inherit' },
  );
  execFileSync(
    'gdalwarp',
    [
      '-t_srs',
      'EPSG:4326',
      '-tr',
      '0.00006',
      '0.00004',
      '-r',
      'bilinear',
      '-overwrite',
      `${OUT}/dem/dem.vrt`,
      `${OUT}/dem/dem4326.tif`,
    ],
    { stdio: 'inherit' },
  );
  execFileSync('gdal_translate', ['-of', 'AAIGrid', `${OUT}/dem/dem4326.tif`, `${OUT}/dem/dem.asc`], {
    stdio: 'inherit',
  });
  if (!existsSync(`${OUT}/dem/dem.asc`)) throw new Error('DEM: AAIGrid conversion failed');
} else {
  console.log('DEM mosaic already present, skipping');
}

// ---------------------------------------------------------------------------
// Step 3: swissBUILDINGS3D tiles, multi-tile via STAC
// Verified via curl on the STAC items endpoint (2026-07-03): item ids follow
// `swissbuildings3d_3_0_<year>_<tile>`, asset key `..._2056_5728.gdb.zip` —
// matches the brief's assetSuffix exactly.
// ---------------------------------------------------------------------------
if (!existsSync(`${OUT}/gdb-list.json`)) {
  const gdbUrls = stacItemUrls({
    pageJsonList: await stacPages('ch.swisstopo.swissbuildings3d_3_0'),
    assetSuffix: '.gdb.zip',
  });
  if (gdbUrls.length < 20)
    throw new Error(`swissBUILDINGS3D: only ${gdbUrls.length} tiles found (expected ~30) — bbox wrong?`);
  console.log(`swissBUILDINGS3D: ${gdbUrls.length} tiles to fetch`);
  const gdbDirs = [];
  let downloaded = 0;
  for (const u of gdbUrls) {
    const base = u.split('/').pop().replace(/\.gdb\.zip$/, ''); // e.g. swissbuildings3d_3_0_2019_1051-44_2056_5728
    const tileMatch = /_(\d+-\d+)_\d+_\d+$/.exec(base);
    if (!tileMatch) throw new Error(`swissBUILDINGS3D: cannot parse tile id from ${base}`);
    // The zip's internal folder uses swisstopo's display convention, not the
    // download filename — verified via `unzip -l` on a real tile (2026-07-03):
    // swissBUILDINGS3D_3-0_<tile>.gdb, matching the pre-existing single-tile pattern.
    const gdbDir = `${OUT}/swissBUILDINGS3D_3-0_${tileMatch[1]}.gdb`;
    if (!existsSync(gdbDir)) {
      const zipPath = `${OUT}/${base}.gdb.zip`;
      if (!existsSync(zipPath)) {
        execFileSync('curl', ['-sf', '-o', zipPath, u], { stdio: 'inherit' });
      }
      execFileSync('unzip', ['-oq', zipPath, '-d', OUT], { stdio: 'inherit' });
    }
    gdbDirs.push(gdbDir);
    downloaded++;
    if (downloaded % 10 === 0 || downloaded === gdbUrls.length) {
      console.log(`  GDB tiles: ${downloaded}/${gdbUrls.length}`);
    }
  }
  writeFileSync(`${OUT}/gdb-list.json`, JSON.stringify(gdbDirs, null, 2));
  console.log(`wrote ${OUT}/gdb-list.json (${gdbDirs.length} GDB dirs)`);
} else {
  console.log('GDB tile list already present, skipping download');
}

// ---------------------------------------------------------------------------
// Step 4: Overpass queries — full Gemeinde bbox, extended layers
// Output mode `out body geom;` (not `out tags geom;`): verified via a live
// probe (2026-07-03) that this is the only single-pass combination that
// gives BOTH `nodes` (node-id list, needed by the graph builder) AND
// `geometry` (coords) on ways, AND `members[].geometry` on relations (route
// relations, restriction relations) — `out tags geom;` omits `members`
// entirely for relations, and `out geom;` without `body` omits `nodes`.
// ---------------------------------------------------------------------------
await overpass(
  (bbox) =>
    `[out:json][timeout:180];(way["highway"](${bbox});way["railway"~"^(rail|tram)$"](${bbox});rel["type"="restriction"](${bbox});node["highway"="traffic_signals"](${bbox}););out body geom;`,
  OSM_BBOX,
  `${OUT}/osm-roads.json`,
  10000, // observed ~35,584
);
await overpass(
  (bbox) =>
    `[out:json][timeout:180];(rel["type"="route"]["route"~"^(bus|tram|train)$"](${bbox});node["public_transport"="platform"](${bbox});node["highway"="bus_stop"](${bbox}););out body geom;`,
  OSM_BBOX,
  `${OUT}/osm-transit.json`,
  100, // observed ~676
);
await overpass(
  (bbox) => `[out:json][timeout:180];(way["landuse"](${bbox});rel["landuse"](${bbox}););out body geom;`,
  OSM_BBOX,
  `${OUT}/osm-landuse.json`,
  500, // observed ~4,483
);
// out geom: full geometry for the polygon join (names sit on building areas)
await overpass(
  (bbox) => `[out:json][timeout:180];(way["building"](${bbox});relation["building"](${bbox}););out body geom;`,
  OSM_BBOX,
  `${OUT}/osm-buildings.json`,
  10000, // observed ~46,111
);
// nature: green areas, woods, water bodies, the Eulach, and individual trees
await overpass(
  (bbox) => `[out:json][timeout:180];(
    way["leisure"~"^(park|garden|pitch|playground)$"](${bbox});
    way["landuse"~"^(grass|meadow|forest|cemetery|village_green|recreation_ground|allotments)$"](${bbox});
    way["natural"~"^(wood|scrub|grassland|water)$"](${bbox});
    way["waterway"~"^(river|stream)$"](${bbox});
    node["natural"="tree"](${bbox});
  );out body geom;`,
  OSM_BBOX,
  `${OUT}/osm-nature.json`,
  5000, // observed ~134,994
);
// traffic control nodes (signals, stop/give-way, crossings) — the traffic-net
// bake reads these to classify intersections (bake-traffic-net.mjs)
await overpass(
  (bbox) => `[out:json][timeout:60];(
    node["highway"~"^(traffic_signals|stop|give_way|crossing)$"](${bbox});
  );out;`,
  OSM_BBOX,
  `${OUT}/osm-traffic-nodes.json`,
  1, // sentinel-only floor; count varies with real intersection density
);
console.log('fetch complete');
