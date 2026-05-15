import { describe, expect, it } from 'vitest';
import { countInvalidRoadDeadEnds, countRoadNetworkComponents, pruneInvalidRoadDeadEnds } from '../../src/city/roadTopology';

function key(x: number, y: number): string {
  return `${x}:${y}`;
}

function makeRoads(coords: Array<[number, number]>): Map<string, string> {
  return new Map(coords.map(([x, y]) => [key(x, y), 'street']));
}

describe('road topology', () => {
  it('allows straight map-edge exits and removes internal dead-end spurs', () => {
    const roads = makeRoads([
      [0, 3], [1, 3], [2, 3], [3, 3], [4, 3], [5, 3], [6, 3],
      [3, 4], [3, 5],
    ]);

    expect(countInvalidRoadDeadEnds(roads, { width: 7, height: 7 })).toBe(1);

    pruneInvalidRoadDeadEnds(roads, { width: 7, height: 7 });

    expect(countInvalidRoadDeadEnds(roads, { width: 7, height: 7 })).toBe(0);
    expect(roads.has(key(0, 3))).toBe(true);
    expect(roads.has(key(6, 3))).toBe(true);
    expect(roads.has(key(3, 4))).toBe(false);
    expect(roads.has(key(3, 5))).toBe(false);
  });

  it('does not treat a sideways road on the map edge as a valid outside connection', () => {
    const roads = makeRoads([
      [0, 2], [0, 3],
    ]);

    expect(countInvalidRoadDeadEnds(roads, { width: 5, height: 5 })).toBe(2);
  });

  it('counts disconnected road islands', () => {
    const roads = makeRoads([
      [0, 3], [1, 3], [2, 3],
      [5, 5], [5, 6],
    ]);

    expect(countRoadNetworkComponents(roads)).toBe(2);
  });
});
