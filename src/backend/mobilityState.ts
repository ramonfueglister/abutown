import {
  isMobilityDeltaDto,
  parseServerMessage,
  type AgentMobilityDto,
  type MobilityDeltaDto,
  type MobilitySnapshotDto,
  type StopMobilityDto,
  type VehicleMobilityDto,
} from './mobilityProtocol';
import {
  applyRoadVehicleSnapshot,
  applyRoadVehicleDelta,
  createRoadVehicleOverlayState,
  type RoadVehicleOverlayState,
} from './roadVehicleState';
import type { RoadVehicleSnapshotDto } from './roadVehicleProtocol';

export type MobilityConnectionStatus = 'connecting' | 'connected' | 'disconnected';

export type MobilityDiagnostics = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: number;
  vehicles: number;
  stops: number;
  roadVehicles: number;
  invalidMessages: number;
  lastError: string | null;
};

export type MobilityOverlayState = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: Map<string, AgentMobilityDto>;
  vehicles: Map<string, VehicleMobilityDto>;
  stops: Map<string, StopMobilityDto>;
  roadVehicles: RoadVehicleOverlayState;
  invalidMessages: number;
  lastError: string | null;
  lastUpdatedAt: number;
};

export function createMobilityOverlayState(): MobilityOverlayState {
  return {
    status: 'disconnected',
    tick: 0,
    agents: new Map(),
    vehicles: new Map(),
    stops: new Map(),
    roadVehicles: createRoadVehicleOverlayState(),
    invalidMessages: 0,
    lastError: null,
    lastUpdatedAt: 0,
  };
}

export function applyRoadVehicleSnapshotToState(
  state: MobilityOverlayState,
  snapshot: RoadVehicleSnapshotDto,
  now = Date.now(),
): MobilityOverlayState {
  return {
    ...state,
    roadVehicles: applyRoadVehicleSnapshot(state.roadVehicles, snapshot, now),
    lastUpdatedAt: now,
  };
}

export function markMobilityConnecting(state: MobilityOverlayState, now = Date.now()): MobilityOverlayState {
  return {
    ...state,
    status: 'connecting',
    lastError: null,
    lastUpdatedAt: now,
  };
}

export function markMobilityDisconnected(state: MobilityOverlayState, error: string | null, now = Date.now()): MobilityOverlayState {
  return {
    ...state,
    status: 'disconnected',
    lastError: error,
    lastUpdatedAt: now,
  };
}

export function applyMobilitySnapshot(state: MobilityOverlayState, snapshot: MobilitySnapshotDto, now = Date.now()): MobilityOverlayState {
  return {
    ...state,
    status: 'connected',
    tick: snapshot.tick,
    agents: new Map(snapshot.agents.map((agent) => [agent.id, agent])),
    vehicles: new Map(snapshot.vehicles.map((vehicle) => [vehicle.id, vehicle])),
    stops: new Map(snapshot.stops.map((stop) => [stop.id, stop])),
    lastError: null,
    lastUpdatedAt: now,
  };
}

export function applyMobilityDelta(state: MobilityOverlayState, delta: MobilityDeltaDto, now = Date.now()): MobilityOverlayState {
  const agents = new Map(state.agents);
  const vehicles = new Map(state.vehicles);
  for (const agent of delta.changed_agents) agents.set(agent.id, agent);
  for (const vehicle of delta.changed_vehicles) vehicles.set(vehicle.id, vehicle);

  return {
    ...state,
    status: 'connected',
    tick: delta.tick,
    agents,
    vehicles,
    lastError: null,
    lastUpdatedAt: now,
  };
}

export function applyServerMessage(state: MobilityOverlayState, value: unknown, now = Date.now()): MobilityOverlayState {
  const message = parseServerMessage(value);
  if (message?.type === 'mobility_delta' && isMobilityDeltaDto(message)) {
    return applyMobilityDelta(state, message, now);
  }
  if (message?.type === 'road_vehicle_delta') {
    return {
      ...state,
      roadVehicles: applyRoadVehicleDelta(state.roadVehicles, message, now),
      lastUpdatedAt: now,
    };
  }
  if (message !== null) return state;
  return {
    ...state,
    invalidMessages: state.invalidMessages + 1,
    lastUpdatedAt: now,
  };
}

export function mobilityDiagnostics(state: MobilityOverlayState): MobilityDiagnostics {
  return {
    status: state.status,
    tick: state.tick,
    agents: state.agents.size,
    vehicles: state.vehicles.size,
    stops: state.stops.size,
    roadVehicles: state.roadVehicles.vehicles.size,
    invalidMessages: state.invalidMessages,
    lastError: state.lastError,
  };
}
