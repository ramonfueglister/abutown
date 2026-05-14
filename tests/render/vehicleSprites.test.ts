import { describe, expect, it } from 'vitest';
import {
  candidateVehicleSprites,
  ROAD_VEHICLE_LANE_OFFSET_PIXELS,
  screenRightLaneOffset,
  vehicleFrameForGridDelta,
} from '../../src/render/vehicleSprites';

describe('vehicle sprites', () => {
  it('uses every available road vehicle sheet candidate instead of a single bus frame', () => {
    const sprites = candidateVehicleSprites();
    const sheets = new Set(sprites.map((sprite) => sprite.sheet));

    expect(sheets).toEqual(new Set(['bus', 'lorry']));
    expect(sprites.length).toBeGreaterThan(20);
  });

  it('selects directional OpenGFX road-vehicle frames from grid movement', () => {
    expect(vehicleFrameForGridDelta({ x: 1, y: 0 })).toBe(3);
    expect(vehicleFrameForGridDelta({ x: 0, y: 1 })).toBe(5);
    expect(vehicleFrameForGridDelta({ x: -1, y: 0 })).toBe(7);
    expect(vehicleFrameForGridDelta({ x: 0, y: -1 })).toBe(1);
  });

  it('places vehicles on the right lane relative to their screen-space travel direction', () => {
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: 10, y: 0 }, 5)).toEqual({ x: 0, y: 5 });
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: 0, y: 10 }, 5)).toEqual({ x: -5, y: 0 });
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: -10, y: 0 }, 5)).toEqual({ x: 0, y: -5 });
  });

  it('keeps the right-lane offset inside the OpenGFX road surface', () => {
    expect(ROAD_VEHICLE_LANE_OFFSET_PIXELS).toBeGreaterThanOrEqual(2.5);
    expect(ROAD_VEHICLE_LANE_OFFSET_PIXELS).toBeLessThanOrEqual(3.5);
  });
});
