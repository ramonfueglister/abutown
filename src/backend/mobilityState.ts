import {
  mobilityChunkDeltaFromProto,
  mobilityChunkSnapshotFromProto,
  type AgentMobilityDto,
  type MobilityChunkDeltaDto,
  type MobilityChunkSnapshotDto,
  type MobilitySnapshotDto,
  type StopMobilityDto,
  type VehicleMobilityDto,
} from './mobilityProtocol';
import type { ServerMessage } from './proto/abutown_pb';

const CHUNK_SIZE = 32;

function chunkOfWorldCoord(coord: { x: number; y: number } | undefined): string {
  if (!coord) return '';
  return `${Math.floor(coord.x / CHUNK_SIZE)},${Math.floor(coord.y / CHUNK_SIZE)}`;
}

export type MobilityConnectionStatus = 'connecting' | 'connected' | 'disconnected';

export type InterpolatedEntry<T> = {
  prev: T;
  current: T;
  lastTickAt: number;
};

export type MobilityDiagnostics = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: number;
  vehicles: number;
  stops: number;
  invalidMessages: number;
  lastError: string | null;
};

export type TrafficDiagnostics = {
  routes: number;
  cars: number;
  movingCars: number;
  stuckCars: number;
  invalidRouteCars: number;
};

export type MobilityOverlayState = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: Map<string, InterpolatedEntry<AgentMobilityDto>>;
  vehicles: Map<string, InterpolatedEntry<VehicleMobilityDto>>;
  stops: Map<string, StopMobilityDto>;
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
    invalidMessages: 0,
    lastError: null,
    lastUpdatedAt: 0,
  };
}

export function markMobilityConnecting(state: MobilityOverlayState, now = Date.now()): MobilityOverlayState {
  return { ...state, status: 'connecting', lastError: null, lastUpdatedAt: now };
}

export function markMobilityDisconnected(
  state: MobilityOverlayState,
  error: string | null,
  now = Date.now(),
): MobilityOverlayState {
  return { ...state, status: 'disconnected', lastError: error, lastUpdatedAt: now };
}

function initEntry<T>(dto: T, lastTickAt: number): InterpolatedEntry<T> {
  return { prev: dto, current: dto, lastTickAt };
}

export function applyMobilitySnapshot(
  state: MobilityOverlayState,
  snapshot: MobilitySnapshotDto,
  now = Date.now(),
): MobilityOverlayState {
  return {
    ...state,
    status: 'connected',
    tick: snapshot.tick,
    agents: new Map(snapshot.agents.map((agent) => [agent.id, initEntry(agent, now)])),
    vehicles: new Map(snapshot.vehicles.map((vehicle) => [vehicle.id, initEntry(vehicle, now)])),
    stops: new Map(snapshot.stops.map((stop) => [stop.id, stop])),
    lastError: null,
    lastUpdatedAt: now,
  };
}

export function applyMobilityChunkSnapshot(
  state: MobilityOverlayState,
  msg: MobilityChunkSnapshotDto,
  now = Date.now(),
): MobilityOverlayState {
  if (msg.tick < state.tick) return state;

  const chunkKey = `${msg.chunk.x},${msg.chunk.y}`;
  const agents = new Map(state.agents);
  for (const [id, entry] of agents) {
    if (chunkOfWorldCoord(entry.current.world_coord) === chunkKey) {
      agents.delete(id);
    }
  }
  for (const dto of msg.agents) {
    agents.set(dto.id, { prev: dto, current: dto, lastTickAt: now });
  }
  const vehicles = new Map(state.vehicles);
  for (const [id, entry] of vehicles) {
    if (chunkOfWorldCoord(entry.current.world_coord) === chunkKey) {
      vehicles.delete(id);
    }
  }
  for (const dto of msg.vehicles) {
    vehicles.set(dto.id, { prev: dto, current: dto, lastTickAt: now });
  }
  return {
    ...state,
    status: 'connected',
    tick: msg.tick,
    agents,
    vehicles,
    lastError: null,
    lastUpdatedAt: now,
  };
}

