import { directionForRoadStep } from './intersections';
import {
  type TrafficIntersection,
  type TrafficVehicleRequest,
  type VehicleId,
  trafficKey,
} from './trafficTypes';

export type TrafficVehicleRouteState = {
  vehicleId: VehicleId;
  path: readonly { x: number; y: number }[];
  offset: number;
  speed: number;
};

export type BuildTrafficRequestsInput = {
  tick: number;
  vehicles: readonly TrafficVehicleRouteState[];
  intersections: ReadonlyMap<string, TrafficIntersection>;
  lookaheadTiles?: number;
  stopDistanceTiles?: number;
  ticksPerTile?: number;
};

export type BuildTrafficRequestsResult = {
  requests: TrafficVehicleRequest[];
  unclassifiedTrafficRequests: number;
};

export function buildTrafficRequestsForVehicles(input: BuildTrafficRequestsInput): TrafficVehicleRequest[] {
  return buildTrafficRequestsForVehiclesWithDiagnostics(input).requests;
}

export function buildTrafficRequestsForVehiclesWithDiagnostics(input: BuildTrafficRequestsInput): BuildTrafficRequestsResult {
  const lookaheadTiles = input.lookaheadTiles ?? 2.25;
  const stopDistanceTiles = input.stopDistanceTiles ?? 0.42;
  const ticksPerTile = input.ticksPerTile ?? 8;
  let unclassifiedTrafficRequests = 0;

  const requests = input.vehicles.flatMap((vehicle) => {
    if (vehicle.path.length < 3) return [];
    const routeOffset = positiveModuloFloat(vehicle.offset, vehicle.path.length);
    const currentOffset = normalizeOffset(routeOffset, vehicle.path.length);
    const base = positiveModulo(Math.floor(routeOffset), vehicle.path.length);
    const fraction = routeOffset - Math.floor(routeOffset);

    for (let step = 1; step <= Math.ceil(lookaheadTiles) + 1; step += 1) {
      const pathIndex = (base + step) % vehicle.path.length;
      const coord = vehicle.path[pathIndex];
      const intersection = input.intersections.get(trafficKey(coord));
      if (!intersection) continue;

      const distanceToIntersection = step - fraction;
      if (distanceToIntersection < 0 || distanceToIntersection > lookaheadTiles) return [];

      const previous = vehicle.path[positiveModulo(pathIndex - 1, vehicle.path.length)];
      const next = vehicle.path[(pathIndex + 1) % vehicle.path.length];
      const approachEdge = directionForRoadStep(previous, coord);
      const exitEdge = directionForRoadStep(next, coord);
      if (!approachEdge || !exitEdge) {
        unclassifiedTrafficRequests += 1;
        return [];
      }

      const enterTick = input.tick + Math.max(
        1,
        Math.ceil((distanceToIntersection / Math.max(vehicle.speed, 0.1)) * ticksPerTile),
      );
      const exitTick = enterTick + Math.max(2, Math.ceil((1 / Math.max(0.1, vehicle.speed)) * ticksPerTile));
      return [{
        vehicleId: vehicle.vehicleId,
        intersectionId: intersection.intersectionId,
        distanceToIntersection: Number(distanceToIntersection.toFixed(3)),
        stopOffset: normalizeOffset(pathIndex - stopDistanceTiles, vehicle.path.length),
        currentOffset,
        enterTick,
        exitTick,
        approachEdge,
        exitEdge,
        conflictMask: 1,
        priority: stableVehiclePriority(vehicle.vehicleId),
      }];
    }

    return [];
  });

  return { requests, unclassifiedTrafficRequests };
}

function stableVehiclePriority(vehicleId: VehicleId): number {
  const id = Number(vehicleId.split(':')[1]);
  return Number.isFinite(id) ? id : 0;
}

function normalizeOffset(offset: number, pathLength: number): number {
  const rounded = Number(positiveModuloFloat(offset, pathLength).toFixed(3));
  return Number(positiveModuloFloat(rounded, pathLength).toFixed(3));
}

function positiveModulo(value: number, divisor: number): number {
  return ((value % divisor) + divisor) % divisor;
}

function positiveModuloFloat(value: number, divisor: number): number {
  return ((value % divisor) + divisor) % divisor;
}
