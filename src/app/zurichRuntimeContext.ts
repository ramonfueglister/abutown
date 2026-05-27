import { type StaticRuntimeDiagnostics } from './runtimeDiagnostics';
import {
  countBuildingsWithoutDirectStreetAdjacency,
  hasDirectStreetAdjacency,
  hasVisibleStreetFrontage,
} from '../city/buildingFrontage';
import { countAdjacentParallelRoadRuns } from '../city/roadParallelCleanup';
import { countInvalidRoadDeadEnds } from '../city/roadTopology';
import { buildZurichPlacement, type ZurichPlacement } from '../city/zurichPlacement';
import { buildZurichTransport, type ZurichTransport } from '../city/zurichTransport';
import { validateZurichCity } from '../city/zurichValidation';
import { buildZurichWorld } from '../city/zurichWorld';
import {
  inside as isInsideWorld,
  key,
  type Coord,
  type ZurichBuilding,
  type ZurichDetail,
  type ZurichValidationResult,
  type ZurichWorld,
} from '../city/worldTypes';

const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;

export type RuntimeTerrain = 'grass' | 'water' | 'riverbank' | 'park';
export type RuntimeRoadKind = 'street' | 'bridge';
export type RuntimeRoadTile = { coord: Coord; kind: RuntimeRoadKind; mask: number };
export type RuntimeRailTile = { coord: Coord; mask: number };
export type RuntimeRailStation = { coord: Coord; frame: number };
export type RuntimeBuilding = { coord: Coord; sheet: ZurichBuilding['sheet']; frame: number; district: string };

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
} satisfies Record<RuntimeBuilding['sheet'], number>;

export type ZurichRuntimeContext = {
  world: ZurichWorld;
  transport: ZurichTransport;
  placement: ZurichPlacement;
  validation: ZurichValidationResult;
  runtime: {
    terrain: Map<string, RuntimeTerrain>;
    roads: Map<string, RuntimeRoadTile>;
    rails: Map<string, RuntimeRailTile>;
    railCrossings: Set<string>;
    railReserved: Set<string>;
    railPaths: Coord[][];
    railStations: RuntimeRailStation[];
    buildings: RuntimeBuilding[];
    trees: Coord[];
    details: ZurichDetail[];
  };
  staticDiagnostics: () => StaticRuntimeDiagnostics;
};

export function createZurichRuntimeContext(options: { seed: number }): ZurichRuntimeContext {
  const world = buildZurichWorld({ seed: options.seed });
  const transport = buildZurichTransport(world);
  const placement = buildZurichPlacement(world, transport);
  const validation = validateZurichCity(world, transport, placement);
  const terrain = new Map([...world.terrain].map(([tileKey, tile]) => [tileKey, toRuntimeTerrain(tile.kind)]));
  const roads = new Map<string, RuntimeRoadTile>(
    [...transport.roads].map(([tileKey, road]) => [tileKey, { coord: road.coord, kind: road.kind, mask: road.mask }]),
  );
  const rails = new Map<string, RuntimeRailTile>(
    [...transport.rails].map(([tileKey, rail]) => [tileKey, { coord: rail.coord, mask: rail.mask }]),
  );
  const railStations: RuntimeRailStation[] = [];
  const runtime = {
    terrain,
    roads,
    rails,
    railCrossings: transport.railCrossings,
    railReserved: new Set(transport.rails.keys()),
    railPaths: transport.railPaths,
    railStations,
    buildings: placement.buildings.map(toRuntimeBuilding),
    trees: placement.trees,
    details: placement.details,
  };

  return {
    world,
    transport,
    placement,
    validation,
    runtime,
    staticDiagnostics: () => buildStaticDiagnostics(world, runtime),
  };
}

export function toRuntimeTerrain(kind: string): RuntimeTerrain {
  if (kind === 'water') return 'water';
  if (kind === 'riverbank') return 'riverbank';
  if (kind === 'park' || kind === 'forest' || kind === 'reserve' || kind === 'plaza') return 'park';
  return 'grass';
}

export function toRuntimeBuilding(building: ZurichBuilding): RuntimeBuilding {
  return {
    coord: building.coord,
    sheet: building.sheet,
    frame: building.frame,
    district: building.zoneId,
  };
}

