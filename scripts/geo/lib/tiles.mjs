// scripts/geo/lib/tiles.mjs
// Quadtree-Kacheln: Zuteilung von Gebäuden/Bäumen/Landuse/Straßen in
// L0/L1/L2-Zellen, plus deterministisches protobuf-Encode (WorldTile,
// WorldManifest, RoadGraph). Alle Iterationen laufen über sortierte
// Schlüssel — keine Map/Set/Object-Insertion-Order darf in die Ausgabe
// durchsickern (Determinismus-Vertrag der Bake-Pipeline).
import { create, toBinary } from '@bufbuild/protobuf';
import { WorldTileSchema, WorldManifestSchema, RoadGraphSchema } from '../proto/world_pb.js';
import { extractPatch } from './dem.mjs';

const LEVEL_CELLS = [1, 4, 16]; // L0, L1, L2 subdivisions per side
const LEVEL_GRID_N = [21, 51, 101];

// Corridor-snap margin cap (Task 5d): the finest (L2) tile vertex step is
// ~12.5 m, so its cell diagonal is ~17.7 m. The corridor-snap sampler widens
// its hard-clamp region by the tile's cell diagonal so a road bench is clamped
// across the whole cell it crosses (a narrow corridor otherwise slips between
// the 12.5 m vertices and the bilinear mesh interpolates the bench away). At
// coarse levels the raw cell diagonal would be huge (L1 ~141 m, L0 ~1414 m) and
// gouge terrain far from any road, so the per-tile margin is CAPPED at the L2
// diagonal — every level clamps the same road footprint, so LOD switches don't
// pop piercing in/out, and no level lowers terrain more than one L2 cell beyond
// the corridor. The metric reads the finest (L2) tiles (worldData.ts), where
// the margin is exactly the cell diagonal and the clamp is tight.
const SNAP_MARGIN_CAP_M = Math.SQRT2 * (1250 / 100); // √2 · L2 vertex step (12.5 m)

// ---- geometry helpers -----------------------------------------------

function ringBBox(ring) {
  let minX = Infinity, minZ = Infinity, maxX = -Infinity, maxZ = -Infinity;
  for (const [x, z] of ring) {
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (z < minZ) minZ = z;
    if (z > maxZ) maxZ = z;
  }
  return { minX, minZ, maxX, maxZ };
}

function ringArea(ring) {
  let a = 0;
  for (let i = 0; i < ring.length; i++) {
    const [x0, z0] = ring[i];
    const [x1, z1] = ring[(i + 1) % ring.length];
    a += x0 * z1 - x1 * z0;
  }
  return Math.abs(a) / 2;
}

function centroid(ring) {
  let sx = 0, sz = 0;
  for (const [x, z] of ring) { sx += x; sz += z; }
  return [sx / ring.length, sz / ring.length];
}

// Standard even-odd raycast point-in-polygon test.
function pointInRing(px, pz, ring) {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    const intersect = ((zi > pz) !== (zj > pz)) &&
      (px < ((xj - xi) * (pz - zi)) / (zj - zi) + xi);
    if (intersect) inside = !inside;
  }
  return inside;
}

// ---- tileGridFor -------------------------------------------------------

export function tileGridFor(boundaryRing, ringPadM) {
  const { minX, minZ, maxX, maxZ } = ringBBox(boundaryRing);
  const padMinX = minX - ringPadM, padMinZ = minZ - ringPadM;
  const padMaxX = maxX + ringPadM, padMaxZ = maxZ + ringPadM;
  const width = padMaxX - padMinX, height = padMaxZ - padMinZ;
  const side = Math.max(width, height);
  const size = Math.ceil(side / 1000) * 1000;
  // Center the padded bbox in the root square, then floor-align nothing
  // further — we just need a fixed deterministic origin.
  const cx = (padMinX + padMaxX) / 2, cz = (padMinZ + padMaxZ) / 2;
  const minXOut = cx - size / 2, minZOut = cz - size / 2;
  return { minX: minXOut, minZ: minZOut, size, levels: 3 };
}

// ---- assignToTiles -------------------------------------------------------

function cellIndexFor(root, level, x, z) {
  const n = LEVEL_CELLS[level];
  const cellSize = root.size / n;
  let ix = Math.floor((x - root.minX) / cellSize);
  let iz = Math.floor((z - root.minZ) / cellSize);
  ix = Math.max(0, Math.min(n - 1, ix));
  iz = Math.max(0, Math.min(n - 1, iz));
  return { ix, iz, cellSize };
}

