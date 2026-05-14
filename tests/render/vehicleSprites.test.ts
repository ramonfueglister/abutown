import { describe, expect, it } from 'vitest';
import {
  hasVisiblePixelsInEveryVehicleFrame,
  MIN_VISIBLE_PIXELS_PER_VEHICLE_FRAME,
  candidateVehicleSprites,
  ROAD_SURFACE_WIDTH_PIXELS,
  ROAD_VEHICLE_LANE_OFFSET_PIXELS,
  screenVehicleRightLaneOffset,
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

  it('pulls isometric screen-vertical travel slightly inward while preserving the right lane', () => {
    const downwardLane = screenVehicleRightLaneOffset({ x: 0, y: 0 }, { x: 0, y: 10 });
    const upwardRightLane = screenVehicleRightLaneOffset({ x: 0, y: 0 }, { x: 32, y: -16 });
    const upwardLeftLane = screenVehicleRightLaneOffset({ x: 0, y: 0 }, { x: -32, y: -16 });

    expect(vectorLength(downwardLane)).toBeLessThan(ROAD_VEHICLE_LANE_OFFSET_PIXELS);
    expect(vectorLength(downwardLane)).toBeGreaterThan(ROAD_VEHICLE_LANE_OFFSET_PIXELS - 1);
    expect(vectorLength(upwardRightLane)).toBeCloseTo(vectorLength(downwardLane), 3);
    expect(vectorLength(upwardLeftLane)).toBeCloseTo(vectorLength(downwardLane), 3);
    expect(upwardRightLane.x).toBeGreaterThan(0);
    expect(upwardRightLane.y).toBeGreaterThan(0);
    expect(upwardLeftLane.x).toBeGreaterThan(0);
    expect(upwardLeftLane.y).toBeLessThan(0);
  });

  it('keeps the right-lane offset inside the OpenGFX road surface', () => {
    expect(ROAD_SURFACE_WIDTH_PIXELS).toBe(18);
    expect(ROAD_VEHICLE_LANE_OFFSET_PIXELS).toBe(ROAD_SURFACE_WIDTH_PIXELS / 4);
    expect(ROAD_VEHICLE_LANE_OFFSET_PIXELS).toBeGreaterThan(4);
    expect(ROAD_VEHICLE_LANE_OFFSET_PIXELS).toBeLessThan(5);
  });

  it('keeps vehicle sprites smaller than the road lane footprint', () => {
    expect(Math.max(...candidateVehicleSprites().map((sprite) => sprite.scale))).toBeLessThanOrEqual(0.84);
  });

  it('requires every direction frame to have visible vehicle pixels', () => {
    expect(hasVisiblePixelsInEveryVehicleFrame(Array(8).fill(MIN_VISIBLE_PIXELS_PER_VEHICLE_FRAME))).toBe(true);
    expect(hasVisiblePixelsInEveryVehicleFrame([18, 12, 0, 14, 19, 20, 22, 11])).toBe(false);
    expect(hasVisiblePixelsInEveryVehicleFrame([18, 12, 14])).toBe(false);
  });
});

function vectorLength(point: { x: number; y: number }): number {
  return Math.hypot(point.x, point.y);
}
