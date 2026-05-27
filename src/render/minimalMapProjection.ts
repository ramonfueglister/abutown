export type Coord = { x: number; y: number };

export const MINIMAL_MAP_TILE_SIZE = { width: 18, height: 18 } as const;

export function mapProject(
  coord: Coord,
  tile: { width: number; height: number } = MINIMAL_MAP_TILE_SIZE,
): Coord {
  return {
    x: coord.x * tile.width + tile.width / 2,
    y: coord.y * tile.height + tile.height / 2,
  };
}

export function mapUnproject(
  point: Coord,
  tile: { width: number; height: number } = MINIMAL_MAP_TILE_SIZE,
): Coord {
  return {
    x: point.x / tile.width - 0.5,
    y: point.y / tile.height - 0.5,
  };
}
