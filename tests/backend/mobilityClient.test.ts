import { describe, expect, it, vi } from 'vitest';
import {
  connectMobilityBackend,
  requireMobilitySnapshot,
  resolveMobilityBackendBaseUrl,
  SUBSCRIPTION_POLL_INTERVAL_MS,
  type MobilityViewportGetters,
} from '../../src/backend/mobilityClient';
import { mobilityDiagnostics } from '../../src/backend/mobilityState';
import type { MobilitySnapshotDto, WorldSummaryDto } from '../../src/backend/mobilityProtocol';
type ScreenToTile = (screen: { x: number; y: number }) => { x: number; y: number };
const identityScreenToTile: ScreenToTile = (s) => ({ x: s.x, y: s.y });
const nullScreenToTile = null as unknown as ScreenToTile;

type MockSocket = {
  readyState: number;
  onopen: (() => void) | null;
  onclose: ((event: { code?: number }) => void) | null;
  onerror: ((event: unknown) => void) | null;
  onmessage: ((event: MessageEvent<string>) => void) | null;
  sent: string[];
  url: string;
  triggerOpen: () => void;
  triggerClose: () => void;
};

function mockWebSocketImpl(): { Impl: typeof WebSocket; sockets: MockSocket[] } {
  const sockets: MockSocket[] = [];
  const Impl = class {
    readyState = 0;
    onopen: (() => void) | null = null;
    onclose: ((event: unknown) => void) | null = null;
    onerror: ((event: unknown) => void) | null = null;
    onmessage: ((event: MessageEvent<string>) => void) | null = null;
    sent: string[] = [];
    url: string;
    constructor(url: string) {
      this.url = url;
      sockets.push(this as unknown as MockSocket);
    }
    close() {
      this.readyState = 3;
      this.onclose?.({ code: 1000 });
    }
    send(text: string) {
      this.sent.push(text);
    }
    triggerOpen() {
      this.readyState = 1;
      this.onopen?.();
    }
    triggerClose() {
      this.readyState = 3;
      this.onclose?.({ code: 1006 });
    }
  } as unknown as typeof WebSocket;
  return { Impl, sockets };
}

// World big enough that viewport rarely covers everything → diff messages matter.
const TEST_WORLD = { widthTiles: 1024, heightTiles: 1024, chunkSize: 32 };

function stubViewport(opts: {
  screenToTile?: ScreenToTile | null;
  viewport?: { width: number; height: number } | null;
  world?: { widthTiles: number; heightTiles: number; chunkSize: number };
}): MobilityViewportGetters {
  const screenToTile = opts.screenToTile === undefined ? identityScreenToTile : opts.screenToTile;
  const viewport = opts.viewport === undefined ? { width: 256, height: 256 } : opts.viewport;
  const world = opts.world ?? TEST_WORLD;
  return {
    getScreenToTile: () => screenToTile,
    getViewport: () => viewport,
    getWorldDims: () => world,
  };
}

const worldSummary: WorldSummaryDto = {
  protocol_version: 1,
  world_id: 'abutown-main',
  chunk_size: 32,
  loaded_chunks: [],
  tick_period_ms: 100,
};

const snapshot: MobilitySnapshotDto = {
  protocol_version: 1,
  world_id: 'abutown-main',
  tick: 42,
  agents: [
    {
      id: 'agent-1',
      state: { type: 'walking', link_id: 'link-1', progress: 0.25 },
      plan_cursor: 2,
      world_coord: { x: 0, y: 0 },
      direction: 'e',
      sprite_key: 'pedestrian:0',
    },
  ],
  vehicles: [
    {
      id: 'vehicle-1',
      kind: 'tram',
      route_id: 'route-1',
      link_index: 1,
      progress: 0.5,
      capacity: 20,
      occupants: ['agent-1'],
      dwell_ticks_remaining: 0,
      world_coord: { x: 0, y: 0 },
      direction: 'e',
      sprite_key: 'tram:0',
    },
  ],
  stops: [
    {
      id: 'stop-1',
      route_id: 'route-1',
      link_index: 0,
      progress: 0,
      waiting_agents: [],
    },
  ],
};

function snapshotFetch(input: RequestInfo | URL): Response {
  const url = String(input);
  if (url.includes('/world')) return Response.json(worldSummary);
  return Response.json(snapshot);
}

