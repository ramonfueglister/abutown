export type DirectionDto = 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'nw';

export type WorldCoordDto = { x: number; y: number };

export type AgentMobilityStateDto =
  | { type: 'at_activity'; activity_id: string }
  | { type: 'walking'; link_id: string; progress: number }
  | { type: 'waiting_at_stop'; stop_id: string }
  | { type: 'boarding'; vehicle_id: string; stop_id: string }
  | { type: 'in_vehicle'; vehicle_id: string; seat_index: number }
  | { type: 'alighting'; vehicle_id: string; stop_id: string };

export type AgentMobilityDto = {
  id: string;
  state: AgentMobilityStateDto;
  plan_cursor: number;
  world_coord: WorldCoordDto;
  direction: DirectionDto;
  sprite_key: string;
};

export type VehicleKindDto = 'car' | 'tram';

export type VehicleMobilityDto = {
  id: string;
  kind: VehicleKindDto;
  route_id: string;
  link_index: number;
  progress: number;
  capacity: number;
  occupants: string[];
  dwell_ticks_remaining: number;
  world_coord: WorldCoordDto;
  direction: DirectionDto;
  sprite_key: string;
};

export type StopMobilityDto = {
  id: string;
  route_id: string;
  link_index: number;
  progress: number;
  waiting_agents: string[];
};

export type MobilitySnapshotDto = {
  protocol_version: number;
  world_id: string;
  tick: number;
  agents: AgentMobilityDto[];
  vehicles: VehicleMobilityDto[];
  stops: StopMobilityDto[];
};

export type ChunkCoordDto = { x: number; y: number };

export type MobilityChunkDeltaDto = {
  type: 'mobility_chunk_delta';
  protocol_version: number;
  world_id: string;
  tick: number;
  chunk: ChunkCoordDto;
  changed_agents: AgentMobilityDto[];
  changed_vehicles: VehicleMobilityDto[];
  left_agents: string[];
  left_vehicles: string[];
};

export type MobilityChunkSnapshotDto = {
  type: 'mobility_chunk_snapshot';
  protocol_version: number;
  world_id: string;
  tick: number;
  chunk: ChunkCoordDto;
  agents: AgentMobilityDto[];
  vehicles: VehicleMobilityDto[];
};

export type ServerHelloDto = {
  type: 'hello';
  protocol_version: number;
  world_id: string;
  chunk_size: number;
};

export type TilePulseDeltaDto = {
  type: 'tile_pulse';
  protocol_version: number;
  world_id: string;
  tick: number;
  version: number;
  coord: { x: number; y: number };
  local_index: number;
};

export type ServerErrorDto = {
  type: 'error';
  protocol_version: number;
  world_id?: string | null;
  code: string;
  message: string;
};

export type ServerMessageDto =
  | ServerHelloDto
  | TilePulseDeltaDto
  | MobilityChunkDeltaDto
  | MobilityChunkSnapshotDto
  | ServerErrorDto;

export type ChunkSubscribeMessage = {
  type: 'chunk_subscribe';
  protocol_version: number;
  coords: ChunkCoordDto[];
};

export type ChunkUnsubscribeMessage = {
  type: 'chunk_unsubscribe';
  protocol_version: number;
  coords: ChunkCoordDto[];
};

export type ClientMessageDto = ChunkSubscribeMessage | ChunkUnsubscribeMessage;

// `encodeClientMessage` / `parseServerMessage` are gone with the binary wire
// migration. Use `toBinary(ClientMessageSchema, ...)` / `fromBinary(
// ServerMessageSchema, ...)` from `@bufbuild/protobuf` plus the converters
// in this module to bridge proto types ↔ legacy DTOs.

export type WorldSummaryDto = {
  protocol_version: number;
  world_id: string;
  chunk_size: number;
  loaded_chunks: ChunkCoordDto[];
  tick_period_ms: number;
};

