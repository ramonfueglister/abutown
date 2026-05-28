import { create, toBinary } from '@bufbuild/protobuf';
import { describe, expect, it, vi } from 'vitest';
import { loadBackendTerrainState } from '../../src/backend/backendTerrain';
import {
  ChunkCoordSchema,
  ChunkSnapshotSchema,
  LayeredTileSchema,
  TileBase,
  TileCover,
  TileSurface,
  WorldSummarySchema,
} from '../../src/backend/proto/abutown_pb';
import { terrainTileAt } from '../../src/backend/terrainState';

function protobufResponse(bytes: Uint8Array, status = 200): Response {
  return new Response(bytes, {
    status,
    headers: { 'content-type': 'application/x-protobuf' },
  });
}

function worldSummaryResponse(): Response {
  return protobufResponse(
    toBinary(
      WorldSummarySchema,
      create(WorldSummarySchema, {
        protocolVersion: 1,
        worldId: 'abutown-main',
        chunkSize: 32,
        loadedChunks: [
          create(ChunkCoordSchema, { x: 0, y: 0 }),
          create(ChunkCoordSchema, { x: 1, y: 0 }),
        ],
        tickPeriodMs: 100,
      }),
    ),
  );
}

function chunkResponse(input: {
  x: number;
  y: number;
  localIndex: number;
  base: TileBase;
  surface: TileSurface;
  roadMask: number;
}): Response {
  return protobufResponse(
    toBinary(
      ChunkSnapshotSchema,
      create(ChunkSnapshotSchema, {
        protocolVersion: 1,
        worldId: 'abutown-main',
        coord: create(ChunkCoordSchema, { x: input.x, y: input.y }),
        tileCount: 1024,
        tiles: [
          create(LayeredTileSchema, {
            localIndex: input.localIndex,
            base: input.base,
            surface: input.surface,
            cover: TileCover.NONE,
            roadMask: input.roadMask,
            version: 1n,
          }),
        ],
      }),
    ),
  );
}

describe('backend terrain loader', () => {
  it('loads the world summary and applies chunk snapshots into terrain state', async () => {
    const responses = new Map<string, Response>([
      ['http://backend.test/world', worldSummaryResponse()],
      [
        'http://backend.test/chunks/0/0',
        chunkResponse({
          x: 0,
          y: 0,
          localIndex: 1,
          base: TileBase.GRASS,
          surface: TileSurface.STREET,
          roadMask: 5,
        }),
      ],
      [
        'http://backend.test/chunks/1/0',
        chunkResponse({
          x: 1,
          y: 0,
          localIndex: 0,
          base: TileBase.WATER,
          surface: TileSurface.BRIDGE,
          roadMask: 10,
        }),
      ],
    ]);
    const requestedUrls: string[] = [];
    const fetchImpl = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      requestedUrls.push(url);
      const response = responses.get(url);
      if (!response) throw new Error(`unexpected request ${url}`);
      return response.clone();
    }) as unknown as typeof fetch;

    const result = await loadBackendTerrainState({ baseUrl: 'http://backend.test', fetchImpl });

    expect(requestedUrls).toEqual([
      'http://backend.test/world',
      'http://backend.test/chunks/0/0',
      'http://backend.test/chunks/1/0',
    ]);
    expect(result.width).toBe(64);
    expect(result.height).toBe(32);
    expect(result.state.width).toBe(64);
    expect(result.state.height).toBe(32);
    expect(result.state.loadedChunks.has('0:0')).toBe(true);
    expect(result.state.loadedChunks.has('1:0')).toBe(true);
    expect(terrainTileAt(result.state, { x: 1, y: 0 })).toEqual(
      expect.objectContaining({ surface: 'Street', roadMask: 5 }),
    );
    expect(terrainTileAt(result.state, { x: 32, y: 0 })).toEqual(
      expect.objectContaining({ base: 'Water', surface: 'Bridge', roadMask: 10 }),
    );
  });

  it('reports chunk HTTP failures with the requested coordinate', async () => {
    const fetchImpl = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith('/world')) return worldSummaryResponse();
      return new Response(new Uint8Array(), { status: 503 });
    }) as unknown as typeof fetch;

    await expect(loadBackendTerrainState({ baseUrl: 'http://backend.test', fetchImpl })).rejects.toThrow(
      'Chunk 0:0 HTTP 503',
    );
  });
});