export function buildStaticDiagnostics(
  world: ZurichWorld,
  runtime: ZurichRuntimeContext['runtime'],
): StaticRuntimeDiagnostics {
  const { terrain, roads, rails, railCrossings, railReserved, railStations, buildings, trees } = runtime;
  let roadRailOverlap = 0;
  let designedRailCrossings = 0;
  for (const tileKey of roads.keys()) {
    if (!rails.has(tileKey)) continue;
    if (railCrossings.has(tileKey)) designedRailCrossings += 1;
    else roadRailOverlap += 1;
  }

  let invalidBuildings = 0;
  let buildingsOutsideStreetFrontageSet = 0;
  let buildingsWithoutAnyStreetAdjacency = 0;
  let buildingsWithoutStreetFrontage = 0;
  let buildingsTouchingRail = 0;
  const buildingFramesOutsideFinishedRow = countBuildingFramesOutsideFinishedRow(buildings);
  const streetFrontages = buildStreetFrontages(world, terrain, roads);
  for (const building of buildings) {
    const tileKey = key(building.coord);
    const terrainKind = terrain.get(tileKey);
    if (roads.has(tileKey) || rails.has(tileKey) || !(terrainKind === 'grass' || terrainKind === 'park')) invalidBuildings += 1;
    if (!streetFrontages.has(tileKey)) buildingsOutsideStreetFrontageSet += 1;
    if (!hasDirectStreetAdjacency(building.coord, roads)) buildingsWithoutAnyStreetAdjacency += 1;
    if (!hasVisibleStreetFrontage(building.coord, roads)) buildingsWithoutStreetFrontage += 1;
    if (touchesRail(building.coord, railReserved)) buildingsTouchingRail += 1;
  }

  const buildingTiles = new Set(buildings.map((building) => key(building.coord)));
  const treeTiles = new Set(trees.map(key));
  let railStationsOnRoad = 0;
  let railStationsOnBuildings = 0;
  let railStationsOnRails = 0;
  let railStationsOnTrees = 0;
  for (const station of railStations) {
    const stationKey = key(station.coord);
    if (roads.has(stationKey)) railStationsOnRoad += 1;
    if (buildingTiles.has(stationKey)) railStationsOnBuildings += 1;
    if (rails.has(stationKey)) railStationsOnRails += 1;
    if (treeTiles.has(stationKey)) railStationsOnTrees += 1;
  }

  let parallelRoadPairs = 0;
  for (const road of roads.values()) {
    const mask = road.mask;
    if (isStraightEastWest(mask)) {
      const south = roads.get(key({ x: road.coord.x, y: road.coord.y + 1 }));
      if (south && isStraightEastWest(south.mask)) parallelRoadPairs += 1;
    }
    if (isStraightNorthSouth(mask)) {
      const east = roads.get(key({ x: road.coord.x + 1, y: road.coord.y }));
      if (east && isStraightNorthSouth(east.mask)) parallelRoadPairs += 1;
    }
  }

  return {
    roadRailOverlap,
    designedRailCrossings,
    invalidBuildings,
    buildingsOutsideStreetFrontageSet,
    buildingsWithoutDirectStreetAdjacency: countBuildingsWithoutDirectStreetAdjacency(buildings, roads),
    buildingsWithoutAnyStreetAdjacency,
    buildingsWithoutStreetFrontage,
    buildingsTouchingRail,
    buildingFramesOutsideFinishedRow,
    railStationsOnRoad,
    railStationsOnBuildings,
    railStationsOnRails,
    railStationsOnTrees,
    adjacentParallelRoadRuns: countAdjacentParallelRoadRuns(roads),
    invalidRoadDeadEnds: countInvalidRoadDeadEnds(roads, { width: world.width, height: world.height }),
    parallelRoadPairs,
  };
}

function countBuildingFramesOutsideFinishedRow(buildings: readonly RuntimeBuilding[]): number {
  let count = 0;
  for (const building of buildings) {
    const limit = finishedRowColumns[building.sheet];
    if (building.frame < 0 || building.frame >= limit) count += 1;
  }
  return count;
}

function buildStreetFrontages(
  world: ZurichWorld,
  terrain: ReadonlyMap<string, RuntimeTerrain>,
  roads: ReadonlyMap<string, RuntimeRoadTile>,
): Set<string> {
  const result = new Set<string>();
  for (const road of roads.values()) {
    if (road.kind !== 'street') continue;
    for (const coord of cardinal(road.coord)) {
      if (inside(world, coord) && isBuildable(terrain, coord)) result.add(key(coord));
    }
  }
  return result;
}

function touchesRail(coord: Coord, railReserved: ReadonlySet<string>): boolean {
  return [coord, ...cardinal(coord)].some((neighbor) => railReserved.has(key(neighbor)));
}

function isBuildable(terrain: ReadonlyMap<string, RuntimeTerrain>, coord: Coord): boolean {
  const kind = terrain.get(key(coord));
  return kind === 'grass' || kind === 'park';
}

function cardinal(coord: Coord): Coord[] {
  return [
    { x: coord.x, y: coord.y - 1 },
    { x: coord.x + 1, y: coord.y },
    { x: coord.x, y: coord.y + 1 },
    { x: coord.x - 1, y: coord.y },
  ];
}

function inside(world: ZurichWorld, coord: Coord): boolean {
  return isInsideWorld(coord, world.width, world.height);
}

function isStraightEastWest(mask: number): boolean {
  return (mask & (EAST | WEST)) === (EAST | WEST) && (mask & (NORTH | SOUTH)) === 0;
}

function isStraightNorthSouth(mask: number): boolean {
  return (mask & (NORTH | SOUTH)) === (NORTH | SOUTH) && (mask & (EAST | WEST)) === 0;
}
