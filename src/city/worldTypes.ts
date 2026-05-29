export type Coord = { x: number; y: number };

export type TerrainKind = 'grass' | 'water' | 'riverbank' | 'park' | 'forest' | 'reserve' | 'plaza';

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

export type WorldDetail = {
  coord: Coord;
  category: 'tree' | 'park' | 'civic' | 'industry' | 'decor' | 'station' | 'dock' | 'quai' | 'field' | 'yard';
  assetCategory: string;
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