export function isMobilitySnapshotDto(value: unknown): value is MobilitySnapshotDto {
  if (!isObject(value)) return false;
  return (
    isNumber(value.protocol_version) &&
    isString(value.world_id) &&
    isNumber(value.tick) &&
    Array.isArray(value.agents) &&
    value.agents.every(isAgentMobilityDto) &&
    Array.isArray(value.vehicles) &&
    value.vehicles.every(isVehicleMobilityDto) &&
    Array.isArray(value.stops) &&
    value.stops.every(isStopMobilityDto)
  );
}

export function isWorldSummaryDto(value: unknown): value is WorldSummaryDto {
  if (!isObject(value)) return false;
  if (
    !isNumber(value.protocol_version) ||
    !isString(value.world_id) ||
    !isNumber(value.chunk_size) ||
    !isNumber(value.tick_period_ms) ||
    value.tick_period_ms <= 0
  ) {
    return false;
  }
  if (!Array.isArray(value.loaded_chunks)) return false;
  return value.loaded_chunks.every((coord) => isObject(coord) && isNumber(coord.x) && isNumber(coord.y));
}

export function isMobilityChunkDeltaDto(value: unknown): value is MobilityChunkDeltaDto {
  if (!isObject(value)) return false;
  return (
    value.type === 'mobility_chunk_delta' &&
    isNumber(value.protocol_version) &&
    isString(value.world_id) &&
    isNumber(value.tick) &&
    isObject(value.chunk) &&
    isNumber(value.chunk.x) &&
    isNumber(value.chunk.y) &&
    Array.isArray(value.changed_agents) &&
    (value.changed_agents as unknown[]).every(isAgentMobilityDto) &&
    Array.isArray(value.changed_vehicles) &&
    (value.changed_vehicles as unknown[]).every(isVehicleMobilityDto) &&
    Array.isArray(value.left_agents) &&
    (value.left_agents as unknown[]).every(isString) &&
    Array.isArray(value.left_vehicles) &&
    (value.left_vehicles as unknown[]).every(isString)
  );
}

export function isMobilityChunkSnapshotDto(value: unknown): value is MobilityChunkSnapshotDto {
  if (!isObject(value)) return false;
  return (
    value.type === 'mobility_chunk_snapshot' &&
    isNumber(value.protocol_version) &&
    isString(value.world_id) &&
    isNumber(value.tick) &&
    isObject(value.chunk) &&
    isNumber(value.chunk.x) &&
    isNumber(value.chunk.y) &&
    Array.isArray(value.agents) &&
    (value.agents as unknown[]).every(isAgentMobilityDto) &&
    Array.isArray(value.vehicles) &&
    (value.vehicles as unknown[]).every(isVehicleMobilityDto)
  );
}

// `parseServerMessage` removed in the binary wire migration. Inbound frames
// are now decoded by `fromBinary(ServerMessageSchema, bytes)` in
// `mobilityClient.ts`, then handed to `applyServerMessage` in
// `mobilityState.ts` which translates the proto envelope into reducer calls.

function isAgentMobilityDto(value: unknown): value is AgentMobilityDto {
  if (!isObject(value)) return false;
  return (
    isString(value.id) &&
    isAgentMobilityStateDto(value.state) &&
    isNonNegativeInteger(value.plan_cursor) &&
    isWorldCoordDto(value.world_coord) &&
    isDirectionDto(value.direction) &&
    isString(value.sprite_key)
  );
}

function isAgentMobilityStateDto(value: unknown): value is AgentMobilityStateDto {
  if (!isObject(value) || !isString(value.type)) return false;
  if (value.type === 'at_activity') return isString(value.activity_id);
  if (value.type === 'walking') return isString(value.link_id) && isFiniteProgress(value.progress);
  if (value.type === 'waiting_at_stop') return isString(value.stop_id);
  if (value.type === 'boarding') return isString(value.vehicle_id) && isString(value.stop_id);
  if (value.type === 'in_vehicle') return isString(value.vehicle_id) && isNonNegativeInteger(value.seat_index);
  if (value.type === 'alighting') return isString(value.vehicle_id) && isString(value.stop_id);
  return false;
}

const VEHICLE_KINDS: ReadonlySet<VehicleKindDto> = new Set(['car', 'tram']);

