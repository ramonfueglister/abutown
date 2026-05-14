import { openTtdImportedMap } from './openTtdHamburg.generated';
import { key, type Coord, type ZurichBuilding, type ZurichDetail, type ZurichTerrainKind, type ZurichTerrainTile, type ZurichWorld, type ZurichZone } from './worldTypes';
import type { ZurichPlacement } from './zurichPlacement';
import type { ZurichTransport } from './zurichTransport';

type TerrainRun = [kindIndex: number, length: number];
type RoadTuple = [x: number, y: number, mask: number, bridge: number];
type RailTuple = [x: number, y: number, mask: number];
type BuildingTuple = [x: number, y: number, sheetIndex: number, frame: number];
type DetailTuple = [x: number, y: number, categoryIndex: number, assetIndex: number];
type CoordTuple = [x: number, y: number];

const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;

type ImportedMapData = {
  id: string;
  source: string;
  sourceWidth: number;
  sourceHeight: number;
  width: number;
  height: number;
  terrainKinds: ZurichTerrainKind[];
  buildingSheets: ZurichBuilding['sheet'][];
  detailCategories: ZurichDetail['category'][];
  detailAssets: string[];
  terrainRle: TerrainRun[];
  roads: RoadTuple[];
  rails: RailTuple[];
  buildings: BuildingTuple[];
  trees: CoordTuple[];
  details: DetailTuple[];
};

const imported = openTtdImportedMap as ImportedMapData;

export function buildOpenTtdImportedWorld(): ZurichWorld {
  const terrain = new Map<string, ZurichTerrainTile>();
  const zones = buildImportedZones(imported.width, imported.height);
  const river: Coord[] = [];
  let tileIndex = 0;

  for (const [kindIndex, length] of imported.terrainRle) {
    const kind = imported.terrainKinds[kindIndex] ?? 'grass';
    for (let offset = 0; offset < length; offset += 1) {
      const x = tileIndex % imported.width;
      const y = Math.floor(tileIndex / imported.width);
      const coord = { x, y };
      const zone = zoneForKind(kind);
      terrain.set(key(coord), { coord, kind, elevation: 0, zoneId: zone });
      if (kind === 'water') river.push(coord);
      tileIndex += 1;
    }
  }

  if (tileIndex !== imported.width * imported.height) {
    throw new Error(`Imported OpenTTD terrain has ${tileIndex} tiles, expected ${imported.width * imported.height}`);
  }

  return {
    id: imported.id,
    seed: 0,
    width: imported.width,
    height: imported.height,
    chunkSize: 32,
    zones,
    terrain,
    river,
  };
}

export function buildOpenTtdImportedTransport(world: ZurichWorld): ZurichTransport {
  const roads = new Map<string, ZurichTransport['roads'] extends Map<string, infer T> ? T : never>();
  const rails = new Map<string, ZurichTransport['rails'] extends Map<string, infer T> ? T : never>();
  const bridges = new Set<string>();

  for (const [x, y, mask, bridge] of imported.roads) {
    const coord = { x, y };
    const tileKey = key(coord);
    const terrain = world.terrain.get(tileKey)?.kind;
    const isBridge = bridge === 1;
    roads.set(tileKey, { coord, mask, kind: isBridge ? 'bridge' : 'street' });
    if (isBridge && (terrain === 'water' || terrain === 'riverbank')) bridges.add(tileKey);
  }

  for (const [x, y, mask] of imported.rails) {
    const coord = { x, y };
    rails.set(key(coord), { coord, mask });
  }

  return {
    roads,
    rails,
    bridges,
    railCrossings: new Set<string>(),
    arterialPaths: buildRoadCorridors(roads, world),
    railPaths: buildRailCorridors(rails, world),
  };
}

export function buildOpenTtdImportedPlacement(): ZurichPlacement {
  const buildings: ZurichBuilding[] = imported.buildings.map(([x, y, sheetIndex, frame]) => ({
    coord: { x, y },
    sheet: imported.buildingSheets[sheetIndex] ?? 'houses',
    frame,
    zoneId: 'zone:imported-city',
  }));

  const trees = imported.trees.map(([x, y]) => ({ x, y }));

  const details: ZurichDetail[] = imported.details.map(([x, y, categoryIndex, assetIndex]) => ({
    coord: { x, y },
    category: imported.detailCategories[categoryIndex] ?? 'decor',
    assetCategory: imported.detailAssets[assetIndex] ?? 'decor',
  }));

  return {
    buildings,
    trees,
    details,
    reserveTiles: new Set<string>(),
  };
}

function buildImportedZones(width: number, height: number): ZurichZone[] {
  return [
    { id: 'zone:imported-water', kind: 'river', name: 'Imported OpenTTD Water', center: { x: width / 2, y: height / 2 }, radius: width / 3, density: 0 },
    { id: 'zone:imported-city', kind: 'residential', name: 'Imported OpenTTD City', center: { x: width / 2, y: height / 2 }, radius: width / 2, density: 0.8 },
    { id: 'zone:imported-forest', kind: 'forest', name: 'Imported OpenTTD Forest', center: { x: width / 2, y: height / 2 }, radius: width / 2, density: 0.25 },
  ];
}

function zoneForKind(kind: ZurichTerrainKind): string {
  if (kind === 'water' || kind === 'riverbank') return 'zone:imported-water';
  if (kind === 'forest') return 'zone:imported-forest';
  return 'zone:imported-city';
}

