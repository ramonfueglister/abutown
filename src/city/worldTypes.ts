export type Coord = { x: number; y: number };

export type TerrainKind = 'grass' | 'water' | 'riverbank' | 'park' | 'forest' | 'reserve' | 'plaza';
export type ZurichTerrainKind = TerrainKind;

export type ZurichZoneKind =
  | 'river'
  | 'old-town'
  | 'rail-center'
  | 'residential'
  | 'forest'
  | 'park'
  | 'industry'
  | 'reserve'
  | 'civic'
  | 'waterfront';

export type ZurichTerrainTile = {
  coord: Coord;
  kind: ZurichTerrainKind;
  elevation: 0;
  zoneId?: string;
};

export type ZurichZone = {
  id: string;
  kind: ZurichZoneKind;
  name: string;
  center: Coord;
  radius: number;
  density: number;
};

export type ZurichWorld = {
  id: string;
  seed: number;
  width: number;
  height: number;
  chunkSize: number;
  zones: ZurichZone[];
  terrain: Map<string, ZurichTerrainTile>;
  river: Coord[];
};

export type ZurichRoadKind = 'street' | 'bridge';

export type ZurichRoadTile = {
  coord: Coord;
  kind: ZurichRoadKind;
  mask: number;
};

export type ZurichRailTile = {
  coord: Coord;
  mask: number;
};

export type BuildingSheet =
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
export type ZurichBuildingSheet = BuildingSheet;

export type ZurichBuilding = {
  coord: Coord;
  sheet: ZurichBuildingSheet;
  frame: number;
  zoneId: string;
};

export type WorldDetail = {
  coord: Coord;
  category: 'tree' | 'park' | 'civic' | 'industry' | 'decor' | 'station' | 'dock' | 'quai' | 'field' | 'yard';
  assetCategory: string;
};
export type ZurichDetail = WorldDetail;

export type ZurichValidationResult = {
  valid: boolean;
  errors: string[];
  stats: Record<string, number>;
};

export function key(coord: Coord): string {
  return `${coord.x}:${coord.y}`;
}

export function parseKey(value: string): Coord {
  const [x, y] = value.split(':').map(Number);
  return { x, y };
}

export function inside(coord: Coord, width: number, height: number): boolean {
  return coord.x >= 0 && coord.y >= 0 && coord.x < width && coord.y < height;
}

export function distance(a: Coord, b: Coord): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}
