// scripts/geo/lib/trafficnet.mjs
// Pure transform: OSM road ways (+ traffic-control nodes) → a lane-level
// traffic network. This is the single source of truth later consumed by both
// the Rust server and the browser client, so it must be deterministic:
// everything is stably sorted by OSM id before sequential ids are assigned,
// coords are quantized to 2 decimals, and the emitted JSON has a fixed key
// order. Projection goes through project.mjs (never reimplemented here).
//
// Conventions (from task-1-brief.md):
//  - Keep only drivable classes (motorway…service + _link); drop foot/cycle/…
//  - Split ways at shared intersection vertices (topology by exact coordinate,
//    since the roads fetch uses `out tags geom` which drops node ids — OSM
//    shares the exact vertex coordinate at a junction, so the rounded lon/lat
//    is an equivalent topology key).
//  - oneway=yes → one directed edge; else one edge per direction.
//  - laneCount from lanes / lanes:forward|backward (default 1 per direction).
//  - Lane polyline offset (index + 0.5)·laneWidth to the RIGHT of travel
//    (Switzerland = right-hand traffic), perpendicular per segment, mitered.
//  - Node kind: signal (traffic_signals on node or ≤20 m up an approach) >
//    roundabout > priority (one through pair carries higher class / priority
//    road) > uncontrolled (right-before-left). dead_end for stubs.
//  - Signals: Webster-style defaults — 60 s cycle, green ∝ approach laneCount,
//    min green 7 s, 2 s all-red between phases; phases gate turn ids.
//  - conflictsWith: turns whose straight paths through the node cross.
//  - yieldsTo: minor→major, entry→circulating, left-turner→oncoming straight.

import { roadStyle } from './join.mjs';
import { roadWidthFromTags } from './style.mjs';

export const LANE_WIDTH = 3.0;
export const CELL_SIZE = 128;

/** Narrowest lane the kernel will bake: below this, opposing cars must share
 * tarmac anyway (real Swiss service alleys), and a smaller offset would put
 * the lane pair inside each other's body width. */
export const MIN_LANE_WIDTH = 1.8;

/**
 * Width-aware lateral lane offsets (m, positive = driver's right of the way
 * centreline) for one directed edge. Root-cause fix for the FIX-D1 symptom
 * chain: lanes must fit the REAL OSM carriage width — never assume 3.0 m and
 * widen the rendered world afterwards.
 *
 * Two-way (reverseLaneCount > 0): this direction owns the right half; lanes
 * stack outward from the centreline at (idx + 0.5)·laneW.
 * One-way: the lane block is centred on the way (single lane drives ON the
 * centreline), matching how one-way streets are actually used.
 * laneW = clamp(widthM / totalLanes, MIN_LANE_WIDTH, LANE_WIDTH); a missing
 * width keeps the classic full-width behaviour.
 */
export function laneOffsets({ laneCount, reverseLaneCount, widthM }) {
  const totalLanes = laneCount + reverseLaneCount;
  const laneW =
    widthM > 0
      ? Math.min(LANE_WIDTH, Math.max(MIN_LANE_WIDTH, widthM / totalLanes))
      : LANE_WIDTH;
  const centerShift = reverseLaneCount > 0 ? 0 : (laneCount * laneW) / 2;
  const offs = [];
  for (let idx = 0; idx < laneCount; idx++) {
    offs.push(Math.round(((idx + 0.5) * laneW - centerShift) * 1000) / 1000);
  }
  return offs;
}

// Drivable highway classes, ranked (higher = more important road). Used for
// priority-road detection and the class filter.
const CLASS_RANK = {
  motorway: 8,
  trunk: 7,
  primary: 6,
  secondary: 5,
  tertiary: 4,
  unclassified: 3,
  residential: 2,
  living_street: 1,
  service: 0,
};

// Default free-flow speed per class (m/s). Swiss urban defaults.
const CLASS_SPEED_MS = {
  motorway: 27.78, // 100 km/h
  trunk: 22.22, // 80
  primary: 13.89, // 50
  secondary: 13.89,
  tertiary: 13.89,
  unclassified: 13.89,
  residential: 8.33, // 30
  living_street: 5.56, // 20
  service: 5.56,
};

function baseClass(hw) {
  return hw.endsWith('_link') ? hw.slice(0, -5) : hw;
}

export function isDrivable(tags) {
  const hw = tags?.highway;
  if (!hw) return false;
  return Object.prototype.hasOwnProperty.call(CLASS_RANK, baseClass(hw));
}

// A stable topology key for a geographic vertex: rounded lon/lat (1e-7 deg ≈
// 1.1 cm) so that shared way vertices collapse to the same node.
function coordKey(lon, lat) {
  return `${Math.round(lon * 1e7)},${Math.round(lat * 1e7)}`;
}

const r2 = (v) => Math.round(v * 100) / 100;

// perpendicular-right unit vector of a travel direction in the (+x east,
// +z south) ground plane. "Right of travel" is the clockwise 90° rotation:
// for dir=(dx,dz), right = (-dz, dx) — verify: dir=+x (east) → right=+z
// (south), which is indeed the right shoulder when facing east. The lane
// offset uses this so lane 0 sits on the right, matching Swiss traffic.
function rightNormal(dx, dz) {
  const len = Math.hypot(dx, dz) || 1;
  return [-dz / len, dx / len];
}

