import { describe, expect, it, vi } from 'vitest';
import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import {
  connectMobilityBackend,
  requireMobilitySnapshot,
  resolveMobilityBackendBaseUrl,
  SUBSCRIPTION_POLL_INTERVAL_MS,
  type MobilityViewportGetters,
} from '../../src/backend/mobilityClient';
import { mobilityDiagnostics } from '../../src/backend/mobilityState';
import type { MobilitySnapshotDto, WorldSummaryDto } from '../../src/backend/mobilityProtocol';
import {
  ClientMessageSchema,
  MobilitySnapshotSchema,
  WorldSummarySchema,
  Direction as DirectionProto,
  VehicleKind as VehicleKindProto,
} from '../../src/backend/proto/abutown_pb';

function worldSummaryProtoResponse(dto: WorldSummaryDto): Response {
  const message = create(WorldSummarySchema, {
    protocolVersion: dto.protocol_version,
    worldId: dto.world_id,
    chunkSize: dto.chunk_size,
    loadedChunks: dto.loaded_chunks.map((c) => ({ x: c.x, y: c.y })),
    tickPeriodMs: dto.tick_period_ms,
  });
  return new Response(toBinary(WorldSummarySchema, message), {
    status: 200,
    headers: { 'content-type': 'application/x-protobuf' },
  });
}

function directionToProto(d: string): number {
  switch (d) {
    case 'n': return DirectionProto.N;
    case 'ne': return DirectionProto.NE;
    case 'e': return DirectionProto.E;
    case 'se': return DirectionProto.SE;
    case 's': return DirectionProto.S;
    case 'sw': return DirectionProto.SW;
    case 'w': return DirectionProto.W;
    case 'nw': return DirectionProto.NW;
    default: return DirectionProto.E;
  }
}

function mobilitySnapshotProtoResponse(dto: MobilitySnapshotDto): Response {
  const message = create(MobilitySnapshotSchema, {
    protocolVersion: dto.protocol_version,
    worldId: dto.world_id,
    tick: BigInt(dto.tick),
    agents: dto.agents.map((a) => ({
      id: a.id,
      planCursor: a.plan_cursor,
      worldCoord: { x: a.world_coord.x, y: a.world_coord.y },
      direction: directionToProto(a.direction),
      spriteKey: a.sprite_key,
      state: agentStateToProto(a.state),
    })),
    vehicles: dto.vehicles.map((v) => ({
      id: v.id,
      kind: v.kind === 'tram' ? VehicleKindProto.TRAM : VehicleKindProto.CAR,
      routeId: v.route_id,
      linkIndex: v.link_index,
      progress: v.progress,
      capacity: v.capacity,
      occupants: v.occupants,
      dwellTicksRemaining: v.dwell_ticks_remaining,
      worldCoord: { x: v.world_coord.x, y: v.world_coord.y },
      direction: directionToProto(v.direction),
      spriteKey: v.sprite_key,
    })),
    stops: dto.stops.map((s) => ({
      id: s.id,
      routeId: s.route_id,
      linkIndex: s.link_index,
      progress: s.progress,
      waitingAgents: s.waiting_agents,
    })),
  });
  return new Response(toBinary(MobilitySnapshotSchema, message), {
    status: 200,
    headers: { 'content-type': 'application/x-protobuf' },
  });
}

function agentStateToProto(state: MobilitySnapshotDto['agents'][number]['state']): {
  state: { case: string; value: unknown };
} {
  switch (state.type) {
    case 'walking':
      return { state: { case: 'walking', value: { linkId: state.link_id, progress: state.progress } } };
    case 'waiting_at_stop':
      return { state: { case: 'waitingAtStop', value: { stopId: state.stop_id } } };
    case 'in_vehicle':
      return { state: { case: 'inVehicle', value: { vehicleId: state.vehicle_id, seatIndex: state.seat_index } } };
    case 'boarding':
      return { state: { case: 'boarding', value: { vehicleId: state.vehicle_id, stopId: state.stop_id } } };
    case 'alighting':
      return { state: { case: 'alighting', value: { vehicleId: state.vehicle_id, stopId: state.stop_id } } };
    case 'at_activity':
      return { state: { case: 'atActivity', value: { activityId: state.activity_id } } };
  }
}
type ScreenToTile = (screen: { x: number; y: number }) => { x: number; y: number };
const identityScreenToTile: ScreenToTile = (s) => ({ x: s.x, y: s.y });
const nullScreenToTile = null as unknown as ScreenToTile;

type MockSocket = {
  readyState: number;
  binaryType: BinaryType;
  onopen: (() => void) | null;
  onclose: ((event: { code?: number }) => void) | null;
  onerror: ((event: unknown) => void) | null;
  onmessage: ((event: MessageEvent<ArrayBuffer>) => void) | null;
  sent: Uint8Array[];
  url: string;
  triggerOpen: () => void;
  triggerClose: () => void;
};

