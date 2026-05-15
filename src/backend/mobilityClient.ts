import { isMobilitySnapshotDto, parseServerMessage } from './mobilityProtocol';
import {
  applyMobilitySnapshot,
  applyServerMessage,
  createMobilityOverlayState,
  markMobilityConnecting,
  markMobilityDisconnected,
  type MobilityOverlayState,
} from './mobilityState';

export type MobilityBackendBridge = {
  state: () => MobilityOverlayState;
  reconnect: () => void;
  stop: () => void;
};

export type MobilityBackendBridgeOptions = {
  baseUrl?: string;
  reconnectDelayMs?: number;
  fetchImpl?: typeof fetch;
  WebSocketImpl?: typeof WebSocket;
  onState?: (state: MobilityOverlayState) => void;
  now?: () => number;
  setTimeoutImpl?: typeof setTimeout;
  clearTimeoutImpl?: typeof clearTimeout;
};

const FALLBACK_BASE_URL = 'http://127.0.0.1:5175';
const DEFAULT_RECONNECT_DELAY_MS = 2500;

export function connectMobilityBackend(options: MobilityBackendBridgeOptions = {}): MobilityBackendBridge {
  const baseUrl = options.baseUrl ?? globalThis.location?.origin ?? FALLBACK_BASE_URL;
  const reconnectDelayMs = Math.max(500, options.reconnectDelayMs ?? DEFAULT_RECONNECT_DELAY_MS);
  const fetchImpl = options.fetchImpl ?? globalThis.fetch?.bind(globalThis);
  const WebSocketImpl = options.WebSocketImpl ?? globalThis.WebSocket;
  const now = options.now ?? Date.now;
  const setTimeoutImpl = options.setTimeoutImpl ?? globalThis.setTimeout.bind(globalThis);
  const clearTimeoutImpl = options.clearTimeoutImpl ?? globalThis.clearTimeout.bind(globalThis);

  let currentState = markMobilityConnecting(createMobilityOverlayState(), now());
  let stopped = false;
  let socket: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  notify();
  void connect();

  return {
    state: () => currentState,
    reconnect: () => {
      if (stopped) return;
      clearReconnectTimer();
      closeSocket();
      currentState = markMobilityConnecting(currentState, now());
      notify();
      void connect();
    },
    stop: () => {
      stopped = true;
      clearReconnectTimer();
      closeSocket();
    },
  };

  async function connect(): Promise<void> {
    if (stopped) return;
    if (!fetchImpl || !WebSocketImpl) {
      markDisconnectedAndSchedule('Browser transport unavailable');
      return;
    }

    try {
      const response = await fetchImpl(new URL('/mobility', baseUrl).toString());
      if (!response.ok) throw new Error(`Mobility snapshot HTTP ${response.status}`);
      const payload: unknown = await response.json();
      if (!isMobilitySnapshotDto(payload)) throw new Error('Invalid mobility snapshot payload');
      if (stopped) return;
      currentState = applyMobilitySnapshot(currentState, payload, now());
      notify();
      openSocket(WebSocketImpl);
    } catch (error) {
      markDisconnectedAndSchedule(errorMessage(error));
    }
  }

  function openSocket(WebSocketConstructor: typeof WebSocket): void {
    closeSocket();
    const url = websocketUrl(baseUrl, '/ws');
    socket = new WebSocketConstructor(url);

    socket.onmessage = (event: MessageEvent<string>) => {
      const parsed = parseJson(event.data);
      const message = parseServerMessage(parsed);
      currentState = applyServerMessage(currentState, message ?? parsed, now());
      notify();
    };

    socket.onerror = () => {
      currentState = markMobilityDisconnected(currentState, 'Mobility websocket error', now());
      notify();
    };

    socket.onclose = () => {
      if (stopped) return;
      markDisconnectedAndSchedule('Mobility websocket closed');
    };
  }

  function markDisconnectedAndSchedule(error: string): void {
    if (stopped) return;
    currentState = markMobilityDisconnected(currentState, error, now());
    notify();
    scheduleReconnect();
  }

  function scheduleReconnect(): void {
    clearReconnectTimer();
    reconnectTimer = setTimeoutImpl(() => {
      reconnectTimer = null;
      if (stopped) return;
      currentState = markMobilityConnecting(currentState, now());
      notify();
      void connect();
    }, reconnectDelayMs);
  }

  function closeSocket(): void {
    if (!socket) return;
    socket.onclose = null;
    socket.onerror = null;
    socket.onmessage = null;
    if (socket.readyState === 0 || socket.readyState === 1) socket.close();
    socket = null;
  }

  function clearReconnectTimer(): void {
    if (reconnectTimer === null) return;
    clearTimeoutImpl(reconnectTimer);
    reconnectTimer = null;
  }

  function notify(): void {
    options.onState?.(currentState);
  }
}

function websocketUrl(baseUrl: string, path: string): string {
  const url = new URL(path, baseUrl);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.toString();
}

function parseJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}
