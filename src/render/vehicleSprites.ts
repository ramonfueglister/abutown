export type ScreenPoint = { x: number; y: number };
export type VehicleSheetName = string;
export type SimutransVehicleDirection = 'W' | 'NW' | 'N' | 'NE' | 'E' | 'SE' | 'S' | 'SW';
export type VehicleSpriteRole =
  | 'vehicle.bus'
  | 'vehicle.truck'
  | 'vehicle.delivery.van'
  | 'vehicle.cooling.truck'
  | 'vehicle.tanker'
  | 'vehicle.concrete.mixer'
  | 'vehicle.bulk.truck'
  | 'vehicle.car.transporter';

export type VehicleSprite = {
  sheet: VehicleSheetName;
  name: string;
  role: VehicleSpriteRole;
  scale: number;
};

export function candidateVehicleSprites(): VehicleSprite[] {
  return [
    { sheet: 'city-bus', name: 'City bus', role: 'vehicle.bus' as const, scale: 1.15 },
    { sheet: 'delivery-van', name: 'Delivery van', role: 'vehicle.delivery.van' as const, scale: 0.82 },
    { sheet: 'box-truck', name: 'Box truck', role: 'vehicle.truck' as const, scale: 1 },
    { sheet: 'cooling-truck', name: 'Cooling truck', role: 'vehicle.cooling.truck' as const, scale: 1 },
    { sheet: 'tanker', name: 'Tanker', role: 'vehicle.tanker' as const, scale: 1.05 },
    { sheet: 'concrete-mixer', name: 'Concrete mixer', role: 'vehicle.concrete.mixer' as const, scale: 1.04 },
    { sheet: 'bulk-truck', name: 'Bulk truck', role: 'vehicle.bulk.truck' as const, scale: 1.02 },
    { sheet: 'car-transporter', name: 'Car transporter', role: 'vehicle.car.transporter' as const, scale: 1.08 },
  ];
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
