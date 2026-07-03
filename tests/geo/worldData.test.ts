import { describe, expect, it } from 'vitest';
import { create, toBinary } from '@bufbuild/protobuf';
import {
  RoadGraphSchema,
  WorldManifestSchema,
  WorldTileSchema,
} from '../../src/proto/world_pb.js';
import { decodeWorld } from '../../src/diorama/ksw/geo/worldData.js';

describe('decodeWorld', () => {
  it('decodes manifest + graph + tiles from in-memory fixtures', () => {
    const manifest = create(WorldManifestSchema, {
      bakeVersion: 1,
      projection: { anchorLon: 8.72, anchorLat: 47.5 },
      minX: -500,
      minZ: -500,
      size: 1000,
      tiles: [{ level: 1, x: 3, y: 2, path: 'tiles/L1/3_2.pb', byteSize: 42 }],
      boundaryRing: [0, 0, 1, 0, 1, 1, 0, 1],
      attribution: ['© OpenStreetMap contributors'],
    });
    const graph = create(RoadGraphSchema, {});
    const tile = create(WorldTileSchema, {
      level: 1,
      x: 3,
      y: 2,
      gridN: 2,
      cellSize: 10,
      originX: -100,
      originZ: 200,
      height: [1, 2, 3, 4],
      landcover: [1, 1, 2, 2],
    });

    const manifestBin = toBinary(WorldManifestSchema, manifest);
    const graphBin = toBinary(RoadGraphSchema, graph);
    const tileBin = toBinary(WorldTileSchema, tile);

    const world = decodeWorld(manifestBin, graphBin, [
      { path: 'tiles/L1/3_2.pb', bin: tileBin },
    ]);

    expect(world.manifest.bakeVersion).toBe(1);
    expect(world.graph.nodeX).toEqual([]);
    expect(world.tiles).toHaveLength(1);
    expect(world.tiles[0].level).toBe(1);
    expect(world.tiles[0].x).toBe(3);
    expect(world.tiles[0].y).toBe(2);
    expect(world.tiles[0].tile.gridN).toBe(2);
    expect(world.tiles[0].tile.height).toEqual([1, 2, 3, 4]);
  });

  it('maps tiles to manifest TileRefs by path, not array order', () => {
    const manifest = create(WorldManifestSchema, {
      tiles: [
        { level: 0, x: 0, y: 0, path: 'tiles/L0/0_0.pb', byteSize: 1 },
        { level: 1, x: 5, y: 6, path: 'tiles/L1/5_6.pb', byteSize: 2 },
      ],
    });
    const graph = create(RoadGraphSchema, {});
    const tileA = create(WorldTileSchema, { level: 0, x: 0, y: 0, gridN: 1 });
    const tileB = create(WorldTileSchema, { level: 1, x: 5, y: 6, gridN: 9 });

    // Provide the bins out of manifest order — mapping must key on path.
    const world = decodeWorld(
      toBinary(WorldManifestSchema, manifest),
      toBinary(RoadGraphSchema, graph),
      [
        { path: 'tiles/L1/5_6.pb', bin: toBinary(WorldTileSchema, tileB) },
        { path: 'tiles/L0/0_0.pb', bin: toBinary(WorldTileSchema, tileA) },
      ],
    );

    expect(world.tiles).toHaveLength(2);
    const byPath = new Map(world.tiles.map((t) => [`${t.level}_${t.x}_${t.y}`, t]));
    expect(byPath.get('0_0_0')?.tile.gridN).toBe(1);
    expect(byPath.get('1_5_6')?.tile.gridN).toBe(9);
  });
});
