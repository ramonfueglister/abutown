import { resolveBackendBaseUrl } from './backendGate';
import { createSubscriptionClient } from './chunkSubscriptionClient';
import {
  isMobilitySnapshotDto,
  isWorldSummaryDto,
  parseServerMessage,
  type MobilitySnapshotDto,
  type WorldSummaryDto,
} from './mobilityProtocol';
import {
  applyMobilitySnapshot,
  applyServerMessage,
  createMobilityOverlayState,
  markMobilityConnecting,
  markMobilityDisconnected,
  type MobilityOverlayState,
} from './mobilityState';
import { visibleChunks } from '../render/viewportChunks';
import type { CameraState } from '../cameraController';

export type MobilityBackendBridge = {
  state: () => MobilityOverlayState;
  reconnect: () => void;
  stop: () => void;
};

export type MobilityViewportGetters = {
  getCamera: () => CameraState | null;
  getViewport: () => { width: number; height: number } | null;
  getWorldDims: () => { widthTiles: number; heightTiles: number; chunkSize: number };
};

export type MobilityBackendBridgeOptions = {
  baseUrl?: string;
  reconnectDelayMs?: number;
  fetchImpl?: typeof fetch;
  WebSocketImpl?: typeof WebSocket;
  onState?: (state: MobilityOverlayState) => void;
  initialState?: MobilityOverlayState;
  now?: () => number;
  setTimeoutImpl?: typeof setTimeout;
  clearTimeoutImpl?: typeof clearTimeout;
  setIntervalImpl?: typeof setInterval;
  clearIntervalImpl?: typeof clearInterval;
  viewport: MobilityViewportGetters;
};

export type MobilitySnapshotOptions = {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
  now?: () => number;
};

export type RequiredMobility = {
  state: MobilityOverlayState;
  tickPeriodMs: number;
};

const DEFAULT_TICK_PERIOD_MS = 100;

const DEFAULT_RECONNECT_DELAY_MS = 2500;

export function resolveMobilityBackendBaseUrl(envUrl?: unknown): string {
  return resolveBackendBaseUrl(envUrl);
}

export async function requireMobilitySnapshot(options: MobilitySnapshotOptions = {}): Promise<RequiredMobility> {
  const baseUrl = options.baseUrl ?? resolveMobilityBackendBaseUrl();
  const fetchImpl = resolveFetch(options);
  if (!fetchImpl) throw new Error('Mobility fetch transport unavailable');

  const now = options.now ?? Date.now;

  const worldSummary = await requestWorldSummary(baseUrl, fetchImpl);
  const tickPeriodMs = worldSummary.tick_period_ms > 0 ? worldSummary.tick_period_ms : DEFAULT_TICK_PERIOD_MS;

  const mobilityPayload = await requestMobilitySnapshot(baseUrl, fetchImpl);

  let state = createMobilityOverlayState();
  state = markMobilityConnecting(state, now());
  state = applyMobilitySnapshot(state, mobilityPayload, now());

  return { state, tickPeriodMs };
}

export function connectMobilityBackend(options: MobilityBackendBridgeOptions): MobilityBackendBridge {
  const baseUrl = options.baseUrl ?? resolveMobilityBackendBaseUrl();
  const reconnectDelayMs = Math.max(500, options.reconnectDelayMs ?? DEFAULT_RECONNECT_DELAY_MS);
  const fetchImpl = resolveFetch(options);
  const WebSocketImpl = options.WebSocketImpl ?? globalThis.WebSocket;
  const now = options.now ?? Date.now;
  const setTimeoutImpl = options.setTimeoutImpl ?? globalThis.setTimeout.bind(globalThis);
  const clearTimeoutImpl = options.clearTimeoutImpl ?? globalThis.clearTimeout.bind(globalThis);
  const setIntervalFn = options.setIntervalImpl ?? globalThis.setInterval.bind(globalThis);
  const clearIntervalFn = options.clearIntervalImpl ?? globalThis.clearInterval.bind(globalThis);

  let currentState = options.initialState ?? markMobilityConnecting(createMobilityOverlayState(), now());
  let stopped = false;
  let socket: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let subscriptionInterval: ReturnType<typeof setInterval> | null = null;

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
      const mobilityPayload = await requestMobilitySnapshot(baseUrl, fetchImpl);
      if (stopped) return;
      currentState = applyMobilitySnapshot(currentState, mobilityPayload, now());
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
    let subscription: ReturnType<typeof createSubscriptionClient> | null = null;

    socket.onopen = () => {
      subscription = createSubscriptionClient({
        send: (text) => socket?.send(text),
      });
      const pollSubscription = () => {
        if (socket?.readyState !== WebSocket.OPEN) return;
        const camera = options.viewport.getCamera();
        const view = options.viewport.getViewport();
        if (!camera || !view) return;
        const world = options.viewport.getWorldDims();
        const visible = visibleChunks(camera, view, world, world.chunkSize, 1);
        subscription?.update(visible);
      };
      pollSubscription(); // Initial subscribe immediately so the client doesn't wait 200 ms for entities.
      subscriptionInterval = setIntervalFn(pollSubscription, 200);
    };

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
      if (subscriptionInterval !== null) {
        clearIntervalFn(subscriptionInterval);
        subscriptionInterval = null;
      }
      subscription?.reset();
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
    if (subscriptionInterval !== null) {
      clearIntervalFn(subscriptionInterval);
      subscriptionInterval = null;
    }
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

async function requestWorldSummary(baseUrl: string, fetchImpl: typeof fetch): Promise<WorldSummaryDto> {
  const response = await fetchImpl(new URL('/world', baseUrl).toString());
  if (!response.ok) throw new Error(`World summary HTTP ${response.status}`);
  const payload: unknown = await response.json();
  if (!isWorldSummaryDto(payload)) throw new Error('Invalid world summary payload');
  return payload;
}

async function requestMobilitySnapshot(baseUrl: string, fetchImpl: typeof fetch): Promise<MobilitySnapshotDto> {
  const response = await fetchImpl(new URL('/mobility', baseUrl).toString());
  if (!response.ok) throw new Error(`Mobility snapshot HTTP ${response.status}`);

  const payload: unknown = await response.json();
  if (!isMobilitySnapshotDto(payload)) throw new Error('Invalid mobility snapshot payload');
  return payload;
}

function resolveFetch(options: { fetchImpl?: typeof fetch }): typeof fetch | undefined {
  return hasOption(options, 'fetchImpl') ? options.fetchImpl : globalThis.fetch?.bind(globalThis);
}

function hasOption<T extends object, K extends PropertyKey>(value: T, key: K): value is T & Record<K, unknown> {
  return Object.prototype.hasOwnProperty.call(value, key);
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
