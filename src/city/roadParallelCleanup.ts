type Coord = { x: number; y: number };

type ParallelRun = {
  direction: 'east-west' | 'north-south';
  fixed: number;
  adjacentFixed: number;
  start: number;
  end: number;
};

const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;
const MIN_RUN_LENGTH = 4;

export function countAdjacentParallelRoadRuns(roads: ReadonlyMap<string, unknown>): number {
  return findAdjacentParallelRoadRuns(roads).length;
}

export function removeAdjacentParallelRoadRuns(roads: Map<string, unknown>, protectedRoads: ReadonlySet<string>): void {
  for (let pass = 0; pass < 4; pass += 1) {
    const runs = findAdjacentParallelRoadRuns(roads);
    if (runs.length === 0) return;
    let changed = false;
    for (const run of runs) {
      const removable = chooseRemovableSide(roads, protectedRoads, run);
      if (!removable) continue;
      for (const coord of expandRunSide(roads, removable, run)) {
        const tileKey = key(coord);
        if (!protectedRoads.has(tileKey)) {
          roads.delete(tileKey);
          changed = true;
        }
      }
    }
    if (!changed) return;
  }
}

function findAdjacentParallelRoadRuns(roads: ReadonlyMap<string, unknown>): ParallelRun[] {
  const bounds = roadBounds(roads);
  if (!bounds) return [];
  return [
    ...findEastWestRuns(roads, bounds),
    ...findNorthSouthRuns(roads, bounds),
  ];
}

function findEastWestRuns(roads: ReadonlyMap<string, unknown>, bounds: ReturnType<typeof roadBounds> & object): ParallelRun[] {
  const runs: ParallelRun[] = [];
  for (let y = bounds.minY; y < bounds.maxY; y += 1) {
    let x = bounds.minX;
    while (x <= bounds.maxX) {
      while (x <= bounds.maxX && !(hasEastWestCorridor(roads, { x, y }) && hasEastWestCorridor(roads, { x, y: y + 1 }))) x += 1;
      const start = x;
      while (x <= bounds.maxX && hasEastWestCorridor(roads, { x, y }) && hasEastWestCorridor(roads, { x, y: y + 1 })) x += 1;
      if (x - start >= MIN_RUN_LENGTH) runs.push({ direction: 'east-west', fixed: y, adjacentFixed: y + 1, start, end: x - 1 });
    }
  }
  return runs;
}

function findNorthSouthRuns(roads: ReadonlyMap<string, unknown>, bounds: ReturnType<typeof roadBounds> & object): ParallelRun[] {
  const runs: ParallelRun[] = [];
  for (let x = bounds.minX; x < bounds.maxX; x += 1) {
    let y = bounds.minY;
    while (y <= bounds.maxY) {
      while (y <= bounds.maxY && !(hasNorthSouthCorridor(roads, { x, y }) && hasNorthSouthCorridor(roads, { x: x + 1, y }))) y += 1;
      const start = y;
      while (y <= bounds.maxY && hasNorthSouthCorridor(roads, { x, y }) && hasNorthSouthCorridor(roads, { x: x + 1, y })) y += 1;
      if (y - start >= MIN_RUN_LENGTH) runs.push({ direction: 'north-south', fixed: x, adjacentFixed: x + 1, start, end: y - 1 });
    }
  }
  return runs;
}

function chooseRemovableSide(
  roads: ReadonlyMap<string, unknown>,
  protectedRoads: ReadonlySet<string>,
  run: ParallelRun,
): 'first' | 'second' | undefined {
  const first = runSideCoords('first', run);
  const second = runSideCoords('second', run);
  const firstProtected = first.filter((coord) => protectedRoads.has(key(coord))).length;
  const secondProtected = second.filter((coord) => protectedRoads.has(key(coord))).length;
  if (firstProtected > 0 && secondProtected > 0) {
    if (firstProtected === first.length && secondProtected === second.length) return undefined;
    if (firstProtected < secondProtected && firstProtected < first.length) return 'first';
    if (secondProtected < firstProtected && secondProtected < second.length) return 'second';
    return undefined;
  }
  if (firstProtected > 0) return 'second';
  if (secondProtected > 0) return 'first';
  return externalBranchCount(roads, 'first', run) <= externalBranchCount(roads, 'second', run) ? 'first' : 'second';
}

function expandRunSide(roads: ReadonlyMap<string, unknown>, side: 'first' | 'second', run: ParallelRun): Coord[] {
  const coords = runSideCoords(side, run);
  const before = run.direction === 'east-west'
    ? { x: run.start - 1, y: side === 'first' ? run.fixed : run.adjacentFixed }
    : { x: side === 'first' ? run.fixed : run.adjacentFixed, y: run.start - 1 };
  const after = run.direction === 'east-west'
    ? { x: run.end + 1, y: side === 'first' ? run.fixed : run.adjacentFixed }
    : { x: side === 'first' ? run.fixed : run.adjacentFixed, y: run.end + 1 };
  return [
    ...(roads.has(key(before)) ? [before] : []),
    ...coords,
    ...(roads.has(key(after)) ? [after] : []),
  ];
}

function runSideCoords(side: 'first' | 'second', run: ParallelRun): Coord[] {
  const coords: Coord[] = [];
  const fixed = side === 'first' ? run.fixed : run.adjacentFixed;
  for (let value = run.start; value <= run.end; value += 1) {
    coords.push(run.direction === 'east-west' ? { x: value, y: fixed } : { x: fixed, y: value });
  }
  return coords;
}

function externalBranchCount(roads: ReadonlyMap<string, unknown>, side: 'first' | 'second', run: ParallelRun): number {
  let count = 0;
  for (const coord of runSideCoords(side, run)) {
    const branches = run.direction === 'east-west'
      ? [
          { x: coord.x, y: side === 'first' ? coord.y - 1 : coord.y + 1 },
        ]
      : [
          { x: side === 'first' ? coord.x - 1 : coord.x + 1, y: coord.y },
        ];
    count += branches.filter((branch) => roads.has(key(branch))).length;
  }
  return count;
}

function hasEastWestCorridor(roads: ReadonlyMap<string, unknown>, coord: Coord): boolean {
  return (roadMask(roads, coord) & (EAST | WEST)) === (EAST | WEST);
}

function hasNorthSouthCorridor(roads: ReadonlyMap<string, unknown>, coord: Coord): boolean {
  return (roadMask(roads, coord) & (NORTH | SOUTH)) === (NORTH | SOUTH);
}

function roadMask(roads: ReadonlyMap<string, unknown>, coord: Coord): number {
  return (
    (roads.has(key({ x: coord.x, y: coord.y - 1 })) ? NORTH : 0) |
    (roads.has(key({ x: coord.x + 1, y: coord.y })) ? EAST : 0) |
    (roads.has(key({ x: coord.x, y: coord.y + 1 })) ? SOUTH : 0) |
    (roads.has(key({ x: coord.x - 1, y: coord.y })) ? WEST : 0)
  );
}

function roadBounds(roads: ReadonlyMap<string, unknown>): { minX: number; maxX: number; minY: number; maxY: number } | undefined {
  const coords = [...roads.keys()].map(parseKey);
  if (coords.length === 0) return undefined;
  return {
    minX: Math.min(...coords.map((coord) => coord.x)),
    maxX: Math.max(...coords.map((coord) => coord.x)),
    minY: Math.min(...coords.map((coord) => coord.y)),
    maxY: Math.max(...coords.map((coord) => coord.y)),
  };
}

function parseKey(value: string): Coord {
  const [x, y] = value.split(':').map(Number);
  return { x, y };
}

function key(coord: Coord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}
