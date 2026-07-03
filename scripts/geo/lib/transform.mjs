// scripts/geo/lib/transform.mjs
// The heart of the bake: swisstopo LoD2 GeoJSON (WGS84 + real Z) plus the
// OSM overlay → the compact baked schema the diorama loads. Groups surfaces
// by swisstopo UUID, normalizes each building to its own ground (min Z → 0),
// projects to local meters around the KSW anchor and quantizes positions to
// integer centimeters (JSON size). Throws on buildings that end up with no
// triangulatable geometry — a bake must never silently drop shape.
import { triangulatePlanarPolygon } from './triangulate.mjs';
import { nameForFootprint, ringCentroid, roadStyle } from './join.mjs';
import {
  convexHull, footprintValid, forestFill, roadWidthFromTags, roofOutlineFootprint, roofSkirts, treeSpec,
} from './style.mjs';

export const KSW_ZONE_RADIUS = 170; // m — hero exclusion zone around the anchor

/**
 * @typedef {{ pos: number[], idx: number[] }} BakedMesh
 * @typedef {{ pos: number[], idx: number[], fuv: number[] }} BakedWallMesh
 * @typedef {{ id: string, name?: string, usage?: string, zone: 'ksw'|'city',
 *   footprint: number[][], height: number, eaveH: number, wall: BakedWallMesh, roof: BakedMesh }} BakedBuilding
 */

// Facade UV quantization (Task 13): u/v are stored in 2-decimetre units
// (round(metres × FUV_PER_M)). 2 dm is well under the 2.4 m window grid, and
// the coarse step is a deliberate size lever — it both shortens the JSON
// numbers AND lets more shared corner vertices weld (their quantized u/v
// coincide), keeping buildings.json inside the 8 MB budget. Runtime divides by
// the SAME factor (cityMassing.ts FUV_PER_M).
export const FUV_PER_M = 5; // 1 unit = 0.2 m (2 dm)

// Facade UV for one wall facet (Task 13): u = horizontal distance along the
// facet's dominant horizontal direction (its XZ extent), v = height above
// building ground, both in 2-dm units. The origin is the facet's min-XZ corner
// so u is monotonic along the wall face; `dir` is the facet's own horizontal
// axis (from its XZ bounding extent). A facet seen perfectly edge-on in XZ (a
// near-horizontal cap) collapses to a point → dir length ~0, and every vertex
// maps to u=0 (harmless: such facets carry no window rows).
function facetFacadeUV(ring, groundY) {
  let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
  for (const [x, , z] of ring) {
    minX = Math.min(minX, x); maxX = Math.max(maxX, x);
    minZ = Math.min(minZ, z); maxZ = Math.max(maxZ, z);
  }
  let dx = maxX - minX;
  let dz = maxZ - minZ;
  const len = Math.hypot(dx, dz);
  if (len < 1e-4) { dx = 1; dz = 0; } else { dx /= len; dz /= len; }
  // u(vertex) = ((x,z) − facet min-corner) · dir, in metres → 2-dm units.
  // Facet-local origin keeps u small (bounded by the facet width) so the JSON
  // numbers stay short. The wall mesh welds by POSITION ONLY (see
  // wallMeshFromRings), so a vertex shared by two perpendicular walls at a
  // building corner keeps whichever facet wrote it first — a 1-vertex grid
  // seam on the few triangles spanning that corner. At clay/city scale that is
  // invisible, and it is the deliberate price of NOT doubling the wall vertex
  // buffer (position+fuv welding pushed the bake to 12 MB, over the 8 MB
  // budget). v (height) is a pure function of position.y, so it never conflicts.
  return (x, y, z) => {
    const u = (x - minX) * dx + (z - minZ) * dz;
    return [Math.round(u * FUV_PER_M), Math.round((y - groundY) * FUV_PER_M)];
  };
}

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

