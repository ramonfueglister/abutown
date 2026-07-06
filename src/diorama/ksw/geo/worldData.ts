// src/diorama/ksw/geo/worldData.ts
// Runtime loader for the municipality-wide tile pyramid baked by
// scripts/geo/bake-world.mjs (data/winterthur/world/*.pb, ~77 MB, gitignored).
//
// Deliberate break from geoData.ts's static-import pattern: the world
// artifacts are far too large to bundle. Instead this module fetches the
// protobuf artifacts over HTTP and decodes them with @bufbuild/protobuf.
// `decodeWorld` is the pure, fetch-free core (unit-tested in-memory);
// `loadWorld` wraps it with the actual network calls. Slice 1 loads every
// tile listed in the manifest; the `keep` filter on `loadWorld` is the hook
// Slice 2's tile manager will use to stream in only nearby tiles.
//
// Dev serving: `data/winterthur/world/` is gitignored and lives outside
// `public/`. To let `loadWorld`'s default baseUrl (`/winterthur-world/`)
// resolve in `vite dev`, symlink it into `public/`:
//   ln -s ../data/winterthur/world public/winterthur-world
// The symlink itself is gitignored (see .gitignore) — each dev machine that
// has run the bake creates it locally. Production/Slice-2 deployment will
// serve these tiles from a CDN or object storage instead; `loadWorld`'s
// `baseUrl` parameter exists precisely so callers can point elsewhere.
import { fromBinary } from '@bufbuild/protobuf';
import {
  RoadGraphSchema,
  WorldManifestSchema,
  WorldTileSchema,
  type RoadGraph,
  type TileRef,
  type WorldManifest,
  type WorldTile,
} from '../../../proto/world_pb.js';
import { FAMILY_CODES } from './treeArchetypes';
import type { TreeSpec } from './geoData';

export type DecodedTile = { level: number; x: number; y: number; tile: WorldTile };
export type World = { manifest: WorldManifest; graph: RoadGraph; tiles: DecodedTile[] };

/**
 * Pure decode step — no I/O. Decodes the manifest and graph, then matches
 * each provided tile binary to its `TileRef` by `path` (not array order:
 * callers/fetchers may resolve tiles out of manifest order).
 */
export function decodeWorld(
  manifestBin: Uint8Array,
  graphBin: Uint8Array,
  tileBins: { path: string; bin: Uint8Array }[],
): World {
  const manifest = fromBinary(WorldManifestSchema, manifestBin);
  const graph = fromBinary(RoadGraphSchema, graphBin);

  const refByPath = new Map<string, TileRef>();
  for (const ref of manifest.tiles) {
    refByPath.set(ref.path, ref);
  }

  const tiles: DecodedTile[] = [];
  for (const { path, bin } of tileBins) {
    const ref = refByPath.get(path);
    if (!ref) {
      throw new Error(`decodeWorld: tile path "${path}" not found in manifest.tiles`);
    }
    const tile = fromBinary(WorldTileSchema, bin);
    tiles.push({ level: ref.level, x: ref.x, y: ref.y, tile });
  }

  return { manifest, graph, tiles };
}

/**
 * Ground height (absolute DEM metres) at world-origin (0,0), read from the
 * finest-level tile whose grid covers the origin. Used to shift the terrain
 * so the anchor (hero city + KSW, which sit at y≈0) lines up with real
 * ground level: `terrainRoot.position.y = -anchorGroundHeight(world)`.
 *
 * Picks the nearest grid VERTEX to (0,0) rather than interpolating — plenty
 * precise at tile cellSize resolution for an anchor offset.
 */
export function anchorGroundHeight(world: World): number {
  let best: { levelRank: number; height: number } | null = null;

  for (const { level, tile } of world.tiles) {
    const { gridN, cellSize, originX, originZ, height } = tile;
    const maxX = originX + (gridN - 1) * cellSize;
    const maxZ = originZ + (gridN - 1) * cellSize;
    if (0 < originX || 0 > maxX || 0 < originZ || 0 > maxZ) continue;

    // nearest grid vertex to (0,0), clamped into [0, gridN-1]
    const i = Math.min(gridN - 1, Math.max(0, Math.round((0 - originX) / cellSize)));
    const j = Math.min(gridN - 1, Math.max(0, Math.round((0 - originZ) / cellSize)));
    const n = j * gridN + i;

    if (best === null || level > best.levelRank) {
      best = { levelRank: level, height: height[n] };
    }
  }

  if (best === null) {
    throw new Error('anchorGroundHeight: no tile in world.tiles covers the world origin (0,0)');
  }
  return best.height;
}

/**
 * Build a fast ground-height sampler over the finest-level (largest `level`)
 * tiles in the world. Returns `heightAt(x, z)` in **absolute DEM metres** —
 * the same values the terrain mesh renders at (see terrain.ts, which writes
 * `positions.y = height[n]`). To place an object on the *visible* (shifted)
 * terrain surface, subtract `anchorGroundHeight(world)`, since the renderer
 * offsets `terrainRoot.position.y = -anchorGroundHeight(world)`.
 *
 * Sampling is bilinear over the covering tile's grid cell, so the height
 * varies smoothly across a cell rather than snapping to vertices — cars
 * following a lane read a continuous surface. Out-of-coverage queries fall
 * back to the anchor height (flat), which only happens outside the baked
 * pyramid; the traffic net lives well inside it.
 *
 * The tiles are pre-sorted finest-first and indexed once; the returned
 * closure allocates nothing on the hot path.
 */