function mockWebSocketImpl(): { Impl: typeof WebSocket; sockets: MockSocket[] } {
  const sockets: MockSocket[] = [];
  const Impl = class {
    readyState = 0;
    binaryType: BinaryType = 'blob';
    onopen: (() => void) | null = null;
    onclose: ((event: unknown) => void) | null = null;
    onerror: ((event: unknown) => void) | null = null;
    onmessage: ((event: MessageEvent<ArrayBuffer>) => void) | null = null;
    sent: Uint8Array[] = [];
    url: string;
    constructor(url: string) {
      this.url = url;
      sockets.push(this as unknown as MockSocket);
    }
    close() {
      this.readyState = 3;
      this.onclose?.({ code: 1000 });
    }
    send(data: ArrayBufferLike | ArrayBufferView | Blob | string) {
      if (data instanceof Uint8Array) {
        this.sent.push(data);
      } else if (data instanceof ArrayBuffer) {
        this.sent.push(new Uint8Array(data));
      } else if (ArrayBuffer.isView(data)) {
        this.sent.push(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
      } else {
        // Tests should not be exercising the text path post-binary migration.
        throw new Error(`mock socket received non-binary send: ${typeof data}`);
      }
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
  if (url.includes('/world')) return worldSummaryProtoResponse(worldSummary);
  return mobilitySnapshotProtoResponse(snapshot);
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
        if (url.includes('/world')) return worldSummaryProtoResponse(worldSummary);
        return new Response(new Uint8Array(), { status: 503 });
      },
    })).rejects.toThrow('Mobility snapshot HTTP 503');
  });

  it('requires a valid initial snapshot payload', async () => {
    // Send bytes that don't decode as a valid MobilitySnapshot protobuf.
    // 0xff is an invalid wire tag in protobuf, so fromBinary throws.
    const invalidBytes = new Uint8Array([0xff, 0xff, 0xff, 0xff]);
    await expect(requireMobilitySnapshot({
      fetchImpl: async (input) => {
        const url = String(input);
        if (url.includes('/world')) return worldSummaryProtoResponse(worldSummary);
        return new Response(invalidBytes, {
          status: 200,
          headers: { 'content-type': 'application/x-protobuf' },
        });
      },
    })).rejects.toThrow();
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
      if (url.includes('/world')) return Promise.resolve(worldSummaryProtoResponse(customWorldSummary));
      return Promise.resolve(mobilitySnapshotProtoResponse(mobilitySnapshot));
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

  it('keeps backend tram chunks in the subscription interest set', async () => {
    const { Impl, sockets } = mockWebSocketImpl();
    const tramSnapshot: MobilitySnapshotDto = {
      ...snapshot,
      vehicles: [
        {
          ...snapshot.vehicles[0],
          id: 'vehicle:tram:outside-viewport',
          kind: 'tram',
          world_coord: { x: 150, y: 224 },
        },
      ],
    };
    const fetchImpl = ((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes('/world')) return Promise.resolve(worldSummaryProtoResponse(worldSummary));
      return Promise.resolve(mobilitySnapshotProtoResponse(tramSnapshot));
    }) as typeof fetch;

    const bridge = connectMobilityBackend({
      fetchImpl,
      WebSocketImpl: Impl,
      viewport: stubViewport({ viewport: { width: 64, height: 64 } }),
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    sockets[0].triggerOpen();

    const firstMsg = fromBinary(ClientMessageSchema, sockets[0].sent[0]);
    expect(firstMsg.body.case).toBe('chunkSubscribe');
    if (firstMsg.body.case !== 'chunkSubscribe') throw new Error('expected chunkSubscribe');
    expect(firstMsg.body.value.coords.map((coord) => `${coord.x},${coord.y}`)).toContain('4,7');
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

  it('pollSubscription is a silent no-op when getScreenToTile() returns null', async () => {
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

    // pollSubscription has been called once immediately, projection getter null.
    // No subscribe should have been sent.
    expect(sockets[0].sent).toEqual([]);
    bridge.stop();
  });

  it('pollSubscription is a silent no-op when getViewport() returns null', async () => {
    const { Impl, sockets } = mockWebSocketImpl();
    const setIntervalImpl = vi.fn<typeof setInterval>(() => 1 as unknown as ReturnType<typeof setInterval>);

    const bridge = connectMobilityBackend({
      fetchImpl: snapshotFetch as unknown as typeof fetch,
      WebSocketImpl: Impl,
      viewport: stubViewport({ viewport: null }),
      setIntervalImpl: setIntervalImpl as unknown as typeof setInterval,
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    sockets[0].triggerOpen();

    // Viewport not yet measured (e.g. first frame before resize fires).
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
    const lastBytes = sockets[0].sent[sockets[0].sent.length - 1];
    const lastMsg = fromBinary(ClientMessageSchema, lastBytes);
    expect(lastMsg.body.case).toBe('chunkSubscribe');
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
    const firstMsg = fromBinary(ClientMessageSchema, sockets[1].sent[0]);
    expect(firstMsg.body.case).toBe('chunkSubscribe');

    bridge.stop();
  });
});
