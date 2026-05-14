export type ScreenPoint = { x: number; y: number };

export const VEHICLE_SHEET_LAYOUTS = [
  { sheet: 'bus', rows: 3, blocks: 1, scale: 0.82 },
  { sheet: 'polroadPrivateCars', rows: 44, blocks: 1, scale: 0.92 },
  { sheet: 'lorryFirstGeneration', rows: 14, blocks: 3, scale: 0.78 },
  { sheet: 'lorryFirstGenerationArctic', rows: 14, blocks: 3, scale: 0.78 },
  { sheet: 'lorryFirstGenerationTropical', rows: 14, blocks: 3, scale: 0.78 },
  { sheet: 'lorrySecondGeneration', rows: 14, blocks: 3, scale: 0.78 },
  { sheet: 'lorrySecondGenerationArctic', rows: 14, blocks: 3, scale: 0.78 },
  { sheet: 'lorrySecondGenerationTropical', rows: 14, blocks: 3, scale: 0.78 },
  { sheet: 'lorryThirdGeneration', rows: 14, blocks: 3, scale: 0.78 },
  { sheet: 'lorryThirdGenerationArctic', rows: 14, blocks: 3, scale: 0.78 },
  { sheet: 'lorryThirdGenerationTropical', rows: 14, blocks: 3, scale: 0.78 },
] as const;

export type VehicleSheetName = typeof VEHICLE_SHEET_LAYOUTS[number]['sheet'];

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

type ImageBounds = {
  width: number;
  height: number;
};

const VEHICLE_BLOCK_WIDTH = 176;
const VEHICLE_ROW_HEIGHT = 24;
export const POLROAD_PRIVATE_CAR_FRAME_WIDTH = 42;
export const POLROAD_PRIVATE_CAR_FRAME_HEIGHT = 28;
export const ROAD_SURFACE_WIDTH_PIXELS = 18;
export const ROAD_VEHICLE_LANE_OFFSET_PIXELS = ROAD_SURFACE_WIDTH_PIXELS / 4;
export const SCREEN_DOWN_LEFT_VEHICLE_LANE_INSET_PIXELS = 0.75;
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

  for (const layout of VEHICLE_SHEET_LAYOUTS) {
    for (let row = 0; row < layout.rows; row += 1) {
      for (let block = 0; block < layout.blocks; block += 1) {
        sprites.push({ sheet: layout.sheet, row, block, scale: layout.scale });
      }
    }
  }

  return sprites;
}

export function trafficVehicleSpriteDeck(sprites: readonly VehicleSprite[]): VehicleSprite[] {
  return sprites
    .flatMap((sprite, spriteIndex) =>
      Array.from({ length: vehicleTrafficWeight(sprite) }, (_, copyIndex) => ({
        sprite,
        rank: stableSpriteRank(sprite, spriteIndex, copyIndex),
      })),
    )
    .sort((a, b) => a.rank - b.rank)
    .map((entry) => entry.sprite);
}

export function vehicleTrafficWeight(sprite: VehicleSprite): number {
  if (sprite.sheet === 'polroadPrivateCars') return 80;
  if (sprite.sheet === 'bus') return 3;
  if (sprite.row === 10) return 18;
  if (sprite.row === 1) return 10;
  return 1;
}

export function vehicleFrameRect(sprite: VehicleSprite, frame: number): VehicleFrameRect {
  if (sprite.sheet === 'polroadPrivateCars') {
    const normalizedFrame = ((frame % 8) + 8) % 8;
    return {
      x: normalizedFrame * POLROAD_PRIVATE_CAR_FRAME_WIDTH,
      y: sprite.row * POLROAD_PRIVATE_CAR_FRAME_HEIGHT,
      width: POLROAD_PRIVATE_CAR_FRAME_WIDTH,
      height: POLROAD_PRIVATE_CAR_FRAME_HEIGHT,
    };
  }

  const rect = DIRECTION_RECTS[((frame % DIRECTION_RECTS.length) + DIRECTION_RECTS.length) % DIRECTION_RECTS.length];
  return {
    x: sprite.block * VEHICLE_BLOCK_WIDTH + rect.x,
    y: sprite.row * VEHICLE_ROW_HEIGHT,
    width: rect.width,
    height: rect.height,
  };
}

export function clippedVehicleFrameRect(
  sprite: VehicleSprite,
  frame: number,
  bounds: ImageBounds,
): VehicleFrameRect | undefined {
  const rect = vehicleFrameRect(sprite, frame);
  const width = Math.min(rect.width, bounds.width - rect.x);
  const height = Math.min(rect.height, bounds.height - rect.y);
  if (rect.x < 0 || rect.y < 0 || width <= 0 || height <= 0) return undefined;
  return { x: rect.x, y: rect.y, width, height };
}

export function vehicleFrameForGridDelta(delta: ScreenPoint): number {
  const x = Math.sign(delta.x);
  const y = Math.sign(delta.y);
  if (x > 0 && y < 0) return 2;
  if (x > 0 && y === 0) return 3;
  if (x > 0 && y > 0) return 4;
  if (x === 0 && y > 0) return 5;
  if (x < 0 && y > 0) return 6;
  if (x < 0 && y === 0) return 7;
  if (x < 0 && y < 0) return 0;
  if (x === 0 && y < 0) return 1;
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

export function screenVehicleRightLaneOffset(from: ScreenPoint, to: ScreenPoint): ScreenPoint {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const pixels = dx < 0 && dy > 0
    ? ROAD_VEHICLE_LANE_OFFSET_PIXELS - SCREEN_DOWN_LEFT_VEHICLE_LANE_INSET_PIXELS
    : ROAD_VEHICLE_LANE_OFFSET_PIXELS;
  return screenRightLaneOffset(from, to, pixels);
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

function stableSpriteRank(sprite: VehicleSprite, spriteIndex: number, copyIndex: number): number {
  const value = `${sprite.sheet}:${sprite.row}:${sprite.block}:${spriteIndex}:${copyIndex}`;
  let hash = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}