// cm-quantized positions naturally coincide at shared edges (ridge/valley/eave
// vertices from adjacent facets round to the same integer triplet), so a
// weld-by-exact-key pass is a pure encoding optimization — it never moves or
// merges geometry that wasn't already bit-identical, just avoids storing the
// same vertex once per triangle that touches it. Roof surfaces are real
// swisstopo LoD2 facets (~60/building) whose skirts (Task 3) roughly triple
// the unwelded vertex count; welding keeps the bake within the size budget
// without altering any coordinate.
function meshFromRings(rings, groundY) {
  const pos = [];
  const idx = [];
  const weld = new Map(); // "x,y,z" -> vertex index
  for (const ring of rings) {
    const tri = triangulatePlanarPolygon(ring);
    if (!tri) continue; // degenerate sliver surface — fine to skip a face
    const localToGlobal = new Array(tri.positions.length / 3);
    for (let i = 0, v = 0; i < tri.positions.length; i += 3, v++) {
      const x = Math.round(tri.positions[i] * 100);
      const y = Math.round((tri.positions[i + 1] - groundY) * 100);
      const z = Math.round(tri.positions[i + 2] * 100);
      const key = `${x},${y},${z}`;
      let idxOf = weld.get(key);
      if (idxOf === undefined) {
        idxOf = pos.length / 3;
        pos.push(x, y, z);
        weld.set(key, idxOf);
      }
      localToGlobal[v] = idxOf;
    }
    for (const t of tri.indices) idx.push(localToGlobal[t]);
  }
  return { pos, idx };
}

// Wall variant of meshFromRings (Task 13): identical position-welded
// triangulation (weld key is POSITION only — same vertex count as the plain
// wall mesh, so the bake stays in budget), plus a facade-UV pair (fuv) per
// vertex, aligned 1:1 with pos. A vertex shared by two facets keeps the fuv of
// whichever facet was triangulated first (see facetFacadeUV for the corner-seam
// tradeoff). fuv.length === pos.length/3 * 2 always.
function wallMeshFromRings(rings, groundY) {
  const pos = [];
  const idx = [];
  const fuv = [];
  const weld = new Map(); // "x,y,z" -> vertex index
  for (const ring of rings) {
    const tri = triangulatePlanarPolygon(ring);
    if (!tri) continue;
    const uvOf = facetFacadeUV(ring, groundY);
    const localToGlobal = new Array(tri.positions.length / 3);
    for (let i = 0, v = 0; i < tri.positions.length; i += 3, v++) {
      const px = tri.positions[i];
      const py = tri.positions[i + 1];
      const pz = tri.positions[i + 2];
      const x = Math.round(px * 100);
      const y = Math.round((py - groundY) * 100);
      const z = Math.round(pz * 100);
      const key = `${x},${y},${z}`;
      let idxOf = weld.get(key);
      if (idxOf === undefined) {
        idxOf = pos.length / 3;
        pos.push(x, y, z);
        const [u, vv] = uvOf(px, py, pz);
        fuv.push(u, vv);
        weld.set(key, idxOf);
      }
      localToGlobal[v] = idxOf;
    }
    for (const t of tri.indices) idx.push(localToGlobal[t]);
  }
  return { pos, idx, fuv };
}

// Append more facets (skirts, hull fallbacks) to a wall mesh, computing fuv for
// the emitted rings the same way and re-welding by position so the combined
// buffer stays vertex-aligned (fuv.length === pos.length/3 * 2).
function appendWallRings(mesh, rings, groundY) {
  const extra = wallMeshFromRings(rings, groundY);
  const weld = new Map();
  for (let i = 0; i < mesh.pos.length; i += 3) weld.set(`${mesh.pos[i]},${mesh.pos[i + 1]},${mesh.pos[i + 2]}`, i / 3);
  const remap = new Array(extra.pos.length / 3);
  for (let v = 0, i = 0; i < extra.pos.length; i += 3, v++) {
    const key = `${extra.pos[i]},${extra.pos[i + 1]},${extra.pos[i + 2]}`;
    let idxOf = weld.get(key);
    if (idxOf === undefined) {
      idxOf = mesh.pos.length / 3;
      mesh.pos.push(extra.pos[i], extra.pos[i + 1], extra.pos[i + 2]);
      mesh.fuv.push(extra.fuv[v * 2], extra.fuv[v * 2 + 1]);
      weld.set(key, idxOf);
    }
    remap[v] = idxOf;
  }
  for (const t of extra.idx) mesh.idx.push(remap[t]);
}