// Offset a polyline to the right by `dist` metres, mitering at interior joints
// so parallel-offset segments stay connected. pts are [x,z] in local metres.
export function offsetRight(pts, dist) {
  const n = pts.length;
  if (n < 2) return pts.map(([x, z]) => [x, z]);
  // per-segment right normals
  const segN = [];
  for (let i = 0; i < n - 1; i++) {
    const dx = pts[i + 1][0] - pts[i][0];
    const dz = pts[i + 1][1] - pts[i][1];
    segN.push(rightNormal(dx, dz));
  }
  const out = [];
  for (let i = 0; i < n; i++) {
    let nx;
    let nz;
    if (i === 0) {
      [nx, nz] = segN[0];
    } else if (i === n - 1) {
      [nx, nz] = segN[n - 2];
    } else {
      // miter = normalized sum of the two adjacent segment normals, scaled by
      // 1/cos(θ/2) so the offset line stays exactly `dist` from each segment.
      const [ax, az] = segN[i - 1];
      const [bx, bz] = segN[i];
      let mx = ax + bx;
      let mz = az + bz;
      const ml = Math.hypot(mx, mz);
      if (ml < 1e-6) {
        // ~180° reversal — fall back to the incoming normal, no miter scale
        mx = ax;
        mz = az;
      } else {
        mx /= ml;
        mz /= ml;
        // cos(half angle) = miter · segNormal
        const cos = mx * bx + mz * bz;
        const scale = Math.min(1 / Math.max(cos, 0.2), 4); // cap spikes at 4×
        mx *= scale;
        mz *= scale;
      }
      nx = mx;
      nz = mz;
    }
    out.push([pts[i][0] + nx * dist, pts[i][1] + nz * dist]);
  }
  return out;
}

function polylineLength(pts) {
  let d = 0;
  for (let i = 1; i < pts.length; i++) {
    d += Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]);
  }
  return d;
}

function laneCounts(tags, oneway) {
  const total = Number.parseInt(tags?.lanes ?? '', 10);
  const fwd = Number.parseInt(tags?.['lanes:forward'] ?? '', 10);
  const bwd = Number.parseInt(tags?.['lanes:backward'] ?? '', 10);
  if (oneway) {
    const c = Number.isFinite(fwd) ? fwd : Number.isFinite(total) ? total : 1;
    return { forward: Math.max(1, c), backward: 0 };
  }
  let f = Number.isFinite(fwd) ? fwd : null;
  let b = Number.isFinite(bwd) ? bwd : null;
  if (f === null && b === null && Number.isFinite(total)) {
    f = Math.max(1, Math.floor(total / 2));
    b = Math.max(1, total - f);
  }
  return { forward: Math.max(1, f ?? 1), backward: Math.max(1, b ?? 1) };
}

function isOneway(tags, cls) {
  const v = tags?.oneway;
  if (v === 'yes' || v === 'true' || v === '1' || v === '-1') return true;
  if (cls === 'motorway' || cls === 'motorway_link') return true;
  return false;
}

// ---------------------------------------------------------------------------
// Boundary clipping (Gemeinde-scale bake). The municipality boundary is a
// GeoJSON Polygon/MultiPolygon in lon/lat; drivable ways are clipped BEFORE
// projection: inside parts are kept, each boundary crossing inserts the
// intersection point as a new terminal vertex whose node later becomes a
// `gateway`. Point-in-polygon by ray casting on lon/lat — roads cross the
// boundary transversally, so exactness at vertices is irrelevant.

// Normalize a boundary input (Feature / FeatureCollection / geometry) to an
// array of polygons, each an array of rings of [lon, lat] (altitude dropped).
export function normalizeBoundary(boundary) {
  let geom = boundary;
  if (geom?.type === 'FeatureCollection') geom = geom.features[0]?.geometry;
  else if (geom?.type === 'Feature') geom = geom.geometry;
  if (geom?.type === 'Polygon') return [geom.coordinates];
  if (geom?.type === 'MultiPolygon') return geom.coordinates;
  throw new Error(`boundary must be a GeoJSON Polygon/MultiPolygon (got ${geom?.type})`);
}

