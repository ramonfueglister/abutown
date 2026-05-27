import { describe, expect, it } from 'vitest';
import { buildLayeredTerrainSeed, validateLayeredTerrainSeed } from '../../src/city/layeredTerrainSeed';
import { buildZurichPlacement } from '../../src/city/zurichPlacement';
import { buildZurichTransport } from '../../src/city/zurichTransport';
import { buildZurichWorld } from '../../src/city/zurichWorld';

describe('layered terrain seed', () => {
  it('builds one physical layered tile for every Zurich coordinate', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);

    const seed = buildLayeredTerrainSeed({ world, transport, placement });

    expect(seed.version).toBe(1);
    expect(seed.world_id).toBe('zurich-river-city-v1');
    expect(seed.width).toBe(256);
    expect(seed.height).toBe(256);
    expect(seed.chunk_size).toBe(32);
    expect(seed.tiles).toHaveLength(256 * 256);
    expect(new Set(seed.tiles.map((tile) => `${tile.x}:${tile.y}`))).toHaveLength(256 * 256);
  });

  it('separates base, surface, and cover layers', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const seed = buildLayeredTerrainSeed({ world, transport, placement });

    const bridgeTile = seed.tiles.find((tile) => tile.surface === 'Bridge');
    expect(bridgeTile).toEqual(expect.objectContaining({
      base: expect.stringMatching(/Water|Riverbank/),
      surface: 'Bridge',
      cover: 'None',
      road_mask: expect.any(Number),
    }));

    const buildingTile = seed.tiles.find((tile) => tile.cover === 'Building');
    expect(buildingTile).toEqual(expect.objectContaining({
      surface: 'None',
      display: expect.any(String),
      zone_id: expect.stringMatching(/^zone:/),
    }));
  });

  it('rejects invalid physical layer combinations', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const seed = buildLayeredTerrainSeed({ world, transport, placement });
    const invalid = {
      ...seed,
      tiles: seed.tiles.map((tile, index) =>
        index === 0 ? { ...tile, base: 'Water' as const, surface: 'Street' as const, cover: 'Building' as const } : tile,
      ),
    };

    expect(validateLayeredTerrainSeed(seed)).toEqual([]);
    expect(validateLayeredTerrainSeed(invalid)).toContain('tile:0:0:building_on_water');
    expect(validateLayeredTerrainSeed(invalid)).toContain('tile:0:0:cover_on_transport_surface');
  });
});
