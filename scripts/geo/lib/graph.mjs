// scripts/geo/lib/graph.mjs
// OSM-Ways → routbarer Graph in SoA-Spalten (Feldnamen = world.proto).
// Knoten = Way-Endpunkte + jeder OSM-Node, der in ≥2 Ways vorkommt.
// Ways werden an Knoten gesplittet; Restrictions von Way-Ids auf
// Kanten-Indizes aufgelöst (via-Node muss Endpunkt beider Kanten sein).
const CLASS = { motorway: 1, trunk: 1, primary: 2, secondary: 3, tertiary: 3,
  residential: 4, unclassified: 4, living_street: 4, service: 5, track: 6,
  path: 7, footway: 8, pedestrian: 8, steps: 8, cycleway: 9 };
const WIDTH = { 1: 12, 2: 7, 3: 6, 4: 5.5, 5: 3.5, 6: 2.5, 7: 1.5, 8: 2, 9: 2, 10: 3 };

export function buildRoadGraph({ osmRoads, projector, dem }) {
  const els = osmRoads.elements ?? [];
  const ways = els.filter((e) => e.type === 'way' && e.geometry?.length >= 2 && e.nodes?.length === e.geometry.length
    && (CLASS[e.tags?.highway] || /^(rail|tram)$/.test(e.tags?.railway ?? '')));
  const signalIds = new Set(els.filter((e) => e.type === 'node' && e.tags?.highway === 'traffic_signals').map((e) => e.id));

  // Knotenkandidaten: Nutzungszähler über alle Way-Node-Ids
  const useCount = new Map();
  for (const w of ways) for (const id of w.nodes) useCount.set(id, (useCount.get(id) ?? 0) + 1);
  const isNode = (w, i) => i === 0 || i === w.nodes.length - 1 || useCount.get(w.nodes[i]) >= 2;

  const nodeIndex = new Map(); // osmId -> idx
  const g = { nodeOsmId: [], nodeX: [], nodeZ: [], nodeY: [], nodeSignal: [],
    edgeA: [], edgeB: [], edgeClass: [], edgeWidth: [], edgeOneway: [], edgeMaxspeed: [], edgeLanes: [],
    edgePtOffset: [], edgePtX: [], edgePtZ: [], edgePtY: [],
    restrictionFromEdge: [], restrictionViaNode: [], restrictionToEdge: [], edgeWayId: [] };
  const addNode = (osmId, lon, lat) => {
    if (nodeIndex.has(osmId)) return nodeIndex.get(osmId);
    const [x, z] = projector.toLocal(lon, lat);
    const idx = g.nodeOsmId.length;
    nodeIndex.set(osmId, idx);
    g.nodeOsmId.push(BigInt(osmId)); g.nodeX.push(x); g.nodeZ.push(z);
    g.nodeY.push(dem.heightAt(x, z)); g.nodeSignal.push(signalIds.has(osmId));
    return idx;
  };

  for (const w of ways) {
    const t = w.tags ?? {};
    const cls = t.railway ? 10 : CLASS[t.highway];
    const oneway = t.oneway === 'yes' || t.oneway === '1' ? 1 : t.oneway === '-1' ? 2 : 0;
    const maxspeed = Number.parseInt(t.maxspeed ?? '', 10) || 0;
    const lanes = Number.parseInt(t.lanes ?? '', 10) || 0;
    const width = Number.parseFloat(t.width ?? '') || WIDTH[cls];
    let segStart = 0;
    for (let i = 1; i < w.nodes.length; i++) {
      if (!isNode(w, i)) continue;
      const a = addNode(w.nodes[segStart], w.geometry[segStart].lon, w.geometry[segStart].lat);
      const b = addNode(w.nodes[i], w.geometry[i].lon, w.geometry[i].lat);
      g.edgeA.push(a); g.edgeB.push(b); g.edgeClass.push(cls); g.edgeWidth.push(width);
      g.edgeOneway.push(oneway); g.edgeMaxspeed.push(maxspeed); g.edgeLanes.push(lanes);
      g.edgeWayId.push(w.id);
      g.edgePtOffset.push(g.edgePtX.length);
      for (let k = segStart; k <= i; k++) {
        const [x, z] = projector.toLocal(w.geometry[k].lon, w.geometry[k].lat);
        g.edgePtX.push(Math.round(x * 100) / 100); g.edgePtZ.push(Math.round(z * 100) / 100);
        g.edgePtY.push(Math.round(dem.heightAt(x, z) * 100) / 100);
      }
      segStart = i;
    }
  }

  // Restrictions: Way-Ids → Kanten, die am via-Node enden
  for (const rel of els.filter((e) => e.type === 'relation' && e.tags?.type === 'restriction')) {
    const from = rel.members?.find((m) => m.role === 'from' && m.type === 'way');
    const via = rel.members?.find((m) => m.role === 'via' && m.type === 'node');
    const to = rel.members?.find((m) => m.role === 'to' && m.type === 'way');
    if (!from || !via || !to || !nodeIndex.has(via.ref)) continue;
    const viaIdx = nodeIndex.get(via.ref);
    const touching = (wayId) => g.edgeWayId.findIndex((wid, e) => wid === wayId && (g.edgeA[e] === viaIdx || g.edgeB[e] === viaIdx));
    const fe = touching(from.ref), te = touching(to.ref);
    if (fe < 0 || te < 0) continue;
    g.restrictionFromEdge.push(fe); g.restrictionViaNode.push(viaIdx); g.restrictionToEdge.push(te);
  }
  return g;
}
