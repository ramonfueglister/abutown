// scripts/geo/lib/style.mjs
// Pure bake-side style/robustness derivations for the diorama-style slice.
// Everything here is a deterministic function of real geometry.
import { pointInRing } from './join.mjs';

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

// Newell's method — robust for the triangle-shaped (one repeated-vertex)
// quads a skirt collapses to when its rising edge already touches the eave.
function quadNormalXZ(quad) {
  let nx = 0, nz = 0;
  for (let i = 0; i < quad.length; i++) {
    const [x0, y0, z0] = quad[i];
    const [x1, y1, z1] = quad[(i + 1) % quad.length];
    nx += (y0 - y1) * (z0 + z1);
    nz += (x0 - x1) * (y0 + y1);
  }
  return [nx, nz];
}

// Vertical fill from each rising roof boundary edge down to the eave —
// closes the open gable triangles left by prism walls. Shared (ridge/valley)
// edges appear in two planes and are deduped; edges lying at eave level need
// no skirt. The source roof ring's winding is not trusted: each skirt quad's
// horizontal normal is checked against (edge midpoint − roof centroid) and
// the ring is reversed if it faces inward, so skirts are always outward-facing
// regardless of upstream winding.
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

  let cx = 0, cz = 0, n = 0;
  for (const ring of roofRings) {
    for (const [x, , z] of ring) {
      cx += x; cz += z; n++;
    }
  }
  cx /= n || 1;
  cz /= n || 1;

  const out = [];
  for (const { a, b, k } of edges) {
    if (seen.get(k) > 1) continue; // interior edge (ridge/valley)
    if (a[1] <= eaveY + EDGE_EPS && b[1] <= eaveY + EDGE_EPS) continue; // flat at eave
    const quad = [
      [a[0], a[1], a[2]],
      [b[0], b[1], b[2]],
      [b[0], eaveY, b[2]],
      [a[0], eaveY, a[2]],
    ];
    const [nx, nz] = quadNormalXZ(quad);
    const mx = (a[0] + b[0]) / 2, mz = (a[2] + b[2]) / 2;
    const outX = mx - cx, outZ = mz - cz;
    if (nx * outX + nz * outZ < 0) quad.reverse();
    out.push(quad);
  }
  return out;
}

export function roofUnderside(roofRings, drop = 0.22) {
  return roofRings.map((ring) => [...ring].reverse().map(([x, y, z]) => [x, y - drop, z]));
}

const h01 = (x, z) => {
  const s = Math.sin(x * 127.1 + z * 311.7) * 43758.5453;
  return s - Math.floor(s);
};
// Slice 2 (Fallback-Variante): Familien + Grössen aus Landuse-Heuristik.
// Wachstumskurven h(age) = h∞ · age/(age + t½) (saturierend, Spec §1) mit
// deterministischem Pseudo-Alter, da ohne Kataster kein Pflanzjahr existiert.
// Konstanten: grobe mitteleuropäische Stadtbaum-/Waldbaum-Endgrössen
// (Platane/Linde/Pappel/Fichte-Klassen), dokumentiert hier als die "einmal
// recherchierte Tabelle" der Spec.
export const GROWTH = {
  spreading: { hInf: 28, rInf: 9.0, tHalf: 35 },
  oval:      { hInf: 30, rInf: 7.0, tHalf: 40 },
  tall:      { hInf: 32, rInf: 5.0, tHalf: 25 },
  conic:     { hInf: 34, rInf: 4.5, tHalf: 45 },
  slender:   { hInf: 20, rInf: 3.0, tHalf: 30 },
};

// Gewichtete Familienwahl je Kontext. Reihenfolge/Schwellen deterministisch.
const FAMILY_MIX = {
  needlewood:  [['conic', 0.7], ['slender', 1.0]],
  broadwood:   [['oval', 0.6], ['spreading', 0.9], ['tall', 1.0]],
  mixedwood:   [['oval', 0.4], ['spreading', 0.6], ['conic', 0.9], ['slender', 1.0]],
  park:        [['spreading', 0.4], ['oval', 0.8], ['tall', 1.0]],
  street:      [['spreading', 0.5], ['oval', 1.0]],
};

