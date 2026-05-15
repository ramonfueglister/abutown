import {
  isMobilityDeltaDto,
  parseServerMessage,
  type AgentMobilityDto,
  type MobilityDeltaDto,
  type MobilitySnapshotDto,
  type StopMobilityDto,
  type VehicleMobilityDto,
} from './mobilityProtocol';

export type MobilityConnectionStatus = 'connecting' | 'connected' | 'disconnected';

export type MobilityCoord = {
  x: number;
  y: number;
};

export type MobilityMarker = {
  id: string;
  kind: 'agent' | 'vehicle' | 'stop';
  coord: MobilityCoord;
  label: string;
  state: string;
};

export type MobilityDiagnostics = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: number;
  vehicles: number;
  stops: number;
  invalidMessages: number;
  seededAgentState: string | null;
  lastError: string | null;
};

export type MobilityOverlayState = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: Map<string, AgentMobilityDto>;
  vehicles: Map<string, VehicleMobilityDto>;
  stops: Map<string, StopMobilityDto>;
  invalidMessages: number;
  lastError: string | null;
  lastUpdatedAt: number;
};

const DEMO_LINKS = new Map<string, { from: MobilityCoord; to: MobilityCoord }>([
  ['link:home-to-old-town-stop', { from: { x: 124, y: 132 }, to: { x: 126, y: 130 } }],
  ['link:old-town-to-station', { from: { x: 126, y: 130 }, to: { x: 128, y: 128 } }],
  ['link:station-to-work', { from: { x: 128, y: 128 }, to: { x: 130, y: 126 } }],
]);

const DEMO_STOPS = new Map<string, MobilityCoord>([
  ['stop:old-town', { x: 126, y: 130 }],
  ['stop:station', { x: 128, y: 128 }],
]);

const DEMO_ACTIVITIES = new Map<string, MobilityCoord>([['activity:work', { x: 130, y: 126 }]]);

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
  if (message !== null) return state;
  return {
    ...state,
    invalidMessages: state.invalidMessages + 1,
    lastUpdatedAt: now,
  };
}

export function mobilityMarkers(state: MobilityOverlayState): MobilityMarker[] {
  if (state.status === 'disconnected' && state.agents.size === 0 && state.vehicles.size === 0) return [];

  const stops = [...state.stops.values()]
    .map(stopMarker)
    .filter((marker): marker is MobilityMarker => marker !== null);
  const vehicles = [...state.vehicles.values()]
    .map(vehicleMarker)
    .filter((marker): marker is MobilityMarker => marker !== null);
  const agents = [...state.agents.values()]
    .map((agent) => agentMarker(agent, state.vehicles))
    .filter((marker): marker is MobilityMarker => marker !== null);

  return [...stops, ...vehicles, ...agents];
}

export function mobilityDiagnostics(state: MobilityOverlayState): MobilityDiagnostics {
  const seededAgent = state.agents.get('agent:seed:0');
  return {
    status: state.status,
    tick: state.tick,
    agents: state.agents.size,
    vehicles: state.vehicles.size,
    stops: state.stops.size,
    invalidMessages: state.invalidMessages,
    seededAgentState: seededAgent?.state.type ?? null,
    lastError: state.lastError,
  };
}

function stopMarker(stop: StopMobilityDto): MobilityMarker | null {
  const coord = DEMO_STOPS.get(stop.id) ?? routeCoord(stop.route_id, stop.link_index, stop.progress);
  if (!coord) return null;
  return {
    id: stop.id,
    kind: 'stop',
    coord,
    label: String(stop.waiting_agents.length),
    state: 'stop',
  };
}

function vehicleMarker(vehicle: VehicleMobilityDto): MobilityMarker | null {
  const coord = routeCoord(vehicle.route_id, vehicle.link_index, vehicle.progress);
  if (!coord) return null;
  return {
    id: vehicle.id,
    kind: 'vehicle',
    coord,
    label: String(vehicle.occupants.length),
    state: 'vehicle',
  };
}

function agentMarker(agent: AgentMobilityDto, vehicles: Map<string, VehicleMobilityDto>): MobilityMarker | null {
  const coord = agentCoord(agent, vehicles);
  if (!coord) return null;
  return {
    id: agent.id,
    kind: 'agent',
    coord,
    label: agent.id.replace('agent:', ''),
    state: agent.state.type,
  };
}

function agentCoord(agent: AgentMobilityDto, vehicles: Map<string, VehicleMobilityDto>): MobilityCoord | null {
  const state = agent.state;
  if (state.type === 'walking') return linkCoord(state.link_id, state.progress);
  if (state.type === 'waiting_at_stop') return DEMO_STOPS.get(state.stop_id) ?? null;
  if (state.type === 'boarding') return DEMO_STOPS.get(state.stop_id) ?? null;
  if (state.type === 'alighting') return DEMO_STOPS.get(state.stop_id) ?? null;
  if (state.type === 'at_activity') return DEMO_ACTIVITIES.get(state.activity_id) ?? null;
  if (state.type === 'in_vehicle') {
    const vehicle = vehicles.get(state.vehicle_id);
    return vehicle ? routeCoord(vehicle.route_id, vehicle.link_index, vehicle.progress) : null;
  }
  return null;
}

function routeCoord(routeId: string, linkIndex: number, progress: number): MobilityCoord | null {
  if (routeId === 'route:old-town-loop' && linkIndex === 0) {
    return linkCoord('link:old-town-to-station', progress);
  }
  return null;
}

function linkCoord(linkId: string, progress: number): MobilityCoord | null {
  const link = DEMO_LINKS.get(linkId);
  if (!link) return null;
  const t = Math.max(0, Math.min(1, progress));
  return {
    x: Math.round(link.from.x + (link.to.x - link.from.x) * t),
    y: Math.round(link.from.y + (link.to.y - link.from.y) * t),
  };
}
