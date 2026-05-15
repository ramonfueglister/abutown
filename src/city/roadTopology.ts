export type RoadDimensions = {
  width: number;
  height: number;
};

type Coord = { x: number; y: number };

const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;

export function countInvalidRoadDeadEnds(roads: ReadonlyMap<string, unknown>, dimensions: RoadDimensions): number {
  return [...roads.keys()]
    .map(parseKey)
    .filter((coord) => isInvalidDeadEnd(roads, dimensions, coord))
    .length;
}

export function countRoadNetworkComponents(roads: ReadonlyMap<string, unknown>): number {
  const remaining = new Set(roads.keys());
  let components = 0;

  for (const start of roads.keys()) {
    if (!remaining.has(start)) continue;
    components += 1;
    const queue = [parseKey(start)];
    remaining.delete(start);

    while (queue.length > 0) {
      const current = queue.shift()!;
      for (const neighbor of cardinal(current)) {
        const neighborKey = key(neighbor);
        if (!remaining.has(neighborKey)) continue;
        remaining.delete(neighborKey);
        queue.push(neighbor);
      }
    }
  }

  return components;
}

export function pruneInvalidRoadDeadEnds(roads: Map<string, unknown>, dimensions: RoadDimensions): void {
  for (let pass = 0; pass < roads.size; pass += 1) {
    const removable = [...roads.keys()]
      .map(parseKey)
      .filter((coord) => isInvalidDeadEnd(roads, dimensions, coord))
      .map(key);
    if (removable.length === 0) return;
    for (const tileKey of removable) roads.delete(tileKey);
  }
}

function cardinal(coord: Coord): Coord[] {
  return [
    { x: coord.x, y: coord.y - 1 },
    { x: coord.x + 1, y: coord.y },
    { x: coord.x, y: coord.y + 1 },
    { x: coord.x - 1, y: coord.y },
  ];
}

function isInvalidDeadEnd(roads: ReadonlyMap<string, unknown>, dimensions: RoadDimensions, coord: Coord): boolean {
  const mask = roadMask(roads, coord);
  const degree = bitCount(mask);
  return degree <= 1 && !isValidMapEdgeExit(coord, mask, dimensions);
}

function isValidMapEdgeExit(coord: Coord, mask: number, dimensions: RoadDimensions): boolean {
  return (
    (coord.x === 0 && mask === EAST) ||
    (coord.x === dimensions.width - 1 && mask === WEST) ||
    (coord.y === 0 && mask === SOUTH) ||
    (coord.y === dimensions.height - 1 && mask === NORTH)
  );
}

function roadMask(roads: ReadonlyMap<string, unknown>, coord: Coord): number {
  return (
    (roads.has(key({ x: coord.x, y: coord.y - 1 })) ? NORTH : 0) |
    (roads.has(key({ x: coord.x + 1, y: coord.y })) ? EAST : 0) |
    (roads.has(key({ x: coord.x, y: coord.y + 1 })) ? SOUTH : 0) |
    (roads.has(key({ x: coord.x - 1, y: coord.y })) ? WEST : 0)
  );
}

function bitCount(value: number): number {
  let count = 0;
  let remaining = value;
  while (remaining > 0) {
    count += remaining & 1;
    remaining >>= 1;
  }
  return count;
}

function parseKey(value: string): Coord {
  const [x, y] = value.split(':').map(Number);
  return { x, y };
}

function key(coord: Coord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}
