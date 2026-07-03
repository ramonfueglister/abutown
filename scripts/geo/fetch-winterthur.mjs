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

// Overpass is picky: it 406s without a form Content-Type and 429s without a
// meaningful User-Agent; individual mirrors also 504 transiently under load.
// So: real headers, and a few rounds over the mirrors before giving up.
const HEADERS = {
  'User-Agent': 'abutown-ksw-diorama/1.0 (winterthur geodata bake)',
  'Content-Type': 'application/x-www-form-urlencoded',
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function overpass(query, outfile) {
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
      try {
        JSON.parse(text);
      } catch {
        continue; // mirror returned an HTML error page — try the next one
      }
      writeFileSync(outfile, text);
      console.log(`wrote ${outfile} (${(text.length / 1024).toFixed(0)} KB)`);
      return;
    }
    console.log(`  all mirrors busy, retrying in 8s (attempt ${attempt + 1}/4)…`);
    await sleep(8000);
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
// nature: green areas, woods, water bodies, the Eulach, and individual trees
await overpass(
  `[out:json][timeout:90];(
    way["leisure"~"^(park|garden|pitch|playground)$"](${BBOX});
    way["landuse"~"^(grass|meadow|forest|cemetery|village_green|recreation_ground|allotments)$"](${BBOX});
    way["natural"~"^(wood|scrub|grassland|water)$"](${BBOX});
    way["waterway"~"^(river|stream)$"](${BBOX});
    node["natural"="tree"](${BBOX});
  );out tags geom;`,
  `${OUT}/osm-nature.json`,
);
console.log('fetch complete');