export function familyFor(x, z, kind, context = {}) {
  const inWood = context.green === 'wood' || context.green === 'forest';
  let mix;
  if (inWood) {
    if (context.leafType === 'needleleaved') mix = FAMILY_MIX.needlewood;
    else if (context.leafType === 'broadleaved') mix = FAMILY_MIX.broadwood;
    else mix = FAMILY_MIX.mixedwood;
  } else if (context.green) {
    mix = FAMILY_MIX.park;
  } else {
    mix = FAMILY_MIX.street;
  }
  // kind ist OSM-Ground-Truth: conifer erzwingt Nadel-Familien, broad Laub.
  const allowed = kind === 'conifer' ? ['conic', 'slender'] : ['spreading', 'oval', 'tall'];
  const filtered = mix.filter(([f]) => allowed.includes(f));
  const pool = filtered.length ? filtered : allowed.map((f, i) => [f, (i + 1) / allowed.length]);
  const hv = h01(x * 12.9898 + 4.1414, z * 78.233);
  const top = pool[pool.length - 1][1];
  for (const [f, cum] of pool) if (hv <= cum / top) return f;
  return pool[pool.length - 1][0];
}

export function sizeFor(family, x, z) {
  const g = GROWTH[family];
  // Pseudo-Alter 8..88 Jahre, quadratisch Richtung jung geschoben (Städte
  // pflanzen nach) — deterministisch pro Koordinate.
  const a01 = h01(x * 3.7 + 11.1, z * 9.3 - 2.2);
  const age = 8 + 80 * a01 * a01;
  const grow = age / (age + g.tHalf);
  const jitter = 0.9 + 0.2 * h01(x * 5.1, z * 13.7);
  return {
    h: Math.round(g.hInf * grow * jitter * 10) / 10,
    r: Math.round(g.rInf * grow * (0.9 + 0.2 * h01(x * 7.9, z * 3.3)) * 10) / 10,
  };
}

// tree specs — real OSM tags win untouched; otherwise a family/growth-curve
// derived size (context = landuse/leaf-type heuristic), with deterministic
// hashing on position (never on a real tag value) so repeat bakes are
// byte-identical.
export function treeSpec(tags, x, z, context = {}) {
  const kind = tags.leaf_type === 'needleleaved' ? 'conifer' : 'broad';
  const family = familyFor(x, z, kind, context);
  const sized = sizeFor(family, x, z);
  const tagH = Number.parseFloat(tags.height ?? '');
  const tagCrown = Number.parseFloat(tags.diameter_crown ?? '');
  return {
    x, z, kind, family,
    h: tagH > 0 ? tagH : sized.h,
    // Tiny-Crown-Guard (Skelett-Defekt): Krone nie unter 30% der Familien-
    // Erwartung — ein OSM diameter_crown=0.5 erzeugte trunk-only-Skelette.
    r: tagCrown > 0 ? Math.max(tagCrown / 2, sized.r * 0.3) : sized.r,
  };
}

// Declared forest fill: a hash-gridded scatter of broad-leaf trees inside a
// real wood/forest polygon, at ~1/60 m² density. Never placed within 4 m of a
// tree OSM already mapped individually — those are the ground truth.
//
// Municipality-scale bakes (Task 9) can carry hundreds of thousands of
// forest-fill candidates across a couple hundred forest/wood polygons — the
// original `existingTrees.some(...)` linear scan per candidate made this
// O(total_trees^2) across the whole bake (existingTrees is the same
// accumulating array for every forest, per transformNature's loop), which
// measured multiple CPU-minutes and climbing on the real Winterthur forest
// coverage (~37 km² estimated forest bbox area, easily 10^5+ candidates).
// A 4 m spatial hash grid over existingTrees turns the "any tree within 4 m"
// check into an O(1)-ish 3x3-cell lookup — same exclusion radius, same
// candidate order (grid scan is unchanged), so output is identical, just fast.
function existingTreeGrid(existingTrees, cell) {
  const grid = new Map(); // "gx,gz" -> [{x,z}]
  for (const t of existingTrees) {
    const gx = Math.floor(t.x / cell), gz = Math.floor(t.z / cell);
    const k = `${gx},${gz}`;
    (grid.get(k) ?? grid.set(k, []).get(k)).push(t);
  }
  return grid;
}

