export const CLIENT_PROTOCOL_VERSION = 1;

export type ChunkCoordDto = {
  x: number;
  y: number;
};

export type HealthResponse = {
  service: string;
  world_id: string;
  ok: boolean;
  protocol_version: number;
};

export type WorldSummaryDto = {
  protocol_version: number;
  world_id: string;
  chunk_size: number;
  loaded_chunks: ChunkCoordDto[];
};

export type ChunkStateDto = 'asleep' | 'warm' | 'active' | 'hot';
export type TileKindDto = 'grass' | 'water' | 'road' | 'building_footprint';

export type TileMutationDto = {
  local_index: number;
  kind: TileKindDto;
  version: number;
};

export type ChunkSnapshotDto = {
  protocol_version: number;
  world_id: string;
  coord: ChunkCoordDto;
  chunk_state: ChunkStateDto;
  chunk_version: number;
  tile_count: number;
  dirty_tiles: TileMutationDto[];
};

export type ServerHelloMessage = {
  type: 'hello';
  protocol_version: number;
  world_id: string;
  chunk_size: number;
};

export type TilePulseMessage = {
  type: 'tile_pulse';
  protocol_version: number;
  world_id: string;
  tick: number;
  version: number;
  coord: ChunkCoordDto;
  local_index: number;
};

export type ServerErrorMessage = {
  type: 'error';
  protocol_version: number;
  world_id?: string;
  code: string;
  message: string;
};

export type ServerMessage = ServerHelloMessage | TilePulseMessage | ServerErrorMessage;

export function parseServerMessage(value: unknown): ServerMessage | undefined {
  if (!isRecord(value) || typeof value.type !== 'string') return undefined;
  if (value.type === 'hello' && isHello(value)) return value;
  if (value.type === 'tile_pulse' && isTilePulse(value)) return value;
  if (value.type === 'error' && isServerError(value)) return value;
  return undefined;
}

function isHello(value: Record<string, unknown>): value is ServerHelloMessage {
  return (
    value.type === 'hello' &&
    typeof value.protocol_version === 'number' &&
    typeof value.world_id === 'string' &&
    typeof value.chunk_size === 'number'
  );
}

function isTilePulse(value: Record<string, unknown>): value is TilePulseMessage {
  return (
    value.type === 'tile_pulse' &&
    typeof value.protocol_version === 'number' &&
    typeof value.world_id === 'string' &&
    typeof value.tick === 'number' &&
    typeof value.version === 'number' &&
    isChunkCoord(value.coord) &&
    typeof value.local_index === 'number'
  );
}

function isServerError(value: Record<string, unknown>): value is ServerErrorMessage {
  return (
    value.type === 'error' &&
    typeof value.protocol_version === 'number' &&
    (value.world_id === undefined || typeof value.world_id === 'string') &&
    typeof value.code === 'string' &&
    typeof value.message === 'string'
  );
}

function isChunkCoord(value: unknown): value is ChunkCoordDto {
  return isRecord(value) && typeof value.x === 'number' && typeof value.y === 'number';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
