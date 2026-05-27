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

    const transportTile = seed.tiles.find((tile) => tile.surface !== 'None');
    expect(transportTile).toEqual(expect.objectContaining({
      cover: 'None',
      display: null,
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

  it('rejects invalid seed shape and coordinate invariants', () => {
    const seed = buildSeed();
    const duplicate = { ...seed.tiles[1], x: seed.tiles[0].x, y: seed.tiles[0].y };
    const outOfBounds = { ...seed.tiles[2], x: seed.width, y: 0 };
    const invalid = {
      ...seed,
      chunk_size: 30,
      tiles: [seed.tiles[0], duplicate, outOfBounds, ...seed.tiles.slice(4)],
    };

    expect(validateLayeredTerrainSeed(invalid)).toEqual(expect.arrayContaining([
      `tile_count:${seed.tiles.length - 1}`,
      'chunk_size:does_not_partition_world',
      'tile:0:0:duplicate',
      `tile:${seed.width}:0:out_of_bounds`,
    ]));
  });

  it('suppresses display metadata when transport suppresses cover', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const street = [...transport.roads.values()].find((road) => road.kind === 'street');
    expect(street).toBeDefined();

    const seed = buildLayeredTerrainSeed({
      world,
      transport,
      placement: {
        ...placement,
        buildings: [
          ...placement.buildings,
          { coord: street!.coord, sheet: 'shops', frame: 0, zoneId: 'zone:test' },
        ],
      },
    });

    expect(seed.tiles.find((tile) => sameCoord(tile, street!.coord))).toEqual(expect.objectContaining({
      surface: 'Street',
      cover: 'None',
      display: null,
    }));
  });

  it('rejects invalid surface and mask symmetry', () => {
    const seed = buildSeed();
    const streetTile = seed.tiles.find((tile) => tile.surface === 'Street');
    const bridgeTile = seed.tiles.find((tile) => tile.surface === 'Bridge');
    const railTile = seed.tiles.find((tile) => tile.surface === 'Rail');
    const railCrossingTile = seed.tiles.find((tile) => tile.surface === 'RailCrossing');
    expect(streetTile).toBeDefined();
    expect(bridgeTile).toBeDefined();
    expect(railTile).toBeDefined();
    expect(railCrossingTile).toBeDefined();

    const invalid = {
      ...seed,
      tiles: seed.tiles.map((tile) => {
        if (sameCoord(tile, streetTile!)) return { ...tile, road_mask: null };
        if (sameCoord(tile, bridgeTile!)) return { ...tile, base: 'Grass' as const, road_mask: null };
        if (sameCoord(tile, railTile!)) return { ...tile, rail_mask: null, road_mask: 3 };
        if (sameCoord(tile, railCrossingTile!)) return { ...tile, road_mask: null, rail_mask: null };
        if (tile.x === 0 && tile.y === 0) return { ...tile, road_mask: 5, rail_mask: 10 };
        return tile;
      }),
    };

    expect(validateLayeredTerrainSeed(invalid)).toEqual(expect.arrayContaining([
      `tile:${streetTile!.x}:${streetTile!.y}:road_surface_without_road_mask`,
      `tile:${bridgeTile!.x}:${bridgeTile!.y}:bridge_without_water`,
      `tile:${bridgeTile!.x}:${bridgeTile!.y}:road_surface_without_road_mask`,
      `tile:${railTile!.x}:${railTile!.y}:rail_surface_without_rail_mask`,
      `tile:${railTile!.x}:${railTile!.y}:road_mask_without_road_surface`,
      `tile:${railCrossingTile!.x}:${railCrossingTile!.y}:road_surface_without_road_mask`,
      `tile:${railCrossingTile!.x}:${railCrossingTile!.y}:rail_surface_without_rail_mask`,
      'tile:0:0:road_mask_without_road_surface',
      'tile:0:0:rail_mask_without_rail_surface',
    ]));
  });
});

function buildSeed() {
  const world = buildZurichWorld({ seed: 1848 });
  const transport = buildZurichTransport(world);
  const placement = buildZurichPlacement(world, transport);
  return buildLayeredTerrainSeed({ world, transport, placement });
}

function sameCoord(a: { x: number; y: number }, b: { x: number; y: number }): boolean {
  return a.x === b.x && a.y === b.y;
}
