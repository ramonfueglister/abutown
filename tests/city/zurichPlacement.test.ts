import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';
import { buildZurichTransport } from '../../src/city/zurichTransport';
import { buildZurichPlacement } from '../../src/city/zurichPlacement';
import { validateZurichCity } from '../../src/city/zurichValidation';
import { distance, key, parseKey, type Coord } from '../../src/city/worldTypes';
import {
  countBuildingsWithoutDirectStreetAdjacency,
  hasVisibleStreetFrontage,
} from '../../src/city/buildingFrontage';

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
    const { world, transport, placement } = placementFixture();
    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(true);
    expect(validation.errors).toEqual([]);
    expect(placement.buildings.length).toBeGreaterThan(2250);
    expect(placement.trees.length).toBeGreaterThan(2600);
    expect(placement.trees.length).toBeLessThan(6500);
    expect(placement.details.length).toBeGreaterThanOrEqual(260);
    expect(placement.reserveTiles.size).toBeGreaterThan(2500);
    expect(new Set(placement.buildings.map((building) => building.sheet)).size).toBeGreaterThanOrEqual(8);
    expect(countBuildingsWithoutDirectStreetAdjacency(placement.buildings, transport.roads)).toBe(0);
    expect(placement.buildings.filter((building) => !hasVisibleStreetFrontage(building.coord, transport.roads))).toEqual([]);
  });

  it('places transport setpiece details before the city filler without rail roofs', () => {
    const { placement } = placementFixture();
    const detailCounts = new Map<string, number>();

    for (const detail of placement.details) {
      detailCounts.set(detail.category, (detailCounts.get(detail.category) ?? 0) + 1);
    }

    expect(detailCounts.get('station') ?? 0).toBe(0);
    expect(detailCounts.get('dock') ?? 0).toBe(0);
    expect(detailCounts.get('industry') ?? 0).toBeGreaterThanOrEqual(16);
    expect(detailCounts.get('field') ?? 0).toBeGreaterThanOrEqual(48);
    expect(placement.details.filter((detail) =>
      detail.assetCategory === 'station-roof' ||
      detail.assetCategory === 'rail-depot' ||
      detail.assetCategory === 'road-stop'
    )).toEqual([]);
  });

  it('keeps the water clear of non-bridge details', () => {
    const { world, placement } = placementFixture();
    const waterDetails = placement.details.filter((detail) =>
      ['water', 'riverbank'].includes(world.terrain.get(key(detail.coord))?.kind ?? '')
    );

    expect(waterDetails).toEqual([]);
  });

  it('keeps non-field detail assets on visible street frontage', () => {
    const { transport, placement } = placementFixture();
    const floatingDetails = placement.details.filter((detail) =>
      detail.category !== 'field' && !hasVisibleStreetFrontage(detail.coord, transport.roads)
    );

    expect(floatingDetails).toEqual([]);
  });

  it('does not build empty street-only neighborhoods in expansion reserves', () => {
    const { world, transport, placement } = placementFixture();
    const hollowReserves = world.zones
      .filter((zone) => zone.kind === 'reserve')
      .map((zone) => {
        const roadTiles = [...transport.roads.values()].filter((road) =>
          world.terrain.get(key(road.coord))?.zoneId === zone.id
        ).length;
        const buildings = placement.buildings.filter((building) => building.zoneId === zone.id).length;
        return { zoneId: zone.id, roadTiles, buildings };
      })
      .filter(({ roadTiles, buildings }) => roadTiles > 72 && buildings < 12);

    expect(hollowReserves).toEqual([]);
  });

  it('keeps buildings off water, roads, and rails', () => {
    const { world, transport, placement } = placementFixture();

    for (const building of placement.buildings) {
      const tileKey = key(building.coord);
      expect(world.terrain.get(tileKey)?.kind).not.toBe('water');
      expect(transport.roads.has(tileKey)).toBe(false);
      expect(transport.rails.has(tileKey)).toBe(false);
    }
  });

  it('keeps buildings off plaza terrain', () => {
    const { world, placement } = placementFixture();
    const plazaBuildings = placement.buildings.filter((building) =>
      world.terrain.get(key(building.coord))?.kind === 'plaza'
    );

    expect(plazaBuildings).toEqual([]);
  });

  it('keeps trees and buildings on separate tiles', () => {
    const { placement } = placementFixture();
    const treeTiles = new Set(placement.trees.map(key));

    const overlaps = placement.buildings.filter((building) => treeTiles.has(key(building.coord)));

    expect(overlaps).toEqual([]);
  });

  it('uses only finished first-row building frames to avoid mask and empty sprite tiles', () => {
    const { placement } = placementFixture();

    for (const building of placement.buildings) {
      expect(building.frame).toBeGreaterThanOrEqual(0);
      expect(building.frame).toBeLessThan(finishedRowColumns[building.sheet]);
    }
  });

  it('keeps the river corridor open and concentrates residential buildings toward district centers', () => {
    const { world, transport, placement } = placementFixture();
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
    const { world, placement } = placementFixture();
    const treeTiles = new Set(placement.trees.map(key));

    for (const zone of world.zones.filter((candidate) => candidate.kind === 'forest')) {
      const windows = forestWindowCounts(world, treeTiles, zone.center, zone.radius);
      expect(windows.filter((count) => count >= 26).length).toBeGreaterThanOrEqual(8);
      expect(windows.filter((count) => count <= 12).length).toBeGreaterThanOrEqual(8);
    }
  });
});