function appendRings(mesh, rings, groundY) {
  const extra = meshFromRings(rings, groundY);
  const weld = new Map();
  for (let i = 0; i < mesh.pos.length; i += 3) weld.set(`${mesh.pos[i]},${mesh.pos[i + 1]},${mesh.pos[i + 2]}`, i / 3);
  const remap = new Array(extra.pos.length / 3);
  for (let v = 0, i = 0; i < extra.pos.length; i += 3, v++) {
    const key = `${extra.pos[i]},${extra.pos[i + 1]},${extra.pos[i + 2]}`;
    let idxOf = weld.get(key);
    if (idxOf === undefined) {
      idxOf = mesh.pos.length / 3;
      mesh.pos.push(extra.pos[i], extra.pos[i + 1], extra.pos[i + 2]);
      weld.set(key, idxOf);
    }
    remap[v] = idxOf;
  }
  for (const t of extra.idx) mesh.idx.push(remap[t]);
}

// Walls by extruding the footprint from ground (y=0) to the eave. swisstopo's
// real wall facets (~40 per building) are invisible at clay scale but ~40×
// heavier than a prism; extrusion keeps every building solid and the JSON
// small while the real LoD2 roof still sits on top. cm-integer positions.
function extrudeWalls(footprint, eaveH) {
  const pos = [];
  const idx = [];
  const fuv = []; // 2 per vertex, 2-dm ints — facade UV per extruded quad
  const top = Math.round(eaveH * 100);
  const topV = Math.round(eaveH * FUV_PER_M); // v in 2-dm units
  for (let i = 0; i < footprint.length; i++) {
    const [x0, z0] = footprint[i];
    const [x1, z1] = footprint[(i + 1) % footprint.length];
    const X0 = Math.round(x0 * 100);
    const Z0 = Math.round(z0 * 100);
    const X1 = Math.round(x1 * 100);
    const Z1 = Math.round(z1 * 100);
    const base = pos.length / 3;
    pos.push(X0, 0, Z0, X1, 0, Z1, X1, top, Z1, X0, top, Z0);
    // u along this edge in dm: 0 at corner 0, edge length at corner 1. v = 0
    // at the base, eaveH at the top. Ground is y=0 for an extruded prism.
    const edgeU = Math.round(Math.hypot(x1 - x0, z1 - z0) * FUV_PER_M);
    fuv.push(0, 0, edgeU, 0, edgeU, topV, 0, topV);
    idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
  }
  return { pos, idx, fuv };
}

// Append an extruded prism (from extrudeWalls) to a wall mesh, carrying its
// fuv through so the combined buffer stays vertex-aligned. No welding needed —
// the prism's positions are new — but fuv MUST grow in lockstep with pos.
function appendPrism(mesh, prism) {
  const before = mesh.pos.length / 3;
  for (let i = 0; i < prism.pos.length; i += 3) mesh.pos.push(prism.pos[i], prism.pos[i + 1], prism.pos[i + 2]);
  for (let i = 0; i < prism.fuv.length; i++) mesh.fuv.push(prism.fuv[i]);
  for (const t of prism.idx) mesh.idx.push(before + t);
}

// Coverage-gate helper (Task 12): pull the wall-BASE points (in meters, XZ)
// out of a baked `wall.pos` mesh — the cm-integer, ground-normalized vertex
// buffer that actually got triangulated and rendered (real facets welded via
// meshFromRings, OR the extrudeWalls prism fallback). y ≤ 60 cm keeps only
// the ground ring, not eave/ridge vertices that happen to sit above a wall.
// Exported so the coverage math is testable against a fixture wall mesh
// directly, independent of the full transformBuildings pipeline.
export function wallBasePointsMeters(wallPos) {
  const pts = [];
  for (let i = 0; i < wallPos.length; i += 3) {
    if (wallPos[i + 1] <= 60) pts.push([wallPos[i] / 100, wallPos[i + 2] / 100]);
  }
  return pts;
}