function isVehicleKindDto(value: unknown): value is VehicleKindDto {
  return typeof value === 'string' && VEHICLE_KINDS.has(value as VehicleKindDto);
}

function isVehicleMobilityDto(value: unknown): value is VehicleMobilityDto {
  if (!isObject(value)) return false;
  return (
    isString(value.id) &&
    isVehicleKindDto(value.kind) &&
    isString(value.route_id) &&
    isNonNegativeInteger(value.link_index) &&
    isFiniteProgress(value.progress) &&
    isNonNegativeInteger(value.capacity) &&
    Array.isArray(value.occupants) &&
    value.occupants.every(isString) &&
    isNonNegativeInteger(value.dwell_ticks_remaining) &&
    isWorldCoordDto(value.world_coord) &&
    isDirectionDto(value.direction) &&
    isString(value.sprite_key)
  );
}

function isStopMobilityDto(value: unknown): value is StopMobilityDto {
  if (!isObject(value)) return false;
  return (
    isString(value.id) &&
    isString(value.route_id) &&
    isNonNegativeInteger(value.link_index) &&
    isFiniteProgress(value.progress) &&
    Array.isArray(value.waiting_agents) &&
    value.waiting_agents.every(isString)
  );
}

function isServerHelloDto(value: Record<string, unknown>): value is ServerHelloDto {
  return value.type === 'hello' && isNumber(value.protocol_version) && isString(value.world_id) && isNonNegativeInteger(value.chunk_size);
}

function isTilePulseDeltaDto(value: Record<string, unknown>): value is TilePulseDeltaDto {
  return (
    value.type === 'tile_pulse' &&
    isNumber(value.protocol_version) &&
    isString(value.world_id) &&
    isNumber(value.tick) &&
    isNumber(value.version) &&
    isObject(value.coord) &&
    isNumber(value.coord.x) &&
    isNumber(value.coord.y) &&
    isNonNegativeInteger(value.local_index)
  );
}

function isServerErrorDto(value: Record<string, unknown>): value is ServerErrorDto {
  return (
    value.type === 'error' &&
    isNumber(value.protocol_version) &&
    (value.world_id === undefined || value.world_id === null || isString(value.world_id)) &&
    isString(value.code) &&
    isString(value.message)
  );
}

export function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

export function isString(value: unknown): value is string {
  return typeof value === 'string';
}

export function isNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isNonNegativeInteger(value: unknown): value is number {
  return Number.isInteger(value) && Number(value) >= 0;
}

function isFiniteProgress(value: unknown): value is number {
  return isNumber(value) && value >= 0 && value <= 1;
}

const DIRECTIONS: ReadonlySet<DirectionDto> = new Set(['n', 'ne', 'e', 'se', 's', 'sw', 'w', 'nw']);

export function isDirectionDto(value: unknown): value is DirectionDto {
  return typeof value === 'string' && DIRECTIONS.has(value as DirectionDto);
}

export function isWorldCoordDto(value: unknown): value is WorldCoordDto {
  return isObject(value) && isNumber(value.x) && isNumber(value.y);
}

// ---------------------------------------------------------------------------
// proto → legacy DTO converters
// ---------------------------------------------------------------------------
//
// The MobilityOverlayState reducer (mobilityState.ts) and downstream render
// helpers still consume the snake_case DTO shape that predates the wire
// migration. Rather than rewrite every consumer (cosmetic churn that risks
// breaking the renderer), we keep the DTOs as the internal type and convert
// at the WS boundary in `applyServerMessage`.

import type {
  AgentMobility as AgentMobilityProto,
  AgentState as AgentStateProto,
  MobilityChunkDelta as MobilityChunkDeltaProto,
  MobilityChunkSnapshot as MobilityChunkSnapshotProto,
  VehicleMobility as VehicleMobilityProto,
} from './proto/abutown_pb';
import { Direction as DirectionProto, VehicleKind as VehicleKindProto } from './proto/abutown_pb';

