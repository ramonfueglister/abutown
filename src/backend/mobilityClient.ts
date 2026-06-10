import { fromBinary } from '@bufbuild/protobuf';
import { resolveBackendBaseUrl } from './backendGate';
import { createSubscriptionClient } from './chunkSubscriptionClient';
import {
  isMobilitySnapshotDto,
  isWorldSummaryDto,
  mobilitySnapshotFromProto,
  tileKindSetEventFromProto,
  worldSummaryFromProto,
  type MobilitySnapshotDto,
  type TileKindSetEventDto,
  type WorldSummaryDto,
} from './mobilityProtocol';
import {
  MobilitySnapshotSchema,
  ServerMessageSchema,
  WorldSummarySchema,
} from './proto/abutown_pb';
import {
  applyMobilitySnapshot,
  applyServerMessage,
  createMobilityOverlayState,
  markMobilityConnecting,
  markMobilityDisconnected,
  type MobilityOverlayState,
} from './mobilityState';
import {
  applyEconomyServerMessage,
  createEconomyOverlayState,
  type EconomyOverlayState,
} from './economyState';
import { visibleChunks } from '../render/viewportChunks';

export type MobilityBackendBridge = {
  state: () => MobilityOverlayState;
  reconnect: () => void;
  stop: () => void;
};

/// `getScreenToTile` returns a projection from CSS screen pixels to mobility
/// tile coordinates (in the same units as the backend's `Position` / `chunk_of`
/// math). For the isometric renderer this composes `worldToGrid` and
/// `screenToWorld`; for tests it can be the identity. Returns `null` while
/// the camera isn't ready yet.
export type MobilityViewportGetters = {
  getScreenToTile: () => ((screen: { x: number; y: number }) => { x: number; y: number }) | null;
  getViewport: () => { width: number; height: number } | null;
  getWorldDims: () => { widthTiles: number; heightTiles: number; chunkSize: number };
};

export type MobilityBackendBridgeOptions = {
  baseUrl?: string;
  reconnectDelayMs?: number;
  fetchImpl?: typeof fetch;
  WebSocketImpl?: typeof WebSocket;
  onState?: (state: MobilityOverlayState) => void;
  onEconomyState?: (state: EconomyOverlayState) => void;
  onTileKindSet?: (event: TileKindSetEventDto) => void;
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
  simTime: number;
};

const DEFAULT_TICK_PERIOD_MS = 100;

const DEFAULT_RECONNECT_DELAY_MS = 2500;

/// Interval between viewport-subscription recomputations. Small enough that a
/// pan/zoom is reflected within a frame the user notices, large enough to bound
/// per-client WS traffic at ~5 diff messages / sec.
export const SUBSCRIPTION_POLL_INTERVAL_MS = 200;

/// `WebSocket.OPEN` per the spec. Hardcoded so the polling guard doesn't read
/// a browser-only global (vitest runs in the `node` environment).
const WS_READY_STATE_OPEN = 1;

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
  const simTime = worldSummary.sim_time;

  const mobilityPayload = await requestMobilitySnapshot(baseUrl, fetchImpl);

  let state = createMobilityOverlayState();
  state = markMobilityConnecting(state, now());
  state = applyMobilitySnapshot(state, mobilityPayload, now());

  return { state, tickPeriodMs, simTime };
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
  let currentEconomyState = createEconomyOverlayState();
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
    socket.binaryType = 'arraybuffer';
    let subscription: ReturnType<typeof createSubscriptionClient> | null = null;

    socket.onopen = () => {
      subscription = createSubscriptionClient({
        // WebSocket.send accepts Uint8Array but its TS overload demands
        // a buffer view backed by ArrayBuffer (not SharedArrayBuffer); the
        // codegen output uses ArrayBufferLike. Copy into a fresh ArrayBuffer
        // slice so the type narrows correctly and avoid mutation aliasing.
        send: (bytes) => {
          const copy = new Uint8Array(bytes.byteLength);
          copy.set(bytes);
          socket?.send(copy.buffer);
        },
      });
      const pollSubscription = () => {
        if (socket?.readyState !== WS_READY_STATE_OPEN) return;
        const screenToTile = options.viewport.getScreenToTile();
        const view = options.viewport.getViewport();
        if (!screenToTile || !view) return;
        const world = options.viewport.getWorldDims();
        const visible = visibleChunks(screenToTile, view, world, world.chunkSize, 1);
        subscription?.update(visible);
      };
      pollSubscription(); // Initial subscribe immediately so the client doesn't wait the poll interval for entities.
      subscriptionInterval = setIntervalFn(pollSubscription, SUBSCRIPTION_POLL_INTERVAL_MS);
    };

    socket.onmessage = (event: MessageEvent<ArrayBuffer>) => {
      // Reject text/blob frames defensively — backend sends binary only.
      if (!(event.data instanceof ArrayBuffer)) {
        // eslint-disable-next-line no-console
        console.warn('mobility ws: ignoring non-binary frame', typeof event.data);
        return;
      }
      const bytes = new Uint8Array(event.data);
      try {
        const message = fromBinary(ServerMessageSchema, bytes);
        currentState = applyServerMessage(currentState, message, now());
        currentEconomyState = applyEconomyServerMessage(currentEconomyState, message);
        if (
          message.body.case === 'worldEvent' &&
          message.body.value.event.case === 'tileKindSet' &&
          options.onTileKindSet
        ) {
          const chunkSize = options.viewport.getWorldDims().chunkSize;
          const tileEvent = tileKindSetEventFromProto(message.body.value.event.value, chunkSize);
          if (tileEvent === null) {
            // eslint-disable-next-line no-console
            console.warn('worldEvent: unrenderable tile kind', message.body.value.event.value.kind, 'coord', message.body.value.event.value.coord, 'localIndex', message.body.value.event.value.localIndex);
          } else {
            options.onTileKindSet(tileEvent);
          }
        }
        notify();
      } catch (err) {
        // Don't tear down the socket on one bad frame — surface via diagnostics.
        // eslint-disable-next-line no-console
        console.warn('failed to decode ServerMessage', err);
        currentState = {
          ...currentState,
          invalidMessages: currentState.invalidMessages + 1,
          lastUpdatedAt: now(),
        };
        notify();
      }
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
    options.onEconomyState?.(currentEconomyState);
  }
}

async function requestWorldSummary(baseUrl: string, fetchImpl: typeof fetch): Promise<WorldSummaryDto> {
  const response = await fetchImpl(new URL('/world', baseUrl).toString());
  if (!response.ok) throw new Error(`World summary HTTP ${response.status}`);
  // Phase: binary wire — /world returns application/x-protobuf.
  const bytes = new Uint8Array(await response.arrayBuffer());
  const payload = worldSummaryFromProto(fromBinary(WorldSummarySchema, bytes));
  if (!isWorldSummaryDto(payload)) throw new Error('Invalid world summary payload');
  return payload;
}

async function requestMobilitySnapshot(baseUrl: string, fetchImpl: typeof fetch): Promise<MobilitySnapshotDto> {
  const response = await fetchImpl(new URL('/mobility', baseUrl).toString());
  if (!response.ok) throw new Error(`Mobility snapshot HTTP ${response.status}`);

  // Phase: binary wire — /mobility returns application/x-protobuf.
  const bytes = new Uint8Array(await response.arrayBuffer());
  const payload = mobilitySnapshotFromProto(fromBinary(MobilitySnapshotSchema, bytes));
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

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}
