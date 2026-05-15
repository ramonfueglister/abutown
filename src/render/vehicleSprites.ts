import { pak128AssetPack } from '../assets/pak128Catalog';
import type { AssetRole } from '../assets/assetPack';

export type ScreenPoint = { x: number; y: number };
export type VehicleSheetName = 'bus' | 'truck';
export type SimutransVehicleDirection = 'W' | 'NW' | 'N' | 'NE' | 'E' | 'SE' | 'S' | 'SW';

export type VehicleSprite = {
  sheet: VehicleSheetName;
  role: Extract<AssetRole, 'vehicle.bus' | 'vehicle.truck'>;
  path: string;
  row: number;
  scale: number;
};

export type VehicleFrameRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

const TILE_SIZE = 128;

const DIRECTION_COLUMNS: Record<SimutransVehicleDirection, number> = {
  W: 0,
  NW: 1,
  N: 2,
  NE: 3,
  E: 4,
  SE: 5,
  S: 6,
  SW: 7,
};

export function candidateVehicleSprites(): VehicleSprite[] {
  const bus = pak128AssetPack.require('vehicle.bus');
  const truck = pak128AssetPack.require('vehicle.truck');

  return [
    { sheet: 'bus', role: 'vehicle.bus', path: bus.path, row: bus.source.y / TILE_SIZE, scale: bus.scale },
    { sheet: 'truck', role: 'vehicle.truck', path: truck.path, row: truck.source.y / TILE_SIZE, scale: truck.scale },
  ];
}

export function vehicleFrameRect(sprite: VehicleSprite, direction: SimutransVehicleDirection): VehicleFrameRect {
  return {
    x: DIRECTION_COLUMNS[direction] * TILE_SIZE,
    y: sprite.row * TILE_SIZE,
    width: TILE_SIZE,
    height: TILE_SIZE,
  };
}

export function vehicleFrameForGridDelta(delta: ScreenPoint): SimutransVehicleDirection {
  const dx = Math.sign(delta.x);
  const dy = Math.sign(delta.y);
  if (dx > 0 && dy > 0) return 'S';
  if (dx < 0 && dy < 0) return 'N';
  if (dx > 0 && dy < 0) return 'E';
  if (dx < 0 && dy > 0) return 'W';
  if (dx > 0) return 'SE';
  if (dy > 0) return 'SW';
  if (dx < 0) return 'NW';
  if (dy < 0) return 'NE';
  return 'S';
}

export function vehicleFrameForScreenDelta(delta: ScreenPoint): SimutransVehicleDirection {
  const angle = (Math.atan2(delta.y, delta.x) + Math.PI * 2) % (Math.PI * 2);
  const index = Math.round(angle / (Math.PI / 4)) % 8;
  return (['E', 'SE', 'S', 'SW', 'W', 'NW', 'N', 'NE'] as const)[index];
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

function normalizeZero(value: number): number {
  return Math.abs(value) < 0.000001 ? 0 : Number(value.toFixed(3));
}
