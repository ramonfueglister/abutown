import { describe, expect, it } from 'vitest';
import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import { RoadGraphSchema, WorldTileSchema } from '../../src/proto/world_pb.js';
import { BROAD_FAMILIES, CONIFER_FAMILIES } from '../../src/diorama/ksw/geo/treeArchetypes';
import { FAMILY_CODES } from '../../scripts/geo/lib/style.mjs';

describe('world.proto roundtrip', () => {
  it('RoadGraph SoA survives encode/decode', () => {
    const g = create(RoadGraphSchema, {
      nodeOsmId: [1n, 2n], nodeX: [0, 10], nodeZ: [0, 0], nodeY: [450, 451],
      nodeSignal: [false, true],
      edgeA: [0], edgeB: [1], edgeClass: [4], edgeWidth: [5.5],
      edgeOneway: [0], edgeMaxspeed: [50], edgeLanes: [2],
      edgePtOffset: [0], edgePtX: [0, 10], edgePtZ: [0, 0], edgePtY: [450, 451],
    });
    const back = fromBinary(RoadGraphSchema, toBinary(RoadGraphSchema, g));
    expect(back.nodeSignal[1]).toBe(true);
    expect(back.edgePtY).toEqual([450, 451]);
  });
  it('WorldTile heightfield roundtrips', () => {
    const t = create(WorldTileSchema, {
      level: 2, x: 3, y: 4, gridN: 2, cellSize: 10,
      originX: -100, originZ: 200, height: [1, 2, 3, 4], landcover: [1, 1, 2, 2],
    });
    const back = fromBinary(WorldTileSchema, toBinary(WorldTileSchema, t));
    expect(back.height.length).toBe(4);
    expect(back.landcover[2]).toBe(2);
  });

  it('WorldTile t_family roundtrips', () => {
    const t = create(WorldTileSchema, {
      level: 2, x: 1, y: 1, gridN: 1,
      tX: [0, 1], tZ: [0, 1], tH: [10, 20], tR: [3, 4], tKind: [0, 1], tFamily: [0, 3],
    });
    const back = fromBinary(WorldTileSchema, toBinary(WorldTileSchema, t));
    expect(back.tFamily).toEqual([0, 3]);
  });

  it('FAMILY_CODES matches [...BROAD_FAMILIES, ...CONIFER_FAMILIES] order', () => {
    expect(FAMILY_CODES).toEqual([...BROAD_FAMILIES, ...CONIFER_FAMILIES]);
  });
});
