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
