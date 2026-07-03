// scripts/geo/lib/join.mjs
// swisstopo gives shape, OSM gives meaning: join OSM names/usage onto the
// swisstopo footprints via centroid containment (no shared id — EGID is not
// mapped area-wide in OSM), and classify OSM ways into renderable road
// ribbons with per-class widths (meters).

export function pointInRing(x, z, ring) {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    if (zi > z !== zj > z && x < ((xj - xi) * (z - zi)) / (zj - zi) + xi) inside = !inside;
  }
  return inside;
}

export function ringCentroid(ring) {
  let a = 0, cx = 0, cz = 0;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const f = ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
    a += f;
    cx += (ring[j][0] + ring[i][0]) * f;
    cz += (ring[j][1] + ring[i][1]) * f;
  }
  if (Math.abs(a) < 1e-9) return ring[0];
  return [cx / (3 * a), cz / (3 * a)];
}

export function nameForFootprint(footprint, osmPolys) {
  const [cx, cz] = ringCentroid(footprint);
  // smallest containing polygon wins: a department building inside the
  // campus polygon should get its own name, not the campus name
  let best = null;
  for (const p of osmPolys) {
    if (!p.tags.name || !pointInRing(cx, cz, p.ring)) continue;
    const size = ringArea(p.ring);
    if (!best || size < best.size) best = { p, size };
  }
  if (!best) return {};
  const t = best.p.tags;
  const out = { name: t.name };
  const usage = t.healthcare || t.amenity || t.building;
  if (usage && usage !== 'yes') out.usage = usage;
  return out;
}

function ringArea(ring) {
  let a = 0;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    a += ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
  }
  return Math.abs(a / 2);
}

const ROAD_WIDTHS = {
  motorway: 12, trunk: 11, primary: 9, secondary: 8, tertiary: 7,
  unclassified: 5.5, residential: 5.5, living_street: 5, service: 4,
  pedestrian: 4, track: 3, cycleway: 2.4, footway: 2.2, path: 2, steps: 2,
};

export function roadStyle(tags) {
  if (tags.railway === 'rail') return { class: 'rail', width: 3.2 };
  if (tags.railway === 'tram') return { class: 'rail', width: 2.8 };
  const hw = tags.highway;
  if (!hw) return null;
  // "_link" ramps inherit their parent class width
  const base = hw.endsWith('_link') ? hw.slice(0, -5) : hw;
  const width = ROAD_WIDTHS[base];
  if (width === undefined) return null; // proposed, construction, corridor, …
  return { class: base, width };
}
