import { describe, expect, it } from 'vitest';
import {
  hasVisiblePixelsInEveryVehicleFrame,
  MIN_VISIBLE_PIXELS_PER_VEHICLE_FRAME,
  candidateVehicleSprites,
  clippedVehicleFrameRect,
  POLROAD_PRIVATE_CAR_FRAME_HEIGHT,
  POLROAD_PRIVATE_CAR_FRAME_WIDTH,
  ROAD_SURFACE_WIDTH_PIXELS,
  ROAD_VEHICLE_LANE_OFFSET_PIXELS,
  screenVehicleRightLaneOffset,
  screenRightLaneOffset,
  trafficVehicleSpriteDeck,
  VEHICLE_SHEET_LAYOUTS,
  vehicleFrameRect,
  vehicleFrameForGridDelta,
} from '../../src/render/vehicleSprites';

describe('vehicle sprites', () => {
  it('uses every available road vehicle sheet candidate instead of a single bus frame', () => {
    const sprites = candidateVehicleSprites();
    const sheets = new Set(sprites.map((sprite) => sprite.sheet));

    expect(sheets).toEqual(new Set(VEHICLE_SHEET_LAYOUTS.map((layout) => layout.sheet)));
    expect(sheets.has('lorryFirstGeneration')).toBe(true);
    expect(sheets.has('lorrySecondGeneration')).toBe(true);
    expect(sheets.has('lorryThirdGeneration')).toBe(true);
    expect(sheets.has('polroadPrivateCars')).toBe(true);
    expect([...sheets].some((sheet) => sheet.toLowerCase().includes('toyland'))).toBe(false);
    expect(sprites.length).toBe(425);
    expect(sprites.filter((sprite) => sprite.sheet === 'bus')).toHaveLength(3);
    expect(sprites.filter((sprite) => sprite.sheet === 'polroadPrivateCars')).toHaveLength(44);
    expect(sprites.filter((sprite) => sprite.sheet !== 'bus' && sprite.sheet !== 'polroadPrivateCars')).toHaveLength(378);
  });

  it('clips edge frames instead of rejecting real OpenGFX vehicle blocks at the atlas border', () => {
    expect(clippedVehicleFrameRect(
      { sheet: 'lorryFirstGeneration', row: 0, block: 2, scale: 0.78 },
      7,
      { width: 523, height: 337 },
    )).toEqual({ x: 507, y: 0, width: 16, height: 24 });
  });

  it('selects fixed eight-direction frames from the extracted PolRoad private-car atlas', () => {
    expect(vehicleFrameRect({ sheet: 'polroadPrivateCars', row: 7, block: 0, scale: 0.92 }, 6)).toEqual({
      x: 6 * POLROAD_PRIVATE_CAR_FRAME_WIDTH,
      y: 7 * POLROAD_PRIVATE_CAR_FRAME_HEIGHT,
      width: POLROAD_PRIVATE_CAR_FRAME_WIDTH,
      height: POLROAD_PRIVATE_CAR_FRAME_HEIGHT,
    });
  });

  it('selects directional OpenGFX road-vehicle frames from grid movement', () => {
    expect(vehicleFrameForGridDelta({ x: 1, y: 0 })).toBe(3);
    expect(vehicleFrameForGridDelta({ x: 0, y: 1 })).toBe(5);
    expect(vehicleFrameForGridDelta({ x: -1, y: 0 })).toBe(7);
    expect(vehicleFrameForGridDelta({ x: 0, y: -1 })).toBe(1);
  });

  it('selects diagonal OpenGFX frames for smoothed curve tangents', () => {
    expect(vehicleFrameForGridDelta({ x: 1, y: -1 })).toBe(2);
    expect(vehicleFrameForGridDelta({ x: 1, y: 1 })).toBe(4);
    expect(vehicleFrameForGridDelta({ x: -1, y: 1 })).toBe(6);
    expect(vehicleFrameForGridDelta({ x: -1, y: -1 })).toBe(0);
  });

  it('places vehicles on the right lane relative to their screen-space travel direction', () => {
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: 10, y: 0 }, 5)).toEqual({ x: 0, y: 5 });
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: 0, y: 10 }, 5)).toEqual({ x: -5, y: 0 });
    expect(screenRightLaneOffset({ x: 0, y: 0 }, { x: -10, y: 0 }, 5)).toEqual({ x: 0, y: -5 });
  });

  it('keeps all four isometric travel directions on their right lane', () => {
    const eastLane = screenVehicleRightLaneOffset({ x: 0, y: 0 }, { x: 32, y: 16 });
    const southLane = screenVehicleRightLaneOffset({ x: 0, y: 0 }, { x: -32, y: 16 });
    const westLane = screenVehicleRightLaneOffset({ x: 0, y: 0 }, { x: -32, y: -16 });
    const northLane = screenVehicleRightLaneOffset({ x: 0, y: 0 }, { x: 32, y: -16 });

    expect(vectorLength(eastLane)).toBeCloseTo(ROAD_VEHICLE_LANE_OFFSET_PIXELS, 3);
    expect(vectorLength(southLane)).toBeLessThan(ROAD_VEHICLE_LANE_OFFSET_PIXELS);
    expect(vectorLength(southLane)).toBeGreaterThan(ROAD_VEHICLE_LANE_OFFSET_PIXELS - 1);
    expect(vectorLength(westLane)).toBeCloseTo(ROAD_VEHICLE_LANE_OFFSET_PIXELS, 3);
    expect(vectorLength(northLane)).toBeCloseTo(ROAD_VEHICLE_LANE_OFFSET_PIXELS, 3);

    expect(eastLane).toMatchObject({ x: expect.any(Number), y: expect.any(Number) });
    expect(eastLane.x).toBeLessThan(0);
    expect(eastLane.y).toBeGreaterThan(0);
    expect(southLane.x).toBeLessThan(0);
    expect(southLane.y).toBeLessThan(0);
    expect(westLane.x).toBeGreaterThan(0);
    expect(westLane.y).toBeLessThan(0);
    expect(northLane.x).toBeGreaterThan(0);
    expect(northLane.y).toBeGreaterThan(0);
  });

  it('keeps the right-lane offset inside the OpenGFX road surface', () => {
    expect(ROAD_SURFACE_WIDTH_PIXELS).toBe(18);
    expect(ROAD_VEHICLE_LANE_OFFSET_PIXELS).toBe(ROAD_SURFACE_WIDTH_PIXELS / 4);
    expect(ROAD_VEHICLE_LANE_OFFSET_PIXELS).toBeGreaterThan(4);
    expect(ROAD_VEHICLE_LANE_OFFSET_PIXELS).toBeLessThan(5);
  });

  it('keeps vehicle sprites smaller than the road lane footprint', () => {
    expect(Math.max(...candidateVehicleSprites().map((sprite) => sprite.scale))).toBeLessThanOrEqual(0.92);
  });

  it('requires every direction frame to have visible vehicle pixels', () => {
    expect(hasVisiblePixelsInEveryVehicleFrame(Array(8).fill(MIN_VISIBLE_PIXELS_PER_VEHICLE_FRAME))).toBe(true);
    expect(hasVisiblePixelsInEveryVehicleFrame([18, 12, 0, 14, 19, 20, 22, 11])).toBe(false);
    expect(hasVisiblePixelsInEveryVehicleFrame([18, 12, 14])).toBe(false);
  });

  it('weights compact road vehicles higher for city traffic without dropping cargo assets', () => {
    const sprites = candidateVehicleSprites();
    const deck = trafficVehicleSpriteDeck(sprites);
    const visibleFleet = deck.slice(0, 156);
    const privateCarCount = deck.filter((sprite) => sprite.sheet === 'polroadPrivateCars').length;
    const compactCount = deck.filter((sprite) => sprite.row === 1 || sprite.row === 10).length;
    const cargoCount = deck.filter((sprite) =>
      sprite.row !== 1 && sprite.row !== 10 && sprite.sheet !== 'bus' && sprite.sheet !== 'polroadPrivateCars'
    ).length;
    const visiblePrivateCarCount = visibleFleet.filter((sprite) => sprite.sheet === 'polroadPrivateCars').length;
    const visibleCompactCount = visibleFleet.filter((sprite) => sprite.row === 1 || sprite.row === 10).length;
    const visibleCargoCount = visibleFleet.filter((sprite) =>
      sprite.row !== 1 && sprite.row !== 10 && sprite.sheet !== 'bus' && sprite.sheet !== 'polroadPrivateCars'
    ).length;

    expect(new Set(deck.map((sprite) => sprite.sheet))).toEqual(new Set(sprites.map((sprite) => sprite.sheet)));
    expect(privateCarCount).toBeGreaterThan(compactCount);
    expect(visiblePrivateCarCount).toBeGreaterThan(100);
    expect(compactCount).toBeGreaterThan(cargoCount);
    expect(visibleCompactCount + visiblePrivateCarCount).toBeGreaterThan(visibleCargoCount);
  });
});

function vectorLength(point: { x: number; y: number }): number {
  return Math.hypot(point.x, point.y);
}
