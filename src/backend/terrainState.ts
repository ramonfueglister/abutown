import type { ChunkSnapshot as ChunkSnapshotProto } from './proto/abutown_pb';
import { TileBase, TileCover, TileSurface } from './proto/abutown_pb';

export type TerrainCoord = { x: number; y: number };

export type TerrainBaseKind = 'Grass' | 'Water' | 'Riverbank' | 'Forest' | 'Park' | 'Reserve' | 'Plaza';
export type TerrainSurfaceKind = 'None' | 'Street' | 'Bridge' | 'Rail' | 'RailCrossing';
export type TerrainCoverKind = 'None' | 'Building' | 'Tree' | 'Detail';

export type TerrainTile = {
  base: TerrainBaseKind;
  surface: TerrainSurfaceKind;
  cover: TerrainCoverKind;
  display: string | null;
  zoneId: string | null;
  roadMask: number | null;
  railMask: number | null;
  version: number;
};

export type TerrainState = {
  width: number;
  height: number;
  chunkSize: number;
  tiles: Map<string, TerrainTile>;
  loadedChunks: Set<string>;
};

export type LayeredChunkTile = TerrainTile & { localIndex: number };

export type LayeredChunkSnapshotLike = {
  coord: TerrainCoord;
  tileCount: number;
  tiles: LayeredChunkTile[];
};

export function createTerrainState(input: { width: number; height: number; chunkSize: number }): TerrainState {
  return {
    width: input.width,
    height: input.height,
    chunkSize: input.chunkSize,
    tiles: new Map(),
    loadedChunks: new Set(),
  };
}

export function applyLayeredChunkSnapshot(state: TerrainState, snapshot: LayeredChunkSnapshotLike): void {
  state.loadedChunks.add(chunkKey(snapshot.coord));

  for (const tile of snapshot.tiles) {
    if (!Number.isInteger(tile.localIndex) || tile.localIndex < 0 || tile.localIndex >= snapshot.tileCount) continue;
    const localX = tile.localIndex % state.chunkSize;
    const localY = Math.floor(tile.localIndex / state.chunkSize);
    const x = snapshot.coord.x * state.chunkSize + localX;
    const y = snapshot.coord.y * state.chunkSize + localY;
    if (x < 0 || y < 0 || x >= state.width || y >= state.height) continue;
    state.tiles.set(terrainKey({ x, y }), {
      base: tile.base,
      surface: tile.surface,
      cover: tile.cover,
      display: tile.display,
      zoneId: tile.zoneId,
      roadMask: tile.roadMask,
      railMask: tile.railMask,
      version: tile.version,
    });
  }
}

export function terrainTileAt(state: TerrainState, coord: TerrainCoord): TerrainTile | undefined {
  return state.tiles.get(terrainKey({ x: Math.floor(coord.x), y: Math.floor(coord.y) }));
}

export function layeredChunkSnapshotFromProto(snapshot: ChunkSnapshotProto): LayeredChunkSnapshotLike {
  return {
    coord: { x: snapshot.coord?.x ?? 0, y: snapshot.coord?.y ?? 0 },
    tileCount: snapshot.tileCount,
    tiles: snapshot.tiles.map((tile) => ({
      localIndex: tile.localIndex,
      base: tileBaseFromProto(tile.base),
      surface: tileSurfaceFromProto(tile.surface),
      cover: tileCoverFromProto(tile.cover),
      display: tile.display ?? null,
      zoneId: tile.zoneId ?? null,
      roadMask: tile.roadMask ?? null,
      railMask: tile.railMask ?? null,
      version: Number(tile.version),
    })),
  };
}

export function terrainKey(coord: TerrainCoord): string {
  return `${coord.x}:${coord.y}`;
}

export function chunkKey(coord: TerrainCoord): string {
  return `${coord.x}:${coord.y}`;
}

function tileBaseFromProto(value: TileBase): TerrainBaseKind {
  switch (value) {
    case TileBase.WATER:
      return 'Water';
    case TileBase.RIVERBANK:
      return 'Riverbank';
    case TileBase.FOREST:
      return 'Forest';
    case TileBase.PARK:
      return 'Park';
    case TileBase.RESERVE:
      return 'Reserve';
    case TileBase.PLAZA:
      return 'Plaza';
    case TileBase.GRASS:
    case TileBase.UNSPECIFIED:
      return 'Grass';
  }
}

function tileSurfaceFromProto(value: TileSurface): TerrainSurfaceKind {
  switch (value) {
    case TileSurface.STREET:
      return 'Street';
    case TileSurface.BRIDGE:
      return 'Bridge';
    case TileSurface.RAIL:
      return 'Rail';
    case TileSurface.RAIL_CROSSING:
      return 'RailCrossing';
    case TileSurface.NONE:
    case TileSurface.UNSPECIFIED:
      return 'None';
  }
}

function tileCoverFromProto(value: TileCover): TerrainCoverKind {
  switch (value) {
    case TileCover.BUILDING:
      return 'Building';
    case TileCover.TREE:
      return 'Tree';
    case TileCover.DETAIL:
      return 'Detail';
    case TileCover.NONE:
    case TileCover.UNSPECIFIED:
      return 'None';
  }
}