// Single shared coverage test (Task 12 completion fix): a roof facet — given
// as its XZ centroid — counts as "covered" when some rendered wall-base point
// sits within 6 m. Used identically by the per-part-hull decision below and
// by the bake's overall coverage-gate stat, so there is exactly one
// definition of "floating" in the codebase.
export function facetCovered(centroidXZ, wallBaseXZ) {
  const [cx, cz] = centroidXZ;
  return wallBaseXZ.some(([wx, wz]) => Math.hypot(wx - cx, wz - cz) < 6);
}

function ringCentroidXZ(ring) {
  let cx = 0, cz = 0, n = 0;
  for (const [x, , z] of ring) {
    cx += x; cz += z; n += 1;
  }
  return [cx / n, cz / n];
}

// Group a building's roof rings into connected components: rings that share
// a vertex (within 20 cm in XZ+Y) belong to the same physical roof part.
// Union-find over ring endpoints keeps this O(n²) per building, which is
// fine — buildings carry dozens of roof facets, never thousands.
function roofComponents(roofRings) {
  const n = roofRings.length;
  const parent = Array.from({ length: n }, (_, i) => i);
  const find = (i) => {
    while (parent[i] !== i) {
      parent[i] = parent[parent[i]];
      i = parent[i];
    }
    return i;
  };
  const union = (a, b) => {
    const ra = find(a), rb = find(b);
    if (ra !== rb) parent[ra] = rb;
  };
  const closeEnough = (a, b) => Math.abs(a[0] - b[0]) < 0.2 && Math.abs(a[1] - b[1]) < 0.2 && Math.abs(a[2] - b[2]) < 0.2;
  for (let i = 0; i < n; i++) {
    for (let j = i + 1; j < n; j++) {
      let linked = false;
      for (const va of roofRings[i]) {
        for (const vb of roofRings[j]) {
          if (closeEnough(va, vb)) {
            linked = true;
            break;
          }
        }
        if (linked) break;
      }
      if (linked) union(i, j);
    }
  }
  const groups = new Map(); // root -> ring indices
  for (let i = 0; i < n; i++) {
    const r = find(i);
    (groups.get(r) ?? groups.set(r, []).get(r)).push(i);
  }
  return [...groups.values()].map((idxs) => idxs.map((i) => roofRings[i]));
}

