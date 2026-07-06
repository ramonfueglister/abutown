// src/diorama/ksw/geo/tileContent.ts
// Task 4 (M3): per-tile materialization for the streaming pyramid. Turns one
// DecodedTile into a THREE.Group holding the terrain grid mesh and (per LOD
// mode) ONE merged building-prism mesh, plus the tile's filtered TreeSpec[]
// for Task 5's instancing layer — no tree meshes are built here.
//
// #141: L1 tiles (level === 1) are materialized in 4×4 SUB-CELLS congruent
// with their L2 children — terrain split into 16 resampled sub-meshes,
// massing partitioned by footprint centroid into per-sub-cell merged meshes,
// trees partitioned into per-sub-cell pool keys (`L1/x_y#i_j`) — so main.ts
// can hide exactly the one region a live L2 tile replaces instead of the
// whole 5-km group (which killed the mid-ring forest + massing).
//
// Coordinate convention (matches main.ts's
// `terrainRoot.position.y = -anchorGroundHeight(world)` pattern): all tile
// content is built in ABSOLUTE bake metres (DEM heights, real bBaseY
// elevations). The caller passes the scene's vertical shift as
// `ctx.groundShiftY`, which is applied ONCE to `group.position.y` — geometry
// is never re-based, so a tile can be materialized before/after the anchor is
// known and the group simply re-parented.
//
// Building prisms are the CHEAP massing variant: footprint ring extruded from
// bBaseY to bBaseY+bHeight via cityMassing's `ringBand` (out=0 → double-sided
// walls) plus a double-sided flat roof cap (ShapeUtils triangulation). All
// kept prisms of a tile merge into ONE tinted mesh (mergeTinted), sharing the
// module-level tintedClay material — dispose() therefore frees geometries
// only, never materials.
import * as THREE from 'three/webgpu';
import { palette } from '../../designTokens';
import { buildTerrainTileMesh, buildTileSubCellTerrain } from './terrain';
import { mergeTinted, ringBand, tintedClay } from './cityMassing';
import { tileTreeSpecs } from './worldData';
import type { DecodedTile } from './worldData';
import type { BakedBuilding, BakedMesh, TreeSpec } from './geoData';
import type { TileKey } from './tileStreamer';
import type { CorridorMask } from './corridorMask';

export type MaterializeCtx = {
  /** Corridor-discard mask (spec §5): streamed tiles must apply the SAME
   * road/rail fragment-discard as the boot-loaded terrain, or roads would
   * sink into un-discarded streamed terrain. Optional for tests. */
  corridorMask?: CorridorMask;
  /** Hero-plate exclusion rect (center x/z, size w/d) or null: buildings whose
   * footprint CENTROID falls inside are skipped. Also the tree-exclusion rect
   * when `treeExcludeRect` is omitted (back-compat). */
  plateRect: { x: number; z: number; w: number; d: number } | null;
  /** Tree-only exclusion rect. buildings.json only covers the plate, so
   * building massing must always exclude via `plateRect` — but the boot
   * nature.json trees extend ~100 m+ past the plate (M3 Task 6 finding:
   * 2501/7350 nature trees sit outside plateRect, 72% of those coincide
   * <0.5 m with streamed tile trees → visible double trees in the boot
   * near-ring). Pass the larger rect that covers the boot nature-tree bbox
   * (main.ts's `samplerRect`) here so streamed tile trees are excluded over
   * the SAME footprint the boot-resident nature trees already cover.
   * Defaults to `plateRect` when omitted, for callers that don't care
   * (L0/L1 paths, tests). */
  treeExcludeRect?: { x: number; z: number; w: number; d: number } | null;
  /** Applied to group.position.y (see header) — never baked into geometry. */
  groundShiftY: number;
  /** L2: buildings+trees, L1: trees only (impostors, Task 5), L0: neither. */
  buildings: boolean;
  trees: boolean;
};

// ── #141 per-sub-cell L1 hide: L2 ↔ L1-sub-cell mapping ─────────────────────
// The bake subdivides 4×4 per level (LEVEL_CELLS = [1, 4, 16]), so every L2
// tile (x, y) is exactly one quarter-quarter of L1 (x>>2, y>>2) — sub-cell
// (x & 3, y & 3). L1 tiles are materialized in 16 sub-cells congruent with
// their L2 children so main.ts can hide precisely the region a live L2 tile
// replaces (terrain + massing + tree pool) instead of the whole 5-km group.
export const SUB_CELLS_PER_SIDE = 4;

export function l1SubCellOfL2(x: number, y: number): { l1x: number; l1y: number; i: number; j: number } {
  return { l1x: x >> 2, l1y: y >> 2, i: x & 3, j: y & 3 };
}

export function subCellKey(i: number, j: number): string {
  return `${i}_${j}`;
}

