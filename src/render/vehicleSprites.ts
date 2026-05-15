import { PAK128_ROAD_VEHICLES } from './pak128RoadVehicleManifest';
import type { AssetRole } from '../assets/assetPack';

export type ScreenPoint = { x: number; y: number };
export type VehicleSheetName = string;
export type SimutransVehicleDirection = 'W' | 'NW' | 'N' | 'NE' | 'E' | 'SE' | 'S' | 'SW';
export type VehicleSpriteRole = Extract<
  AssetRole,
  | 'vehicle.bus'
  | 'vehicle.truck'
  | 'vehicle.delivery.van'
  | 'vehicle.cooling.truck'
  | 'vehicle.tanker'
  | 'vehicle.concrete.mixer'
  | 'vehicle.bulk.truck'
  | 'vehicle.car.transporter'
>;

export type VehicleSprite = {
  sheet: VehicleSheetName;
  name: string;
  datPath: string;
  role: VehicleSpriteRole;
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
  return PAK128_ROAD_VEHICLES.map((vehicle) => ({
    sheet: vehicle.id,
    name: vehicle.name,
    datPath: vehicle.datPath,
    role: vehicleRoleForManifestEntry(vehicle),
    path: vehicle.path,
    row: vehicle.row,
    scale: vehicle.scale,
  }));
}

export function vehicleFrameRect(sprite: VehicleSprite, direction: SimutransVehicleDirection): VehicleFrameRect {
  return {
    x: DIRECTION_COLUMNS[direction] * TILE_SIZE,
    y: sprite.row * TILE_SIZE,
    width: TILE_SIZE,
    height: TILE_SIZE,
  };
}

export function vehicleSpriteForTrafficIndex(sprites: readonly VehicleSprite[], index: number): VehicleSprite {
  if (sprites.length === 0) throw new Error('Cannot select a vehicle sprite from an empty sprite list');
  return sprites[hashTrafficIndex(index) % sprites.length];
}

export function vehicleFrameForGridDelta(delta: ScreenPoint): SimutransVehicleDirection {
  const dx = Math.sign(delta.x);
  const dy = Math.sign(delta.y);
  if (dx > 0 && dy > 0) return 'SE';
  if (dx < 0 && dy < 0) return 'NW';
  if (dx > 0 && dy < 0) return 'NE';
  if (dx < 0 && dy > 0) return 'SW';
  if (dx > 0) return 'E';
  if (dy > 0) return 'S';
  if (dx < 0) return 'W';
  if (dy < 0) return 'N';
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

function vehicleRoleForManifestEntry(vehicle: { id: string; name: string; datPath: string }): VehicleSpriteRole {
  const value = `${vehicle.id} ${vehicle.name} ${vehicle.datPath}`.toLowerCase();
  if (value.includes('road-psg+mail') || /bus|coach|interliner|shuttle|ikarus|citaro|cruiser/u.test(value)) {
    return 'vehicle.bus';
  }
  if (/car[_-]?trans|transporter/u.test(value)) return 'vehicle.car.transporter';
  if (/cool|refriger/u.test(value)) return 'vehicle.cooling.truck';
  if (/concrete|cement|mixer/u.test(value)) return 'vehicle.concrete.mixer';
  if (/bulk|grain/u.test(value)) return 'vehicle.bulk.truck';
  if (/fluid|oil|milk|cistern|tanker/u.test(value)) return 'vehicle.tanker';
  if (/mail|post|sprinter|van|goods/u.test(value)) return 'vehicle.delivery.van';
  return 'vehicle.truck';
}

function normalizeZero(value: number): number {
  return Math.abs(value) < 0.000001 ? 0 : Number(value.toFixed(3));
}

function hashTrafficIndex(index: number): number {
  let value = (index + 1) >>> 0;
  value ^= value >>> 16;
  value = Math.imul(value, 0x7feb352d);
  value ^= value >>> 15;
  value = Math.imul(value, 0x846ca68b);
  value ^= value >>> 16;
  return value >>> 0;
}