// `footprints` (optional): Map<uuid, ring[[x,z]]> of 2D footprints already in
// LOCAL meters — used in production, where swisstopo's Floor layer is a
// GeoJSON-incompatible 3D solid so the footprint comes from Building_solid
// flattened to 2D instead. When absent (unit tests), the footprint falls back
// to the largest floor/wall ring. Footprints never feed ground normalization.
/** @returns {BakedBuilding[]} */
export function transformBuildings({
  floors,
  walls,
  roofs,
  osmBuildings,
  projector,
  footprints = null,
  stats = { traced: 0, fallback: 0, wallFallback: 0, roofFacetsTotal: 0, roofFacetsCovered: 0, floatingBuildings: 0 },
}) {
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

    // harden: a footprint that doesn't carry its roof is worse than the
    // roof's own projected outline (real swisstopo geometry either way)
    if (!footprintValid(footprint, b.roofs)) {
      const hull = roofOutlineFootprint(b.roofs.length ? b.roofs : b.walls);
      if (hull.length >= 3) {
        footprint = hull.map(([x, z]) => [Math.round(x * 100) / 100, Math.round(z * 100) / 100]);
        stats.fallback += 1;
      }
    } else {
      stats.traced += 1;
    }

    const eaveH = Math.max(eaveY - groundY, 0.1); // clamp: never a zero/neg prism
    const skirts = roofSkirts(b.roofs, eaveY);
    // Real wall facets carry every disjoint building part under its own
    // roof by construction (they come from the same per-UUID collection as
    // the roofs) — a single-footprint prism only covers the largest part.
    // extrudeWalls is kept ONLY as a fallback for the rare building with
    // zero wall facets (stats.wallFallback counts + the bake logs them).
    let wall;
    if (b.walls.length > 0) {
      wall = wallMeshFromRings(b.walls, groundY);
    } else {
      wall = extrudeWalls(footprint, eaveH);
      stats.wallFallback = (stats.wallFallback ?? 0) + 1;
    }
    appendWallRings(wall, skirts, groundY);

    // Per-roof-part hull closure (Task 12 completion fix): some swisstopo
    // roof PARTS genuinely have zero wall facets in the source — not "walls
    // far away", but none at all. Group this building's roof rings into
    // connected components (rings sharing a vertex), and for any component
    // whose facet centroids aren't covered by the wall mesh built so far,
    // extrude a ground→eave prism from the CONVEX HULL of that component's
    // own XZ vertices. This is geodetically honest: the hull derives
    // strictly from that part's real roof geometry, never from an unrelated
    // part's footprint.
    let partHulls = 0;
    if (b.roofs.length > 0) {
      for (const component of roofComponents(b.roofs)) {
        const wallBaseXZ = wallBasePointsMeters(wall.pos);
        const centroids = component.map(ringCentroidXZ);
        const coveredCount = centroids.filter((c) => facetCovered(c, wallBaseXZ)).length;
        if (coveredCount / centroids.length < 0.5) {
          const hullXZ = convexHull(component.flatMap((ring) => ring.map(([x, , z]) => [x, z])));
          if (hullXZ.length >= 3) {
            let compEave = Infinity;
            for (const ring of component) for (const [, y] of ring) compEave = Math.min(compEave, y);
            const compEaveH = Math.max(compEave - groundY, 0.1);
            appendPrism(wall, extrudeWalls(hullXZ, compEaveH));
            partHulls += 1;
          }
        }
      }
    }
    // Second pass, per FACET: a large complex roof component (real hospital/
    // school wings, dozens–hundreds of facets) can clear the component-level
    // 0.5 average above yet still leave individual interior facets — deep
    // ridges/valleys/dormers — outside 6 m of the component hull's own
    // perimeter wall, a real geometric ceiling of a single convex-hull prism
    // over a large footprint. Close any facet still uncovered after the
    // component pass with the smallest possible honest closure: a prism from
    // that ONE facet's own XZ vertices — no roof anywhere left without a hull
    // under it.
    if (b.roofs.length > 0) {
      for (const ring of b.roofs) {
        const wallBaseXZ = wallBasePointsMeters(wall.pos);
        if (facetCovered(ringCentroidXZ(ring), wallBaseXZ)) continue;
        const hullXZ = convexHull(ring.map(([x, , z]) => [x, z]));
        if (hullXZ.length < 3) continue;
        let facetEave = Infinity;
        for (const [, y] of ring) facetEave = Math.min(facetEave, y);
        const facetEaveH = Math.max(facetEave - groundY, 0.1);
        appendPrism(wall, extrudeWalls(hullXZ, facetEaveH));
        partHulls += 1;
      }
    }
    if (partHulls > 0) stats.partHulls = (stats.partHulls ?? 0) + partHulls;

    // roofUnderside dropped (Task 12 completion fix): the per-part/per-facet
    // hull closures above push total output past the 8 MB budget; the
    // underside triangles were an invisible-from-outside backface refinement
    // (never load-bearing for the floating-roof fix), so cutting them is the
    // documented, honest way back under budget rather than loosening a size
    // or coverage gate.
    const roof = meshFromRings(b.roofs, groundY);
    if (wall.idx.length === 0 && roof.idx.length === 0)
      throw new Error(`bake: building ${uuid} has surfaces but none triangulated`);

    // Coverage gate data (Task 12): does every roof FACET sit on a real wall,
    // or is it a disjoint part with none (the floating-roof bug)? Per-facet
    // (not per-vertex) because a hip/gable roof's interior ridge vertices sit
    // naturally many meters from any wall CORNER — a strict vertex↔vertex
    // check false-flags healthy geometry. A facet's own centroid pulls those
    // interior points back toward its eave, so "no wall vertex within 6 m of
    // the facet centroid" only fires when the facet truly has no wall nearby.
    //
    // Measured against the BAKED `wall.pos` mesh (post per-part-hull closure)
    // — not the raw per-UUID `b.walls` facets. The raw facets exist
    // unconditionally (they're the ogr2ogr extraction), even on the branch
    // where `wall` falls back to `extrudeWalls(footprint, ...)`
    // (single-footprint prism, the exact bug this gate exists to catch) — so
    // gating on `b.walls` read ~100% on broken output. `wall.pos` is what
    // actually got rendered, so basing the proximity test on it makes the
    // gate honest.
    if (b.roofs.length > 0) {
      const wallBaseXZ = wallBasePointsMeters(wall.pos);
      let buildingBad = 0;
      for (const ring of b.roofs) {
        const covered = facetCovered(ringCentroidXZ(ring), wallBaseXZ);
        stats.roofFacetsTotal += 1;
        if (covered) stats.roofFacetsCovered += 1;
        else buildingBad += 1;
      }
      if (buildingBad / b.roofs.length > 0.3) stats.floatingBuildings += 1;
    }

    const [cx, cz] = ringCentroid(footprint);
    const building = {
      id: uuid,
      zone: Math.hypot(cx, cz) < KSW_ZONE_RADIUS ? 'ksw' : 'city',
      footprint,
      height: Math.round((topY - groundY) * 100) / 100,
      // Real eave height (m, 1 decimal): the shader clamps the window raster so
      // no storey top sits above it — walls end at the eave, gable skirts reach
      // the ridge, so the wall mesh's own max-y would be WRONG here.
      eaveH: Math.round(eaveH * 10) / 10,
      wall,
      roof,
      ...nameForFootprint(footprint, osmBuildings),
    };
    out.push(building);
  }
  return out;
}