export type SubCellContent = {
  /** This sub-cell's scene meshes (terrain sub-mesh + optional massing mesh)
   * — main.ts toggles `visible` on exactly these when the congruent L2 tile
   * arrives/leaves. */
  meshes: THREE.Mesh[];
  /** Tree-pool key (`L1/x_y#i_j`) for treeLayer add/removeTileTrees; null
   * when no trees survived the exclusion rects in this sub-cell. */
  treeKey: string | null;
  /** The sub-cell's filtered specs, held for re-registration on L2 unload. */
  trees: TreeSpec[];
};

export type TileContent = {
  key: TileKey;
  group: THREE.Group;
  /** Identifier Task 5 uses to (un)register this tile's tree batch; null when
   * the tile contributes no trees (trees:false or none survived the plate) —
   * and ALWAYS null for sub-celled (L1) tiles, whose trees register per
   * sub-cell via `subCells` instead. */
  treeKey: string | null;
  /** Plate-filtered specs for Task 5's instancing — no meshes built here. */
  trees: TreeSpec[];
  /** L1 only (level === 1): the 16 sub-cells keyed `subCellKey(i, j)`; null
   * for L0/L2 tiles, which keep the single-mesh whole-tile behavior. */
  subCells: Map<string, SubCellContent> | null;
  dispose(): void;
};

// Same excludeRect predicate as treeLayer.ts/nature.ts: inside the
// axis-aligned rect given as center (x,z) and size (w,d).
function insideRect(x: number, z: number, ex: { x: number; z: number; w: number; d: number }): boolean {
  return Math.abs(x - ex.x) <= ex.w / 2 && Math.abs(z - ex.z) <= ex.d / 2;
}

// Shared clay material for ALL tile-building meshes (module-level, like
// terrain.ts's terrainMat) — TileContent.dispose must never dispose it.
let buildingMat: THREE.MeshPhysicalMaterial | null = null;
function sharedBuildingMat(): THREE.MeshPhysicalMaterial {
  buildingMat ??= tintedClay(palette.creamBase);
  return buildingMat;
}

// One cheap prism (cm-int BakedMesh) for a footprint ring: ringBand walls
// (out=0) + a double-sided flat cap at the top. Double-sided cap keeps us
// winding-agnostic w.r.t. the baked ring orientation.
function prismParts(fp: number[][], y0: number, y1: number): BakedMesh {
  const walls = ringBand(fp, y0, y1, 0);
  const pos = [...walls.pos];
  const idx = [...walls.idx];
  const base = pos.length / 3;
  for (const [x, z] of fp) {
    pos.push(Math.round(x * 100), Math.round(y1 * 100), Math.round(z * 100));
  }
  const tris = THREE.ShapeUtils.triangulateShape(
    fp.map(([x, z]) => new THREE.Vector2(x, z)),
    [],
  );
  for (const [a, b, c] of tris) {
    idx.push(base + a, base + b, base + c);
    idx.push(base + c, base + b, base + a);
  }
  return { pos, idx };
}

/** Materializes one decoded tile into a group of meshes + filtered TreeSpecs.
 * See the header comment for the absolute-bake-metres/groundShiftY convention. */
