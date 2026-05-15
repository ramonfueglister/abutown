import type { AssetRole } from '../assets/assetPack';
import type { VehicleSheetName } from './vehicleSprites';

export type LocalRoadVehicleCoord = {
  x: number;
  y: number;
};

export type LocalRoadVehicleSource = {
  path: LocalRoadVehicleCoord[];
  offset: number;
  speed: number;
  sprite: {
    sheet: VehicleSheetName;
    role: Extract<AssetRole, 'vehicle.bus' | 'vehicle.truck'>;
  };
};

export type LocalRoadVehicle = {
  id: string;
  kind: 'road-vehicle';
  state: 'driving';
  coord: LocalRoadVehicleCoord;
  pathIndex: number;
  nextCoord: LocalRoadVehicleCoord;
  speed: number;
  spriteSheet: VehicleSheetName;
  role: Extract<AssetRole, 'vehicle.bus' | 'vehicle.truck'>;
};

export function localRoadVehicleId(index: number): string {
  return `vehicle:road:${index}`;
}

export function buildLocalRoadVehicles(vehicles: readonly LocalRoadVehicleSource[]): LocalRoadVehicle[] {
  return vehicles
    .filter((vehicle) => vehicle.path.length > 0)
    .map((vehicle, index) => {
      const pathIndex = normalizedPathIndex(vehicle);
      const nextCoord = vehicle.path[(pathIndex + 1) % vehicle.path.length];
      return {
        id: localRoadVehicleId(index),
        kind: 'road-vehicle',
        state: 'driving',
        coord: vehiclePosition(vehicle, pathIndex),
        pathIndex,
        nextCoord,
        speed: vehicle.speed,
        spriteSheet: vehicle.sprite.sheet,
        role: vehicle.sprite.role,
      };
    });
}

export function findNearestLocalRoadVehicle(
  vehicles: readonly LocalRoadVehicle[],
  point: LocalRoadVehicleCoord,
  project: (coord: LocalRoadVehicleCoord) => LocalRoadVehicleCoord,
  radius: number,
): LocalRoadVehicle | null {
  let nearest: { vehicle: LocalRoadVehicle; distance: number } | null = null;
  for (const vehicle of vehicles) {
    const projected = project(vehicle.coord);
    const distance = Math.hypot(projected.x - point.x, projected.y - point.y);
    if (distance > radius) continue;
    if (!nearest || distance < nearest.distance) nearest = { vehicle, distance };
  }
  return nearest?.vehicle ?? null;
}

function normalizedPathIndex(vehicle: LocalRoadVehicleSource): number {
  const base = Math.floor(vehicle.offset);
  return ((base % vehicle.path.length) + vehicle.path.length) % vehicle.path.length;
}

function vehiclePosition(vehicle: LocalRoadVehicleSource, pathIndex: number): LocalRoadVehicleCoord {
  const next = (pathIndex + 1) % vehicle.path.length;
  const t = vehicle.offset - Math.floor(vehicle.offset);
  return {
    x: lerp(vehicle.path[pathIndex].x, vehicle.path[next].x, t),
    y: lerp(vehicle.path[pathIndex].y, vehicle.path[next].y, t),
  };
}

function lerp(start: number, end: number, t: number): number {
  return start + (end - start) * t;
}
