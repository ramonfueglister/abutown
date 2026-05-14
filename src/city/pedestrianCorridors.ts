import { key, type Coord } from './worldTypes';

const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;

export type PedestrianCorridorRoad = {
  coord: Coord;
  mask: number;
};

export type PedestrianCorridorOptions = {
  minLength?: number;
  maxCorridors?: number;
};

export function buildPedestrianCorridors(
  roads: ReadonlyMap<string, PedestrianCorridorRoad>,
  options: PedestrianCorridorOptions = {},
): Coord[][] {
  const minLength = options.minLength ?? 5;
  const maxCorridors = options.maxCorridors ?? 240;
  const corridors = [
    ...axisCorridors(roads, 'horizontal', minLength),
    ...axisCorridors(roads, 'vertical', minLength),
  ];

  return corridors
    .sort((a, b) => b.length - a.length || a[0].y - b[0].y || a[0].x - b[0].x)
    .slice(0, maxCorridors);
}

export function makeNonTeleportingPedestrianLoop(path: readonly Coord[]): Coord[] {
  if (path.length <= 2 || areAdjacent(path[0], path[path.length - 1])) return path.map(copyCoord);
  return [
    ...path.map(copyCoord),
    ...path.slice(1, -1).reverse().map(copyCoord),
  ];
}

function axisCorridors(
  roads: ReadonlyMap<string, PedestrianCorridorRoad>,
  axis: 'horizontal' | 'vertical',
  minLength: number,
): Coord[][] {
  const corridors: Coord[][] = [];
  const sortedRoads = [...roads.values()].sort((a, b) => a.coord.y - b.coord.y || a.coord.x - b.coord.x);

  for (const road of sortedRoads) {
    if (!hasAxisConnection(road.mask, axis)) continue;
    if (canTravel(roads, previousCoord(road.coord, axis), road.coord)) continue;

    const path = [road.coord];
    let current = road.coord;
    while (canTravel(roads, current, nextCoord(current, axis))) {
      current = nextCoord(current, axis);
      path.push(current);
    }
    if (path.length >= minLength) corridors.push(path);
  }

  return corridors;
}

function hasAxisConnection(mask: number, axis: 'horizontal' | 'vertical'): boolean {
  return axis === 'horizontal' ? (mask & (EAST | WEST)) !== 0 : (mask & (NORTH | SOUTH)) !== 0;
}

function canTravel(roads: ReadonlyMap<string, PedestrianCorridorRoad>, from: Coord, to: Coord): boolean {
  const fromRoad = roads.get(key(from));
  const toRoad = roads.get(key(to));
  if (!fromRoad || !toRoad) return false;

  if (to.x === from.x + 1 && to.y === from.y) return (fromRoad.mask & EAST) !== 0 && (toRoad.mask & WEST) !== 0;
  if (to.x === from.x - 1 && to.y === from.y) return (fromRoad.mask & WEST) !== 0 && (toRoad.mask & EAST) !== 0;
  if (to.y === from.y + 1 && to.x === from.x) return (fromRoad.mask & SOUTH) !== 0 && (toRoad.mask & NORTH) !== 0;
  if (to.y === from.y - 1 && to.x === from.x) return (fromRoad.mask & NORTH) !== 0 && (toRoad.mask & SOUTH) !== 0;
  return false;
}

function previousCoord(coord: Coord, axis: 'horizontal' | 'vertical'): Coord {
  return axis === 'horizontal' ? { x: coord.x - 1, y: coord.y } : { x: coord.x, y: coord.y - 1 };
}

function nextCoord(coord: Coord, axis: 'horizontal' | 'vertical'): Coord {
  return axis === 'horizontal' ? { x: coord.x + 1, y: coord.y } : { x: coord.x, y: coord.y + 1 };
}

function areAdjacent(a: Coord, b: Coord): boolean {
  return Math.abs(a.x - b.x) + Math.abs(a.y - b.y) <= 1;
}

function copyCoord(coord: Coord): Coord {
  return { x: coord.x, y: coord.y };
}
