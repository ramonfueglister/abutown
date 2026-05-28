import type { TerrainBaseKind, TerrainState, TerrainTile } from '../backend/terrainState';
import type { Coord } from '../types';

export type Terrain = 'grass' | 'water' | 'riverbank' | 'park' | 'plaza';
export type RoadKind = 'street' | 'bridge';

export type RailTile = {
  coord: Coord;
  mask: number;
};

export type RoadTile = {
  coord: Coord;
  kind: RoadKind;
  mask: number;
};

export type BuildingSheetName =
  | 'houses'
  | 'oldhouses'
  | 'cottages'
  | 'townhouses'
  | 'shops'
  | 'flats'
  | 'office'
  | 'modern'
  | 'tower'
  | 'church';

export type Building = {
  coord: Coord;
  sheet: BuildingSheetName;
  frame: number;
  district: string;
};

export type Detail = {
  coord: Coord;
  category: string;
  assetCategory: string;
};

export type RailStation = {
  coord: Coord;
  frame: number;
};

export type BackendTerrainRenderState = {
  terrain: Map<string, Terrain>;
  roads: Map<string, RoadTile>;
  rails: Map<string, RailTile>;
  railCrossings: Set<string>;
  railReserved: Set<string>;
  railPaths: Coord[][];
  railYardPaths: Coord[][];
  railStations: RailStation[];
  buildings: Building[];
  trees: Coord[];
  details: Detail[];
};

export function createBackendTerrainRenderState(
  terrainState: TerrainState,
  options: { buildingFrameVariants?: number } = {},
): BackendTerrainRenderState {
  const buildingFrameVariants = Math.max(1, options.buildingFrameVariants ?? 4);
  const terrain = new Map<string, Terrain>();
  const roads = new Map<string, RoadTile>();
  const rails = new Map<string, RailTile>();
  const railCrossings = new Set<string>();
  const buildings: Building[] = [];
  const trees: Coord[] = [];
  const details: Detail[] = [];

  for (const [tileKey, tile] of terrainState.tiles) {
    const coord = parseKey(tileKey);
    const base = terrainFromLayer(tile.base);
    if (base !== 'grass') terrain.set(tileKey, base);
    if ((tile.surface === 'Street' || tile.surface === 'Bridge' || tile.surface === 'RailCrossing') && tile.roadMask !== null) {
      roads.set(tileKey, {
        coord,
        kind: tile.surface === 'Bridge' ? 'bridge' : 'street',
        mask: tile.roadMask,
      });
    }
    if ((tile.surface === 'Rail' || tile.surface === 'RailCrossing') && tile.railMask !== null) {
      rails.set(tileKey, { coord, mask: tile.railMask });
    }
    if (tile.surface === 'RailCrossing') railCrossings.add(tileKey);
    if (tile.cover === 'Building') buildings.push(buildingFromBackendTile(coord, tile, buildingFrameVariants));
    if (tile.cover === 'Tree') trees.push(coord);
    if (tile.cover === 'Detail') details.push(detailFromBackendTile(coord, tile));
  }

  const railReserved = new Set(rails.keys());
  return {
    terrain,
    roads,
    rails,
    railCrossings,
    railReserved,
    railPaths: buildRailPathsFromTiles(rails),
    railYardPaths: [],
    railStations: [],
    buildings,
    trees,
    details,
  };
}

function terrainFromLayer(base: TerrainBaseKind): Terrain {
  switch (base) {
    case 'Water':
      return 'water';
    case 'Riverbank':
      return 'riverbank';
    case 'Park':
    case 'Forest':
    case 'Reserve':
      return 'park';
    case 'Plaza':
      return 'plaza';
    case 'Grass':
      return 'grass';
  }
}

function buildingFromBackendTile(coord: Coord, tile: TerrainTile, buildingFrameVariants: number): Building {
  const sheet = buildingSheetFromDisplay(tile.display);
  return {
    coord,
    sheet,
    frame: hash(`backend-building:${sheet}:${key(coord)}`) % buildingFrameVariants,
    district: tile.zoneId ?? 'zone:backend',
  };
}

function buildingSheetFromDisplay(display: string | null): BuildingSheetName {
  const sheets: readonly BuildingSheetName[] = [
    'houses',
    'oldhouses',
    'cottages',
    'townhouses',
    'shops',
    'flats',
    'office',
    'modern',
    'tower',
    'church',
  ];
  return sheets.includes(display as BuildingSheetName) ? display as BuildingSheetName : 'houses';
}

function detailFromBackendTile(coord: Coord, tile: TerrainTile): Detail {
  const assetCategory = tile.display ?? 'decor';
  return {
    coord,
    assetCategory,
    category: detailCategoryFromDisplay(assetCategory),
  };
}

function detailCategoryFromDisplay(assetCategory: string): string {
  if (assetCategory === 'factory' || assetCategory === 'road-depot' || assetCategory.includes('depot')) return 'industry';
  if (assetCategory.includes('dock')) return 'dock';
  if (assetCategory.includes('station')) return 'station';
  if (assetCategory.includes('field')) return 'field';
  return 'decor';
}

function buildRailPathsFromTiles(railTiles: Map<string, RailTile>): Coord[][] {
  const unvisited = new Set(railTiles.keys());
  const paths: Coord[][] = [];
  while (unvisited.size > 0) {
    const startKey = unvisited.values().next().value as string;
    const stack = [parseKey(startKey)];
    const component: Coord[] = [];
    unvisited.delete(startKey);
    while (stack.length > 0) {
      const coord = stack.pop()!;
      component.push(coord);
      for (const neighbor of cardinal(coord)) {
        const neighborKey = key(neighbor);
        if (!unvisited.has(neighborKey)) continue;
        unvisited.delete(neighborKey);
        stack.push(neighbor);
      }
    }
    const minX = Math.min(...component.map((coord) => coord.x));
    const maxX = Math.max(...component.map((coord) => coord.x));
    const minY = Math.min(...component.map((coord) => coord.y));
    const maxY = Math.max(...component.map((coord) => coord.y));
    const vertical = maxY - minY >= maxX - minX;
    component.sort((a, b) => vertical ? a.y - b.y || a.x - b.x : a.x - b.x || a.y - b.y);
    paths.push(component);
  }
  return paths.sort((a, b) => b.length - a.length);
}

function cardinal(coord: Coord): Coord[] {
  return [
    { x: coord.x, y: coord.y - 1 },
    { x: coord.x + 1, y: coord.y },
    { x: coord.x, y: coord.y + 1 },
    { x: coord.x - 1, y: coord.y },
  ];
}

function key(coord: Coord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}

function parseKey(value: string): Coord {
  const [x, y] = value.split(':').map(Number);
  return { x, y };
}

function hash(value: string): number {
  let result = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    result ^= value.charCodeAt(i);
    result = Math.imul(result, 16777619);
  }
  return result >>> 0;
}
