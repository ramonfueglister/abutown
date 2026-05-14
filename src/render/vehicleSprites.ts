export type ScreenPoint = { x: number; y: number };
export type VehicleSheetName = 'bus' | 'lorry';

export type VehicleSprite = {
  sheet: VehicleSheetName;
  row: number;
  block: number;
  scale: number;
};

export type VehicleFrameRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

const VEHICLE_BLOCK_WIDTH = 176;
const VEHICLE_ROW_HEIGHT = 24;
export const ROAD_SURFACE_WIDTH_PIXELS = 18;
export const ROAD_VEHICLE_LANE_OFFSET_PIXELS = ROAD_SURFACE_WIDTH_PIXELS / 4;
export const MIN_VISIBLE_PIXELS_PER_VEHICLE_FRAME = 10;

const DIRECTION_RECTS: readonly Omit<VehicleFrameRect, 'y'>[] = [
  { x: 0, width: 9, height: 24 },
  { x: 10, width: 23, height: 24 },
  { x: 34, width: 31, height: 24 },
  { x: 66, width: 22, height: 24 },
  { x: 89, width: 9, height: 24 },
  { x: 99, width: 23, height: 24 },
  { x: 122, width: 32, height: 24 },
  { x: 155, width: 21, height: 24 },
];

export function candidateVehicleSprites(): VehicleSprite[] {
  const sprites: VehicleSprite[] = [];

  for (let row = 0; row < 3; row += 1) {
    sprites.push({ sheet: 'bus', row, block: 0, scale: 0.82 });
  }

  for (let row = 0; row < 14; row += 1) {
    for (let block = 0; block < 3; block += 1) {
      sprites.push({ sheet: 'lorry', row, block, scale: 0.78 });
    }
  }

  return sprites;
}

export function vehicleFrameRect(sprite: VehicleSprite, frame: number): VehicleFrameRect {
  const rect = DIRECTION_RECTS[((frame % DIRECTION_RECTS.length) + DIRECTION_RECTS.length) % DIRECTION_RECTS.length];
  return {
    x: sprite.block * VEHICLE_BLOCK_WIDTH + rect.x,
    y: sprite.row * VEHICLE_ROW_HEIGHT,
    width: rect.width,
    height: rect.height,
  };
}

export function vehicleFrameForGridDelta(delta: ScreenPoint): number {
  if (delta.x > 0 && delta.y === 0) return 3;
  if (delta.x === 0 && delta.y > 0) return 5;
  if (delta.x < 0 && delta.y === 0) return 7;
  if (delta.x === 0 && delta.y < 0) return 1;
  return vehicleFrameForScreenDelta(delta);
}

export function vehicleFrameForScreenDelta(delta: ScreenPoint): number {
  const angle = (Math.atan2(delta.y, delta.x) + Math.PI * 2) % (Math.PI * 2);
  return Math.round(angle / (Math.PI / 4)) % 8;
}

export function screenRightLaneOffset(from: ScreenPoint, to: ScreenPoint, pixels: number): ScreenPoint {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const length = Math.hypot(dx, dy);
  if (length === 0) return { x: 0, y: 0 };
  return {
    x: normalizeZero((-dy / length) * pixels),
    y: normalizeZero((dx / length) * pixels),
  };
}

export function hasVisiblePixelsInEveryVehicleFrame(frameVisiblePixels: readonly number[]): boolean {
  return (
    frameVisiblePixels.length === DIRECTION_RECTS.length &&
    frameVisiblePixels.every((count) => count >= MIN_VISIBLE_PIXELS_PER_VEHICLE_FRAME)
  );
}

function normalizeZero(value: number): number {
  return Math.abs(value) < 0.000001 ? 0 : Number(value.toFixed(3));
}
