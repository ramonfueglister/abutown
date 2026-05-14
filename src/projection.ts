import type { Coord } from './types';

export const TILE_WIDTH = 32;
export const TILE_HEIGHT = 16;

export function projectIso(coord: Coord): Coord {
  return {
    x: (coord.x - coord.y) * (TILE_WIDTH / 2),
    y: (coord.x + coord.y) * (TILE_HEIGHT / 2),
  };
}

export function coordKey(coord: Coord): string {
  return `${coord.x}:${coord.y}`;
}
