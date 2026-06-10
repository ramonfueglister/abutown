export type DirectionDto = 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'nw';

export type WorldCoordDto = { x: number; y: number };

export type AgentMobilityStateDto =
  | { type: 'at_activity'; activity_id: string }
  | { type: 'walking'; link_id: string; progress: number }
  | { type: 'waiting_at_stop'; stop_id: string }
  | { type: 'in_vehicle'; vehicle_id: string; seat_index: number };

export type AgentMobilityDto = {
  id: string;
  state: AgentMobilityStateDto;
  plan_cursor: number;
  world_coord: WorldCoordDto;
  direction: DirectionDto;
  sprite_key: string;
  age_seconds: number; // elapsed sim-seconds; 0 when not provided by the backend
};

export type VehicleKindDto = 'car';

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
  sim_time: number;
};

export type BackendPersistenceStatusDto = 'starting' | 'healthy' | 'degraded' | 'stale';

export type BackendPersistenceHealthDto = {
  status: BackendPersistenceStatusDto;
  world_id: string;
  mobility_tick: number;
  last_attempt_unix_ms: number | null;
  last_success_unix_ms: number | null;
  consecutive_failures: number;
  last_error: string | null;
  freshness_ms: number | null;
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
  if (value.type === 'in_vehicle') return isString(value.vehicle_id) && isNonNegativeInteger(value.seat_index);
  return false;
}

const VEHICLE_KINDS: ReadonlySet<VehicleKindDto> = new Set(['car']);

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
  ChunkCoord as ChunkCoordProto,
  EconomySnapshot,
  HealthResponse as HealthResponseProto,
  MobilityChunkDelta as MobilityChunkDeltaProto,
  MobilityChunkSnapshot as MobilityChunkSnapshotProto,
  MobilitySnapshot as MobilitySnapshotProto,
  Stop as StopProto,
  VehicleMobility as VehicleMobilityProto,
  WorldSummary as WorldSummaryProto,
} from './proto/abutown_pb';
import { Direction as DirectionProto, PersistenceHealthStatus, VehicleKind as VehicleKindProto } from './proto/abutown_pb';

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
  const direction = DIRECTION_PROTO_TO_DTO[value];
  if (!direction) {
    throw new Error('missing direction');
  }
  return direction;
}

function vehicleKindFromProto(value: VehicleKindProto): VehicleKindDto {
  if (value === VehicleKindProto.CAR) return 'car';
  throw new Error('unsupported vehicle kind');
}

function chunkCoordFromProto(chunk: ChunkCoordProto | undefined): ChunkCoordDto {
  if (!chunk) {
    throw new Error('missing chunk');
  }
  return { x: chunk.x, y: chunk.y };
}

function agentStateFromProto(state: AgentStateProto | undefined): AgentMobilityStateDto {
  if (!state || state.state.case === undefined) {
    throw new Error('missing AgentState');
  }
  switch (state.state.case) {
    case 'walking': {
      const { linkId, progress } = state.state.value;
      if (!isString(linkId) || linkId.length === 0 || !isFiniteProgress(progress)) {
        throw new Error('invalid walking state');
      }
      return { type: 'walking', link_id: linkId, progress };
    }
    case 'waitingAtStop':
      return { type: 'waiting_at_stop', stop_id: state.state.value.stopId };
    case 'inVehicle':
      return { type: 'in_vehicle', vehicle_id: state.state.value.vehicleId, seat_index: state.state.value.seatIndex };
    case 'atActivity':
      return { type: 'at_activity', activity_id: state.state.value.activityId };
  }
}

export function agentMobilityFromProto(p: AgentMobilityProto): AgentMobilityDto {
  if (!p.worldCoord) {
    throw new Error('missing world_coord');
  }
  return {
    id: p.id,
    state: agentStateFromProto(p.state),
    plan_cursor: p.planCursor,
    world_coord: { x: p.worldCoord.x, y: p.worldCoord.y },
    direction: directionFromProto(p.direction),
    sprite_key: p.spriteKey,
    age_seconds: Number(p.ageSeconds),
  };
}

export function vehicleMobilityFromProto(p: VehicleMobilityProto): VehicleMobilityDto {
  if (!p.worldCoord) {
    throw new Error('missing world_coord');
  }
  return {
    id: p.id,
    kind: vehicleKindFromProto(p.kind),
    route_id: p.routeId,
    link_index: p.linkIndex,
    progress: p.progress,
    capacity: p.capacity,
    occupants: [...p.occupants],
    dwell_ticks_remaining: p.dwellTicksRemaining,
    world_coord: { x: p.worldCoord.x, y: p.worldCoord.y },
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
    chunk: chunkCoordFromProto(p.chunk),
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
    chunk: chunkCoordFromProto(p.chunk),
    agents: p.agents.map(agentMobilityFromProto),
    vehicles: p.vehicles.map(vehicleMobilityFromProto),
  };
}

