// scripts/geo/rederive-traffic-conflicts.mjs
//
// FIX-C1 re-derive: recompute every turn's `conflictsWith` + `yieldsTo` on the
// ALREADY-BAKED data/winterthur/trafficnet.json using the new distance-based
// conflict rule (scripts/geo/lib/trafficnet.mjs :: deriveConflictsAndYields),
// preserving all node/edge/lane/turn IDs and every other field byte-for-byte.
//
// Why re-derive in place instead of a full `bake-traffic-net.mjs` run?  The
// committed trafficnet.json was baked from a traffic-plate-scoped OSM road
// fetch that predates the full-Gemeinde `fetch-winterthur.mjs` expansion (PR
// #119), so the current scratch/geo/osm-roads.json no longer reproduces it (it
// now covers the whole municipality — ~20× the plate). Re-baking would mint a
// completely different, city-wide net with fresh IDs, breaking the running
// server, the client's build-time import, the Rust soak, and the seed-42
// investigation baseline. Re-deriving ONLY the conflict/yield fields on the
// committed geometry keeps every ID stable while applying exactly the new rule.
//
// Class-rank recovery: the serialized edge does not carry `cls` (stripped by
// buildTrafficNet), so the "minor road yields to major road" rule recovers a
// coarse class rank from the serialized `speedMs` (5.56 → 0, 8.33 → 1, else →
// 2). This preserves the ordering that rule needs (service/living < residential
// < the 50 km/h+ tier) up to ties inside the 13.89 m/s tier — acceptable for
// yields, which are dominated by the geometry + priorityRoad rules. The SOURCE
// transform still uses exact `cls`, so a clean rebake from a plate-scoped fetch
// remains fully correct.
import { readFileSync, writeFileSync } from 'node:fs';
import { deriveConflictsAndYields } from './lib/trafficnet.mjs';

const OUT = 'data/winterthur/trafficnet.json';
const net = JSON.parse(readFileSync(OUT, 'utf8'));

const laneById = new Map(net.lanes.map((l) => [l.id, l]));
const edgeById = new Map(net.edges.map((e) => [e.id, e]));

// Reconstruct the internal turn shape deriveConflictsAndYields expects:
// inEdge/outEdge are recovered from the fromLane/toLane's edge.  Reset the
// conflict/yield arrays so the derivation is a pure recompute.
const turns = net.turns.map((t) => ({
  id: t.id,
  node: t.node,
  fromLane: t.fromLane,
  toLane: t.toLane,
  inEdge: laneById.get(t.fromLane).edge,
  outEdge: laneById.get(t.toLane).edge,
  conflictsWith: [],
  yieldsTo: [],
}));

// edges indexed by id (ids are dense 0..n in the bake, but index defensively).
const edges = [];
for (const e of net.edges) edges[e.id] = e;

const dirInto = (eid) => {
  const l = laneById.get(edgeById.get(eid).lanes[0]);
  const p = l.pts;
  const a = p[p.length - 2];
  const b = p[p.length - 1];
  const len = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1;
  return [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
};
const dirOutOf = (eid) => {
  const l = laneById.get(edgeById.get(eid).lanes[0]);
  const p = l.pts;
  const len = Math.hypot(p[1][0] - p[0][0], p[1][1] - p[0][1]) || 1;
  return [(p[1][0] - p[0][0]) / len, (p[1][1] - p[0][1]) / len];
};
// Coarse class rank from the serialized free-flow speed (see banner).
const rankOfEdge = (eid) => {
  const s = edgeById.get(eid).speedMs;
  if (s <= 5.56 + 1e-6) return 0; // living_street / service
  if (s <= 8.33 + 1e-6) return 1; // residential
  return 2; // tertiary and above
};
const roundaboutNodeIds = new Set(net.nodes.filter((n) => n.kind === 'roundabout').map((n) => n.id));

deriveConflictsAndYields({ nodes: net.nodes, edges, turns, dirInto, dirOutOf, rankOfEdge, roundaboutNodeIds });

// Write the recomputed conflict/yield fields back into the serialized turns,
// keeping the exact same sorted-array form buildTrafficNet emits.
const turnById = new Map(turns.map((t) => [t.id, t]));
let turnsWithConflicts = 0;
let conflictEdges = 0;
for (const st of net.turns) {
  const dt = turnById.get(st.id);
  st.conflictsWith = dt.conflictsWith.slice().sort((a, b) => a - b);
  st.yieldsTo = dt.yieldsTo.slice().sort((a, b) => a - b);
  if (st.conflictsWith.length > 0) turnsWithConflicts++;
  conflictEdges += st.conflictsWith.length;
}

writeFileSync(OUT, `${JSON.stringify(net, null, 2)}\n`);
console.log(
  `re-derived ${OUT}: ${net.turns.length} turns, ${turnsWithConflicts} with conflicts, ${conflictEdges} conflict edges`,
);