function tileId(level, x, y) {
  return `L${level}/${x}_${y}`;
}

function makeBucket(level, x, y, root) {
  const n = LEVEL_CELLS[level];
  const cellSize = root.size / n;
  return {
    level, x, y,
    originX: root.minX + x * cellSize,
    originZ: root.minZ + y * cellSize,
    cellSize,
    buildings: [],
    trees: [],
    landuse: [],
    roadSegs: [],
  };
}

function ensureBucket(tiles, level, x, y, root) {
  const id = tileId(level, x, y);
  let b = tiles.get(id);
  if (!b) {
    b = makeBucket(level, x, y, root);
    tiles.set(id, b);
  }
  return b;
}

function ensureAllBuckets(tiles, root) {
  for (let level = 0; level < 3; level++) {
    const n = LEVEL_CELLS[level];
    for (let y = 0; y < n; y++)
      for (let x = 0; x < n; x++)
        ensureBucket(tiles, level, x, y, root);
  }
}

function segClipsCell(bucket, x0, z0, x1, z1) {
  // Cheap AABB overlap test between the segment's bbox and the cell — good
  // enough for render-band assignment (exact clipping happens at draw time).
  const minX = Math.min(x0, x1), maxX = Math.max(x0, x1);
  const minZ = Math.min(z0, z1), maxZ = Math.max(z0, z1);
  const cMinX = bucket.originX, cMaxX = bucket.originX + bucket.cellSize;
  const cMinZ = bucket.originZ, cMaxZ = bucket.originZ + bucket.cellSize;
  return maxX >= cMinX && minX <= cMaxX && maxZ >= cMinZ && minZ <= cMaxZ;
}

function bucketsOverlappingBBox(root, level, minX, minZ, maxX, maxZ) {
  const n = LEVEL_CELLS[level];
  const cellSize = root.size / n;
  let ix0 = Math.floor((minX - root.minX) / cellSize);
  let ix1 = Math.floor((maxX - root.minX) / cellSize);
  let iz0 = Math.floor((minZ - root.minZ) / cellSize);
  let iz1 = Math.floor((maxZ - root.minZ) / cellSize);
  ix0 = Math.max(0, Math.min(n - 1, ix0));
  ix1 = Math.max(0, Math.min(n - 1, ix1));
  iz0 = Math.max(0, Math.min(n - 1, iz0));
  iz1 = Math.max(0, Math.min(n - 1, iz1));
  const out = [];
  for (let iz = iz0; iz <= iz1; iz++)
    for (let ix = ix0; ix <= ix1; ix++)
      out.push({ ix, iz });
  return out;
}