const DIRECTION_PROTO_TO_DTO: Record<number, DirectionDto> = {
  [DirectionProto.N]: 'n',
  [DirectionProto.NE]: 'ne',
  [DirectionProto.E]: 'e',
  [DirectionProto.SE]: 'se',
  [DirectionProto.S]: 's',
  [DirectionProto.SW]: 'sw',
  [DirectionProto.W]: 'w',
  [DirectionProto.NW]: 'nw',
};

export function directionFromProto(value: DirectionProto): DirectionDto {
  return DIRECTION_PROTO_TO_DTO[value] ?? 'e';
}

function vehicleKindFromProto(value: VehicleKindProto): VehicleKindDto {
  if (value === VehicleKindProto.TRAM) return 'tram';
  return 'car';
}

function agentStateFromProto(state: AgentStateProto | undefined): AgentMobilityStateDto {
  if (!state || state.state.case === undefined) {
    // Backend should always send a populated AgentState; fall back to
    // at_activity with empty id to keep the reducer alive on malformed frames.
    return { type: 'at_activity', activity_id: '' };
  }
  switch (state.state.case) {
    case 'walking':
      return { type: 'walking', link_id: state.state.value.linkId, progress: state.state.value.progress };
    case 'waitingAtStop':
      return { type: 'waiting_at_stop', stop_id: state.state.value.stopId };
    case 'inVehicle':
      return { type: 'in_vehicle', vehicle_id: state.state.value.vehicleId, seat_index: state.state.value.seatIndex };
    case 'boarding':
      return { type: 'boarding', vehicle_id: state.state.value.vehicleId, stop_id: state.state.value.stopId };
    case 'alighting':
      return { type: 'alighting', vehicle_id: state.state.value.vehicleId, stop_id: state.state.value.stopId };
    case 'atActivity':
      return { type: 'at_activity', activity_id: state.state.value.activityId };
  }
}

export function agentMobilityFromProto(p: AgentMobilityProto): AgentMobilityDto {
  return {
    id: p.id,
    state: agentStateFromProto(p.state),
    plan_cursor: p.planCursor,
    world_coord: { x: p.worldCoord?.x ?? 0, y: p.worldCoord?.y ?? 0 },
    direction: directionFromProto(p.direction),
    sprite_key: p.spriteKey,
  };
}

export function vehicleMobilityFromProto(p: VehicleMobilityProto): VehicleMobilityDto {
  return {
    id: p.id,
    kind: vehicleKindFromProto(p.kind),
    route_id: p.routeId,
    link_index: p.linkIndex,
    progress: p.progress,
    capacity: p.capacity,
    occupants: [...p.occupants],
    dwell_ticks_remaining: p.dwellTicksRemaining,
    world_coord: { x: p.worldCoord?.x ?? 0, y: p.worldCoord?.y ?? 0 },
    direction: directionFromProto(p.direction),
    sprite_key: p.spriteKey,
  };
}

export function mobilityChunkDeltaFromProto(p: MobilityChunkDeltaProto): MobilityChunkDeltaDto {
  return {
    type: 'mobility_chunk_delta',
    protocol_version: p.protocolVersion,
    world_id: p.worldId,
    tick: Number(p.tick),
    chunk: { x: p.chunk?.x ?? 0, y: p.chunk?.y ?? 0 },
    changed_agents: p.changedAgents.map(agentMobilityFromProto),
    changed_vehicles: p.changedVehicles.map(vehicleMobilityFromProto),
    left_agents: [...p.leftAgents],
    left_vehicles: [...p.leftVehicles],
  };
}

export function mobilityChunkSnapshotFromProto(p: MobilityChunkSnapshotProto): MobilityChunkSnapshotDto {
  return {
    type: 'mobility_chunk_snapshot',
    protocol_version: p.protocolVersion,
    world_id: p.worldId,
    tick: Number(p.tick),
    chunk: { x: p.chunk?.x ?? 0, y: p.chunk?.y ?? 0 },
    agents: p.agents.map(agentMobilityFromProto),
    vehicles: p.vehicles.map(vehicleMobilityFromProto),
  };
}
