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

async function fetchBinary(url: string): Promise<Uint8Array> {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`worldData: failed to fetch ${url}: ${res.status} ${res.statusText}`);
  }
  return new Uint8Array(await res.arrayBuffer());
}

/**
 * Fetches manifest.pb, graph.pb, and every tile the manifest lists (Slice 1)
 * from `baseUrl`, then decodes via `decodeWorld`.
 *
 * `keep` is the Slice-2 streaming hook: pass a predicate to fetch only a
 * subset of `manifest.tiles` (e.g. tiles near the camera). Defaults to
 * keep-all, matching Slice-1 behavior.
 */
export async function loadWorld(
  baseUrl = '/winterthur-world/',
  keep: (ref: TileRef) => boolean = () => true,
): Promise<World> {
  const manifestBin = await fetchBinary(`${baseUrl}manifest.pb`);
  const manifest = fromBinary(WorldManifestSchema, manifestBin);

  const graphBin = await fetchBinary(`${baseUrl}graph.pb`);

  const refs = manifest.tiles.filter(keep);
  const tileBins = await Promise.all(
    refs.map(async (ref) => ({ path: ref.path, bin: await fetchBinary(`${baseUrl}${ref.path}`) })),
  );

  return decodeWorld(manifestBin, graphBin, tileBins);
}
