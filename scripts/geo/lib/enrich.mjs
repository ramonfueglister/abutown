// scripts/geo/lib/enrich.mjs
// Pure enrichment geometry for the building-attributes bake: LV95→WGS84,
// point-in-polygon, area centroid, and the two deterministic joins
// (ÖREB Bauzone via footprint centroid, GWR via building points inside the
// footprint). All join inputs are in LOCAL PLATE METRES — the caller projects
// attribute data with makeProjector().toLocal, never the other way around.

// swisstopo approximation formulas (EN→WGS84), accurate to ~1 m — far below
// parcel-polygon accuracy. Source: swisstopo "Näherungslösungen für die
// direkte Transformation LV95 ⇄ WGS84".
export function lv95ToWgs84(e, n) {
  const y = (e - 2600000) / 1000000;
  const x = (n - 1200000) / 1000000;
  const lonSec =
    2.6779094 + 4.728982 * y + 0.791484 * y * x + 0.1306 * y * x * x - 0.0436 * y * y * y;
  const latSec =
    16.9023892 + 3.238272 * x - 0.270978 * y * y - 0.002528 * x * x - 0.0447 * y * y * x -
    0.014 * x * x * x;
  return { lon: (lonSec * 100) / 36, lat: (latSec * 100) / 36 };
}

// Ray-casting point-in-polygon. Ring may or may not repeat its first vertex.
export function pointInPolygon([px, pz], ring) {
  let inside = false;
  const n = ring.length;
  for (let i = 0, j = n - 1; i < n; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    if (zi > pz !== zj > pz && px < ((xj - xi) * (pz - zi)) / (zj - zi) + xi) inside = !inside;
  }
  return inside;
}

// Area-weighted centroid (shoelace). Degenerate ring → vertex average.
export function centroid(ring) {
  let a = 0;
  let cx = 0;
  let cz = 0;
  const n = ring.length;
  for (let i = 0, j = n - 1; i < n; j = i++) {
    const cross = ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
    a += cross;
    cx += (ring[j][0] + ring[i][0]) * cross;
    cz += (ring[j][1] + ring[i][1]) * cross;
  }
  if (Math.abs(a) < 1e-9) {
    let sx = 0;
    let sz = 0;
    for (const [x, z] of ring) {
      sx += x;
      sz += z;
    }
    return [sx / n, sz / n];
  }
  return [cx / (3 * a), cz / (3 * a)];
}

// GWR Gebäudekategorie (GKAT) labels, Merkmalskatalog 4.2.
export const GKAT_LABELS = {
  1010: 'Provisorische Unterkunft',
  1020: 'Gebäude mit ausschliesslicher Wohnnutzung',
  1030: 'Wohngebäude mit Nebennutzung',
  1040: 'Gebäude mit teilweiser Wohnnutzung',
  1060: 'Gebäude ohne Wohnnutzung',
  1080: 'Sonderbau',
};

// Bauzone via footprint centroid: the centroid's containing zone wins
// (spec: building spanning >1 zone → centroid decides). No zone → null.
export function joinBauzone(footprint, zones) {
  const c = centroid(footprint);
  for (const z of zones) {
    if (pointInPolygon(c, z.ring)) {
      return { bauzone: z.bauzone, bauzoneCode: z.bauzoneCode, zhCode: z.zhCode };
    }
  }
  return null;
}

// GWR: every point inside the footprint counts; dominant GKAT wins, ties
// break to the LOWEST EGID (determinism). No point inside → null.
export function joinGwr(footprint, points) {
  const hits = points.filter((p) => pointInPolygon([p.x, p.z], footprint));
  if (hits.length === 0) return null;
  const byKat = new Map();
  for (const h of hits) {
    if (!byKat.has(h.gkat)) byKat.set(h.gkat, []);
    byKat.get(h.gkat).push(h);
  }
  let best = null;
  for (const [gkat, group] of byKat) {
    group.sort((a, b) => a.egid - b.egid);
    if (!best || group.length > best.group.length ||
        (group.length === best.group.length && group[0].egid < best.group[0].egid)) {
      best = { gkat, group };
    }
  }
  const primary = best.group[0];
  return {
    egid: primary.egid,
    gwrCategory: GKAT_LABELS[best.gkat] ?? `GKAT ${best.gkat}`,
    gwrClass: primary.gklas ?? null,
    egids: hits.map((h) => h.egid).sort((a, b) => a - b),
  };
}