function placementFixture(): {
  world: ReturnType<typeof buildZurichWorld>;
  transport: ReturnType<typeof buildZurichTransport>;
  placement: ReturnType<typeof buildZurichPlacement>;
} {
  if (!cachedPlacementFixture) {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    cachedPlacementFixture = { world, transport, placement: buildZurichPlacement(world, transport) };
  }
  return cachedPlacementFixture;
}

let cachedPlacementFixture:
  | {
      world: ReturnType<typeof buildZurichWorld>;
      transport: ReturnType<typeof buildZurichTransport>;
      placement: ReturnType<typeof buildZurichPlacement>;
    }
  | undefined;

describe('validateZurichCity', () => {
  it('reports road and rail overlap outside rail crossings', () => {
    const { world, placement } = placementFixture();
    const transport = mutableTransportFixture();
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
    const { world, placement } = placementFixture();
    const transport = mutableTransportFixture();
    const grassKey = [...world.terrain.entries()].find(([, tile]) => tile.kind === 'grass')?.[0];
    expect(grassKey).toBeDefined();

    transport.bridges.add(grassKey!);

    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(false);
    expect(validation.errors).toContain('bridgeErrors:1');
    expect(validation.stats.bridgeErrors).toBe(1);
  });

  it('reports invalid buildings on missing terrain and outside world bounds', () => {
    const { transport } = placementFixture();
    const world = mutableWorldFixture();
    const placement = mutablePlacementFixture();
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
    const { world, transport } = placementFixture();
    const placement = mutablePlacementFixture();

    placement.trees.push(placement.buildings[0].coord);

    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(false);
    expect(validation.errors).toContain('treeBuildingOverlap:1');
    expect(validation.stats.treeBuildingOverlap).toBe(1);
  });
});

function mutableWorldFixture(): ReturnType<typeof buildZurichWorld> {
  const world = placementFixture().world;
  return {
    ...world,
    zones: world.zones.map((zone) => ({ ...zone, center: { ...zone.center } })),
    terrain: new Map([...world.terrain].map(([tileKey, tile]) => [tileKey, { ...tile, coord: { ...tile.coord } }])),
    river: world.river.map((coord) => ({ ...coord })),
  };
}

function mutableTransportFixture(): ReturnType<typeof buildZurichTransport> {
  const transport = placementFixture().transport;
  return {
    roads: new Map([...transport.roads].map(([tileKey, road]) => [tileKey, { ...road, coord: { ...road.coord } }])),
    rails: new Map([...transport.rails].map(([tileKey, rail]) => [tileKey, { ...rail, coord: { ...rail.coord } }])),
    bridges: new Set(transport.bridges),
    railCrossings: new Set(transport.railCrossings),
    arterialPaths: transport.arterialPaths.map((path) => path.map((coord) => ({ ...coord }))),
    railPaths: transport.railPaths.map((path) => path.map((coord) => ({ ...coord }))),
  };
}

function mutablePlacementFixture(): ReturnType<typeof buildZurichPlacement> {
  const placement = placementFixture().placement;
  return {
    buildings: placement.buildings.map((building) => ({ ...building, coord: { ...building.coord } })),
    trees: placement.trees.map((coord) => ({ ...coord })),
    details: placement.details.map((detail) => ({ ...detail, coord: { ...detail.coord } })),
    reserveTiles: new Set(placement.reserveTiles),
  };
}

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
