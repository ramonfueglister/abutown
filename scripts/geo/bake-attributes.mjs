// scripts/geo/bake-attributes.mjs
// Deterministic enrichment bake: joins the fetched attribute data
// (scratch/geo/{bauzonen.geojson,gwr-buildings.json}) against the ALREADY
// BAKED footprints in data/winterthur/buildings.json — in local plate metres,
// via the same projector as every other bake. Idempotent: re-running
// overwrites the four attribute fields and building-attributes.json.
import { readFileSync, writeFileSync } from 'node:fs';
import { ANCHOR, makeProjector } from './lib/project.mjs';
import { joinBauzone, joinGwr } from './lib/enrich.mjs';

const projector = makeProjector(ANCHOR);

const buildingsPath = 'data/winterthur/buildings.json';
const doc = JSON.parse(readFileSync(buildingsPath, 'utf8'));

// Zones: outer ring only (Grundnutzung holes are inner courtyards of the SAME
// zone's complement — the centroid test tolerates this; a centroid inside a
// hole belongs to whichever zone the hole cuts to, and those cases are logged).
const zonesFc = JSON.parse(readFileSync('scratch/geo/bauzonen.geojson', 'utf8'));
// Polygon AND MultiPolygon (the WFS mostly emits Polygon, but not always):
// every outer ring becomes one zone entry carrying the same properties.
const zones = zonesFc.features.flatMap((f) => {
  const polys =
    f.geometry.type === 'MultiPolygon' ? f.geometry.coordinates
    : f.geometry.type === 'Polygon' ? [f.geometry.coordinates]
    : [];
  if (polys.length === 0) throw new Error(`bauzonen: unexpected geometry ${f.geometry.type}`);
  return polys.map((rings) => ({
    ring: rings[0].map(([lon, lat]) => projector.toLocal(lon, lat)),
    bauzone: f.properties.bauzone,
    bauzoneCode: f.properties.bauzoneCode,
    zhCode: f.properties.zhCode,
  }));
});

const gwrDoc = JSON.parse(readFileSync('scratch/geo/gwr-buildings.json', 'utf8'));
const gwrPoints = gwrDoc.buildings.map((b) => {
  const [x, z] = projector.toLocal(b.lon, b.lat);
  return { x, z, egid: b.egid, gkat: b.gkat, gklas: b.gklas };
});

let zoned = 0;
let gwred = 0;
const attributes = [];
for (const b of doc.buildings) {
  const zone = joinBauzone(b.footprint, zones);
  const gwr = joinGwr(b.footprint, gwrPoints);
  b.bauzone = zone?.bauzone ?? null;
  b.bauzoneCode = zone?.bauzoneCode ?? null;
  b.egid = gwr?.egid ?? null;
  b.gwrCategory = gwr?.gwrCategory ?? null;
  if (zone) zoned++;
  if (gwr) gwred++;
  attributes.push({
    id: b.id,
    egid: gwr?.egid ?? null,
    gwrCategory: gwr?.gwrCategory ?? null,
    gwrClass: gwr?.gwrClass ?? null,
    bauzone: zone?.bauzone ?? null,
    bauzoneCode: zone?.bauzoneCode ?? null,
    raw: { egids: gwr?.egids ?? [], zhCode: zone?.zhCode ?? null },
  });
}

// Coverage gates — fail loudly, never ship a silently-empty join.
const n = doc.buildings.length;
console.log(`bauzone: ${zoned}/${n} (${((100 * zoned) / n).toFixed(1)}%)`);
console.log(`gwr:     ${gwred}/${n} (${((100 * gwred) / n).toFixed(1)}%)`);
if (zoned / n < 0.85) throw new Error(`bake-attributes: bauzone coverage ${zoned}/${n} < 85% — projection or fetch broken`);
if (gwred / n < 0.5) throw new Error(`bake-attributes: GWR coverage ${gwred}/${n} < 50% — projection or fetch broken`);

writeFileSync(buildingsPath, JSON.stringify(doc));
writeFileSync(
  'data/winterthur/building-attributes.json',
  JSON.stringify({ worldId: 'winterthur', buildings: attributes }),
);
console.log(`wrote ${buildingsPath} + data/winterthur/building-attributes.json`);
