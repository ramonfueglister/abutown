import { describe, expect, it } from 'vitest';
import {
  EAST,
  NORTH,
  SOUTH,
  WEST,
  coordKey,
  distanceOutsideMap,
  isInsideMap,
  maskSegments,
  movementAngle,
  outwardExits,
  stableHash,
} from '../../src/render/gridMath';

describe('gridMath', () => {
  it('rounds grid coords into stable coord keys', () => {
    expect(coordKey({ x: 10.2, y: 11.7 })).toBe('10:12');
  });

  it('hashes strings with the existing unsigned FNV-1a variant', () => {
    expect(stableHash('vehicle-color:car:1')).toBe(1149577068);
    expect(stableHash('tree-lod:12:20')).toBe(3729335004);
  });

  it('detects whether coords are inside map bounds', () => {
    const map = { width: 4, height: 3 };

    expect(isInsideMap({ x: 0, y: 0 }, map)).toBe(true);
    expect(isInsideMap({ x: 3, y: 2 }, map)).toBe(true);
    expect(isInsideMap({ x: 4, y: 2 }, map)).toBe(false);
    expect(isInsideMap({ x: 3, y: -1 }, map)).toBe(false);
  });

  it('returns zero distance inside the map and max axis overflow outside', () => {
    const map = { width: 4, height: 3 };

    expect(distanceOutsideMap({ x: 2, y: 2 }, map)).toBe(0);
    expect(distanceOutsideMap({ x: -2, y: 1 }, map)).toBe(2);
    expect(distanceOutsideMap({ x: 7, y: 6 }, map)).toBe(4);
  });

  it('turns road masks into tile edge segment endpoints', () => {
    const tileSize = { width: 64, height: 32 };

    expect(maskSegments(NORTH | EAST | WEST, tileSize)).toEqual([
      { x: 0, y: -16 },
      { x: 32, y: 0 },
      { x: -32, y: 0 },
    ]);
  });

  it('emits outward exits only on matching map edges', () => {
    const map = { width: 5, height: 4 };

    expect(outwardExits({ x: 0, y: 0 }, NORTH | WEST | EAST, map)).toEqual([
      { dx: 0, dy: -1, mask: NORTH | SOUTH },
      { dx: -1, dy: 0, mask: EAST | WEST },
    ]);
    expect(outwardExits({ x: 4, y: 3 }, SOUTH | EAST, map)).toEqual([
      { dx: 1, dy: 0, mask: EAST | WEST },
      { dx: 0, dy: 1, mask: NORTH | SOUTH },
    ]);
  });

  it('returns a movement angle and treats tiny movement as zero', () => {
    expect(movementAngle({ x: 0, y: 0 }, { x: 0.0002, y: 0.0002 })).toBe(0);
    expect(movementAngle({ x: 0, y: 0 }, { x: 0, y: 2 })).toBeCloseTo(Math.PI / 2);
  });
});
