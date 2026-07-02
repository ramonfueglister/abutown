// scripts/geo/lib/style.mjs
// Pure bake-side style/robustness derivations for the diorama-style slice.
// Everything here is a deterministic function of real geometry.

function bboxOf(pts) {
  let x0 = Infinity, x1 = -Infinity, z0 = Infinity, z1 = -Infinity;
  for (const p of pts) {
    x0 = Math.min(x0, p[0]); x1 = Math.max(x1, p[0]);
    z0 = Math.min(z0, p[1]); z1 = Math.max(z1, p[1]);
  }
  return { x0, x1, z0, z1 };
}

function areaOf(ring) {
  let a = 0;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++)
    a += ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
  return Math.abs(a / 2);
}

const roofXZ = (roofRings) => roofRings.flatMap((r) => r.map(([x, , z]) => [x, z]));

// A footprint is trustworthy when it encloses the roof (±1.5 m) and is not a
// stray facet (≥ half the roof's projected area).
export function footprintValid(footprint, roofRings) {
  if (!footprint || footprint.length < 3) return false;
  if (roofRings.length === 0) return true; // nothing to validate against
  const fb = bboxOf(footprint);
  const rb = bboxOf(roofXZ(roofRings));
  const M = 1.5;
  if (rb.x0 < fb.x0 - M || rb.x1 > fb.x1 + M || rb.z0 < fb.z0 - M || rb.z1 > fb.z1 + M) return false;
  const roofArea = (rb.x1 - rb.x0) * (rb.z1 - rb.z0);
  return areaOf(footprint) >= 0.5 * Math.min(roofArea, areaOf(convexHull(roofXZ(roofRings))));
}

// Andrew's monotone chain — the roof IS swisstopo geometry, its projected
// hull is an honest footprint fallback.
export function convexHull(pts) {
  const p = [...pts].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  if (p.length < 3) return p;
  const cross = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  const lower = [];
  for (const pt of p) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], pt) <= 0) lower.pop();
    lower.push(pt);
  }
  const upper = [];
  for (const pt of p.reverse()) {
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], pt) <= 0) upper.pop();
    upper.push(pt);
  }
  return lower.slice(0, -1).concat(upper.slice(0, -1));
}

export function roofOutlineFootprint(roofRings) {
  return convexHull(roofXZ(roofRings));
}

const EDGE_EPS = 0.05;
const ekey = (a, b) => {
  const ka = `${Math.round(a[0] * 50)},${Math.round(a[1] * 50)},${Math.round(a[2] * 50)}`;
  const kb = `${Math.round(b[0] * 50)},${Math.round(b[1] * 50)},${Math.round(b[2] * 50)}`;
  return ka < kb ? `${ka}|${kb}` : `${kb}|${ka}`;
};

// Vertical fill from each rising roof boundary edge down to the eave —
// closes the open gable triangles left by prism walls. Shared (ridge/valley)
// edges appear in two planes and are deduped; edges lying at eave level need
// no skirt.
export function roofSkirts(roofRings, eaveY) {
  const seen = new Map(); // ekey -> count
  const edges = [];
  for (const ring of roofRings) {
    for (let i = 0; i < ring.length; i++) {
      const a = ring[i];
      const b = ring[(i + 1) % ring.length];
      const k = ekey(a, b);
      seen.set(k, (seen.get(k) ?? 0) + 1);
      edges.push({ a, b, k });
    }
  }
  const out = [];
  for (const { a, b, k } of edges) {
    if (seen.get(k) > 1) continue; // interior edge (ridge/valley)
    if (a[1] <= eaveY + EDGE_EPS && b[1] <= eaveY + EDGE_EPS) continue; // flat at eave
    out.push([
      [a[0], a[1], a[2]],
      [b[0], b[1], b[2]],
      [b[0], eaveY, b[2]],
      [a[0], eaveY, a[2]],
    ]);
  }
  return out;
}

export function roofUnderside(roofRings, drop = 0.22) {
  return roofRings.map((ring) => [...ring].reverse().map(([x, y, z]) => [x, y - drop, z]));
}
