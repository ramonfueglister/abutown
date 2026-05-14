import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';
import { buildZurichTransport } from '../../src/city/zurichTransport';
import { buildZurichPlacement } from '../../src/city/zurichPlacement';
import { validateZurichCity } from '../../src/city/zurichValidation';
import { distance, key, parseKey, type Coord } from '../../src/city/worldTypes';

const finishedRowColumns = {
  houses: 4,
  oldhouses: 4,
  cottages: 1,
  townhouses: 2,
  shops: 6,
  flats: 3,
  office: 4,
  modern: 2,
  tower: 4,
  church: 1,
};

describe('buildZurichPlacement', () => {
  it('places varied buildings, forests, and reserves without hard-rule conflicts', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(true);
    expect(validation.errors).toEqual([]);
    expect(placement.buildings.length).toBeGreaterThan(1800);
    expect(placement.trees.length).toBeGreaterThan(2600);
    expect(placement.trees.length).toBeLessThan(6500);
    expect(placement.details.length).toBeGreaterThan(120);
    expect(placement.reserveTiles.size).toBeGreaterThan(2500);
    expect(new Set(placement.buildings.map((building) => building.sheet)).size).toBeGreaterThanOrEqual(8);
  });

  it('places OpenTTD diorama setpiece details before the city filler', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const detailCounts = new Map<string, number>();

    for (const detail of placement.details) {
      detailCounts.set(detail.category, (detailCounts.get(detail.category) ?? 0) + 1);
    }

    expect(detailCounts.get('station') ?? 0).toBeGreaterThanOrEqual(24);
    expect(detailCounts.get('dock') ?? 0).toBeGreaterThanOrEqual(6);
    expect(detailCounts.get('dock') ?? 0).toBeLessThanOrEqual(10);
    expect(detailCounts.get('industry') ?? 0).toBeGreaterThanOrEqual(35);
    expect(detailCounts.get('field') ?? 0).toBeGreaterThanOrEqual(48);
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

  it('uses only finished first-row building frames to avoid mask and empty sprite tiles', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);

    for (const building of placement.buildings) {
      expect(building.frame).toBeGreaterThanOrEqual(0);
      expect(building.frame).toBeLessThan(finishedRowColumns[building.sheet]);
    }
  });

  it('keeps the river corridor open and concentrates residential buildings toward district centers', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const waterTiles = [...world.terrain.values()].filter((tile) => tile.kind === 'water').map((tile) => tile.coord);
    const nearWaterBuildings = placement.buildings.filter((building) => manhattanDistanceToNearest(building.coord, waterTiles, 2) <= 2);

    expect(nearWaterBuildings.length).toBeLessThanOrEqual(45);

    for (const zone of world.zones.filter((candidate) => candidate.kind === 'residential')) {
      const buildings = placement.buildings.filter((building) => building.zoneId === zone.id);
      const farBuildings = buildings.filter((building) => distance(building.coord, zone.center) > zone.radius * 0.72);
      expect(farBuildings.length / buildings.length).toBeLessThan(0.33);
    }
  });

  it('creates forest patches with dense pockets and irregular sparse edges', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const treeTiles = new Set(placement.trees.map(key));

    for (const zone of world.zones.filter((candidate) => candidate.kind === 'forest')) {
      const windows = forestWindowCounts(world, treeTiles, zone.center, zone.radius);
      expect(windows.filter((count) => count >= 26).length).toBeGreaterThanOrEqual(8);
      expect(windows.filter((count) => count <= 12).length).toBeGreaterThanOrEqual(8);
    }
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

function manhattanDistanceToNearest(coord: Coord, candidates: Coord[], maxDistance: number): number {
  let best = maxDistance + 1;
  for (const candidate of candidates) {
    const distance = Math.abs(coord.x - candidate.x) + Math.abs(coord.y - candidate.y);
    if (distance < best) best = distance;
    if (best <= 1) return best;
  }
  return best;
}

function forestWindowCounts(
  world: ReturnType<typeof buildZurichWorld>,
  treeTiles: ReadonlySet<string>,
  center: Coord,
  radius: number,
): number[] {
  const counts: number[] = [];
  for (let y = center.y - radius; y <= center.y + radius - 7; y += 4) {
    for (let x = center.x - radius; x <= center.x + radius - 7; x += 4) {
      let treeCount = 0;
      let forestTiles = 0;
      for (let yy = y; yy < y + 8; yy += 1) {
        for (let xx = x; xx < x + 8; xx += 1) {
          const coord = { x: xx, y: yy };
          if (distance(coord, center) > radius || world.terrain.get(key(coord))?.kind !== 'forest') continue;
          forestTiles += 1;
          if (treeTiles.has(key(coord))) treeCount += 1;
        }
      }
      if (forestTiles >= 40) counts.push(treeCount);
    }
  }
  return counts;
}
