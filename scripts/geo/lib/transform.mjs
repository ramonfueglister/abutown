// scripts/geo/lib/transform.mjs
// The heart of the bake: swisstopo LoD2 GeoJSON (WGS84 + real Z) plus the
// OSM overlay → the compact baked schema the diorama loads. Groups surfaces
// by swisstopo UUID, normalizes each building to its own ground (min Z → 0),
// projects to local meters around the KSW anchor and quantizes positions to
// integer centimeters (JSON size). Throws on buildings that end up with no
// triangulatable geometry — a bake must never silently drop shape.
import { triangulatePlanarPolygon } from './triangulate.mjs';
import { nameForFootprint, ringCentroid, roadStyle } from './join.mjs';

export const KSW_ZONE_RADIUS = 170; // m — hero exclusion zone around the anchor

/**
 * @typedef {{ pos: number[], idx: number[] }} BakedMesh
 * @typedef {{ id: string, name?: string, usage?: string, zone: 'ksw'|'city',
 *   footprint: number[][], height: number, wall: BakedMesh, roof: BakedMesh }} BakedBuilding
 */

// Trace the real footprint from the wall surfaces: every wall facet has a
// bottom edge sitting at ground level; collected and chained end-to-end they
// form the building outline. This is the honest footprint — flattening the 3D
// solid instead just overlays all faces and yields a single stray facet.
// Returns the largest closed ring as [[x,z],…] in local meters, or null.
function footprintFromWalls(wallRings, groundY) {
  const EPS = 0.6; // m above the base still counts as a ground vertex
  const key = (x, z) => `${Math.round(x * 100)},${Math.round(z * 100)}`;
  const pts = new Map(); // key → [x, z]
  const adj = new Map(); // key → Set(neighbour keys)
  const link = (ka, kb) => {
    if (ka === kb) return;
    (adj.get(ka) ?? adj.set(ka, new Set()).get(ka)).add(kb);
    (adj.get(kb) ?? adj.set(kb, new Set()).get(kb)).add(ka);
  };
  for (const ring of wallRings) {
    for (let i = 0; i < ring.length; i++) {
      const a = ring[i];
      const b = ring[(i + 1) % ring.length];
      if (a[1] > groundY + EPS || b[1] > groundY + EPS) continue;
      const ka = key(a[0], a[2]);
      const kb = key(b[0], b[2]);
      pts.set(ka, [a[0], a[2]]);
      pts.set(kb, [b[0], b[2]]);
      link(ka, kb);
    }
  }
  if (adj.size < 3) return null;
  // walk loops greedily, always leaving by an unused edge
  const used = new Set();
  const edgeId = (a, b) => (a < b ? `${a}|${b}` : `${b}|${a}`);
  let best = null;
  let bestArea = 0;
  for (const startKey of adj.keys()) {
    for (const first of adj.get(startKey)) {
      if (used.has(edgeId(startKey, first))) continue;
      const ring = [pts.get(startKey)];
      let prev = startKey;
      let cur = first;
      used.add(edgeId(startKey, first));
      let ok = false;
      for (let guard = 0; guard < 10000; guard++) {
        ring.push(pts.get(cur));
        if (cur === startKey) {
          ok = true;
          break;
        }
        let next = null;
        for (const n of adj.get(cur)) {
          if (n !== prev && !used.has(edgeId(cur, n))) {
            next = n;
            break;
          }
        }
        if (next === null) break; // dead end
        used.add(edgeId(cur, next));
        prev = cur;
        cur = next;
      }
      if (ok && ring.length >= 4) {
        const a = Math.abs(polyArea(ring));
        if (a > bestArea) {
          bestArea = a;
          best = ring;
        }
      }
    }
  }
  return bestArea > 1 ? best : null;
}

function polyArea(ring) {
  let a = 0;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    a += ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
  }
  return a / 2;
}

function* ringsOf(geometry) {
  // MultiPolygon: [poly][ring][pt]; we take every outer ring (holes are
  // rare in LoD2 surfaces and negligible at clay scale)
  if (!geometry) return;
  if (geometry.type === 'MultiPolygon') for (const poly of geometry.coordinates) yield poly[0];
  else if (geometry.type === 'Polygon') yield geometry.coordinates[0];
}

function collectByUuid(fc, projector, into, key) {
  for (const f of fc.features) {
    const uuid = f.properties?.UUID;
    if (!uuid || !f.geometry) continue;
    const b = into.get(uuid) ?? { floors: [], walls: [], roofs: [] };
    for (const ring of ringsOf(f.geometry)) {
      // project each vertex: [lon, lat, Z] → [x, Z, z]
      b[key].push(ring.map(([lon, lat, y]) => {
        const [x, z] = projector.toLocal(lon, lat);
        return [x, y ?? 0, z];
      }));
    }
    into.set(uuid, b);
  }
}

function meshFromRings(rings, groundY) {
  const pos = [];
  const idx = [];
  for (const ring of rings) {
    const tri = triangulatePlanarPolygon(ring);
    if (!tri) continue; // degenerate sliver surface — fine to skip a face
    const base = pos.length / 3;
    for (let i = 0; i < tri.positions.length; i += 3) {
      pos.push(
        Math.round(tri.positions[i] * 100),
        Math.round((tri.positions[i + 1] - groundY) * 100),
        Math.round(tri.positions[i + 2] * 100),
      );
    }
    for (const t of tri.indices) idx.push(base + t);
  }
  return { pos, idx };
}

