import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

function loadJson<T>(path: string): T {
  return JSON.parse(readFileSync(join(process.cwd(), path), 'utf8')) as T;
}

type TransportPath = {
  id: string;
  points: { x: number; y: number }[];
};

describe('generated base world bundle', () => {
  it('contains the authored abutopia layers', () => {
    const manifest = loadJson<{
      schema_version: number;
      world_id: string;
      chunk_size: number;
      world_tiles: { width: number; height: number };
      layers: Record<string, string>;
    }>('data/worlds/abutopia/manifest.json');
    const terrain = loadJson<{ tiles: unknown[] }>(
      `data/worlds/abutopia/${manifest.layers.terrain}`,
    );
    const transport = loadJson<{
      roads: unknown[];
      rails: unknown[];
      arterial_paths: unknown[];
      rail_paths: unknown[];
      pedestrian_corridors: TransportPath[];
    }>(`data/worlds/abutopia/${manifest.layers.transport}`);
    const buildings = loadJson<{ footprints: unknown[] }>(
      `data/worlds/abutopia/${manifest.layers.buildings}`,
    );
    const decorations = loadJson<{ trees: unknown[]; details: unknown[] }>(
      `data/worlds/abutopia/${manifest.layers.decorations}`,
    );
    const markets = loadJson<{
      markets: { id: number; name: string; anchor: [number, number] }[];
      distances: { from: number; to: number }[];
    }>(
      `data/worlds/abutopia/${manifest.layers.markets}`,
    );

    expect(manifest.schema_version).toBe(4);
    expect(manifest.world_id).toBe('abutopia');
    expect(manifest.chunk_size).toBe(32);
    expect(manifest.world_tiles).toEqual({ width: 80, height: 48 });
    expect(terrain.tiles.length).toBe(0);
    expect(transport.roads.length).toBe(52);
    expect(transport.rails.length).toBe(0);
    expect(transport.arterial_paths.length).toBe(0);
    expect(transport.rail_paths.length).toBe(0);
    expect(transport.pedestrian_corridors.map((path) => path.id)).toEqual([
      'corridor:edge:north',
      'corridor:edge:east',
      'corridor:edge:south',
      'corridor:edge:west',
    ]);
    expect(transport.pedestrian_corridors[0].points[0]).toEqual({ x: 8, y: 8 });
    expect(transport.pedestrian_corridors[0].points.at(-1)).toEqual({ x: 72, y: 8 });
    expect(transport.pedestrian_corridors[1].points[0]).toEqual({ x: 72, y: 8 });
    expect(transport.pedestrian_corridors[1].points.at(-1)).toEqual({ x: 72, y: 40 });
    expect(transport.pedestrian_corridors[2].points[0]).toEqual({ x: 8, y: 40 });
    expect(transport.pedestrian_corridors[2].points.at(-1)).toEqual({ x: 72, y: 40 });
    expect(transport.pedestrian_corridors[3].points[0]).toEqual({ x: 8, y: 8 });
    expect(transport.pedestrian_corridors[3].points.at(-1)).toEqual({ x: 8, y: 40 });
    expect(
      transport.pedestrian_corridors
        .flatMap((path) => path.points)
        .every((point) => point.x === 8 || point.x === 72 || point.y === 8 || point.y === 40),
    ).toBe(true);
    expect(buildings.footprints.length).toBe(10);
    expect(JSON.stringify(buildings)).not.toContain(['old', 'houses'].join(''));
    expect(decorations.trees.length).toBe(0);
    expect(decorations.details.length).toBe(0);
    expect(markets.markets.map((market) => [market.id, market.name])).toEqual([
      [9001, 'Central Works'],
      [9002, 'Market Square'],
      [9003, 'Harbor Depot'],
      [9004, 'Homes Quarter'],
    ]);
    expect(markets.markets.map((market) => [market.id, market.anchor])).toEqual([
      [9001, [8, 8]],
      [9002, [72, 8]],
      [9003, [8, 40]],
      [9004, [72, 40]],
    ]);
    for (const market of markets.markets) {
      expect([8, 72]).toContain(market.anchor[0]);
      expect([8, 40]).toContain(market.anchor[1]);
    }
    expect(markets.distances.map((distance) => [distance.from, distance.to])).toEqual([
      [9001, 9002],
      [9003, 9004],
      [9001, 9003],
    ]);
  });
});