function buildRoadCorridors(roads: ZurichTransport['roads'], world: ZurichWorld): Coord[][] {
  const networkCorridors = buildRoadNetworkCorridors(roads, world).slice(0, 24);
  if (networkCorridors.length > 0) return networkCorridors;

  const corridors: Coord[][] = [];
  const centerY = Math.floor(world.height / 2);
  const centerX = Math.floor(world.width / 2);
  corridors.push(...longRuns(roads, world.width, world.height, 'row', centerY));
  corridors.push(...longRuns(roads, world.width, world.height, 'column', centerX));
  return corridors.slice(0, 24);
}

function buildRoadNetworkCorridors(roads: ZurichTransport['roads'], world: ZurichWorld): Coord[][] {
  const remaining = new Set(roads.keys());
  const components: Coord[][] = [];

  for (const startKey of roads.keys()) {
    if (!remaining.has(startKey)) continue;
    const component: Coord[] = [];
    const queue = [roads.get(startKey)!.coord];
    remaining.delete(startKey);

    for (let index = 0; index < queue.length; index += 1) {
      const coord = queue[index];
      component.push(coord);
      for (const next of roadNeighbors(coord, roads)) {
        const nextKey = key(next);
        if (!remaining.has(nextKey)) continue;
        remaining.delete(nextKey);
        queue.push(next);
      }
    }

    if (component.length >= 8) components.push(component);
  }

  return components
    .sort((a, b) => b.length - a.length)
    .flatMap((component) => longestComponentPaths(component, roads, world))
    .filter((path) => path.length >= 8);
}

function longestComponentPaths(component: Coord[], roads: ZurichTransport['roads'], world: ZurichWorld): Coord[][] {
  const first = farthestRoadCoord(component[0], roads);
  const second = farthestRoadCoord(first.coord, roads);
  const path = shortestRoadPath(first.coord, second.coord, roads);
  if (path.length < 8) return [];
  return [path];
}

function farthestRoadCoord(start: Coord, roads: ZurichTransport['roads']): { coord: Coord; distance: number } {
  const seen = new Set<string>([key(start)]);
  const queue = [{ coord: start, distance: 0 }];
  let farthest = queue[0];

  for (let index = 0; index < queue.length; index += 1) {
    const current = queue[index];
    if (current.distance > farthest.distance) farthest = current;
    for (const next of roadNeighbors(current.coord, roads)) {
      const nextKey = key(next);
      if (seen.has(nextKey)) continue;
      seen.add(nextKey);
      queue.push({ coord: next, distance: current.distance + 1 });
    }
  }

  return farthest;
}

function shortestRoadPath(start: Coord, end: Coord, roads: ZurichTransport['roads']): Coord[] {
  const startKey = key(start);
  const endKey = key(end);
  const queue = [start];
  const previous = new Map<string, string | undefined>([[startKey, undefined]]);

  for (let index = 0; index < queue.length; index += 1) {
    const current = queue[index];
    if (key(current) === endKey) break;
    for (const next of roadNeighbors(current, roads)) {
      const nextKey = key(next);
      if (previous.has(nextKey)) continue;
      previous.set(nextKey, key(current));
      queue.push(next);
    }
  }

  if (!previous.has(endKey)) return [];
  const path: Coord[] = [];
  for (let cursor: string | undefined = endKey; cursor; cursor = previous.get(cursor)) {
    const [x, y] = cursor.split(':').map(Number);
    path.push({ x, y });
  }
  return path.reverse();
}

function roadNeighbors(coord: Coord, roads: ZurichTransport['roads']): Coord[] {
  const road = roads.get(key(coord));
  if (!road) return [];
  const candidates: Array<[number, Coord]> = [
    [NORTH, { x: coord.x, y: coord.y - 1 }],
    [EAST, { x: coord.x + 1, y: coord.y }],
    [SOUTH, { x: coord.x, y: coord.y + 1 }],
    [WEST, { x: coord.x - 1, y: coord.y }],
  ];

  return candidates
    .filter(([direction, next]) => (road.mask & direction) !== 0 && roads.has(key(next)))
    .map(([, next]) => next);
}

function buildRailCorridors(rails: ZurichTransport['rails'], world: ZurichWorld): Coord[][] {
  return [
    ...longRuns(rails, world.width, world.height, 'row', Math.floor(world.height / 2)),
    ...longRuns(rails, world.width, world.height, 'column', Math.floor(world.width / 2)),
  ].slice(0, 12);
}

function longRuns(points: ReadonlyMap<string, unknown>, width: number, height: number, axis: 'row' | 'column', center: number): Coord[][] {
  const runs: Coord[][] = [];
  const outer = axis === 'row' ? height : width;
  const inner = axis === 'row' ? width : height;

  for (let major = 0; major < outer; major += 1) {
    let current: Coord[] = [];
    for (let minor = 0; minor < inner; minor += 1) {
      const coord = axis === 'row' ? { x: minor, y: major } : { x: major, y: minor };
      if (points.has(key(coord))) {
        current.push(coord);
      } else {
        if (current.length >= 10) runs.push(current);
        current = [];
      }
    }
    if (current.length >= 10) runs.push(current);
  }

  return runs.sort((a, b) => {
    const ac = a[Math.floor(a.length / 2)];
    const bc = b[Math.floor(b.length / 2)];
    const aDistance = axis === 'row' ? Math.abs(ac.y - center) : Math.abs(ac.x - center);
    const bDistance = axis === 'row' ? Math.abs(bc.y - center) : Math.abs(bc.x - center);
    return b.length - a.length || aDistance - bDistance;
  });
}
