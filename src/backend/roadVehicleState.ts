import {
  isRoadVehicleDeltaDto,
  isRoadVehicleSnapshotDto,
  type RoadVehicleDeltaDto,
  type RoadVehicleDto,
  type RoadVehicleSnapshotDto,
} from './roadVehicleProtocol';

export type InterpolatedRoadVehicleEntry = {
  prev: RoadVehicleDto;
  current: RoadVehicleDto;
  lastTickAt: number;
};

export type RoadVehicleOverlayState = {
  tick: number;
  vehicles: Map<string, InterpolatedRoadVehicleEntry>;
  invalidMessages: number;
  lastUpdatedAt: number;
};

export function createRoadVehicleOverlayState(): RoadVehicleOverlayState {
  return { tick: 0, vehicles: new Map(), invalidMessages: 0, lastUpdatedAt: 0 };
}

function initEntry(dto: RoadVehicleDto, lastTickAt: number): InterpolatedRoadVehicleEntry {
  return { prev: dto, current: dto, lastTickAt };
}

export function applyRoadVehicleSnapshot(
  state: RoadVehicleOverlayState,
  snapshot: RoadVehicleSnapshotDto,
  now = Date.now(),
): RoadVehicleOverlayState {
  return {
    ...state,
    tick: snapshot.tick,
    vehicles: new Map(snapshot.vehicles.map((vehicle) => [vehicle.id, initEntry(vehicle, now)])),
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
    const previous = vehicles.get(vehicle.id);
    vehicles.set(vehicle.id, {
      prev: previous?.current ?? vehicle,
      current: vehicle,
      lastTickAt: now,
    });
  }
  return { ...state, tick: delta.tick, vehicles, lastUpdatedAt: now };
}

export function applyRoadVehicleMessage(
  state: RoadVehicleOverlayState,
  value: unknown,
  now = Date.now(),
): RoadVehicleOverlayState {
  if (isRoadVehicleDeltaDto(value)) return applyRoadVehicleDelta(state, value, now);
  if (isRoadVehicleSnapshotDto(value)) return applyRoadVehicleSnapshot(state, value, now);
  return { ...state, invalidMessages: state.invalidMessages + 1 };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function interpolatedRoadVehicles(
  state: RoadVehicleOverlayState,
  now: number,
  tickPeriodMs: number,
): RoadVehicleDto[] {
  const out: RoadVehicleDto[] = [];
  for (const entry of state.vehicles.values()) {
    const t = clamp((now - entry.lastTickAt) / tickPeriodMs, 0, 1);
    const x = entry.prev.world_coord.x + (entry.current.world_coord.x - entry.prev.world_coord.x) * t;
    const y = entry.prev.world_coord.y + (entry.current.world_coord.y - entry.prev.world_coord.y) * t;
    out.push({ ...entry.current, world_coord: { x, y } });
  }
  return out;
}
