import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

function loadJson<T>(path: string): T {
  return JSON.parse(readFileSync(join(process.cwd(), path), 'utf8')) as T;
}

describe('generated base world bundle', () => {
  it('contains the authored Zurich layers', () => {
    const manifest = loadJson<{
      world_id: string;
      chunk_size: number;
      world_tiles: { width: number; height: number };
      layers: Record<string, string>;
    }>('data/worlds/zurich-river-city-v1/manifest.json');
    const terrain = loadJson<{ tiles: unknown[] }>(
      `data/worlds/zurich-river-city-v1/${manifest.layers.terrain}`,
    );
    const transport = loadJson<{
      roads: unknown[];
      rails: unknown[];
      arterial_paths: unknown[];
      rail_paths: unknown[];
      pedestrian_corridors: unknown[];
    }>(`data/worlds/zurich-river-city-v1/${manifest.layers.transport}`);
    const buildings = loadJson<{ footprints: unknown[] }>(
      `data/worlds/zurich-river-city-v1/${manifest.layers.buildings}`,
    );
    const decorations = loadJson<{ trees: unknown[]; details: unknown[] }>(
      `data/worlds/zurich-river-city-v1/${manifest.layers.decorations}`,
    );

    expect(manifest.world_id).toBe('zurich-river-city-v1');
    expect(manifest.chunk_size).toBe(32);
    expect(manifest.world_tiles).toEqual({ width: 256, height: 256 });
    expect(terrain.tiles.length).toBeGreaterThan(0);
    expect(transport.roads.length).toBe(3396);
    expect(transport.rails.length).toBe(256);
    expect(transport.arterial_paths.length).toBe(3);
    expect(transport.rail_paths.length).toBe(1);
    expect(transport.pedestrian_corridors.length).toBe(160);
    expect(buildings.footprints.length).toBeGreaterThanOrEqual(2268);
    expect(decorations.trees.length).toBeGreaterThan(3000);
    expect(decorations.details.length).toBeGreaterThanOrEqual(260);
  });
});
