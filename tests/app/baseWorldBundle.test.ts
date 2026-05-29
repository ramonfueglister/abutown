import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

function loadJson<T>(path: string): T {
  return JSON.parse(readFileSync(join(process.cwd(), path), 'utf8')) as T;
}

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
      pedestrian_corridors: unknown[];
    }>(`data/worlds/abutopia/${manifest.layers.transport}`);
    const buildings = loadJson<{ footprints: unknown[] }>(
      `data/worlds/abutopia/${manifest.layers.buildings}`,
    );
    const decorations = loadJson<{ trees: unknown[]; details: unknown[] }>(
      `data/worlds/abutopia/${manifest.layers.decorations}`,
    );

    expect(manifest.world_id).toBe('abutopia');
    expect(manifest.chunk_size).toBe(32);
    expect(manifest.world_tiles).toEqual({ width: 16, height: 8 });
    expect(terrain.tiles.length).toBe(0);
    expect(transport.roads.length).toBe(10);
    expect(transport.rails.length).toBe(0);
    expect(transport.arterial_paths.length).toBe(0);
    expect(transport.rail_paths.length).toBe(0);
    expect(transport.pedestrian_corridors.length).toBe(1);
    expect(buildings.footprints.length).toBe(2);
    expect(decorations.trees.length).toBe(0);
    expect(decorations.details.length).toBe(0);
  });
});
