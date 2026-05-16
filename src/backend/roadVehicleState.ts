import {
  isRoadVehicleDeltaDto,
  isRoadVehicleSnapshotDto,
  type RoadVehicleDeltaDto,
  type RoadVehicleDto,
  type RoadVehicleSnapshotDto,
} from './roadVehicleProtocol';

export type RoadVehicleOverlayState = {
  tick: number;
  vehicles: Map<string, RoadVehicleDto>;
  invalidMessages: number;
  lastUpdatedAt: number;
};

export function createRoadVehicleOverlayState(): RoadVehicleOverlayState {
  return { tick: 0, vehicles: new Map(), invalidMessages: 0, lastUpdatedAt: 0 };
}

export function applyRoadVehicleSnapshot(
  state: RoadVehicleOverlayState,
  snapshot: RoadVehicleSnapshotDto,
  now = Date.now(),
): RoadVehicleOverlayState {
  return {
    ...state,
    tick: snapshot.tick,
    vehicles: new Map(snapshot.vehicles.map((vehicle) => [vehicle.id, vehicle])),
    lastUpdatedAt: now,
  };
}

export function applyRoadVehicleDelta(
  state: RoadVehicleOverlayState,
  delta: RoadVehicleDeltaDto,
  now = Date.now(),
): RoadVehicleOverlayState {
  const vehicles = new Map(state.vehicles);
  for (const vehicle of delta.changed) {
    vehicles.set(vehicle.id, vehicle);
  }
  return { ...state, tick: delta.tick, vehicles, lastUpdatedAt: now };
}

export function applyRoadVehicleMessage(
  state: RoadVehicleOverlayState,
  value: unknown,
  now = Date.now(),
): RoadVehicleOverlayState {
  if (isRoadVehicleDeltaDto(value)) {
    return applyRoadVehicleDelta(state, value, now);
  }
  if (isRoadVehicleSnapshotDto(value)) {
    return applyRoadVehicleSnapshot(state, value, now);
  }
  return { ...state, invalidMessages: state.invalidMessages + 1 };
}
