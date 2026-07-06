// src/diorama/ksw/geo/tileContent.ts
// Task 4 (M3): per-tile materialization for the streaming pyramid. Turns one
// DecodedTile into a THREE.Group holding the terrain grid mesh and (per LOD
// mode) ONE merged building-prism mesh, plus the tile's filtered TreeSpec[]
// for Task 5's instancing layer — no tree meshes are built here.
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
import { buildTerrainTileMesh } from './terrain';
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
   * footprint CENTROID and trees whose (x,z) fall inside are skipped. */
  plateRect: { x: number; z: number; w: number; d: number } | null;
  /** Applied to group.position.y (see header) — never baked into geometry. */
  groundShiftY: number;
  /** L2: buildings+trees, L1: trees only (impostors, Task 5), L0: neither. */
  buildings: boolean;
  trees: boolean;
};

export type TileContent = {
  key: TileKey;
  group: THREE.Group;
  /** Identifier Task 5 uses to (un)register this tile's tree batch; null when
   * the tile contributes no trees (trees:false or none survived the plate). */
  treeKey: string | null;
  /** Plate-filtered specs for Task 5's instancing — no meshes built here. */
  trees: TreeSpec[];
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

  // --- Terrain (always) -----------------------------------------------------
  const terrain = buildTerrainTileMesh(dec, { corridorMask: ctx.corridorMask });
  terrain.name = `tileTerrain/${key}`;
  group.add(terrain);

  // --- Buildings: one merged prism mesh per tile (L2 only) -------------------
  let buildingCount = 0;
  if (ctx.buildings && tile.bHeight.length > 0) {
    const prisms: BakedMesh[] = [];
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
      prisms.push(prismParts(fp, tile.bBaseY[i], tile.bBaseY[i] + tile.bHeight[i]));
    }
    if (prisms.length > 0) {
      // mergeTinted only calls `pick(b)` — wrap the prisms so we reuse its
      // per-building tint + merge path without inventing a parallel merger.
      const wrapped = prisms.map((prism) => ({ prism })) as unknown as BakedBuilding[];
      const geo = mergeTinted(wrapped, (b) => (b as unknown as { prism: BakedMesh }).prism, palette.creamBase);
      const mesh = new THREE.Mesh(geo, sharedBuildingMat());
      mesh.name = `tileBuildings/${key}`;
      mesh.castShadow = true;
      mesh.receiveShadow = true;
      group.add(mesh);
      buildingCount = prisms.length;
    }
  }

  // --- Trees: filter specs only, Task 5 instantiates -------------------------
  let trees: TreeSpec[] = [];
  if (ctx.trees) {
    const specs = tileTreeSpecs(tile);
    if (ctx.plateRect) {
      const rect = ctx.plateRect;
      trees = specs.filter((s) => {
        const skip = insideRect(s.x, s.z, rect);
        if (skip) plateSkipped++;
        return !skip;
      });
    } else {
      trees = specs;
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

  return { key, group, treeKey: trees.length > 0 ? key : null, trees, dispose };
}
