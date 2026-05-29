import type { Coord } from '../cameraController';

export const NORTH = 1;
export const EAST = 2;
export const SOUTH = 4;
export const WEST = 8;

export type MapBounds = {
  width: number;
  height: number;
};

export type TileSize = {
  width: number;
  height: number;
};

export type OutwardExit = {
  dx: number;
  dy: number;
  mask: number;
};

export function coordKey(coord: Coord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}

export function stableHash(value: string): number {
  let result = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    result ^= value.charCodeAt(i);
    result = Math.imul(result, 16777619);
  }
  return result >>> 0;
}

export function isInsideMap(coord: Coord, map: MapBounds): boolean {
  return coord.x >= 0 && coord.y >= 0 && coord.x < map.width && coord.y < map.height;
}

export function distanceOutsideMap(coord: Coord, map: MapBounds): number {
  return Math.max(0, -coord.x, coord.x - (map.width - 1), -coord.y, coord.y - (map.height - 1));
}

export function maskSegments(mask: number, tileSize: TileSize): Coord[] {
  const result: Coord[] = [];
  if ((mask & NORTH) !== 0) result.push({ x: 0, y: -tileSize.height / 2 });
  if ((mask & EAST) !== 0) result.push({ x: tileSize.width / 2, y: 0 });
  if ((mask & SOUTH) !== 0) result.push({ x: 0, y: tileSize.height / 2 });
  if ((mask & WEST) !== 0) result.push({ x: -tileSize.width / 2, y: 0 });
  return result;
}

export function outwardExits(coord: Coord, mask: number, map: MapBounds): OutwardExit[] {
  const exits: OutwardExit[] = [];
  if (coord.y === 0 && (mask & NORTH) !== 0) exits.push({ dx: 0, dy: -1, mask: NORTH | SOUTH });
  if (coord.x === map.width - 1 && (mask & EAST) !== 0) exits.push({ dx: 1, dy: 0, mask: EAST | WEST });
  if (coord.y === map.height - 1 && (mask & SOUTH) !== 0) exits.push({ dx: 0, dy: 1, mask: NORTH | SOUTH });
  if (coord.x === 0 && (mask & WEST) !== 0) exits.push({ dx: -1, dy: 0, mask: EAST | WEST });
  return exits;
}

export function movementAngle(currentPoint: Coord, nextPoint: Coord): number {
  const dx = nextPoint.x - currentPoint.x;
  const dy = nextPoint.y - currentPoint.y;
  if (Math.abs(dx) + Math.abs(dy) < 0.001) return 0;
  return Math.atan2(dy, dx);
}

export function screenForwardOffset(from: Coord, to: Coord, distance: number): Coord {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const length = Math.hypot(dx, dy);
  if (length === 0 || distance === 0) return { x: 0, y: 0 };
  return {
    x: (dx / length) * distance,
    y: (dy / length) * distance,
  };
}