export function makeHeightSampler(world: World): (x: number, z: number) => number {
  // Finest (highest-level) tiles first so we prefer detail where tiles overlap.
  const tiles = [...world.tiles].sort((a, b) => b.level - a.level).map((t) => t.tile);
  const anchor = anchorGroundHeight(world);

  return function heightAt(x: number, z: number): number {
    for (const t of tiles) {
      const { gridN, cellSize, originX, originZ, height } = t;
      const fx = (x - originX) / cellSize;
      const fz = (z - originZ) / cellSize;
      if (fx < 0 || fz < 0 || fx > gridN - 1 || fz > gridN - 1) continue;
      const i0 = Math.floor(fx);
      const j0 = Math.floor(fz);
      const i1 = Math.min(i0 + 1, gridN - 1);
      const j1 = Math.min(j0 + 1, gridN - 1);
      const tx = fx - i0;
      const tz = fz - j0;
      const h00 = height[j0 * gridN + i0];
      const h10 = height[j0 * gridN + i1];
      const h01 = height[j1 * gridN + i0];
      const h11 = height[j1 * gridN + i1];
      const h0 = h00 + (h10 - h00) * tx;
      const h1 = h01 + (h11 - h01) * tx;
      return h0 + (h1 - h0) * tz;
    }
    return anchor;
  };
}

async function fetchBinary(url: string): Promise<Uint8Array> {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`worldData: failed to fetch ${url}: ${res.status} ${res.statusText}`);
  }
  return new Uint8Array(await res.arrayBuffer());
}

/**
 * Thin wrapper around the internal `fetchBinary`, exposed for the M3
 * per-tile streaming layer (`tileStreamer.ts`'s `TileStreamer.fetchTile`
 * callback), which fetches one tile at a time instead of `loadWorld`'s
 * fetch-everything-up-front behavior.
 */
export async function fetchTileBin(baseUrl: string, path: string): Promise<Uint8Array> {
  return fetchBinary(`${baseUrl}${path}`);
}

/** Decodes a single tile binary. Pure — no I/O. */
export function decodeTileBin(bin: Uint8Array): WorldTile {
  return fromBinary(WorldTileSchema, bin);
}

// Cross-tile-call warning counter for tileTreeSpecs' family/kind consistency
// check (see below) — logged once via console.warn rather than per-tree, so
// a whole-world decode doesn't spam thousands of lines.
let familyKindMismatchCount = 0;

/**
 * Decodes a WorldTile's tree SoA fields (t_x/t_z/t_h/t_r/t_kind/t_family)
 * into TreeSpec[] (Task 5's consumption format).
 *
 * `family` is left `undefined` when `t_family` is empty (older bakes, or
 * bakes predating this task — "leer = Alt-Bake" per world.proto's comment).
 * When present, family is mapped via FAMILY_CODES.
 *
 * Consistency guard: `kind` (t_kind: 0 broad, 1 conifer) is the authoritative
 * field for rendering — but family and kind are baked from the same source
 * (treeSpec in style.mjs) and should never disagree. If a family implies a
 * different kind than t_kind says (conic/slender ⇒ conifer, others ⇒ broad),
 * this derives kind FROM family (family is the more specific signal) and
 * counts the mismatch, logging a single summarized console.warn — so a
 * decode of many tiles doesn't spam one line per tree, but the anomaly is
 * still visible.
 */
export function tileTreeSpecs(tile: WorldTile): TreeSpec[] {
  const { tX, tZ, tH, tR, tKind, tFamily } = tile;
  const hasFamily = tFamily.length > 0;
  const specs: TreeSpec[] = [];
  let mismatches = 0;

  for (let i = 0; i < tX.length; i++) {
    let kind: TreeSpec['kind'] = tKind[i] === 1 ? 'conifer' : 'broad';
    let family: TreeSpec['family'] | undefined;

    if (hasFamily) {
      const code = tFamily[i];
      family = FAMILY_CODES[code];
      const familyImpliesConifer = family === 'conic' || family === 'slender';
      const kindSaysConifer = kind === 'conifer';
      if (familyImpliesConifer !== kindSaysConifer) {
        kind = familyImpliesConifer ? 'conifer' : 'broad';
        mismatches += 1;
      }
    }

    specs.push({ x: tX[i], z: tZ[i], h: tH[i], r: tR[i], kind, family });
  }

  if (mismatches > 0) {
    familyKindMismatchCount += mismatches;
    console.warn(
      `tileTreeSpecs: ${mismatches} tree(s) in this tile had t_family/t_kind mismatches; ` +
        `kind derived from family. Running total across this session: ${familyKindMismatchCount}.`,
    );
  }

  return specs;
}

/**
 * Fetches manifest.pb, graph.pb, and every tile the manifest lists (Slice 1)
 * from `baseUrl`, then decodes via `decodeWorld`.
 *
 * `keep` is the M3 streaming hook: pass a predicate to fetch only a subset
 * of `manifest.tiles` (e.g. L0 plus the fine tiles under the hero plate).
 * It receives the decoded manifest as second argument so callers can compute
 * tile bounds (minX/minZ/size quadtree root) without a separate manifest
 * fetch. Defaults to keep-all, matching Slice-1 behavior.
 */
export async function loadWorld(
  baseUrl = '/winterthur-world/',
  keep: (ref: TileRef, manifest: WorldManifest) => boolean = () => true,
): Promise<World> {
  const manifestBin = await fetchBinary(`${baseUrl}manifest.pb`);
  const manifest = fromBinary(WorldManifestSchema, manifestBin);

  const graphBin = await fetchBinary(`${baseUrl}graph.pb`);

  const refs = manifest.tiles.filter((ref) => keep(ref, manifest));
  const tileBins = await Promise.all(
    refs.map(async (ref) => ({ path: ref.path, bin: await fetchBinary(`${baseUrl}${ref.path}`) })),
  );

  return decodeWorld(manifestBin, graphBin, tileBins);
}
