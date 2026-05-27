import { describe, expect, it } from 'vitest';
import {
  candidateVehicleSprites,
  screenRightLaneOffset,
  vehicleSpriteForTrafficIndex,
  vehicleFrameForGridDelta,
} from '../../src/render/vehicleSprites';

describe('vehicle sprites', () => {
  it('uses the minimal vector vehicle catalog', () => {
    const sprites = candidateVehicleSprites();
    const sheets = new Set(sprites.map((sprite) => sprite.sheet));

    expect(sprites.length).toBe(8);
    expect(sheets.size).toBe(sprites.length);
    expect(sheets).toContain('city-bus');
    expect(sheets).toContain('delivery-van');
    expect(sheets).toContain('box-truck');
    const roles = new Set(sprites.map((sprite) => sprite.role));
    expect(roles.has('vehicle.bus')).toBe(true);
    expect(roles.has('vehicle.truck')).toBe(true);
  });

  it('assigns available vehicle sprites in a stable pseudo-random order', () => {
    const sprites = candidateVehicleSprites();
    const assignments = Array.from({ length: sprites.length * 6 }, (_, index) => vehicleSpriteForTrafficIndex(sprites, index).sheet);

    expect(new Set(assignments)).toEqual(new Set(sprites.map((sprite) => sprite.sheet)));
    expect(assignments).toContain('delivery-van');
    expect(new Set(assignments).size).toBe(sprites.length);
    expect(assignments.slice(0, 8)).not.toEqual(sprites.slice(0, 8).map((sprite) => sprite.sheet));
    expect(assignments).toEqual(
      Array.from({ length: sprites.length * 6 }, (_, index) => vehicleSpriteForTrafficIndex(sprites, index).sheet),
    );
  });

  it('selects road-vehicle compass directions from grid deltas', () => {
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
