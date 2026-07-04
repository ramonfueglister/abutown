import { describe, expect, it } from 'vitest';
import { fromBinary } from '@bufbuild/protobuf';
import { WorldTileSchema } from '../../src/proto/world_pb.js';
import { assignToTiles, encodeTile, tileGridFor } from '../../scripts/geo/lib/tiles.mjs';

const boundary = [[-4000, -4000], [4000, -4000], [4000, 4000], [-4000, 4000]];
const dem = { heightAt: (x: number, z: number) => 400 + x / 1000 };

describe('tiles', () => {
  it('roots a 1km-aligned square over boundary+ring', () => {
    const g = tileGridFor(boundary, 4000);
    expect(g.size % 1000).toBe(0);
    expect(g.size).toBeGreaterThanOrEqual(16000);
  });
  it('assigns a building to its L2 cell and drops it from L0', () => {
    const g = tileGridFor(boundary, 4000);
    const b = { id: 'x', footprint: [[10, 10], [20, 10], [20, 20]], height: 9, usage: 1, baseY: 401, access: { edge: 0, offsetM: 5 } };
    const tiles = assignToTiles(g, { buildings: [b], trees: [], landuse: [], graph: { edgeA: [], edgePtOffset: [], edgePtX: [], edgePtZ: [], edgePtY: [], edgeClass: [], edgeWidth: [] } });
    const l2 = [...tiles.keys()].filter((k) => k.startsWith('L2/'));
    const l0 = tiles.get('L0/0_0');
    expect(l2.some((k) => tiles.get(k).buildings.length === 1)).toBe(true);
    expect(l0.buildings.length).toBe(0);
  });
  it('encodes a decodable tile with the right grid resolution', () => {
    const g = tileGridFor(boundary, 4000);
    const tiles = assignToTiles(g, { buildings: [], trees: [], landuse: [], graph: { edgeA: [], edgePtOffset: [], edgePtX: [], edgePtZ: [], edgePtY: [], edgeClass: [], edgeWidth: [] } });
    const [id, bucket] = [...tiles.entries()].find(([k]) => k.startsWith('L0/'))!;
    const bin = encodeTile(bucket, dem);
    const t = fromBinary(WorldTileSchema, bin);
    expect(t.gridN).toBe(21);
    expect(t.height.length).toBe(21 * 21);
    expect(id).toBe('L0/0_0');
  });
  it('is byte-deterministic', () => {
    const g = tileGridFor(boundary, 4000);
    const mk = () => {
      const tiles = assignToTiles(g, { buildings: [], trees: [], landuse: [], graph: { edgeA: [], edgePtOffset: [], edgePtX: [], edgePtZ: [], edgePtY: [], edgeClass: [], edgeWidth: [] } });
      return encodeTile(tiles.get('L0/0_0'), dem);
    };
    expect(Buffer.from(mk()).equals(Buffer.from(mk()))).toBe(true);
  });
});
