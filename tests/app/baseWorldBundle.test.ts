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

    expect(manifest.world_id).toBe('abutopia');
    expect(manifest.chunk_size).toBe(32);
    expect(manifest.world_tiles).toEqual({ width: 224, height: 128 });
    expect(terrain.tiles.length).toBe(0);
    expect(transport.roads.length).toBe(10);
    expect(transport.rails.length).toBe(0);
    expect(transport.arterial_paths.length).toBe(0);
    expect(transport.rail_paths.length).toBe(0);
    expect(transport.pedestrian_corridors.map((path) => path.id)).toEqual([
      'corridor:sidewalk:north',
      'corridor:sidewalk:south',
    ]);
    expect(transport.pedestrian_corridors[0].points).toHaveLength(12);
    expect(transport.pedestrian_corridors[1].points).toHaveLength(12);
    expect(transport.pedestrian_corridors[0].points[0]).toEqual({ x: 106, y: 63.49 });
    expect(transport.pedestrian_corridors[1].points[0]).toEqual({ x: 106, y: 64.51 });
    expect(
      transport.pedestrian_corridors
        .flatMap((path) => path.points)
        .some((point) => point.y === 64),
    ).toBe(false);
    expect(buildings.footprints.length).toBe(2);
    expect(decorations.trees.length).toBe(0);
    expect(decorations.details.length).toBe(0);
  });
});