export function assignToTiles(root, features) {
  const tiles = new Map();
  ensureAllBuckets(tiles, root);

  // --- buildings: assign by centroid into L2, mirror as prisms in L1,
  //     dropped entirely in L0.
  const buildings = [...(features.buildings ?? [])].sort((a, b) =>
    a.id < b.id ? -1 : a.id > b.id ? 1 : 0
  );
  for (const b of buildings) {
    const [cx, cz] = centroid(b.footprint);
    const l2 = cellIndexFor(root, 2, cx, cz);
    const l1 = cellIndexFor(root, 1, cx, cz);
    ensureBucket(tiles, 2, l2.ix, l2.iz, root).buildings.push(b);
    ensureBucket(tiles, 1, l1.ix, l1.iz, root).buildings.push(b);
    // L0: buildings dropped entirely (terrain+landcover only).
  }

  // --- trees: assign by point into L2 (and L1, coarser); dropped in L0.
  const trees = [...(features.trees ?? [])].sort((a, b) => {
    if (a.x !== b.x) return a.x - b.x;
    return a.z - b.z;
  });
  for (const t of trees) {
    const l2 = cellIndexFor(root, 2, t.x, t.z);
    const l1 = cellIndexFor(root, 1, t.x, t.z);
    ensureBucket(tiles, 2, l2.ix, l2.iz, root).trees.push(t);
    ensureBucket(tiles, 1, l1.ix, l1.iz, root).trees.push(t);
  }

  // --- landuse: assign to every overlapping cell at every level (bbox test;
  //     exact point-in-ring resolution happens at encode time).
  const landuse = [...(features.landuse ?? [])]
    .map((lu, idx) => ({ ...lu, _idx: idx }))
    .sort((a, b) => {
      if (a.kind !== b.kind) return a.kind - b.kind;
      const areaDiff = ringArea(b.ring) - ringArea(a.ring); // area descending
      if (areaDiff !== 0) return areaDiff;
      return a._idx - b._idx; // stable tie-break
    });
  for (const lu of landuse) {
    const { minX, minZ, maxX, maxZ } = ringBBox(lu.ring);
    for (let level = 0; level < 3; level++) {
      const cells = bucketsOverlappingBBox(root, level, minX, minZ, maxX, maxZ);
      for (const { ix, iz } of cells) {
        ensureBucket(tiles, level, ix, iz, root).landuse.push({ kind: lu.kind, ring: lu.ring });
      }
    }
  }

  // --- graph edges -> road render segments, clipped per cell (bbox test).
  // Class thresholds: L2 = all, L1 = class <= 5, L0 = class <= 3 (mains only).
  const g = features.graph ?? {};
  const edgeCount = (g.edgeA ?? []).length;
  for (let e = 0; e < edgeCount; e++) {
    const cls = g.edgeClass[e];
    const start = g.edgePtOffset[e];
    const end = e + 1 < edgeCount ? g.edgePtOffset[e + 1] : g.edgePtX.length;
    const seg = {
      edgeIndex: e,
      class: cls,
      width: g.edgeWidth[e],
      x: g.edgePtX.slice(start, end),
      z: g.edgePtZ.slice(start, end),
      y: g.edgePtY.slice(start, end),
    };
    if (seg.x.length < 2) continue;
    let minX = Infinity, minZ = Infinity, maxX = -Infinity, maxZ = -Infinity;
    for (let i = 0; i < seg.x.length; i++) {
      if (seg.x[i] < minX) minX = seg.x[i];
      if (seg.x[i] > maxX) maxX = seg.x[i];
      if (seg.z[i] < minZ) minZ = seg.z[i];
      if (seg.z[i] > maxZ) maxZ = seg.z[i];
    }
    const levelsToAssign = [];
    if (cls <= 5) levelsToAssign.push(1);
    levelsToAssign.push(2); // all classes in L2
    if (cls <= 3) levelsToAssign.push(0); // mains additionally in L0
    for (const level of levelsToAssign) {
      const cells = bucketsOverlappingBBox(root, level, minX, minZ, maxX, maxZ);
      for (const { ix, iz } of cells) {
        const bucket = ensureBucket(tiles, level, ix, iz, root);
        // Precise-enough per-segment clip test against this cell's bbox.
        let overlaps = false;
        for (let i = 0; i < seg.x.length - 1 && !overlaps; i++) {
          if (segClipsCell(bucket, seg.x[i], seg.z[i], seg.x[i + 1], seg.z[i + 1])) overlaps = true;
        }
        if (overlaps) bucket.roadSegs.push(seg);
      }
    }
  }

  return tiles;
}

// ---- encodeTile -------------------------------------------------------

function resolveLandcover(px, pz, sortedRings) {
  // Rings are pre-sorted by kind asc, then area desc, with a stable
  // tie-break — the first ring containing the point wins.
  for (const lu of sortedRings) {
    if (pointInRing(px, pz, lu.ring)) return lu.kind;
  }
  return 1; // MEADOW default
}