// Walls by extruding the footprint from ground (y=0) to the eave. swisstopo's
// real wall facets (~40 per building) are invisible at clay scale but ~40×
// heavier than a prism; extrusion keeps every building solid and the JSON
// small while the real LoD2 roof still sits on top. cm-integer positions.
function extrudeWalls(footprint, eaveH) {
  const pos = [];
  const idx = [];
  const top = Math.round(eaveH * 100);
  for (let i = 0; i < footprint.length; i++) {
    const [x0, z0] = footprint[i];
    const [x1, z1] = footprint[(i + 1) % footprint.length];
    const X0 = Math.round(x0 * 100);
    const Z0 = Math.round(z0 * 100);
    const X1 = Math.round(x1 * 100);
    const Z1 = Math.round(z1 * 100);
    const base = pos.length / 3;
    pos.push(X0, 0, Z0, X1, 0, Z1, X1, top, Z1, X0, top, Z0);
    idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
  }
  return { pos, idx };
}

// `footprints` (optional): Map<uuid, ring[[x,z]]> of 2D footprints already in
// LOCAL meters — used in production, where swisstopo's Floor layer is a
// GeoJSON-incompatible 3D solid so the footprint comes from Building_solid
// flattened to 2D instead. When absent (unit tests), the footprint falls back
// to the largest floor/wall ring. Footprints never feed ground normalization.
/** @returns {BakedBuilding[]} */
export function transformBuildings({ floors, walls, roofs, osmBuildings, projector, footprints = null }) {
  const byUuid = new Map();
  collectByUuid(floors ?? { features: [] }, projector, byUuid, 'floors');
  collectByUuid(walls, projector, byUuid, 'walls');
  collectByUuid(roofs, projector, byUuid, 'roofs');

  const out = [];
  for (const [uuid, b] of byUuid) {
    if (b.roofs.length === 0 && b.walls.length === 0) continue; // floor-only stub
    // groundY = building base (walls reach the ground; roofs start at the eave)
    let groundY = Infinity;
    for (const ring of [...b.floors, ...b.walls, ...b.roofs])
      for (const [, y] of ring) groundY = Math.min(groundY, y);
    // eaveY = where walls meet the roof = the roof's lowest point (fallback:
    // wall top when a building has no roof surface); topY = ridge.
    let eaveY = Infinity;
    let topY = -Infinity;
    const capRings = b.roofs.length ? b.roofs : b.walls;
    for (const ring of capRings)
      for (const [, y] of ring) {
        eaveY = Math.min(eaveY, y);
        topY = Math.max(topY, y);
      }
    if (!b.roofs.length) eaveY = topY; // no roof: wall goes straight to the top

    // footprint, best source first: an explicitly supplied ring, then the
    // real outline traced from the wall bases, then the largest floor/wall ring
    let footprint;
    const fpLocal = footprints?.get(uuid);
    const traced = fpLocal ? null : footprintFromWalls(b.walls, groundY);
    if (fpLocal) {
      footprint = fpLocal.map(([x, z]) => [Math.round(x * 100) / 100, Math.round(z * 100) / 100]);
    } else if (traced) {
      footprint = traced.map(([x, z]) => [Math.round(x * 100) / 100, Math.round(z * 100) / 100]);
    } else {
      const candidateRings = b.floors.length ? b.floors : b.walls.length ? b.walls : b.roofs;
      if (candidateRings.length === 0) continue; // no footprint source at all
      const footprint3d = candidateRings.reduce((best, r) => (r.length > best.length ? r : best), candidateRings[0]);
      footprint = footprint3d.map(([x, , z]) => [Math.round(x * 100) / 100, Math.round(z * 100) / 100]);
    }

    const eaveH = Math.max(eaveY - groundY, 0.1); // clamp: never a zero/neg prism
    const wall = extrudeWalls(footprint, eaveH);
    const roof = meshFromRings(b.roofs, groundY);
    if (wall.idx.length === 0 && roof.idx.length === 0)
      throw new Error(`bake: building ${uuid} has surfaces but none triangulated`);

    const [cx, cz] = ringCentroid(footprint);
    const building = {
      id: uuid,
      zone: Math.hypot(cx, cz) < KSW_ZONE_RADIUS ? 'ksw' : 'city',
      footprint,
      height: Math.round((topY - groundY) * 100) / 100,
      wall,
      roof,
      ...nameForFootprint(footprint, osmBuildings),
    };
    out.push(building);
  }
  return out;
}

export function transformRoads({ osmRoads, projector }) {
  const roads = [];
  const rails = [];
  for (const el of osmRoads.elements ?? []) {
    if (el.type !== 'way' || !el.geometry || el.geometry.length < 2) continue;
    const style = roadStyle(el.tags ?? {});
    if (!style) continue;
    const pts = el.geometry.map(({ lon, lat }) => {
      const [x, z] = projector.toLocal(lon, lat);
      return [Math.round(x * 100) / 100, Math.round(z * 100) / 100];
    });
    (style.class === 'rail' ? rails : roads).push({ class: style.class, width: style.width, pts });
  }
  return { roads, rails };
}