// Nature overlay: green areas (parks/woods/grass/…), water bodies, river
// centerlines and individual tree points — all straight from OSM, projected
// to local meters. Greens carry their kind so the renderer can shade parks
// lighter than woods; rivers keep their tagged width (default 5 m).
const GREEN_KINDS = new Set([
  'park', 'garden', 'pitch', 'playground', // leisure
  'grass', 'meadow', 'forest', 'cemetery', 'village_green', 'recreation_ground', 'allotments', // landuse
  'wood', 'scrub', 'grassland', // natural
]);

export function transformNature({ osmNature, projector }) {
  const greens = [];
  const waterAreas = [];
  const rivers = [];
  const trees = [];
  const toLocal = ({ lon, lat }) => {
    const [x, z] = projector.toLocal(lon, lat);
    return [Math.round(x * 100) / 100, Math.round(z * 100) / 100];
  };
  for (const el of osmNature.elements ?? []) {
    const t = el.tags ?? {};
    if (el.type === 'node') {
      if (t.natural === 'tree') {
        const [x, z] = toLocal(el);
        trees.push(treeSpec(t, x, z));
      }
      continue;
    }
    if (el.type !== 'way' || !el.geometry || el.geometry.length < 2) continue;
    if (t.waterway === 'river' || t.waterway === 'stream') {
      const width = Number.parseFloat(t.width ?? '') || (t.waterway === 'river' ? 5 : 2);
      rivers.push({ width, pts: el.geometry.map(toLocal) });
      continue;
    }
    if (el.geometry.length < 4) continue; // areas need a closed-ish ring
    const ring = el.geometry.map(toLocal);
    if (t.natural === 'water') {
      waterAreas.push({ ring });
      continue;
    }
    const kind = t.leisure || t.landuse || t.natural;
    if (kind && GREEN_KINDS.has(kind)) greens.push({ kind, ring });
  }
  // declared forest fill: every wood/forest green gets a hash-gridded scatter
  // of additional trees, clear of any tree OSM already mapped individually
  for (const g of greens) {
    if (g.kind === 'wood' || g.kind === 'forest') trees.push(...forestFill(g.ring, trees));
  }
  return { greens, waterAreas, rivers, trees };
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
    const width = roadWidthFromTags(el.tags ?? {}, style.width);
    (style.class === 'rail' ? rails : roads).push({ class: style.class, width, pts });
  }
  return { roads, rails };
}