describe('mobility backend client', () => {
  it('uses the live local backend by default', () => {
    expect(resolveMobilityBackendBaseUrl()).toBe('http://127.0.0.1:8080');
  });

  it('allows an explicit backend URL override', () => {
    expect(resolveMobilityBackendBaseUrl('https://backend.example.test')).toBe('https://backend.example.test');
  });

  it('requires fetch transport for the initial snapshot', async () => {
    await expect(requireMobilitySnapshot({ fetchImpl: undefined })).rejects.toThrow('Mobility fetch transport unavailable');
  });

  it('requires HTTP success for the initial snapshot', async () => {
    await expect(requireMobilitySnapshot({
      fetchImpl: async (input) => {
        const url = String(input);
        if (url.includes('/world')) return Response.json(worldSummary);
        return new Response('{}', { status: 503 });
      },
    })).rejects.toThrow('Mobility snapshot HTTP 503');
  });

  it('requires a valid initial snapshot payload', async () => {
    await expect(requireMobilitySnapshot({
      fetchImpl: async (input) => {
        const url = String(input);
        if (url.includes('/world')) return Response.json(worldSummary);
        return Response.json({ world_id: 'abutown-main', tick: 1 });
      },
    })).rejects.toThrow('Invalid mobility snapshot payload');
  });

  it('returns a connected state from the initial snapshot', async () => {
    const requestedUrls: string[] = [];
    const result = await requireMobilitySnapshot({
      fetchImpl: async (input) => {
        requestedUrls.push(String(input));
        return snapshotFetch(input);
      },
      now: () => 123,
    });

    expect(requestedUrls).toEqual([
      'http://127.0.0.1:8080/world',
      'http://127.0.0.1:8080/mobility',
    ]);
    expect(result.tickPeriodMs).toBe(100);
    expect(mobilityDiagnostics(result.state)).toEqual({
      status: 'connected',
      tick: 42,
      agents: 1,
      vehicles: 1,
      stops: 1,
      invalidMessages: 0,
      lastError: null,
    });
    expect(result.state.lastUpdatedAt).toBe(123);
  });

  it('requireMobilitySnapshot surfaces tickPeriodMs from /world', async () => {
    const customWorldSummary: WorldSummaryDto = {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [],
      tick_period_ms: 250,
    };
    const mobilitySnapshot: MobilitySnapshotDto = {
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 1,
      agents: [],
      vehicles: [],
      stops: [],
    };
    const fetchImpl = ((input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url.includes('/world')) return Promise.resolve(new Response(JSON.stringify(customWorldSummary)));
      return Promise.resolve(new Response(JSON.stringify(mobilitySnapshot)));
    }) as typeof fetch;

    const result = await requireMobilitySnapshot({ baseUrl: 'http://localhost:8080', fetchImpl });
    expect(result.tickPeriodMs).toBe(250);
    expect(result.state.status).toBe('connected');
  });

  it('connects websocket streaming to the same backend', async () => {
    const requested: string[] = [];
    const { Impl, sockets } = mockWebSocketImpl();

    const bridge = connectMobilityBackend({
      fetchImpl: async (input) => {
        requested.push(String(input));
        return snapshotFetch(input);
      },
      WebSocketImpl: Impl,
      now: () => 123,
      viewport: stubViewport({}),
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    bridge.stop();

    expect(requested).toEqual([
      'http://127.0.0.1:8080/mobility',
    ]);
    expect(sockets.map((s) => s.url)).toEqual(['ws://127.0.0.1:8080/ws']);
  });

  it('schedules the subscription poll at SUBSCRIPTION_POLL_INTERVAL_MS', async () => {
    const setIntervalImpl = vi.fn<typeof setInterval>(() => 1 as unknown as ReturnType<typeof setInterval>);
    const { Impl, sockets } = mockWebSocketImpl();

    const bridge = connectMobilityBackend({
      fetchImpl: snapshotFetch as unknown as typeof fetch,
      WebSocketImpl: Impl,
      viewport: stubViewport({}),
      setIntervalImpl: setIntervalImpl as unknown as typeof setInterval,
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    sockets[0].triggerOpen();

    expect(setIntervalImpl).toHaveBeenCalledTimes(1);
    expect(setIntervalImpl.mock.calls[0][1]).toBe(SUBSCRIPTION_POLL_INTERVAL_MS);
    bridge.stop();
  });

  it('calls injected clearIntervalImpl when the socket closes and when stop() runs', async () => {
    const intervalToken = Symbol('interval') as unknown as ReturnType<typeof setInterval>;
    const setIntervalImpl = vi.fn<typeof setInterval>(() => intervalToken);
    const clearIntervalImpl = vi.fn<typeof clearInterval>();
    const { Impl, sockets } = mockWebSocketImpl();

    const bridge = connectMobilityBackend({
      fetchImpl: snapshotFetch as unknown as typeof fetch,
      WebSocketImpl: Impl,
      viewport: stubViewport({}),
      setIntervalImpl: setIntervalImpl as unknown as typeof setInterval,
      clearIntervalImpl: clearIntervalImpl as unknown as typeof clearInterval,
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    sockets[0].triggerOpen();

    sockets[0].triggerClose();
    expect(clearIntervalImpl).toHaveBeenCalledWith(intervalToken);
    expect(clearIntervalImpl).toHaveBeenCalledTimes(1);

    // stop() must not double-clear an already-cleared interval.
    bridge.stop();
    expect(clearIntervalImpl).toHaveBeenCalledTimes(1);
  });

  it('pollSubscription is a silent no-op when getCamera() returns null', async () => {
    const { Impl, sockets } = mockWebSocketImpl();
    const setIntervalImpl = vi.fn<typeof setInterval>(() => 1 as unknown as ReturnType<typeof setInterval>);

    const bridge = connectMobilityBackend({
      fetchImpl: snapshotFetch as unknown as typeof fetch,
      WebSocketImpl: Impl,
      viewport: stubViewport({ screenToTile: nullScreenToTile }),
      setIntervalImpl: setIntervalImpl as unknown as typeof setInterval,
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    sockets[0].triggerOpen();

    // pollSubscription has been called once immediately, with camera=null.
    // No subscribe should have been sent.
    expect(sockets[0].sent).toEqual([]);
    bridge.stop();
  });

  it('viewport-size changes between polls emit a chunk_subscribe diff', async () => {
    const { Impl, sockets } = mockWebSocketImpl();
    let pollFn: (() => void) | null = null;
    const setIntervalImpl = vi.fn<typeof setInterval>((fn) => {
      pollFn = fn as () => void;
      return 1 as unknown as ReturnType<typeof setInterval>;
    });

    let viewport = { width: 64, height: 64 }; // tiny → very few chunks
    const getters: MobilityViewportGetters = {
      getScreenToTile: () => identityScreenToTile,
      getViewport: () => viewport,
      getWorldDims: () => TEST_WORLD,
    };

    const bridge = connectMobilityBackend({
      fetchImpl: snapshotFetch as unknown as typeof fetch,
      WebSocketImpl: Impl,
      viewport: getters,
      setIntervalImpl: setIntervalImpl as unknown as typeof setInterval,
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    sockets[0].triggerOpen();
    const sendsBefore = sockets[0].sent.length;

    // Simulate window resize → more chunks become visible.
    viewport = { width: 512, height: 512 };
    pollFn?.();

    expect(sockets[0].sent.length).toBeGreaterThan(sendsBefore);
    const lastMsg = JSON.parse(sockets[0].sent[sockets[0].sent.length - 1]);
    expect(lastMsg.type).toBe('chunk_subscribe');
    bridge.stop();
  });

  it('after a socket close the next socket re-subscribes from a fresh state', async () => {
    const { Impl, sockets } = mockWebSocketImpl();

    const bridge = connectMobilityBackend({
      fetchImpl: snapshotFetch as unknown as typeof fetch,
      WebSocketImpl: Impl,
      viewport: stubViewport({}),
      reconnectDelayMs: 500,
      setTimeoutImpl: ((fn: () => void) => {
        // Fire reconnect immediately for the test.
        Promise.resolve().then(fn);
        return 1 as unknown as ReturnType<typeof setTimeout>;
      }) as unknown as typeof setTimeout,
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    sockets[0].triggerOpen();
    const firstSendCount = sockets[0].sent.length;
    expect(firstSendCount).toBeGreaterThan(0);

    sockets[0].triggerClose();
    // Reconnect runs through the fake setTimeout → second socket appears.
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(sockets.length).toBe(2);

    sockets[1].triggerOpen();
    // The fresh subscription has no cached set, so the first poll emits
    // chunk_subscribe with every currently-visible chunk again.
    expect(sockets[1].sent.length).toBeGreaterThan(0);
    const firstMsg = JSON.parse(sockets[1].sent[0]);
    expect(firstMsg.type).toBe('chunk_subscribe');

    bridge.stop();
  });
});