// ===== HTTP-endpoint proto → DTO converters (Task 6) =====
//
// The /health, /world, and /mobility HTTP endpoints now return binary
// protobuf bodies (`application/x-protobuf`). These helpers decode the
// generated proto types into the legacy DTO shapes the rest of the
// frontend consumes.

export function healthResponseFromProto(p: HealthResponseProto): {
  service: string;
  world_id: string;
  ok: boolean;
  protocol_version: number;
  persistence?: BackendPersistenceHealthDto;
} {
  return {
    service: p.service,
    world_id: p.worldId,
    ok: p.ok,
    protocol_version: p.protocolVersion,
    persistence: p.persistence
      ? {
          status: persistenceStatusFromProto(p.persistence.status),
          world_id: p.persistence.worldId,
          mobility_tick: Number(p.persistence.mobilityTick),
          last_attempt_unix_ms: positiveNumberOrNull(p.persistence.lastAttemptUnixMs),
          last_success_unix_ms: positiveNumberOrNull(p.persistence.lastSuccessUnixMs),
          consecutive_failures: p.persistence.consecutiveFailures,
          last_error: p.persistence.lastError.length > 0 ? p.persistence.lastError : null,
          freshness_ms: positiveNumberOrNull(p.persistence.freshnessMs),
        }
      : undefined,
  };
}

function persistenceStatusFromProto(value: PersistenceHealthStatus): BackendPersistenceStatusDto {
  if (value === PersistenceHealthStatus.STARTING) return 'starting';
  if (value === PersistenceHealthStatus.HEALTHY) return 'healthy';
  if (value === PersistenceHealthStatus.DEGRADED) return 'degraded';
  if (value === PersistenceHealthStatus.STALE) return 'stale';
  throw new Error('unsupported persistence health status');
}

function positiveNumberOrNull(value: bigint | number): number | null {
  const n = Number(value);
  return Number.isFinite(n) && n > 0 ? n : null;
}

export function worldSummaryFromProto(p: WorldSummaryProto): WorldSummaryDto {
  return {
    protocol_version: p.protocolVersion,
    world_id: p.worldId,
    chunk_size: p.chunkSize,
    loaded_chunks: p.loadedChunks.map((c) => ({ x: c.x, y: c.y })),
    tick_period_ms: p.tickPeriodMs,
    sim_time: Number(p.simTime),
  };
}

function stopFromProto(p: StopProto): StopMobilityDto {
  return {
    id: p.id,
    route_id: p.routeId,
    link_index: p.linkIndex,
    progress: p.progress,
    waiting_agents: [...p.waitingAgents],
  };
}

export function mobilitySnapshotFromProto(p: MobilitySnapshotProto): MobilitySnapshotDto {
  return {
    protocol_version: p.protocolVersion,
    world_id: p.worldId,
    tick: Number(p.tick),
    agents: p.agents.map(agentMobilityFromProto),
    vehicles: p.vehicles.map(vehicleMobilityFromProto),
    stops: p.stops.map(stopFromProto),
  };
}

// ---------------------------------------------------------------------------
// Economy DTO types + proto → DTO converter
// ---------------------------------------------------------------------------

export type MarketLocationDto = { marketId: number; name: string; tileX: number; tileY: number; wagePaidLastTick: number };
export type MarketGoodDto = { marketId: number; goodId: number; lastSettlementPrice: number; ewmaReferencePrice: number; tradedQtyLastTick: number; unmetDemandLastTick: number; unsoldSupplyLastTick: number };
export type EconomyFlowDto = { srcMarketId: number; dstMarketId: number; goodId: number; rate: number };
export type EconomySnapshotDto = { tick: number; markets: MarketLocationDto[]; goods: MarketGoodDto[]; flows: EconomyFlowDto[] };

export function economySnapshotFromProto(p: EconomySnapshot): EconomySnapshotDto {
  return {
    tick: Number(p.tick),
    markets: p.markets.map((m) => ({ marketId: m.marketId, name: m.name, tileX: m.tileX, tileY: m.tileY, wagePaidLastTick: Number(m.wagePaidLastTick) })),
    goods: p.goods.map((g) => ({ marketId: g.marketId, goodId: g.goodId, lastSettlementPrice: Number(g.lastSettlementPrice), ewmaReferencePrice: Number(g.ewmaReferencePrice), tradedQtyLastTick: Number(g.tradedQtyLastTick), unmetDemandLastTick: Number(g.unmetDemandLastTick), unsoldSupplyLastTick: Number(g.unsoldSupplyLastTick) })),
    flows: p.flows.map((f) => ({ srcMarketId: f.srcMarketId, dstMarketId: f.dstMarketId, goodId: f.goodId, rate: Number(f.rate) })),
  };
}
