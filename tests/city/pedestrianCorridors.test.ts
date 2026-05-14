import { describe, expect, it } from 'vitest';
import { buildPedestrianCorridors, makeNonTeleportingPedestrianLoop } from '../../src/city/pedestrianCorridors';

const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;

type Coord = { x: number; y: number };

function roads(coords: Coord[]): Map<string, { coord: Coord; mask: number }> {
  const keys = new Set(coords.map((coord) => `${coord.x}:${coord.y}`));
  return new Map(coords.map((coord) => {
    const mask =
      (keys.has(`${coord.x}:${coord.y - 1}`) ? NORTH : 0) |
      (keys.has(`${coord.x + 1}:${coord.y}`) ? EAST : 0) |
      (keys.has(`${coord.x}:${coord.y + 1}`) ? SOUTH : 0) |
      (keys.has(`${coord.x - 1}:${coord.y}`) ? WEST : 0);
    return [`${coord.x}:${coord.y}`, { coord, mask }];
  }));
}

describe('pedestrian corridors', () => {
  it('extracts horizontal and vertical walking routes from the road network', () => {
    const corridors = buildPedestrianCorridors(roads([
      ...Array.from({ length: 7 }, (_, x) => ({ x, y: 2 })),
      ...Array.from({ length: 7 }, (_, y) => ({ x: 4, y })),
      { x: 10, y: 10 },
      { x: 11, y: 10 },
    ]), { minLength: 4 });

    expect(corridors).toContainEqual([
      { x: 0, y: 2 },
      { x: 1, y: 2 },
      { x: 2, y: 2 },
      { x: 3, y: 2 },
      { x: 4, y: 2 },
      { x: 5, y: 2 },
      { x: 6, y: 2 },
    ]);
    expect(corridors).toContainEqual([
      { x: 4, y: 0 },
      { x: 4, y: 1 },
      { x: 4, y: 2 },
      { x: 4, y: 3 },
      { x: 4, y: 4 },
      { x: 4, y: 5 },
      { x: 4, y: 6 },
    ]);
    expect(corridors.some((path) => path.length === 2)).toBe(false);
  });

  it('mirrors open walking corridors so modulo animation cannot teleport to the start', () => {
    const loop = makeNonTeleportingPedestrianLoop([
      { x: 0, y: 0 },
      { x: 1, y: 0 },
      { x: 2, y: 0 },
      { x: 3, y: 0 },
    ]);

    expect(loop).toEqual([
      { x: 0, y: 0 },
      { x: 1, y: 0 },
      { x: 2, y: 0 },
      { x: 3, y: 0 },
      { x: 2, y: 0 },
      { x: 1, y: 0 },
    ]);
    for (let index = 0; index < loop.length; index += 1) {
      const current = loop[index];
      const next = loop[(index + 1) % loop.length];
      expect(Math.abs(current.x - next.x) + Math.abs(current.y - next.y)).toBeLessThanOrEqual(1);
    }
  });
});
