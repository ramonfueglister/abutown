// ÖV-Linien + Stops aus OSM Route-Relationen. Gruppiert nach tags.ref (Fallback
// tags.name), Modus aus tags.route, Platform-/Stop-Nodes werden aufs nächste
// befahrbare/schienengebundene Kantensegment (Klasse ≤6) projiziert — teilt
// sich die Grid+Projektions-Primitive mit access.mjs (nearestEdgePoint).
import { nearestEdgePoint } from './access.mjs';

const MODE_MAP = { bus: 0, tram: 1, train: 2, light_rail: 2, subway: 2 };
const STOP_ROLES = new Set(['platform', 'stop', 'platform_entry_only', 'platform_exit_only']);
const MAX_STOP_DIST = 80;

export function transformTransit({ osmTransit, graph, projector }) {
  const lineRef = [];
  const lineMode = [];
  const lineStopOffset = [];
  const stopEdge = [];
  const stopOffsetM = [];
  const stopName = [];

  for (const el of osmTransit.elements ?? []) {
    if (el.type !== 'relation') continue;
    const t = el.tags ?? {};
    const mode = MODE_MAP[t.route];
    if (mode === undefined) continue;
    const ref = t.ref ?? t.name;
    if (!ref) continue;

    lineStopOffset.push(stopEdge.length);
    lineRef.push(ref);
    lineMode.push(mode);

    for (const m of el.members ?? []) {
      if (m.type !== 'node' || !STOP_ROLES.has(m.role)) continue;
      if (typeof m.lon !== 'number' || typeof m.lat !== 'number') continue;
      const [x, z] = projector.toLocal(m.lon, m.lat);
      const hit = nearestEdgePoint(graph, x, z, MAX_STOP_DIST, (cls) => cls <= 6);
      stopEdge.push(hit ? hit.edge : 0xffffffff);
      stopOffsetM.push(hit ? hit.offsetM : 0);
      stopName.push(m.tags?.name ?? '');
    }
  }

  return { lineRef, lineMode, lineStopOffset, stopEdge, stopOffsetM, stopName };
}
