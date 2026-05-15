import { describe, expect, it } from 'vitest';
import {
  buildingStreetFrontageOffset,
  countBuildingsWithoutDirectStreetAdjacency,
  hasDirectStreetAdjacency,
  hasVisibleStreetFrontage,
} from '../../src/city/buildingFrontage';

function key(x: number, y: number): string {
  return `${x}:${y}`;
}

function roads(coords: Array<[number, number]>): Map<string, { coord: { x: number; y: number }; kind: 'street' }> {
  return new Map(coords.map(([x, y]) => [key(x, y), { coord: { x, y }, kind: 'street' }]));
}

describe('building frontage', () => {
  it('requires every building to touch a street cardinally', () => {
    const streetRoads = roads([[4, 3], [8, 8]]);
    const buildings = [
      { coord: { x: 4, y: 4 } },
      { coord: { x: 2, y: 2 } },
    ];

    expect(hasDirectStreetAdjacency({ x: 4, y: 4 }, streetRoads)).toBe(true);
    expect(hasDirectStreetAdjacency({ x: 2, y: 2 }, streetRoads)).toBe(false);
    expect(countBuildingsWithoutDirectStreetAdjacency(buildings, streetRoads)).toBe(1);
  });

  it('keeps the visible south/east frontage rule for isometric draw order', () => {
    expect(hasVisibleStreetFrontage({ x: 4, y: 4 }, roads([[4, 5]]))).toBe(true);
    expect(hasVisibleStreetFrontage({ x: 4, y: 4 }, roads([[5, 4]]))).toBe(true);
    expect(hasVisibleStreetFrontage({ x: 4, y: 4 }, roads([[4, 3]]))).toBe(false);
    expect(hasVisibleStreetFrontage({ x: 4, y: 4 }, roads([[3, 4]]))).toBe(false);
  });

  it('nudges rendered buildings toward their visible street frontage', () => {
    expect(buildingStreetFrontageOffset({ x: 4, y: 4 }, roads([[5, 4]]))).toEqual({ x: 7, y: 4 });
    expect(buildingStreetFrontageOffset({ x: 4, y: 4 }, roads([[4, 5]]))).toEqual({ x: -7, y: 4 });
    expect(buildingStreetFrontageOffset({ x: 4, y: 4 }, roads([[5, 4], [4, 5]]))).toEqual({ x: 0, y: 7 });
  });
});