export function encodeTile(bucket, demSampler) {
  const gridN = LEVEL_GRID_N[bucket.level];
  const cellStep = bucket.cellSize / (gridN - 1);
  // Widen the corridor-snap hard-clamp region by this tile's cell diagonal
  // (capped at the L2 diagonal) so a road bench is clamped across the whole
  // cell it crosses. Plain samplers ignore the extra arg (see extractPatch).
  const snapMarginM = Math.min(Math.SQRT2 * cellStep, SNAP_MARGIN_CAP_M);
  const height = extractPatch(demSampler, {
    originX: bucket.originX,
    originZ: bucket.originZ,
    gridN,
    cellSize: cellStep,
  }, snapMarginM);

  const sortedRings = [...bucket.landuse].sort((a, b) => {
    if (a.kind !== b.kind) return a.kind - b.kind;
    return ringArea(b.ring) - ringArea(a.ring);
  });

  const landcover = new Array(gridN * gridN);
  for (let j = 0; j < gridN; j++) {
    for (let i = 0; i < gridN; i++) {
      const px = bucket.originX + i * cellStep;
      const pz = bucket.originZ + j * cellStep;
      landcover[j * gridN + i] = resolveLandcover(px, pz, sortedRings);
    }
  }

  const msg = {
    level: bucket.level,
    x: bucket.x,
    y: bucket.y,
    gridN,
    cellSize: cellStep,
    originX: bucket.originX,
    originZ: bucket.originZ,
    height: Array.from(height),
    landcover,
  };

  if (bucket.level >= 1) {
    // Buildings, sorted by id for determinism.
    const buildings = [...bucket.buildings].sort((a, b) =>
      a.id < b.id ? -1 : a.id > b.id ? 1 : 0
    );
    const bId = [], bUsage = [], bHeight = [], bBaseY = [];
    const bFpOffset = [], bFpX = [], bFpZ = [];
    const bAccessEdge = [], bAccessOffset = [];
    const bMeshVoffset = [], bMeshPos = [];
    const bMeshIoffset = [], bMeshIdx = [];
    for (const b of buildings) {
      bId.push(b.id);
      bUsage.push(b.usage ?? 0);
      bHeight.push(b.height ?? 0);
      bBaseY.push(b.baseY ?? 0);
      bFpOffset.push(bFpX.length);
      for (const [fx, fz] of b.footprint) {
        bFpX.push(fx);
        bFpZ.push(fz);
      }
      bAccessEdge.push(b.access?.edge ?? 0xffffffff);
      bAccessOffset.push(b.access?.offsetM ?? 0);
      if (bucket.level === 2 && b.mesh) {
        bMeshVoffset.push(bMeshPos.length / 3);
        for (const v of b.mesh.pos) bMeshPos.push(v);
        bMeshIoffset.push(bMeshIdx.length);
        for (const idx of b.mesh.idx) bMeshIdx.push(idx);
      }
    }
    Object.assign(msg, {
      bId, bUsage, bHeight, bBaseY,
      bFpOffset, bFpX, bFpZ,
      bAccessEdge, bAccessOffset,
      bMeshVoffset, bMeshPos, bMeshIoffset, bMeshIdx,
    });
  }

  // Road render segments, sorted by (edgeIndex) for determinism.
  const roadSegs = [...bucket.roadSegs].sort((a, b) => a.edgeIndex - b.edgeIndex);
  const rClass = [], rWidth = [], rPtOffset = [], rPtX = [], rPtZ = [], rPtY = [];
  for (const seg of roadSegs) {
    rClass.push(seg.class);
    rWidth.push(seg.width);
    rPtOffset.push(rPtX.length);
    for (let i = 0; i < seg.x.length; i++) {
      rPtX.push(seg.x[i]);
      rPtZ.push(seg.z[i]);
      rPtY.push(seg.y[i]);
    }
  }
  Object.assign(msg, { rClass, rWidth, rPtOffset, rPtX, rPtZ, rPtY });

  // Trees, sorted by (x, z) for determinism.
  const trees = [...bucket.trees].sort((a, b) => (a.x !== b.x ? a.x - b.x : a.z - b.z));
  const tX = [], tZ = [], tH = [], tR = [], tKind = [], tFamily = [];
  for (const t of trees) {
    tX.push(t.x);
    tZ.push(t.z);
    tH.push(t.h ?? t.height ?? 0);
    tR.push(t.r ?? t.radius ?? 0);
    tKind.push(t.kind ?? 0);
    // family is optional (older callers/tests may omit it); an entirely
    // absent family across all trees keeps t_family empty (proto3-additive:
    // "leer = Alt-Bake" per world.proto), which tileTreeSpecs treats as
    // family undefined rather than 0/spreading.
    if (t.family !== undefined) tFamily.push(t.family);
  }
  Object.assign(msg, { tX, tZ, tH, tR, tKind });
  if (tFamily.length === trees.length && trees.length > 0) Object.assign(msg, { tFamily });

  return toBinary(WorldTileSchema, create(WorldTileSchema, msg));
}

// ---- thin encode wrappers -------------------------------------------------

export function encodeManifest(manifest) {
  return toBinary(WorldManifestSchema, create(WorldManifestSchema, manifest));
}

export function encodeGraph(graph) {
  return toBinary(RoadGraphSchema, create(RoadGraphSchema, graph));
}
