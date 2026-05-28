import { fromBinary } from '@bufbuild/protobuf';
import { resolveBackendBaseUrl } from './backendGate';
import { isWorldSummaryDto, worldSummaryFromProto } from './mobilityProtocol';
import { ChunkSnapshotSchema, WorldSummarySchema } from './proto/abutown_pb';
import {
  applyLayeredChunkSnapshot,
  createTerrainState,
  layeredChunkSnapshotFromProto,
  type TerrainCoord,
  type TerrainState,
} from './terrainState';

export type BackendTerrainOptions = {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
};

export type BackendTerrainResult = {
  state: TerrainState;
  width: number;
  height: number;
};

export async function loadBackendTerrainState(options: BackendTerrainOptions = {}): Promise<BackendTerrainResult> {
  const baseUrl = options.baseUrl ?? resolveBackendBaseUrl();
  const fetchImpl = resolveFetch(options);
  if (!fetchImpl) throw new Error('Backend terrain fetch transport unavailable');

  const worldSummary = await requestBackendWorldSummary(baseUrl, fetchImpl);
  const maxChunkX = Math.max(...worldSummary.loaded_chunks.map((coord) => coord.x), 0);
  const maxChunkY = Math.max(...worldSummary.loaded_chunks.map((coord) => coord.y), 0);
  const width = (maxChunkX + 1) * worldSummary.chunk_size;
  const height = (maxChunkY + 1) * worldSummary.chunk_size;
  const state = createTerrainState({ width, height, chunkSize: worldSummary.chunk_size });

  const snapshots = await Promise.all(
    worldSummary.loaded_chunks.map((coord) => requestBackendChunkSnapshot(baseUrl, coord, fetchImpl)),
  );
  for (const snapshot of snapshots) applyLayeredChunkSnapshot(state, snapshot);

  return { state, width, height };
}

async function requestBackendWorldSummary(baseUrl: string, fetchImpl: typeof fetch) {
  const response = await fetchImpl(new URL('/world', baseUrl).toString());
  if (!response.ok) throw new Error(`World summary HTTP ${response.status}`);
  const payload = worldSummaryFromProto(fromBinary(WorldSummarySchema, new Uint8Array(await response.arrayBuffer())));
  if (!isWorldSummaryDto(payload)) throw new Error('Invalid world summary payload');
  return payload;
}

async function requestBackendChunkSnapshot(baseUrl: string, coord: TerrainCoord, fetchImpl: typeof fetch) {
  const response = await fetchImpl(new URL(`/chunks/${coord.x}/${coord.y}`, baseUrl).toString());
  if (!response.ok) throw new Error(`Chunk ${coord.x}:${coord.y} HTTP ${response.status}`);
  return layeredChunkSnapshotFromProto(fromBinary(ChunkSnapshotSchema, new Uint8Array(await response.arrayBuffer())));
}

function resolveFetch(options: { fetchImpl?: typeof fetch }): typeof fetch | undefined {
  return hasOption(options, 'fetchImpl') ? options.fetchImpl : globalThis.fetch?.bind(globalThis);
}

function hasOption<T extends object, K extends PropertyKey>(value: T, key: K): value is T & Record<K, unknown> {
  return Object.prototype.hasOwnProperty.call(value, key);
}
