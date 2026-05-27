import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import {
  applyLayeredChunkSnapshot,
  createTerrainState,
  layeredChunkSnapshotFromProto,
  terrainTileAt,
} from '../../src/backend/terrainState';
import {
  ChunkCoordSchema,
  ChunkSnapshotSchema,
  LayeredTileSchema,
  TileBase,
  TileCover,
  TileSurface,
} from '../../src/backend/proto/abutown_pb';

describe('terrain state', () => {
  it('stores layered chunk snapshots by world coordinate', () => {
    const state = createTerrainState({ width: 256, height: 256, chunkSize: 32 });

    applyLayeredChunkSnapshot(state, {
      coord: { x: 1, y: 2 },
      tileCount: 1024,
      tiles: [
        {
          localIndex: 5,
          base: 'Grass',
          surface: 'Street',
          cover: 'None',
          display: null,
          zoneId: 'zone:test',
          roadMask: 5,
          railMask: null,
          version: 1,
        },
      ],
    });

    expect(terrainTileAt(state, { x: 37, y: 64 })).toEqual(
      expect.objectContaining({
        base: 'Grass',
        surface: 'Street',
        roadMask: 5,
      }),
    );
    expect(state.loadedChunks.has('1:2')).toBe(true);
  });

  it('converts layered chunk protobuf snapshots into terrain state input', () => {
    const proto = create(ChunkSnapshotSchema, {
      coord: create(ChunkCoordSchema, { x: 4, y: 3 }),
      tileCount: 1024,
      tiles: [
        create(LayeredTileSchema, {
          localIndex: 33,
          base: TileBase.WATER,
          surface: TileSurface.BRIDGE,
          cover: TileCover.NONE,
          roadMask: 10,
          version: 7n,
        }),
      ],
    });

    expect(layeredChunkSnapshotFromProto(proto)).toEqual({
      coord: { x: 4, y: 3 },
      tileCount: 1024,
      tiles: [
        {
          localIndex: 33,
          base: 'Water',
          surface: 'Bridge',
          cover: 'None',
          display: null,
          zoneId: null,
          roadMask: 10,
          railMask: null,
          version: 7,
        },
      ],
    });
  });
});
