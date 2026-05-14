import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';
import { buildZurichTransport } from '../../src/city/zurichTransport';
import { buildZurichPlacement } from '../../src/city/zurichPlacement';
import { validateZurichCity } from '../../src/city/zurichValidation';
import { key, parseKey } from '../../src/city/worldTypes';

describe('buildZurichPlacement', () => {
  it('places varied buildings, forests, and reserves without hard-rule conflicts', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(true);
    expect(validation.errors).toEqual([]);
    expect(placement.buildings.length).toBeGreaterThan(1800);
    expect(placement.trees.length).toBeGreaterThan(3200);
    expect(placement.details.length).toBeGreaterThan(120);
    expect(placement.reserveTiles.size).toBeGreaterThan(2500);
    expect(new Set(placement.buildings.map((building) => building.sheet)).size).toBeGreaterThanOrEqual(8);
  });

  it('keeps buildings off water, roads, and rails', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);

    for (const building of placement.buildings) {
      const tileKey = key(building.coord);
      expect(world.terrain.get(tileKey)?.kind).not.toBe('water');
      expect(transport.roads.has(tileKey)).toBe(false);
      expect(transport.rails.has(tileKey)).toBe(false);
    }
  });

  it('keeps trees and buildings on separate tiles', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const treeTiles = new Set(placement.trees.map(key));

    const overlaps = placement.buildings.filter((building) => treeTiles.has(key(building.coord)));

    expect(overlaps).toEqual([]);
  });
});

describe('validateZurichCity', () => {
  it('reports road and rail overlap outside rail crossings', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const overlapKey = [...transport.roads.keys()].find((roadKey) => !transport.railCrossings.has(roadKey));
    expect(overlapKey).toBeDefined();

    const road = transport.roads.get(overlapKey!);
    expect(road).toBeDefined();
    transport.rails.set(overlapKey!, { coord: road!.coord, mask: 0 });

    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(false);
    expect(validation.errors).toContain('roadRailOverlap:1');
    expect(validation.stats.roadRailOverlap).toBe(1);
  });

  it('reports bridges that are not on water or riverbank terrain', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const grassKey = [...world.terrain.entries()].find(([, tile]) => tile.kind === 'grass')?.[0];
    expect(grassKey).toBeDefined();

    transport.bridges.add(grassKey!);

    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(false);
    expect(validation.errors).toContain('bridgeErrors:1');
    expect(validation.stats.bridgeErrors).toBe(1);
  });

  it('reports invalid buildings on missing terrain and outside world bounds', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const missingTerrainKey = [...world.terrain.entries()].find(([, tile]) => tile.kind === 'grass')?.[0];
    expect(missingTerrainKey).toBeDefined();

    world.terrain.delete(missingTerrainKey!);
    placement.buildings.push(
      { coord: parseKey(missingTerrainKey!), sheet: 'houses', frame: 0, zoneId: 'test:missing-terrain' },
      { coord: { x: world.width, y: world.height }, sheet: 'houses', frame: 0, zoneId: 'test:out-of-bounds' },
    );

    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(false);
    expect(validation.errors).toContain('invalidBuildings:2');
    expect(validation.stats.invalidBuildings).toBe(2);
  });

  it('reports tree and building tile overlap', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);

    placement.trees.push(placement.buildings[0].coord);

    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(false);
    expect(validation.errors).toContain('treeBuildingOverlap:1');
    expect(validation.stats.treeBuildingOverlap).toBe(1);
  });
});