export function materializeTile(dec: DecodedTile, ctx: MaterializeCtx): TileContent {
  const { level, x, y, tile } = dec;
  const key: TileKey = `L${level}/${x}_${y}`;

  const group = new THREE.Group();
  group.name = `tile/${key}`;
  group.position.y = ctx.groundShiftY;

  let plateSkipped = 0;

  // #141: L1 tiles are materialized in 4×4 sub-cells congruent with their L2
  // children (see l1SubCellOfL2), so a live L2 tile can hide exactly the one
  // region it replaces. L0/L2 keep the single-mesh whole-tile behavior.
  const subbed = level === 1;
  const subCells: Map<string, SubCellContent> | null = subbed ? new Map() : null;
  if (subCells) {
    for (let j = 0; j < SUB_CELLS_PER_SIDE; j++) {
      for (let i = 0; i < SUB_CELLS_PER_SIDE; i++) {
        subCells.set(subCellKey(i, j), { meshes: [], treeKey: null, trees: [] });
      }
    }
  }
  // Sub-cell index of a world point — clamped so content that sits exactly on
  // (or a hair past) the tile border still lands in a valid sub-cell.
  const subExtent = ((tile.gridN - 1) * tile.cellSize) / SUB_CELLS_PER_SIDE;
  const subIdxOf = (px: number, pz: number): [number, number] => [
    Math.min(SUB_CELLS_PER_SIDE - 1, Math.max(0, Math.floor((px - tile.originX) / subExtent))),
    Math.min(SUB_CELLS_PER_SIDE - 1, Math.max(0, Math.floor((pz - tile.originZ) / subExtent))),
  ];

  // --- Terrain (always) -----------------------------------------------------
  if (subCells) {
    // 16 resampled sub-meshes, seam-exact on the source bilinear surface.
    const { group: cellsGroup, meshes } = buildTileSubCellTerrain(dec, SUB_CELLS_PER_SIDE, {
      corridorMask: ctx.corridorMask,
    });
    group.add(cellsGroup);
    for (const [k, mesh] of meshes) subCells.get(k)!.meshes.push(mesh);
  } else {
    const terrain = buildTerrainTileMesh(dec, { corridorMask: ctx.corridorMask });
    terrain.name = `tileTerrain/${key}`;
    group.add(terrain);
  }

  // --- Buildings: one merged prism mesh per tile (or per L1 sub-cell) --------
  let buildingCount = 0;
  if (ctx.buildings && tile.bHeight.length > 0) {
    // Partition slot per sub-cell (footprint-centroid assignment) — or one
    // global slot for the un-subbed L0/L2 path.
    const slots = subbed ? SUB_CELLS_PER_SIDE * SUB_CELLS_PER_SIDE : 1;
    const prismsBySlot: BakedMesh[][] = Array.from({ length: slots }, () => []);
    for (let i = 0; i < tile.bHeight.length; i++) {
      const start = tile.bFpOffset[i];
      const end = i + 1 < tile.bFpOffset.length ? tile.bFpOffset[i + 1] : tile.bFpX.length;
      if (end - start < 3) continue; // degenerate ring
      const fp: number[][] = [];
      let cx = 0;
      let cz = 0;
      for (let k = start; k < end; k++) {
        fp.push([tile.bFpX[k], tile.bFpZ[k]]);
        cx += tile.bFpX[k];
        cz += tile.bFpZ[k];
      }
      cx /= fp.length;
      cz /= fp.length;
      if (ctx.plateRect && insideRect(cx, cz, ctx.plateRect)) {
        plateSkipped++;
        continue;
      }
      const [si, sj] = subbed ? subIdxOf(cx, cz) : [0, 0];
      prismsBySlot[subbed ? sj * SUB_CELLS_PER_SIDE + si : 0].push(
        prismParts(fp, tile.bBaseY[i], tile.bBaseY[i] + tile.bHeight[i]),
      );
    }
    for (let slot = 0; slot < slots; slot++) {
      const prisms = prismsBySlot[slot];
      if (prisms.length === 0) continue;
      // mergeTinted only calls `pick(b)` — wrap the prisms so we reuse its
      // per-building tint + merge path without inventing a parallel merger.
      const wrapped = prisms.map((prism) => ({ prism })) as unknown as BakedBuilding[];
      const geo = mergeTinted(wrapped, (b) => (b as unknown as { prism: BakedMesh }).prism, palette.creamBase);
      const mesh = new THREE.Mesh(geo, sharedBuildingMat());
      const cellK = subCellKey(slot % SUB_CELLS_PER_SIDE, Math.floor(slot / SUB_CELLS_PER_SIDE));
      mesh.name = subbed ? `tileBuildings/${key}#${cellK}` : `tileBuildings/${key}`;
      mesh.castShadow = true;
      mesh.receiveShadow = true;
      group.add(mesh);
      if (subCells) subCells.get(cellK)!.meshes.push(mesh);
      buildingCount += prisms.length;
    }
  }

  // --- Trees: filter specs only, Task 5 instantiates -------------------------
  // Uses treeExcludeRect when given (the wider samplerRect-equivalent band
  // that covers the boot nature-tree bbox) so streamed tile trees don't
  // double up with the boot-resident nature trees outside the plate proper;
  // falls back to plateRect for callers that don't distinguish the two.
  let trees: TreeSpec[] = [];
  if (ctx.trees) {
    const specs = tileTreeSpecs(tile);
    const rect = ctx.treeExcludeRect !== undefined ? ctx.treeExcludeRect : ctx.plateRect;
    if (rect) {
      trees = specs.filter((s) => {
        const skip = insideRect(s.x, s.z, rect);
        if (skip) plateSkipped++;
        return !skip;
      });
    } else {
      trees = specs;
    }
    if (subCells) {
      // Partition the surviving specs into the sub-cells; each non-empty
      // sub-cell gets its own tree-pool key so main.ts can add/remove pools
      // per L2 arrival/departure.
      for (const s of trees) {
        const [si, sj] = subIdxOf(s.x, s.z);
        subCells.get(subCellKey(si, sj))!.trees.push(s);
      }
      for (const [k, sc] of subCells) {
        if (sc.trees.length > 0) sc.treeKey = `${key}#${k}`;
      }
    }
  }

  group.userData.buildingCount = buildingCount;
  group.userData.treeCount = trees.length;
  group.userData.plateSkipped = plateSkipped;

  let disposed = false;
  const dispose = (): void => {
    if (disposed) return;
    disposed = true;
    group.traverse((o) => {
      const mesh = o as THREE.Mesh;
      if (mesh.isMesh) mesh.geometry.dispose(); // shared materials stay alive
    });
  };

  return {
    key,
    group,
    // Sub-celled tiles register their trees per sub-cell (subCells[*].treeKey)
    // — a whole-tile treeKey would double-register them.
    treeKey: !subbed && trees.length > 0 ? key : null,
    trees,
    subCells,
    dispose,
  };
}
