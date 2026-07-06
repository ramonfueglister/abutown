// Landuse-Ringe aus OSM: Landcover-Enum-Kind + Ring in lokalen Metern
// (gerundet wie transformRoads). Unbekannte landuse/natural-Tags → weglassen.
const KIND_MAP = {
  meadow: 1, grass: 1,
  forest: 2, wood: 2,
  farmland: 3,
  residential: 4,
  industrial: 5, commercial: 5,
  basin: 6, reservoir: 6, water: 6, riverbank: 6,
};

// Stitches the `outer` members of a multipolygon relation into closed rings
// by joining ways at shared endpoints (OSM shared nodes project to identical
// lon/lat, so exact comparison is safe). Ways may run in either orientation.
// Rings that fail to close (broken data) are still emitted — polygon fill
// closes them implicitly, and an approximate water mask beats a missing one.
function stitchOuterRings(members) {
  const segs = (members ?? [])
    .filter((m) => m.role === 'outer' && m.geometry && m.geometry.length >= 2)
    .map((m) => m.geometry.map(({ lon, lat }) => [lon, lat]));
  const eq = (a, b) => a[0] === b[0] && a[1] === b[1];
  const closed = (r) => r.length >= 4 && eq(r[0], r[r.length - 1]);
  const rings = [];
  while (segs.length) {
    let ring = segs.pop();
    let extended = true;
    while (extended && !closed(ring)) {
      extended = false;
      for (let i = 0; i < segs.length; i++) {
        const s = segs[i];
        const head = ring[0];
        const tail = ring[ring.length - 1];
        if (eq(tail, s[0])) ring = ring.concat(s.slice(1));
        else if (eq(tail, s[s.length - 1])) ring = ring.concat(s.slice(0, -1).reverse());
        else if (eq(head, s[s.length - 1])) ring = s.slice(0, -1).concat(ring);
        else if (eq(head, s[0])) ring = s.slice(1).reverse().concat(ring);
        else continue;
        segs.splice(i, 1);
        extended = true;
        break;
      }
    }
    if (ring.length >= 3) rings.push(ring);
  }
  return rings;
}

export function transformLanduse({ osmLanduse, projector }) {
  const project = (lon, lat) => {
    const [x, z] = projector.toLocal(lon, lat);
    return [Math.round(x * 100) / 100, Math.round(z * 100) / 100];
  };
  const out = [];
  for (const el of osmLanduse.elements ?? []) {
    const t = el.tags ?? {};
    const tag = t.landuse ?? t.natural ?? t.waterway;
    const kind = KIND_MAP[tag];
    if (!kind) continue;
    if (el.type === 'way') {
      if (!el.geometry || el.geometry.length < 3) continue;
      out.push({ kind, ring: el.geometry.map(({ lon, lat }) => project(lon, lat)) });
    } else if (el.type === 'relation' && kind === 6) {
      // Multipolygon relations are expanded for WATER only: the Töss and the
      // Eulach are mapped as natural=water relations, and terrain grading
      // must never touch water (spec §4.4). Other landuse kinds stay
      // way-only until something needs them. Inner rings (islands) are
      // ignored — that slightly over-masks water, which is fine for grading.
      for (const ring of stitchOuterRings(el.members)) {
        out.push({ kind, ring: ring.map(([lon, lat]) => project(lon, lat)) });
      }
    }
  }
  return out;
}

// Water rings for terrain grading: all kind-6 (basin/reservoir → WATER)
// items from transformLanduse output. Grading must never touch water cells.
export const waterRingsFrom = (items) => items.filter((l) => l.kind === 6).map((l) => l.ring);