export function forestFill(ring, existingTrees, density = 1 / 60, context = { green: 'wood' }) {
  const cell = Math.sqrt(1 / density);
  let x0 = Infinity, x1 = -Infinity, z0 = Infinity, z1 = -Infinity;
  for (const [x, z] of ring) {
    x0 = Math.min(x0, x); x1 = Math.max(x1, x);
    z0 = Math.min(z0, z); z1 = Math.max(z1, z);
  }
  const EXCLUDE = 4;
  const grid = existingTreeGrid(existingTrees, EXCLUDE);
  const tooClose = (x, z) => {
    const gx = Math.floor(x / EXCLUDE), gz = Math.floor(z / EXCLUDE);
    for (let dx = -1; dx <= 1; dx++) {
      for (let dz = -1; dz <= 1; dz++) {
        const pts = grid.get(`${gx + dx},${gz + dz}`);
        if (!pts) continue;
        for (const t of pts) if (Math.hypot(t.x - x, t.z - z) < EXCLUDE) return true;
      }
    }
    return false;
  };
  const out = [];
  for (let gx = Math.floor(x0 / cell); gx * cell < x1; gx++) {
    for (let gz = Math.floor(z0 / cell); gz * cell < z1; gz++) {
      const jx = (h01(gx * 13.7, gz * 71.3) - 0.5) * cell * 0.8;
      const jz = (h01(gx * 91.7, gz * 23.1) - 0.5) * cell * 0.8;
      const x = (gx + 0.5) * cell + jx;
      const z = (gz + 0.5) * cell + jz;
      if (!pointInRing(x, z, ring)) continue;
      if (tooClose(x, z)) continue;
      const family = familyFor(x, z, context.leafType === 'needleleaved' ? 'conifer' : 'broad', context);
      const kind = ['conic', 'slender'].includes(family) ? 'conifer' : 'broad';
      const sized = sizeFor(family, x, z);
      out.push({ x: Math.round(x * 100) / 100, z: Math.round(z * 100) / 100, kind, family, h: sized.h, r: sized.r });
    }
  }
  return out;
}

// road width: explicit width tag wins, then lanes × 3.2 m, then the
// class-keyed fallback already resolved by join.mjs's roadStyle.
export function roadWidthFromTags(tags, fallbackWidth) {
  const w = Number.parseFloat(tags.width ?? '');
  if (w > 0) return w;
  const lanes = Number.parseInt(tags.lanes ?? '', 10);
  if (lanes > 0) return lanes * 3.2;
  return fallbackWidth;
}

// door placement: the facade segment whose midpoint is closest to any nearby
// road point gets a door at its midpoint, yaw pointing outward toward the road.
export function doorForBuilding(footprint, roadPts) {
  if (!roadPts.length) return null;
  let best = null;
  for (let i = 0; i < footprint.length; i++) {
    const [ax, az] = footprint[i];
    const [bx, bz] = footprint[(i + 1) % footprint.length];
    if (Math.hypot(bx - ax, bz - az) < 2.2) continue; // too short for a door
    const mx = (ax + bx) / 2;
    const mz = (az + bz) / 2;
    for (const [rx, rz] of roadPts) {
      const dist = Math.hypot(rx - mx, rz - mz);
      if (!best || dist < best.dist) best = { dist, mx, mz, ax, az, bx, bz, rx, rz };
    }
  }
  if (!best) return null;
  // outward normal of the edge, flipped toward the road
  let nx = -(best.bz - best.az);
  let nz = best.bx - best.ax;
  const len = Math.hypot(nx, nz) || 1;
  nx /= len; nz /= len;
  if (nx * (best.rx - best.mx) + nz * (best.rz - best.mz) < 0) { nx = -nx; nz = -nz; }
  return { x: Math.round(best.mx * 100) / 100, z: Math.round(best.mz * 100) / 100, yaw: Math.round(Math.atan2(nx, nz) * 1000) / 1000 };
}
