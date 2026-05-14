import { describe, expect, it } from 'vitest';
import { countAdjacentParallelRoadRuns, removeAdjacentParallelRoadRuns } from '../../src/city/roadParallelCleanup';

const road = 'street';

function key(x: number, y: number): string {
  return `${x}:${y}`;
}

function makeRoads(coords: Array<[number, number]>): Map<string, string> {
  return new Map(coords.map(([x, y]) => [key(x, y), road]));
}

describe('road parallel cleanup', () => {
  it('removes an unprotected adjacent parallel street even when it has junctions', () => {
    const roads = makeRoads([
      ...Array.from({ length: 10 }, (_, x) => [x, 10] as [number, number]),
      ...Array.from({ length: 8 }, (_, index) => [index + 1, 11] as [number, number]),
      [3, 12],
      [6, 12],
    ]);
    const protectedRoads = new Set(Array.from({ length: 10 }, (_, x) => key(x, 10)));

    expect(countAdjacentParallelRoadRuns(roads)).toBe(1);

    removeAdjacentParallelRoadRuns(roads, protectedRoads);

    expect(countAdjacentParallelRoadRuns(roads)).toBe(0);
    for (let x = 0; x < 10; x += 1) expect(roads.has(key(x, 10))).toBe(true);
    for (let x = 1; x < 9; x += 1) expect(roads.has(key(x, 11))).toBe(false);
  });

  it('collapses the less protected side when only one crossing tile is protected', () => {
    const roads = makeRoads([
      ...Array.from({ length: 10 }, (_, x) => [x, 10] as [number, number]),
      ...Array.from({ length: 10 }, (_, x) => [x, 11] as [number, number]),
      [5, 9],
      [5, 12],
    ]);
    const protectedRoads = new Set([
      ...Array.from({ length: 10 }, (_, x) => key(x, 10)),
      key(5, 11),
    ]);

    removeAdjacentParallelRoadRuns(roads, protectedRoads);

    expect(countAdjacentParallelRoadRuns(roads)).toBe(0);
    expect(roads.has(key(5, 11))).toBe(true);
    for (const x of [0, 1, 2, 3, 4, 6, 7, 8, 9]) expect(roads.has(key(x, 11))).toBe(false);
  });
});
