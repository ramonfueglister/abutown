import { countBuildingsWithoutDirectStreetAdjacency, hasDirectStreetAdjacency, hasVisibleStreetFrontage } from '../city/buildingFrontage';
import { countAdjacentParallelRoadRuns } from '../city/roadParallelCleanup';
import { countInvalidRoadDeadEnds } from '../city/roadTopology';
import type { Building, RailStation, RailTile, RoadTile, Terrain } from './backendTerrainRenderState';
import type { Coord } from '../types';

export type CityDiagnostics = {
  roadRailOverlap: number;
  designedRailCrossings: number;
  invalidBuildings: number;
  buildingsOutsideStreetFrontageSet: number;
  buildingsWithoutDirectStreetAdjacency: number;
  buildingsWithoutAnyStreetAdjacency: number;
  buildingsWithoutStreetFrontage: number;
  buildingsTouchingRail: number;
  buildingFramesOutsideFinishedRow: number;
  treeBuildingOverlap: number;
  railStationsOnRoad: number;
  railStationsOnBuildings: number;
  railStationsOnRails: number;
  railStationsOnTrees: number;
  adjacentParallelRoadRuns: number;
  invalidRoadDeadEnds: number;
  parallelRoadPairs: number;
};

export type CityDiagnosticsInput = {
  width: number;
  height: number;
  terrainAt: (coord: Coord) => Terrain;
  roads: ReadonlyMap<string, RoadTile>;
  rails: ReadonlyMap<string, RailTile>;
  railCrossings: ReadonlySet<string>;
  railReserved: ReadonlySet<string>;
  railStations: readonly RailStation[];
  buildings: readonly Building[];
  trees: readonly Coord[];
};

export function createCityDiagnostics(input: CityDiagnosticsInput): CityDiagnostics {
  let roadRailOverlap = 0;
  let designedRailCrossings = 0;
  for (const tileKey of input.roads.keys()) {
    if (!input.rails.has(tileKey)) continue;
    if (input.railCrossings.has(tileKey)) designedRailCrossings += 1;
    else roadRailOverlap += 1;
  }

  let invalidBuildings = 0;
  let buildingsOutsideStreetFrontageSet = 0;
  let buildingsWithoutAnyStreetAdjacency = 0;
  let buildingsWithoutStreetFrontage = 0;
  let buildingsTouchingRail = 0;
  let buildingFramesOutsideFinishedRow = 0;
  const streetFrontages = buildStreetFrontages(input);
  for (const building of input.buildings) {
    const tileKey = key(building.coord);
    const terrainKind = input.terrainAt(building.coord);
    if (input.roads.has(tileKey) || input.rails.has(tileKey) || !(terrainKind === 'grass' || terrainKind === 'park')) invalidBuildings += 1;
    if (!streetFrontages.has(tileKey)) buildingsOutsideStreetFrontageSet += 1;
    if (!hasDirectStreetAdjacency(building.coord, input.roads)) buildingsWithoutAnyStreetAdjacency += 1;
    if (!hasVisibleStreetFrontage(building.coord, input.roads)) buildingsWithoutStreetFrontage += 1;
    if (touchesRail(building.coord, input.railReserved)) buildingsTouchingRail += 1;
  }

  const buildingTiles = new Set(input.buildings.map((building) => key(building.coord)));
  const treeTiles = new Set(input.trees.map(key));
  let treeBuildingOverlap = 0;
  for (const tileKey of treeTiles) if (buildingTiles.has(tileKey)) treeBuildingOverlap += 1;
  let railStationsOnRoad = 0;
  let railStationsOnBuildings = 0;
  let railStationsOnRails = 0;
  let railStationsOnTrees = 0;
  for (const station of input.railStations) {
    const stationKey = key(station.coord);
    if (input.roads.has(stationKey)) railStationsOnRoad += 1;
    if (buildingTiles.has(stationKey)) railStationsOnBuildings += 1;
    if (input.rails.has(stationKey)) railStationsOnRails += 1;
    if (treeTiles.has(stationKey)) railStationsOnTrees += 1;
  }

  let parallelRoadPairs = 0;
  for (const road of input.roads.values()) {
    const mask = road.mask;
    if (isStraightEastWest(mask)) {
      const south = input.roads.get(key({ x: road.coord.x, y: road.coord.y + 1 }));
      if (south && isStraightEastWest(south.mask)) parallelRoadPairs += 1;
    }
    if (isStraightNorthSouth(mask)) {
      const east = input.roads.get(key({ x: road.coord.x + 1, y: road.coord.y }));
      if (east && isStraightNorthSouth(east.mask)) parallelRoadPairs += 1;
    }
  }

  return {
    roadRailOverlap,
    designedRailCrossings,
    invalidBuildings,
    buildingsOutsideStreetFrontageSet,
    buildingsWithoutDirectStreetAdjacency: countBuildingsWithoutDirectStreetAdjacency(input.buildings, input.roads),
    buildingsWithoutAnyStreetAdjacency,
    buildingsWithoutStreetFrontage,
    buildingsTouchingRail,
    buildingFramesOutsideFinishedRow,
    treeBuildingOverlap,
    railStationsOnRoad,
    railStationsOnBuildings,
    railStationsOnRails,
    railStationsOnTrees,
    adjacentParallelRoadRuns: countAdjacentParallelRoadRuns(input.roads),
    invalidRoadDeadEnds: countInvalidRoadDeadEnds(input.roads, { width: input.width, height: input.height }),
    parallelRoadPairs,
  };
}

function buildStreetFrontages(input: CityDiagnosticsInput): Set<string> {
  const result = new Set<string>();
  for (const road of input.roads.values()) {
    if (road.kind !== 'street') continue;
    for (const coord of cardinal(road.coord)) {
      const terrainKind = input.terrainAt(coord);
      if (isInside(coord, input.width, input.height) && (terrainKind === 'grass' || terrainKind === 'park')) result.add(key(coord));
    }
  }
  return result;
}

function touchesRail(coord: Coord, railReserved: ReadonlySet<string>): boolean {
  return [coord, ...cardinal(coord)].some((neighbor) => railReserved.has(key(neighbor)));
}

function isStraightEastWest(mask: number): boolean {
  return (mask & (2 | 8)) === (2 | 8) && (mask & (1 | 4)) === 0;
}

function isStraightNorthSouth(mask: number): boolean {
  return (mask & (1 | 4)) === (1 | 4) && (mask & (2 | 8)) === 0;
}

function cardinal(coord: Coord): Coord[] {
  return [
    { x: coord.x, y: coord.y - 1 },
    { x: coord.x + 1, y: coord.y },
    { x: coord.x, y: coord.y + 1 },
    { x: coord.x - 1, y: coord.y },
  ];
}

function isInside(coord: Coord, width: number, height: number): boolean {
  return coord.x >= 0 && coord.y >= 0 && coord.x < width && coord.y < height;
}

function key(coord: Coord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}
