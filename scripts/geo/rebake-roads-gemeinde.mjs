// scripts/geo/rebake-roads-gemeinde.mjs
//
// Re-bake ONLY data/winterthur/roads.json (render ribbons + rails) from the
// full-Gemeinde OSM road fetch (scratch/geo/osm-roads.json, PR #119's
// fetch-winterthur.mjs). The committed roads.json predated that expansion and
// was clipped to the 1.1×1.4 km KSW plate bbox, so ribbons ended mid-street at
// the clip edge while traffic (trafficnet.json) and demand run Gemeinde-wide —
// cars visibly drove across bare grass everywhere outside the plate.
//
// Deliberately does NOT touch buildings.json / nature.json / meta.json: the
// outskirt buildings already live in the #119 world tiles, and re-baking them
// here would double-render the city massing. Roads are the only layer whose
// render source was still plate-scoped.
import { readFileSync, writeFileSync, statSync } from 'node:fs';
import { ANCHOR, makeProjector } from './lib/project.mjs';
import { transformRoads } from './lib/transform.mjs';
import { clipWayAtBoundary, normalizeBoundary } from './lib/trafficnet.mjs';

const SCRATCH = 'scratch/geo';
const OUT = 'data/winterthur/roads.json';

const projector = makeProjector(ANCHOR);
const osmRoads = JSON.parse(readFileSync(`${SCRATCH}/osm-roads.json`, 'utf8'));

// Clip at the municipality boundary (same rule as the traffic net) — the
// terrain pyramid only covers the Gemeinde + apron, and beyond it the height
// sampler falls back to the flat anchor height, which would float/sink any
// ribbon out there.
const polys = normalizeBoundary(
  JSON.parse(readFileSync(`${SCRATCH}/boundary-winterthur.geojson`, 'utf8')),
);
const clippedElements = [];
for (const el of osmRoads.elements ?? []) {
  if (el.type !== 'way' || !el.geometry || el.geometry.length < 2) continue;
  for (const seg of clipWayAtBoundary(el.geometry, polys)) {
    clippedElements.push({ ...el, geometry: seg.pts });
  }
}

const { roads, rails } = transformRoads({ osmRoads: { elements: clippedElements }, projector });

if (roads.length < 5000) {
  throw new Error(
    `rebake-roads: only ${roads.length} roads — is ${SCRATCH}/osm-roads.json the full-Gemeinde fetch?`,
  );
}

let minx = Infinity, maxx = -Infinity, minz = Infinity, maxz = -Infinity;
for (const r of roads) for (const [x, z] of r.pts) {
  if (x < minx) minx = x; if (x > maxx) maxx = x;
  if (z < minz) minz = z; if (z > maxz) maxz = z;
}

writeFileSync(OUT, JSON.stringify({ roads, rails }));
console.log(
  `wrote ${OUT}: ${roads.length} roads, ${rails.length} rails, ` +
  `bounds x ${minx.toFixed(0)}..${maxx.toFixed(0)} z ${minz.toFixed(0)}..${maxz.toFixed(0)}, ` +
  `${(statSync(OUT).size / 1e6).toFixed(1)} MB`,
);
