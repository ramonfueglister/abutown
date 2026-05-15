import { describe, expect, it } from 'vitest';
import {
  candidateVehicleSprites,
  screenRightLaneOffset,
  vehicleFrameForGridDelta,
} from '../../src/render/vehicleSprites';

describe('vehicle sprites', () => {
  it('uses pak128 road vehicle sheets from the active asset pack', () => {
    const sprites = candidateVehicleSprites();
    const sheets = new Set(sprites.map((sprite) => sprite.sheet));
    const paths = sprites.map((sprite) => sprite.path);

    expect(sheets).toEqual(new Set(['bus', 'truck']));
    expect(paths.every((path) => path.startsWith('/simutrans-assets/pak128/'))).toBe(true);
    expect(paths.every((path) => !path.includes('/opengfx') && !path.includes('/openttd'))).toBe(true);
  });

  it('selects directional pak128 road-vehicle frames from grid movement', () => {
    expect(vehicleFrameForGridDelta({ x: 1, y: 0 })).toBe('SE');
    expect(vehicleFrameForGridDelta({ x: 0, y: 1 })).toBe('SW');
    expect(vehicleFrameForGridDelta({ x: -1, y: 0 })).toBe('NW');
    expect(vehicleFrameForGridDelta({ x: 0, y: -1 })).toBe('NE');
  });

  it('places vehicles on the right lane relative to their screen-space travel direction', () => {
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: 10, y: 0 }, 5)).toEqual({ x: 0, y: 5 });
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: 0, y: 10 }, 5)).toEqual({ x: -5, y: 0 });
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: -10, y: 0 }, 5)).toEqual({ x: 0, y: -5 });
  });
});
