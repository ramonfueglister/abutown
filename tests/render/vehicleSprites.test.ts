import { describe, expect, it } from 'vitest';
import {
  candidateVehicleSprites,
  screenRightLaneOffset,
  vehicleSpriteForTrafficIndex,
  vehicleFrameForGridDelta,
} from '../../src/render/vehicleSprites';

describe('vehicle sprites', () => {
  const legacyPathPattern = new RegExp(`/${['open', 'gfx'].join('')}|/${['open', 'ttd'].join('')}`, 'i');

  it('uses the full pak128 road vehicle manifest', () => {
    const sprites = candidateVehicleSprites();
    const sheets = new Set(sprites.map((sprite) => sprite.sheet));
    const paths = sprites.map((sprite) => sprite.path);

    expect(sprites.length).toBeGreaterThan(80);
    expect(sheets.size).toBe(sprites.length);
    expect(sheets).toContain('rvg_type_s_van');
    expect(sheets).toContain('man_lions_city');
    expect(sheets).toContain('goods_truck_0');
    expect(paths.every((path) => path.startsWith('/simutrans-assets/pak128/'))).toBe(true);
    expect(paths.every((path) => path.endsWith('.png'))).toBe(true);
    expect(paths.every((path) => !legacyPathPattern.test(path))).toBe(true);
  });

  it('assigns available vehicle sprites in a stable pseudo-random order', () => {
    const sprites = candidateVehicleSprites();
    const assignments = Array.from({ length: sprites.length * 6 }, (_, index) => vehicleSpriteForTrafficIndex(sprites, index).sheet);

    expect(new Set(assignments)).toEqual(new Set(sprites.map((sprite) => sprite.sheet)));
    expect(assignments).toContain('rvg_type_s_van');
    expect(new Set(assignments).size).toBeGreaterThan(24);
    expect(assignments.slice(0, 8)).not.toEqual(sprites.slice(0, 8).map((sprite) => sprite.sheet));
    expect(assignments).toEqual(
      Array.from({ length: sprites.length * 6 }, (_, index) => vehicleSpriteForTrafficIndex(sprites, index).sheet),
    );
  });

  it('selects pak128 road-vehicle frames using the same grid compass as road DAT masks', () => {
    expect(vehicleFrameForGridDelta({ x: 1, y: 0 })).toBe('E');
    expect(vehicleFrameForGridDelta({ x: 0, y: 1 })).toBe('S');
    expect(vehicleFrameForGridDelta({ x: -1, y: 0 })).toBe('W');
    expect(vehicleFrameForGridDelta({ x: 0, y: -1 })).toBe('N');
  });

  it('places vehicles on the right lane relative to their screen-space travel direction', () => {
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: 10, y: 0 }, 5)).toEqual({ x: 0, y: 5 });
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: 0, y: 10 }, 5)).toEqual({ x: -5, y: 0 });
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: -10, y: 0 }, 5)).toEqual({ x: 0, y: -5 });
  });
});
