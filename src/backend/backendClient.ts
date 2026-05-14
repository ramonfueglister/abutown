import {
  applyChunkSnapshot,
  applyHealth,
  applyServerMessage,
  applyWorldSummary,
  createInitialBackendOverlayState,
  markBackendConnecting,
  markBackendDisconnected,
  type BackendOverlayState,
} from './backendState';
import { parseServerMessage, type ChunkSnapshotDto, type HealthResponse, type WorldSummaryDto } from './protocol';

export type BackendBridgeOptions = {
  baseUrl?: string;
  onState: (state: BackendOverlayState) => void;
  now?: () => number;
  WebSocketCtor?: typeof WebSocket;
};

export type BackendBridge = {
  stop: () => void;
};

const DEFAULT_BASE_URL = 'http://127.0.0.1:8080';
const RECONNECT_DELAY_MS = 2500;

export function startBackendBridge(options: BackendBridgeOptions): BackendBridge {
  const baseUrl = normalizeBaseUrl(options.baseUrl ?? import.meta.env.VITE_SIM_SERVER_URL ?? DEFAULT_BASE_URL);
  const now = options.now ?? (() => performance.now());
  const WebSocketCtor = options.WebSocketCtor ?? WebSocket;
  let stopped = false;
  let socket: WebSocket | undefined;
  let state = markBackendConnecting(createInitialBackendOverlayState());

  const publish = (next: BackendOverlayState): void => {
    state = next;
    options.onState(state);
  };

  publish(state);

  void loadAndConnect();

  async function loadAndConnect(): Promise<void> {
    try {
      const next = await loadSnapshot(baseUrl, state);
      if (stopped) return;
      publish(next);
      connectWebSocket();
    } catch (error: unknown) {
      if (stopped) return;
      publish(markBackendDisconnected(state, error instanceof Error ? error.message : 'Backend snapshot failed'));
      scheduleReconnect();
    }
  }

  function scheduleReconnect(): void {
    window.setTimeout(() => {
      if (!stopped) void loadAndConnect();
    }, RECONNECT_DELAY_MS);
  }

  function connectWebSocket(): void {
    socket?.close();
    socket = new WebSocketCtor(toWebSocketUrl(baseUrl, '/ws'));

    socket.addEventListener('message', (event) => {
      try {
        const parsed = parseServerMessage(JSON.parse(String(event.data)));
        if (!parsed) {
          publish({ ...state, warning: 'Ignored unknown websocket message' });
          return;
        }
        publish(applyServerMessage(state, parsed, now()));
      } catch {
        publish({ ...state, warning: 'Ignored malformed websocket message' });
      }
    });

    socket.addEventListener('close', () => {
      if (stopped) return;
      publish(markBackendDisconnected(state, 'Backend websocket disconnected'));
      scheduleReconnect();
    });

    socket.addEventListener('error', () => {
      if (stopped) return;
      publish(markBackendDisconnected(state, 'Backend websocket error'));
    });
  }

  return {
    stop: () => {
      stopped = true;
      socket?.close();
    },
  };
}

async function loadSnapshot(baseUrl: string, state: BackendOverlayState): Promise<BackendOverlayState> {
  const health = await fetchJson<HealthResponse>(`${baseUrl}/health`);
  let next = applyHealth(state, health);
  if (next.status === 'incompatible') return next;

  const world = await fetchJson<WorldSummaryDto>(`${baseUrl}/world`);
  next = applyWorldSummary(next, world);

  const firstChunk = world.loaded_chunks[0];
  if (!firstChunk) return next;

  const snapshot = await fetchJson<ChunkSnapshotDto>(`${baseUrl}/chunks/${firstChunk.x}/${firstChunk.y}`);
  return applyChunkSnapshot(next, snapshot);
}

async function fetchJson<T>(url: string): Promise<T> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url} returned ${response.status}`);
  return response.json() as Promise<T>;
}

function normalizeBaseUrl(value: string): string {
  return value.endsWith('/') ? value.slice(0, -1) : value;
}

function toWebSocketUrl(baseUrl: string, path: string): string {
  const url = new URL(path, `${baseUrl}/`);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.toString();
}