function pointInRing(lon, lat, ring) {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const xi = ring[i][0];
    const yi = ring[i][1];
    const xj = ring[j][0];
    const yj = ring[j][1];
    if (yi > lat !== yj > lat && lon < ((xj - xi) * (lat - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

function pointInPolygons(lon, lat, polys) {
  for (const rings of polys) {
    if (!pointInRing(lon, lat, rings[0])) continue;
    let inHole = false;
    for (let r = 1; r < rings.length; r++) {
      if (pointInRing(lon, lat, rings[r])) {
        inHole = true;
        break;
      }
    }
    if (!inHole) return true;
  }
  return false;
}

// Parameter t ∈ [0,1] along a→b where it crosses segment c→d, or null.
function segCrossT(a, b, c, d) {
  const rX = b.lon - a.lon;
  const rY = b.lat - a.lat;
  const sX = d[0] - c[0];
  const sY = d[1] - c[1];
  const denom = rX * sY - rY * sX;
  if (denom === 0) return null;
  const t = ((c[0] - a.lon) * sY - (c[1] - a.lat) * sX) / denom;
  const u = ((c[0] - a.lon) * rY - (c[1] - a.lat) * rX) / denom;
  if (t < -1e-9 || t > 1 + 1e-9 || u < -1e-9 || u > 1 + 1e-9) return null;
  return Math.min(1, Math.max(0, t));
}

// Where does the way segment a→b (one endpoint inside, one outside) cross the
// boundary? With multiple crossings: exiting takes the FIRST (min t), entering
// the LAST (max t) — both keep the returned point on the inside portion's rim.
function boundaryCrossing(a, b, polys, exiting) {
  let best = null;
  for (const rings of polys) {
    for (const ring of rings) {
      for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
        const t = segCrossT(a, b, ring[j], ring[i]);
        if (t === null) continue;
        if (best === null || (exiting ? t < best : t > best)) best = t;
      }
    }
  }
  // PIP said the segment crosses, so an intersection must exist; a numeric
  // near-tangency could still miss — fall back to the midpoint (≤ half a
  // segment of error, deterministic).
  const t = best ?? 0.5;
  return { lon: a.lon + (b.lon - a.lon) * t, lat: a.lat + (b.lat - a.lat) * t };
}

// Clip one way geometry ([{lon,lat},…]) at the boundary. Returns the inside
// segments in way order, each `{ pts, startGateway, endGateway }` — the
// gateway flags mark terminal vertices created by a boundary cut. A way fully
// outside yields []; in-out-in ways yield multiple segments.
export function clipWayAtBoundary(geometry, polys) {
  const inside = geometry.map((p) => pointInPolygons(p.lon, p.lat, polys));
  const segments = [];
  let cur = null;
  for (let i = 0; i < geometry.length; i++) {
    if (inside[i]) {
      if (!cur) cur = { pts: [], startGateway: false, endGateway: false };
      cur.pts.push(geometry[i]);
    }
    if (i < geometry.length - 1 && inside[i] !== inside[i + 1]) {
      const ip = boundaryCrossing(geometry[i], geometry[i + 1], polys, inside[i]);
      if (inside[i]) {
        cur.pts.push(ip);
        cur.endGateway = true;
        segments.push(cur);
        cur = null;
      } else {
        cur = { pts: [ip], startGateway: true, endGateway: false };
      }
    }
  }
  if (cur) segments.push(cur);
  return segments.filter((s) => s.pts.length >= 2);
}

// Split a drivable way's vertex list into segments at any vertex shared by ≥2
// kept ways (or an endpoint of another kept way). `shared` is a Set of
// coordinate keys that are intersection points; every way endpoint is always a
// split point. Returns arrays of index ranges [start..end] (inclusive).
function splitAtIntersections(way, sharedKeys) {
  const g = way.geometry;
  const cuts = [0];
  for (let i = 1; i < g.length - 1; i++) {
    if (sharedKeys.has(coordKey(g[i].lon, g[i].lat))) cuts.push(i);
  }
  cuts.push(g.length - 1);
  const segs = [];
  for (let i = 0; i < cuts.length - 1; i++) {
    if (cuts[i + 1] > cuts[i]) segs.push([cuts[i], cuts[i + 1]]);
  }
  return segs;
}

/**
 * Build the traffic network.
 * @param {{osmRoads:object, osmTrafficNodes:object, projector:{toLocal:(lon:number,lat:number)=>[number,number]}, anchor:{lon:number,lat:number}, boundary?:object}} args
 *   `boundary` (optional): GeoJSON Polygon/MultiPolygon (or Feature/
 *   FeatureCollection wrapping one) in lon/lat — the municipality limit.
 *   Ways are clipped at it; boundary-cut terminal nodes get kind 'gateway'.
 */
export function buildTrafficNet({ osmRoads, osmTrafficNodes, projector, anchor, boundary }) {
  // 1) filter to drivable ways, sorted by OSM id for determinism
  let ways = (osmRoads.elements ?? [])
    .filter((e) => e.type === 'way' && e.geometry && e.geometry.length >= 2 && isDrivable(e.tags))
    .sort((a, b) => a.id - b.id);

  // 1b) boundary clip (before projection). Each way is replaced by its inside
  //     segments in way order (splitting preserves the id sort, so ids stay
  //     deterministic downstream); the cut points' topology keys are recorded
  //     so their nodes classify as gateways later.
  const gatewayKeys = new Set();
  if (boundary) {
    const polys = normalizeBoundary(boundary);
    const clipped = [];
    for (const w of ways) {
      for (const seg of clipWayAtBoundary(w.geometry, polys)) {
        clipped.push({ ...w, geometry: seg.pts });
        if (seg.startGateway) {
          gatewayKeys.add(coordKey(seg.pts[0].lon, seg.pts[0].lat));
        }
        if (seg.endGateway) {
          const last = seg.pts[seg.pts.length - 1];
          gatewayKeys.add(coordKey(last.lon, last.lat));
        }
      }
    }
    ways = clipped;
  }

  // 2) topology: count how many kept ways touch each coordinate. A vertex is an
  //    intersection when ≥2 way-vertices coincide there OR it is a way endpoint
  //    that lands on another way's interior (captured by the same count, since
  //    every endpoint contributes to the count).
  const vertexCount = new Map();
  for (const w of ways) {
    for (const p of w.geometry) {
      const k = coordKey(p.lon, p.lat);
      vertexCount.set(k, (vertexCount.get(k) ?? 0) + 1);
    }
  }
  const sharedKeys = new Set();
  for (const [k, c] of vertexCount) if (c >= 2) sharedKeys.add(k);

  // 3+4) split ways into raw segments at intersection vertices.
  let rawSegs = []; // { wayId, tags, cls, pts:[{lon,lat}], fromKey, toKey }
  for (const w of ways) {
    const cls = baseClass(w.tags.highway);
    const segs = splitAtIntersections(w, sharedKeys);
    for (const [s, e] of segs) {
      const pts = w.geometry.slice(s, e + 1);
      const fromKey = coordKey(pts[0].lon, pts[0].lat);
      const toKey = coordKey(pts[pts.length - 1].lon, pts[pts.length - 1].lat);
      if (fromKey === toKey && pts.length < 3) continue; // degenerate
      rawSegs.push({ wayId: w.id, tags: w.tags, cls, pts, fromKey, toKey });
    }
  }

  // 4b) largest-connected-component pruning: OSM extracts contain dead
  //     fragments (parking aisles, clipped stubs) that would strand vehicles.
  //     Keep only the component with the greatest lane length (centreline
  //     length × directed lane count); gateway stubs attached to it survive
  //     by construction. Union-find over topology keys, insertion-ordered →
  //     deterministic.
  if (rawSegs.length > 0) {
    const parent = new Map();
    const find = (k) => {
      let r = k;
      while (parent.get(r) !== r) r = parent.get(r);
      // path compression
      let c = k;
      while (parent.get(c) !== r) {
        const next = parent.get(c);
        parent.set(c, r);
        c = next;
      }
      return r;
    };
    const ensure = (k) => {
      if (!parent.has(k)) parent.set(k, k);
    };
    for (const seg of rawSegs) {
      ensure(seg.fromKey);
      ensure(seg.toKey);
      parent.set(find(seg.fromKey), find(seg.toKey));
    }
    const weightByRoot = new Map();
    for (const seg of rawSegs) {
      const pts = seg.pts.map((p) => projector.toLocal(p.lon, p.lat));
      const { forward, backward } = laneCounts(seg.tags, isOneway(seg.tags, seg.cls));
      seg.laneLenM = polylineLength(pts) * (forward + backward);
      const r = find(seg.fromKey);
      weightByRoot.set(r, (weightByRoot.get(r) ?? 0) + seg.laneLenM);
    }
    let bestRoot = null;
    let bestW = -1;
    for (const [r, w] of weightByRoot) {
      if (w > bestW) {
        bestW = w;
        bestRoot = r;
      }
    }
    const total = [...weightByRoot.values()].reduce((s, w) => s + w, 0);
    const dropped = total - bestW;
    if (dropped > 0) {
      rawSegs = rawSegs.filter((seg) => find(seg.fromKey) === bestRoot);
      console.log(
        `trafficnet: dropped ${Math.round(dropped)} m lane-length in ${weightByRoot.size - 1} disconnected fragment(s); ` +
          `kept ${((bestW / total) * 100).toFixed(2)}% of ${Math.round(total)} m`,
      );
    }
  }

  // 4c) node registry from the kept segments only (endpoints become nodes).
  const nodeKeyToInfo = new Map(); // key -> { lon, lat }
  for (const seg of rawSegs) {
    if (!nodeKeyToInfo.has(seg.fromKey)) {
      nodeKeyToInfo.set(seg.fromKey, { lon: seg.pts[0].lon, lat: seg.pts[0].lat });
    }
    if (!nodeKeyToInfo.has(seg.toKey)) {
      const last = seg.pts[seg.pts.length - 1];
      nodeKeyToInfo.set(seg.toKey, { lon: last.lon, lat: last.lat });
    }
  }

  // 5) assign node ids deterministically: sort keys by lon,lat.
  const nodeKeys = [...nodeKeyToInfo.keys()].sort((a, b) => {
    const [ax, ay] = a.split(',').map(Number);
    const [bx, by] = b.split(',').map(Number);
    return ax - bx || ay - by;
  });
  const nodeIdOf = new Map();
  const nodes = [];
  nodeKeys.forEach((k, i) => {
    nodeIdOf.set(k, i);
    const { lon, lat } = nodeKeyToInfo.get(k);
    const [x, z] = projector.toLocal(lon, lat);
    nodes.push({ id: i, lon, lat, x: r2(x), z: r2(z), kind: 'uncontrolled', signal: null });
  });

  // 6) edges + lanes. rawSegs are already in (wayId, order) sequence which is
  //    deterministic. Assign edge/lane ids sequentially.
  const edges = [];
  const lanes = [];
  // per-node adjacency for turn synthesis + kind classification
  const incomingByNode = new Map(); // nodeId -> [edgeId]
  const outgoingByNode = new Map();
  const pushAdj = (map, node, edgeId) => {
    if (!map.has(node)) map.set(node, []);
    map.get(node).push(edgeId);
  };

  const addEdge = (fromKey, toKey, ptsLonLat, tags, cls, laneCount, reverseLaneCount) => {
    const from = nodeIdOf.get(fromKey);
    const to = nodeIdOf.get(toKey);
    // projected centreline
    const center = ptsLonLat.map((p) => {
      const [x, z] = projector.toLocal(p.lon, p.lat);
      return [x, z];
    });
    const speedMs = CLASS_SPEED_MS[cls] ?? 13.89;
    const edgeId = edges.length;
    const laneIds = [];
    // Width-aware lane placement (see laneOffsets): the OSM width tag (or the
    // class default) bounds the whole lane block so cars stay on real tarmac.
    const widthM = roadWidthFromTags(tags ?? {}, roadStyle(tags ?? {})?.width);
    const offs = laneOffsets({ laneCount, reverseLaneCount, widthM });
    for (let idx = 0; idx < laneCount; idx++) {
      const off = offs[idx];
      const raw = offsetRight(center, off);
      const pts = raw.map(([x, z]) => [r2(x), r2(z)]);
      const laneId = lanes.length;
      lanes.push({
        id: laneId,
        edge: edgeId,
        index: idx,
        lengthM: r2(polylineLength(pts)),
        pts,
      });
      laneIds.push(laneId);
    }
    edges.push({
      id: edgeId,
      from,
      to,
      speedMs: r2(speedMs),
      laneCount,
      priorityRoad: false, // filled in pass 7
      lanes: laneIds,
      cls, // internal, stripped before serialize
      priorityTag: tags?.priority_road != null && tags.priority_road !== 'no', // internal
    });
    pushAdj(outgoingByNode, from, edgeId);
    pushAdj(incomingByNode, to, edgeId);
    return edgeId;
  };

  for (const seg of rawSegs) {
    const oneway = isOneway(seg.tags, seg.cls);
    const { forward, backward } = laneCounts(seg.tags, oneway);
    const reversed = seg.tags.oneway === '-1';
    if (oneway) {
      if (reversed) {
        addEdge(seg.toKey, seg.fromKey, [...seg.pts].reverse(), seg.tags, seg.cls, forward, 0);
      } else {
        addEdge(seg.fromKey, seg.toKey, seg.pts, seg.tags, seg.cls, forward, 0);
      }
    } else {
      addEdge(seg.fromKey, seg.toKey, seg.pts, seg.tags, seg.cls, forward, backward);
      addEdge(seg.toKey, seg.fromKey, [...seg.pts].reverse(), seg.tags, seg.cls, backward, forward);
    }
  }

  // 7) priority-road flag: an edge is on a priority road when it carries a
  //    priority_road tag, OR when at its `to` node exactly one through pair
  //    (an incoming + outgoing that continues roughly straight) has the highest
  //    class among all edges at that node and no other class ties it. We take
  //    the simpler, deterministic reading: flag edges whose class rank is
  //    strictly the maximum at BOTH endpoints, or that carry priority_road.
  const classAtNode = (nodeId) => {
    let max = -1;
    const all = [...(incomingByNode.get(nodeId) ?? []), ...(outgoingByNode.get(nodeId) ?? [])];
    for (const eid of all) max = Math.max(max, CLASS_RANK[edges[eid].cls] ?? 0);
    return max;
  };
  for (const e of edges) {
    const tagged = e.priorityTag;
    const rank = CLASS_RANK[e.cls] ?? 0;
    if (tagged || (rank === classAtNode(e.from) && rank === classAtNode(e.to) && rank >= CLASS_RANK.tertiary)) {
      e.priorityRoad = true;
    }
  }

  // 8) roundabouts: junction=roundabout ways form a one-way ring. Mark every
  //    node on such a ring "roundabout". (None on this plate, but modelled.)
  const roundaboutNodeIds = new Set();
  for (const w of ways) {
    if (w.tags?.junction !== 'roundabout') continue;
    for (const p of w.geometry) {
      const k = coordKey(p.lon, p.lat);
      if (nodeIdOf.has(k)) roundaboutNodeIds.add(nodeIdOf.get(k));
    }
  }

  // 9) node kind classification. signal > roundabout > priority > uncontrolled;
  //    dead_end for nodes with a single incident direction.
  //    Signals: traffic_signals node within 20 m of the node OR on an approach.
  const signalNodes = (osmTrafficNodes.elements ?? []).filter(
    (n) => n.type === 'node' && n.tags?.highway === 'traffic_signals',
  );
  const signalLocal = signalNodes.map((n) => {
    const [x, z] = projector.toLocal(n.lon, n.lat);
    return [x, z];
  });
  const nearSignal = (nx, nz) =>
    signalLocal.some(([sx, sz]) => Math.hypot(sx - nx, sz - nz) <= 20);

  const gatewayNodeIds = new Set();
  for (const k of gatewayKeys) {
    if (nodeIdOf.has(k)) gatewayNodeIds.add(nodeIdOf.get(k));
  }

  for (const n of nodes) {
    const inN = incomingByNode.get(n.id)?.length ?? 0;
    const outN = outgoingByNode.get(n.id)?.length ?? 0;
    const degree = new Set(
      [...(incomingByNode.get(n.id) ?? []), ...(outgoingByNode.get(n.id) ?? [])].flatMap((eid) => [
        edges[eid].from,
        edges[eid].to,
      ]),
    );
    degree.delete(n.id);
    // A terminal created by a boundary cut is a gateway — a spawn/despawn
    // portal for external demand. Takes precedence over dead_end (gateways
    // are degree-1 by construction) and everything else.
    if (gatewayNodeIds.has(n.id)) {
      n.kind = 'gateway';
    } else if (degree.size <= 1) {
      // A stub with a single neighbour is a dead_end regardless of a nearby
      // signal — it has no cross-traffic to control, and its only "turn" would
      // be a U-turn (which we don't emit). Checking this before `signal` keeps
      // such nodes out of the turn-coverage requirement.
      n.kind = 'dead_end';
    } else if (nearSignal(n.x, n.z)) {
      n.kind = 'signal';
    } else if (roundaboutNodeIds.has(n.id)) {
      n.kind = 'roundabout';
    } else {
      // priority when exactly one class rank strictly dominates the node
      const ranks = [...(incomingByNode.get(n.id) ?? []), ...(outgoingByNode.get(n.id) ?? [])].map(
        (eid) => CLASS_RANK[edges[eid].cls] ?? 0,
      );
      const max = Math.max(...ranks);
      const countMax = ranks.filter((r) => r === max).length;
      const hasPriorityEdge = [...(incomingByNode.get(n.id) ?? []), ...(outgoingByNode.get(n.id) ?? [])].some(
        (eid) => edges[eid].priorityRoad,
      );
      // a through pair = 2 directed edges (an in + an out) on the dominant road
      n.kind = hasPriorityEdge && countMax >= 2 && max > Math.min(...ranks) ? 'priority' : 'uncontrolled';
    }
    void inN;
    void outN;
  }

  // 10) turns: for each node, every (incoming edge → outgoing edge) pair where
  //     the outgoing does not go straight back where the incoming came from
  //     (no U-turn onto the reverse edge of the same road). Turn connects the
  //     rightmost lanes by default (index 0 → 0). conflictsWith + yieldsTo
  //     computed geometrically below.
  const turns = [];
  const dirInto = (edgeId) => {
    // travel direction arriving at the `to` node (last segment of lane 0)
    const l = lanes[edges[edgeId].lanes[0]];
    const p = l.pts;
    const a = p[p.length - 2];
    const b = p[p.length - 1];
    return normalize(b[0] - a[0], b[1] - a[1]);
  };
  const dirOutOf = (edgeId) => {
    const l = lanes[edges[edgeId].lanes[0]];
    const p = l.pts;
    return normalize(p[1][0] - p[0][0], p[1][1] - p[0][1]);
  };

  for (const n of nodes) {
    // dead_ends have no cross traffic; gateways are pure spawn/despawn portals
    // (sources/sinks only — the Rust validator enforces "no turns at gateways")
    if (n.kind === 'dead_end' || n.kind === 'gateway') continue;
    const ins = incomingByNode.get(n.id) ?? [];
    const outs = outgoingByNode.get(n.id) ?? [];
    for (const ie of ins) {
      const inEdge = edges[ie];
      for (const oe of outs) {
        const outEdge = edges[oe];
        // skip immediate U-turn (out returns along the same road we came in on)
        if (outEdge.to === inEdge.from && outEdge.from === inEdge.to) continue;
        const fromLane = inEdge.lanes[0];
        const toLane = outEdge.lanes[0];
        turns.push({
          id: turns.length,
          fromLane,
          toLane,
          node: n.id,
          inEdge: ie,
          outEdge: oe,
          conflictsWith: [],
          yieldsTo: [],
        });
      }
    }
  }

  // 11) conflict + yield geometry. Two turns at the same node conflict when
  //     their straight chords (in-direction → out-direction through the node)
  //     either CROSS or pass within CONFLICT_CLEARANCE_M of each other without
  //     crossing (distance-based detection — root-cause fix for cross-stream
  //     near-collisions the pure-intersection test missed), OR they merge onto
  //     the same toLane. yieldsTo: minor→major (lower priorityRoad/class yields
  //     to higher), entry→circulating at roundabouts, and left-turner→oncoming
  //     straight.
  const rankOfEdge = (eid) => CLASS_RANK[edges[eid].cls] ?? 0;
  deriveConflictsAndYields({ nodes, edges, turns, dirInto, dirOutOf, rankOfEdge, roundaboutNodeIds });

  // 12) signal phases (Webster-style). Group the incoming turns of a signal
  //     node into phases by approach (incoming edge); green split ∝ approach
  //     laneCount, min 7 s, 2 s all-red between phases, cycle 60 s. Every
  //     incoming turn id is gated exactly once.
  for (const n of nodes) {
    if (n.kind !== 'signal') continue;
    const group = turnsByNode.get(n.id) ?? [];
    // group turns by incoming edge (one phase per opposing-approach group would
    // be ideal; we keep it simple + safe: one phase per incoming approach).
    const byApproach = new Map();
    for (const t of group) {
      if (!byApproach.has(t.inEdge)) byApproach.set(t.inEdge, []);
      byApproach.get(t.inEdge).push(t.id);
    }
    const approaches = [...byApproach.keys()].sort((a, b) => a - b);
    const CYCLE = 60;
    const ALL_RED = 2;
    const MIN_GREEN = 7;
    const nPhases = approaches.length;
    const weights = approaches.map((eid) => edges[eid].laneCount);
    const totalWeight = weights.reduce((s, w) => s + w, 0) || 1;
    const greenBudget = Math.max(CYCLE - ALL_RED * nPhases, nPhases * MIN_GREEN);
    const phases = approaches.map((eid, i) => {
      const share = (weights[i] / totalWeight) * greenBudget;
      return {
        greenS: Math.max(MIN_GREEN, Math.round(share)),
        turns: byApproach.get(eid),
      };
    });
    n.signal = { cycleS: CYCLE, phases };
  }

  // 13) serialize with fixed key order + no internal fields.
  const outNodes = nodes.map((n) => ({
    id: n.id,
    x: n.x,
    z: n.z,
    kind: n.kind,
    signal: n.signal
      ? {
          cycleS: n.signal.cycleS,
          phases: n.signal.phases.map((p) => ({ greenS: p.greenS, turns: p.turns })),
        }
      : null,
  }));
  const outEdges = edges.map((e) => ({
    id: e.id,
    from: e.from,
    to: e.to,
    speedMs: e.speedMs,
    laneCount: e.laneCount,
    priorityRoad: e.priorityRoad,
    lanes: e.lanes,
  }));
  const outLanes = lanes.map((l) => ({
    id: l.id,
    edge: l.edge,
    index: l.index,
    lengthM: l.lengthM,
    pts: l.pts,
  }));
  const outTurns = turns.map((t) => ({
    id: t.id,
    fromLane: t.fromLane,
    toLane: t.toLane,
    node: t.node,
    conflictsWith: t.conflictsWith.slice().sort((a, b) => a - b),
    yieldsTo: t.yieldsTo.slice().sort((a, b) => a - b),
  }));

  return {
    meta: {
      anchor: { lon: anchor.lon, lat: anchor.lat },
      laneWidth: LANE_WIDTH,
      cellSize: CELL_SIZE,
      gatewayCount: gatewayNodeIds.size,
    },
    nodes: outNodes,
    edges: outEdges,
    lanes: outLanes,
    turns: outTurns,
  };
}

function normalize(dx, dz) {
  const len = Math.hypot(dx, dz) || 1;
  return [dx / len, dz / len];
}

/**
 * Populate every turn's `conflictsWith` + `yieldsTo` in place (the single
 * source of truth for junction arbitration, used by both `buildTrafficNet` and
 * the in-place re-derive script). Pure over the passed structures.
 *
 * `turns[i]` must carry `{ id, node, inEdge, outEdge, fromLane, toLane,
 * conflictsWith:[], yieldsTo:[] }`; `edges[eid]` must carry `{ from, to,
 * priorityRoad }`; `nodes[nodeId]` must carry `{ kind }`. `dirInto(eid)` /
 * `dirOutOf(eid)` return the unit travel direction into/out of an edge at the
 * node; `rankOfEdge(eid)` returns the road-class rank (higher = major);
 * `roundaboutNodeIds` is a Set of node ids on a roundabout ring.
 *
 * @param {{nodes:any[], edges:any[], turns:any[], dirInto:(eid:number)=>number[], dirOutOf:(eid:number)=>number[], rankOfEdge:(eid:number)=>number, roundaboutNodeIds:Set<number>}} args
 */
export function deriveConflictsAndYields({ nodes, edges, turns, dirInto, dirOutOf, rankOfEdge, roundaboutNodeIds }) {
  const turnsByNode = new Map();
  for (const t of turns) {
    if (!turnsByNode.has(t.node)) turnsByNode.set(t.node, []);
    turnsByNode.get(t.node).push(t);
  }
  const nodeById = new Map(nodes.map((n) => [n.id, n]));

  for (const [nodeId, group] of turnsByNode) {
    const node = nodeById.get(nodeId);
    for (let i = 0; i < group.length; i++) {
      for (let j = 0; j < group.length; j++) {
        if (i === j) continue;
        const a = group[i];
        const b = group[j];
        if (turnsConflict(a, b, edges, dirInto, dirOutOf)) {
          if (!a.conflictsWith.includes(b.id)) a.conflictsWith.push(b.id);
        }
      }
    }
    // yields — computed over the (now enlarged) conflict set so any new minor↔
    // major conflict also gets its yield, reusing the existing priority rules.
    for (const a of group) {
      for (const b of group) {
        if (a === b) continue;
        if (!a.conflictsWith.includes(b.id)) continue;
        let yields = false;
        // roundabout: entry (from a non-ring edge) yields to circulating turns
        if (node && node.kind === 'roundabout') {
          const aEntry = !roundaboutNodeIds.has(edges[a.inEdge].from);
          const bCirc = roundaboutNodeIds.has(edges[b.inEdge].from);
          if (aEntry && bCirc) yields = true;
        }
        // minor road yields to major road
        if (rankOfEdge(a.inEdge) < rankOfEdge(b.inEdge)) yields = true;
        // priority: unmarked approaches yield to the priority through road
        if (!edges[a.inEdge].priorityRoad && edges[b.inEdge].priorityRoad) yields = true;
        // left-turner yields to oncoming straight (right-hand traffic): if a
        // turns left and b comes from a's opposite approach going straight.
        if (isLeftTurn(a, edges, dirInto, dirOutOf) && isStraight(b, edges, dirInto, dirOutOf)) {
          yields = true;
        }
        if (yields && !a.yieldsTo.includes(b.id)) a.yieldsTo.push(b.id);
      }
    }
  }
}

// signed turn angle from the incoming travel dir to the outgoing travel dir,
// in the (+x east, +z south) plane. Positive = clockwise = a RIGHT turn (in a
// left-handed screen sense z-down); we classify by magnitude/sign consistently.
function turnAngle(t, edges, dirInto, dirOutOf) {
  const [ix, iz] = dirInto(t.inEdge);
  const [ox, oz] = dirOutOf(t.outEdge);
  // angle of out relative to in: cross gives sign, dot gives cos
  const cross = ix * oz - iz * ox;
  const dot = ix * ox + iz * oz;
  return Math.atan2(cross, dot); // (-π, π]
}

function isStraight(t, edges, dirInto, dirOutOf) {
  const a = turnAngle(t, edges, dirInto, dirOutOf);
  return Math.abs(a) < Math.PI / 6; // within 30°
}

function isLeftTurn(t, edges, dirInto, dirOutOf) {
  const a = turnAngle(t, edges, dirInto, dirOutOf);
  // in +x-east/+z-south, a left turn (counter-clockwise on the map) rotates
  // the heading toward negative cross → a < 0 beyond the straight band.
  return a < -Math.PI / 6;
}

// Clearance (m) for distance-based conflict detection: two turn chords that
// pass within this distance of each other (without crossing) still conflict —
// they would let two streams occupy overlapping node space simultaneously.
// Sized just over a lane width (3 m) so genuinely separated parallel movements
// on adjacent lanes do NOT serialize, while near-perpendicular streams grazing
// the shared node point DO. This is the root-cause fix for the cross-stream
// near-collisions the pure chord-INTERSECTION test missed (two turn chords that
// pass within <2 m without a proper crossing were treated as non-conflicting).
export const CONFLICT_CLEARANCE_M = 2.5;

// Direction chords whose unit tangents differ by less than this dot magnitude
// from parallel/anti-parallel are treated as NON-conflicting when they only
// come close (never actually cross). This guards the classic opposing-through
// case: two straight movements on opposite directions of the same road have
// collinear chords (min-distance ≈ 0) but are physically separated onto their
// own lanes and must not serialize. cos(15°) ≈ 0.966.
const PARALLEL_DOT = 0.966;

// The node-origin direction chord of a turn: a straight segment from an entry
// point (behind the node, along the incoming heading) to an exit point (ahead
// of the node, along the outgoing heading), with the node centre at the origin.
// Pure direction (no lane offset) so opposing straight-throughs are collinear
// (handled by the parallel guard) rather than laterally 3 m apart.
function turnChord(t, dirInto, dirOutOf) {
  const [ix, iz] = dirInto(t.inEdge);
  const [ox, oz] = dirOutOf(t.outEdge);
  const L = 12;
  return [
    [-ix * L, -iz * L], // entry (behind node)
    [ox * L, oz * L], // exit (ahead of node)
  ];
}

// Do two turns at the same node conflict? A turn `a` conflicts with `b` when:
//   * they share the same incoming edge → NEVER (they diverge from one lane);
//   * they are parallel same-movement (same inEdge AND same outEdge, i.e. a
//     multilane through movement) → NEVER (no false serialization of a
//     two-lane through-street);
//   * they merge onto the SAME toLane → ALWAYS (two vehicles would land on the
//     same lane at s≈0 — closes the Task-7 "bake should emit shared-toLane
//     conflicts" follow-up; the kernel's implicit shared-toLane rule stays as
//     belt-and-braces);
//   * their direction chords CROSS → conflict (classic proper crossing);
//   * their direction chords pass within CONFLICT_CLEARANCE_M without crossing
//     AND are not near-parallel → conflict (distance-based near-miss, the
//     root-cause fix). The parallel guard prevents opposing/aligned throughs,
//     whose chords are collinear, from being flagged.
function turnsConflict(a, b, edges, dirInto, dirOutOf) {
  // Same incoming edge → diverging streams, never conflict.
  if (a.inEdge === b.inEdge) return false;
  // Parallel same-movement (same in-edge already excluded above; this catches
  // the general multilane-same-movement pair which shares both edges).
  if (a.inEdge === b.inEdge && a.outEdge === b.outEdge) return false;
  // Explicit merge conflict: two distinct turns feeding the SAME toLane share a
  // physical merge point.
  if (a.toLane === b.toLane) return true;
  const A = turnChord(a, dirInto, dirOutOf);
  const B = turnChord(b, dirInto, dirOutOf);
  return chordsConflict(A, B);
}

// Pure geometric conflict test between two direction chords (each `[[x,z],
// [x,z]]`): they conflict when they cross, OR pass within
// CONFLICT_CLEARANCE_M without crossing AND are not near-parallel. Exported so
// the distance-vs-intersection behaviour can be unit-tested on hand-built
// chords independently of the OSM plumbing.
export function chordsConflict(A, B) {
  if (segsIntersect(A[0], A[1], B[0], B[1])) return true;
  if (segSegDist(A[0], A[1], B[0], B[1]) < CONFLICT_CLEARANCE_M) {
    const da = [A[1][0] - A[0][0], A[1][1] - A[0][1]];
    const db = [B[1][0] - B[0][0], B[1][1] - B[0][1]];
    const la = Math.hypot(da[0], da[1]) || 1;
    const lb = Math.hypot(db[0], db[1]) || 1;
    const dot = (da[0] * db[0] + da[1] * db[1]) / (la * lb);
    if (Math.abs(dot) < PARALLEL_DOT) return true;
  }
  return false;
}

// Minimum distance between two 2-D segments p1p2 and p3p4. Standard clamped
// projection (Ericson, Real-Time Collision Detection §5.1.9): solve the
// unconstrained closest-point parameters, clamp to [0,1], recover the points.
function segSegDist(p1, p2, p3, p4) {
  const clamp01 = (v) => (v < 0 ? 0 : v > 1 ? 1 : v);
  const ux = p2[0] - p1[0];
  const uy = p2[1] - p1[1];
  const vx = p4[0] - p3[0];
  const vy = p4[1] - p3[1];
  const wx = p1[0] - p3[0];
  const wy = p1[1] - p3[1];
  const a = ux * ux + uy * uy;
  const b = ux * vx + uy * vy;
  const c = vx * vx + vy * vy;
  const d = ux * wx + uy * wy;
  const e = vx * wx + vy * wy;
  const D = a * c - b * b;
  let sc;
  let tc;
  if (D < 1e-9) {
    // Parallel (or a degenerate chord): fix sc at the start, solve tc.
    sc = 0;
    tc = c > 1e-9 ? e / c : 0;
  } else {
    sc = (b * e - c * d) / D;
    tc = (a * e - b * d) / D;
  }
  sc = clamp01(sc);
  tc = clamp01(tc);
  const cx1 = p1[0] + sc * ux;
  const cy1 = p1[1] + sc * uy;
  const cx2 = p3[0] + tc * vx;
  const cy2 = p3[1] + tc * vy;
  return Math.hypot(cx1 - cx2, cy1 - cy2);
}

function segsIntersect(p1, p2, p3, p4) {
  const d = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  const d1 = d(p3, p4, p1);
  const d2 = d(p3, p4, p2);
  const d3 = d(p1, p2, p3);
  const d4 = d(p1, p2, p4);
  if (((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0))) {
    return true;
  }
  return false;
}
