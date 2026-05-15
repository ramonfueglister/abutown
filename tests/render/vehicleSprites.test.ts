import { describe, expect, it } from 'vitest';
import {
  candidateVehicleSprites,
  screenRightLaneOffset,
  vehicleSpriteForTrafficIndex,
  vehicleFrameForGridDelta,
} from '../../src/render/vehicleSprites';

describe('vehicle sprites', () => {
  const removedAssetPathPattern = new RegExp(`/${['open', 'gfx'].join('')}|/${['open', 'ttd'].join('')}`, 'i');

  it('uses pak128 road vehicle sheets from the active asset pack', () => {
    const sprites = candidateVehicleSprites();
    const sheets = new Set(sprites.map((sprite) => sprite.sheet));
    const paths = sprites.map((sprite) => sprite.path);

    expect(sheets).toEqual(new Set(['bus', 'truck', 'delivery-van', 'cooling-truck', 'tanker', 'concrete-mixer', 'bulk-truck', 'car-transporter']));
    expect(paths.every((path) => path.startsWith('/simutrans-assets/pak128/'))).toBe(true);
    expect(paths.every((path) => !removedAssetPathPattern.test(path))).toBe(true);
  });

  it('assigns available vehicle sprites in a stable pseudo-random order', () => {
    const sprites = candidateVehicleSprites();
    const assignments = Array.from({ length: 48 }, (_, index) => vehicleSpriteForTrafficIndex(sprites, index).sheet);

    expect(new Set(assignments)).toEqual(new Set(sprites.map((sprite) => sprite.sheet)));
    expect(assignments).toContain('delivery-van');
    expect(assignments.slice(0, 8)).not.toEqual(sprites.map((sprite) => sprite.sheet));
    expect(assignments).toEqual(
      Array.from({ length: 48 }, (_, index) => vehicleSpriteForTrafficIndex(sprites, index).sheet),
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
