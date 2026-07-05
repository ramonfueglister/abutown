// scripts/geo/bake-traffic-net.mjs
// Offline bake: scratch/geo/osm-roads.json + osm-traffic-nodes.json → the
// lane-level traffic network at data/winterthur/trafficnet.json. Pure I/O
// wrapper around lib/trafficnet.mjs; projection via lib/project.mjs. Writes
// deterministically (stable ids, 2-decimal coords, fixed key order) so the
// committed asset only changes when the input or the transform does.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { ANCHOR, makeProjector } from './lib/project.mjs';
import { buildTrafficNet } from './lib/trafficnet.mjs';

const SCRATCH = 'scratch/geo';
const OUT_DIR = 'data/winterthur';
const OUT = `${OUT_DIR}/trafficnet.json`;

const roadsPath = `${SCRATCH}/osm-roads.json`;
const nodesPath = `${SCRATCH}/osm-traffic-nodes.json`;
const boundaryPath = `${SCRATCH}/boundary-winterthur.geojson`;
if (!existsSync(roadsPath)) throw new Error(`missing ${roadsPath} — run \`node scripts/geo/fetch-winterthur.mjs\` first`);
if (!existsSync(nodesPath)) throw new Error(`missing ${nodesPath} — run \`node scripts/geo/fetch-winterthur.mjs\` first`);
if (!existsSync(boundaryPath)) {
  throw new Error(`missing ${boundaryPath} — run \`node scripts/geo/fetch-winterthur.mjs\` first (gateway stubs need the municipality boundary)`);
}

const osmRoads = JSON.parse(readFileSync(roadsPath, 'utf8'));
const osmTrafficNodes = JSON.parse(readFileSync(nodesPath, 'utf8'));
const boundary = JSON.parse(readFileSync(boundaryPath, 'utf8'));

const net = buildTrafficNet({
  osmRoads,
  osmTrafficNodes,
  projector: makeProjector(ANCHOR),
  anchor: ANCHOR,
  boundary,
});

if (net.edges.length < 100) throw new Error(`bake: only ${net.edges.length} edges — expected >100, input looks wrong`);
if (net.meta.gatewayCount < 10) {
  throw new Error(`bake: only ${net.meta.gatewayCount} gateways — a Gemeinde-scale net needs ≥10 boundary crossings`);
}
if (!net.edges.some((e) => e.speedMs >= 27)) {
  throw new Error('bake: no motorway-speed edge (≥27 m/s) — the A1 is missing from the net');
}

mkdirSync(OUT_DIR, { recursive: true });
// Pretty-print with 2 spaces; key order is already fixed by buildTrafficNet's
// object construction (V8 preserves insertion order for string keys).
writeFileSync(OUT, `${JSON.stringify(net, null, 2)}\n`);
console.log(
  `wrote ${OUT}: ${net.nodes.length} nodes, ${net.edges.length} edges, ${net.lanes.length} lanes, ${net.turns.length} turns`,
);
const kinds = {};
for (const n of net.nodes) kinds[n.kind] = (kinds[n.kind] ?? 0) + 1;
console.log(`  node kinds: ${JSON.stringify(kinds)}`);
