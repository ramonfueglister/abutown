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

export function transformBuildings({ floors, walls, roofs, osmBuildings, projector }) {
  const byUuid = new Map();
  collectByUuid(floors, projector, byUuid, 'floors');
  collectByUuid(walls, projector, byUuid, 'walls');
  collectByUuid(roofs, projector, byUuid, 'roofs');

  const out = [];
  for (const [uuid, b] of byUuid) {
    if (b.roofs.length === 0 && b.walls.length === 0) continue; // floor-only stub
    let groundY = Infinity;
    for (const ring of [...b.floors, ...b.walls, ...b.roofs])
      for (const [, y] of ring) groundY = Math.min(groundY, y);
    let topY = -Infinity;
    for (const ring of b.roofs.length ? b.roofs : b.walls)
      for (const [, y] of ring) topY = Math.max(topY, y);

    // footprint: largest floor ring (fallback: lowest wall ring projected)
    const floorRings = b.floors.length ? b.floors : b.walls;
    const footprint3d = floorRings.reduce((best, r) => (r.length > best.length ? r : best), floorRings[0]);
    const footprint = footprint3d.map(([x, , z]) => [Math.round(x * 100) / 100, Math.round(z * 100) / 100]);

    const wall = meshFromRings(b.walls, groundY);
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