export function applyMobilityChunkDelta(
  state: MobilityOverlayState,
  msg: MobilityChunkDeltaDto,
  now = Date.now(),
): MobilityOverlayState {
  if (msg.tick < state.tick) return state;

  const agents = new Map(state.agents);
  for (const id of msg.left_agents) agents.delete(id);
  for (const agent of msg.changed_agents) {
    const previous = agents.get(agent.id);
    agents.set(agent.id, {
      prev: previous?.current ?? agent,
      current: agent,
      lastTickAt: now,
    });
  }
  const vehicles = new Map(state.vehicles);
  for (const id of msg.left_vehicles) {
    vehicles.delete(id);
  }
  for (const vehicle of msg.changed_vehicles) {
    const previous = vehicles.get(vehicle.id);
    vehicles.set(vehicle.id, {
      prev: previous?.current ?? vehicle,
      current: vehicle,
      lastTickAt: now,
    });
  }
  return {
    ...state,
    status: 'connected',
    tick: msg.tick,
    agents,
    vehicles,
    lastError: null,
    lastUpdatedAt: now,
  };
}

export function applyServerMessage(
  state: MobilityOverlayState,
  message: ServerMessage,
  now = Date.now(),
): MobilityOverlayState {
  switch (message.body.case) {
    case 'mobilityChunkSnapshot':
      return applyMobilityChunkSnapshot(state, mobilityChunkSnapshotFromProto(message.body.value), now);
    case 'mobilityChunkDelta':
      return applyMobilityChunkDelta(state, mobilityChunkDeltaFromProto(message.body.value), now);
    case 'hello':
      // The bridge marks the connection live via markMobilityConnected on the
      // initial snapshot fetch; the wire Hello is informational for now.
      return state;
    case 'tilePulse':
      // Consumed elsewhere (tile rendering); mobility overlay ignores it.
      return state;
    case 'economySnapshot':
      return state; // economy rides a separate overlay state; mobility ignores it
    case 'worldEvent':
      // No mobility-side reducer; tile-kind set events are handled by the
      // tile/chunk layer.
      return state;
    case 'error':
      // eslint-disable-next-line no-console
      console.warn('mobility server error', message.body.value.code, message.body.value.message);
      return state;
    case undefined:
      // Malformed envelope (no oneof set). Bump diagnostics so the UI can
      // surface "something arrived but we couldn't route it".
      return { ...state, invalidMessages: state.invalidMessages + 1, lastUpdatedAt: now };
  }
}

export function mobilityDiagnostics(state: MobilityOverlayState): MobilityDiagnostics {
  return {
    status: state.status,
    tick: state.tick,
    agents: state.agents.size,
    vehicles: state.vehicles.size,
    stops: state.stops.size,
    invalidMessages: state.invalidMessages,
    lastError: state.lastError,
  };
}

function moved(entry: InterpolatedEntry<VehicleMobilityDto>): boolean {
  return (
    Math.abs(entry.current.world_coord.x - entry.prev.world_coord.x) > 0.001 ||
    Math.abs(entry.current.world_coord.y - entry.prev.world_coord.y) > 0.001
  );
}

export function trafficDiagnostics(state: MobilityOverlayState): TrafficDiagnostics {
  const cars = [...state.vehicles.values()].filter((entry) => entry.current.kind === 'car');
  const movingCars = cars.filter(moved).length;
  return {
    routes: new Set(cars.map((entry) => entry.current.route_id)).size,
    cars: cars.length,
    movingCars,
    stuckCars: cars.length - movingCars,
    invalidRouteCars: state.invalidMessages,
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function lerpCoord(
  prev: { x: number; y: number },
  current: { x: number; y: number },
  t: number,
): { x: number; y: number } {
  return {
    x: prev.x + (current.x - prev.x) * t,
    y: prev.y + (current.y - prev.y) * t,
  };
}

export function interpolatedAgents(
  state: MobilityOverlayState,
  now: number,
  tickPeriodMs: number,
): AgentMobilityDto[] {
  const out: AgentMobilityDto[] = [];
  for (const entry of state.agents.values()) {
    const t = clamp((now - entry.lastTickAt) / tickPeriodMs, 0, 1);
    out.push({
      ...entry.current,
      world_coord: lerpCoord(entry.prev.world_coord, entry.current.world_coord, t),
    });
  }
  return out;
}

export function interpolatedVehicles(
  state: MobilityOverlayState,
  now: number,
  tickPeriodMs: number,
): VehicleMobilityDto[] {
  const out: VehicleMobilityDto[] = [];
  for (const entry of state.vehicles.values()) {
    const t = clamp((now - entry.lastTickAt) / tickPeriodMs, 0, 1);
    out.push({
      ...entry.current,
      world_coord: lerpCoord(entry.prev.world_coord, entry.current.world_coord, t),
    });
  }
  return out;
}
