import {
  CLIENT_PROTOCOL_VERSION,
  type ChunkCoordDto,
  type ChunkSnapshotDto,
  type HealthResponse,
  type ServerMessage,
  type WorldSummaryDto,
} from './protocol';

export type BackendOverlayStatus = 'idle' | 'connecting' | 'snapshot' | 'live' | 'disconnected' | 'incompatible';

export type BackendPulse = {
  coord: ChunkCoordDto;
  localIndex: number;
  tick: number;
  version: number;
  receivedAtMs: number;
};

export type LoadedBackendChunk = {
  coord: ChunkCoordDto;
  state: string;
  version: number;
  tileCount: number;
};

export type BackendOverlayState = {
  status: BackendOverlayStatus;
  protocolVersion: number;
  worldId?: string;
  service?: string;
  ok: boolean;
  chunkSize?: number;
  loadedChunk?: LoadedBackendChunk;
  latestTick?: number;
  latestVersion?: number;
  pulses: BackendPulse[];
  warning?: string;
};

export function createInitialBackendOverlayState(): BackendOverlayState {
  return {
    status: 'idle',
    protocolVersion: CLIENT_PROTOCOL_VERSION,
    ok: false,
    pulses: [],
  };
}

export function markBackendConnecting(state: BackendOverlayState): BackendOverlayState {
  return { ...state, status: 'connecting', warning: undefined };
}

export function markBackendDisconnected(state: BackendOverlayState, warning: string): BackendOverlayState {
  return { ...state, status: 'disconnected', ok: false, warning };
}

export function applyHealth(state: BackendOverlayState, health: HealthResponse): BackendOverlayState {
  if (health.protocol_version !== CLIENT_PROTOCOL_VERSION) {
    return {
      ...state,
      status: 'incompatible',
      ok: false,
      warning: `Protocol mismatch: client ${CLIENT_PROTOCOL_VERSION}, server ${health.protocol_version}`,
    };
  }

  return {
    ...state,
    status: health.ok ? 'snapshot' : 'disconnected',
    service: health.service,
    worldId: health.world_id,
    ok: health.ok,
    warning: health.ok ? undefined : 'Backend health check failed',
  };
}

export function applyWorldSummary(state: BackendOverlayState, world: WorldSummaryDto): BackendOverlayState {
  if (world.protocol_version !== CLIENT_PROTOCOL_VERSION) {
    return {
      ...state,
      status: 'incompatible',
      warning: `Protocol mismatch: client ${CLIENT_PROTOCOL_VERSION}, server ${world.protocol_version}`,
    };
  }

  return {
    ...state,
    worldId: world.world_id,
    chunkSize: world.chunk_size,
  };
}

export function applyChunkSnapshot(state: BackendOverlayState, snapshot: ChunkSnapshotDto): BackendOverlayState {
  if (snapshot.protocol_version !== CLIENT_PROTOCOL_VERSION) {
    return {
      ...state,
      status: 'incompatible',
      warning: `Protocol mismatch: client ${CLIENT_PROTOCOL_VERSION}, server ${snapshot.protocol_version}`,
    };
  }

  return {
    ...state,
    status: state.status === 'live' ? 'live' : 'snapshot',
    worldId: snapshot.world_id,
    loadedChunk: {
      coord: snapshot.coord,
      state: snapshot.chunk_state,
      version: snapshot.chunk_version,
      tileCount: snapshot.tile_count,
    },
  };
}

export function applyServerMessage(
  state: BackendOverlayState,
  message: ServerMessage,
  receivedAtMs: number,
): BackendOverlayState {
  if (message.protocol_version !== CLIENT_PROTOCOL_VERSION) {
    return {
      ...state,
      status: 'incompatible',
      warning: `Protocol mismatch: client ${CLIENT_PROTOCOL_VERSION}, server ${message.protocol_version}`,
    };
  }

  if (message.type === 'hello') {
    return {
      ...state,
      status: 'live',
      ok: true,
      worldId: message.world_id,
      chunkSize: message.chunk_size,
      warning: undefined,
    };
  }

  if (message.type === 'error') {
    return {
      ...state,
      warning: `${message.code}: ${message.message}`,
    };
  }

  if (state.worldId !== undefined && message.world_id !== state.worldId) {
    return {
      ...state,
      warning: `Ignored websocket message for ${message.world_id}`,
    };
  }

  const pulse: BackendPulse = {
    coord: message.coord,
    localIndex: message.local_index,
    tick: message.tick,
    version: message.version,
    receivedAtMs,
  };

  return {
    ...state,
    status: 'live',
    ok: true,
    worldId: message.world_id,
    latestTick: message.tick,
    latestVersion: message.version,
    pulses: [pulse, ...state.pulses].slice(0, 8),
    warning: undefined,
  };
}
