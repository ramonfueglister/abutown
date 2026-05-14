import { afterEach, describe, expect, it, vi } from 'vitest';
import { startBackendBridge } from '../../src/backend/backendClient';
import {
  applyChunkSnapshot,
  applyHealth,
  applyServerMessage,
  applyWorldSummary,
  createInitialBackendOverlayState,
} from '../../src/backend/backendState';
import { isChunkSnapshotDto, parseServerMessage } from '../../src/backend/protocol';

class TestWebSocket extends EventTarget {
  static instances: TestWebSocket[] = [];

  readonly url: string;

  constructor(url: string) {
    super();
    this.url = url;
    TestWebSocket.instances.push(this);
  }

  close(): void {
    this.dispatchEvent(new Event('close'));
  }
}

const flushPromises = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

describe('backend overlay state', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    TestWebSocket.instances = [];
  });

  it('loads HTTP snapshot state without requiring websocket data', () => {
    let state = createInitialBackendOverlayState();

    state = applyHealth(state, {
      service: 'abutown-sim',
      world_id: 'abutown-main',
      ok: true,
      protocol_version: 1,
    });
    state = applyWorldSummary(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [{ x: 4, y: 4 }],
    });
    state = applyChunkSnapshot(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      coord: { x: 4, y: 4 },
      chunk_state: 'active',
      chunk_version: 1,
      tile_count: 1024,
      dirty_tiles: [{ local_index: 0, kind: 'road', version: 1 }],
    });

    expect(state.status).toBe('snapshot');
    expect(state.worldId).toBe('abutown-main');
    expect(state.chunkSize).toBe(32);
    expect(state.loadedChunk?.coord).toEqual({ x: 4, y: 4 });
  });

  it('applies websocket tile pulses only when protocol and world match', () => {
    let state = createInitialBackendOverlayState();
    state = applyWorldSummary(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [{ x: 4, y: 4 }],
    });
    state = applyChunkSnapshot(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      coord: { x: 4, y: 4 },
      chunk_state: 'active',
      chunk_version: 1,
      tile_count: 1024,
      dirty_tiles: [],
    });

    state = applyServerMessage(
      state,
      {
        type: 'tile_pulse',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 4,
        version: 9,
        coord: { x: 4, y: 4 },
        local_index: 99,
      },
      1200,
    );

    expect(state.status).toBe('live');
    expect(state.latestTick).toBe(4);
    expect(state.latestVersion).toBe(9);
    expect(state.pulses).toHaveLength(1);
    expect(state.pulses[0]).toMatchObject({ localIndex: 99, receivedAtMs: 1200 });

    const afterWrongWorld = applyServerMessage(
      state,
      {
        type: 'tile_pulse',
        protocol_version: 1,
        world_id: 'other-world',
        tick: 5,
        version: 10,
        coord: { x: 4, y: 4 },
        local_index: 100,
      },
      1300,
    );

    expect(afterWrongWorld.pulses).toHaveLength(1);
    expect(afterWrongWorld.warning).toBe('Ignored websocket message for other-world');
  });

  it('ignores tile pulses outside the loaded chunk snapshot', () => {
    const state = applyChunkSnapshot(
      applyWorldSummary(createInitialBackendOverlayState(), {
        protocol_version: 1,
        world_id: 'abutown-main',
        chunk_size: 32,
        loaded_chunks: [{ x: 4, y: 4 }],
      }),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        coord: { x: 4, y: 4 },
        chunk_state: 'active',
        chunk_version: 1,
        tile_count: 1024,
        dirty_tiles: [],
      },
    );

    const wrongChunk = applyServerMessage(
      state,
      {
        type: 'tile_pulse',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 4,
        version: 9,
        coord: { x: 5, y: 4 },
        local_index: 99,
      },
      1200,
    );

    expect(wrongChunk.pulses).toHaveLength(0);
    expect(wrongChunk.warning).toBe('Ignored websocket message for chunk 5:4');

    const outOfRangeTile = applyServerMessage(
      state,
      {
        type: 'tile_pulse',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 4,
        version: 9,
        coord: { x: 4, y: 4 },
        local_index: 1024,
      },
      1200,
    );

    expect(outOfRangeTile.pulses).toHaveLength(0);
    expect(outOfRangeTile.warning).toBe('Ignored websocket tile 1024 outside chunk 4:4');
  });

  it('ignores tile pulses until a chunk snapshot is loaded', () => {
    const state = applyWorldSummary(createInitialBackendOverlayState(), {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [{ x: 4, y: 4 }],
    });

    const next = applyServerMessage(
      state,
      {
        type: 'tile_pulse',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 4,
        version: 9,
        coord: { x: 4, y: 4 },
        local_index: 99,
      },
      1200,
    );

    expect(next.pulses).toHaveLength(0);
    expect(next.warning).toBe('Ignored websocket tile pulse without loaded chunk');
  });

  it('parses server error messages with null world id', () => {
    expect(
      parseServerMessage({
        type: 'error',
        protocol_version: 1,
        world_id: null,
        code: 'backend_error',
        message: 'Something failed',
      }),
    ).toEqual({
      type: 'error',
      protocol_version: 1,
      world_id: null,
      code: 'backend_error',
      message: 'Something failed',
    });
  });

  it('rejects malformed numeric websocket DTOs', () => {
    const tilePulse = {
      type: 'tile_pulse',
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 4,
      version: 9,
      coord: { x: 4, y: 4 },
      local_index: 99,
    };

    expect(parseServerMessage({ ...tilePulse, local_index: 1.5 })).toBeUndefined();
    expect(parseServerMessage({ ...tilePulse, tick: Number.MAX_SAFE_INTEGER + 1 })).toBeUndefined();
    expect(parseServerMessage({ ...tilePulse, coord: { x: 4.5, y: 4 } })).toBeUndefined();
    expect(
      parseServerMessage({
        type: 'hello',
        protocol_version: 1,
        world_id: 'abutown-main',
        chunk_size: 65536,
      }),
    ).toBeUndefined();
  });

  it('rejects malformed numeric chunk snapshot DTOs', () => {
    const snapshot = {
      protocol_version: 1,
      world_id: 'abutown-main',
      coord: { x: 4, y: 4 },
      chunk_state: 'active',
      chunk_version: 1,
      tile_count: 1024,
      dirty_tiles: [{ local_index: 0, kind: 'road', version: 1 }],
    };

    expect(isChunkSnapshotDto(snapshot)).toBe(true);
    expect(isChunkSnapshotDto({ ...snapshot, tile_count: 65536 })).toBe(false);
    expect(isChunkSnapshotDto({ ...snapshot, chunk_version: 1.5 })).toBe(false);
    expect(
      isChunkSnapshotDto({
        ...snapshot,
        dirty_tiles: [{ local_index: 0.5, kind: 'road', version: 1 }],
      }),
    ).toBe(false);
  });

  it('does not continue snapshot loading or open websocket when health is unhealthy', async () => {
    const fetch = vi.fn(async (url: string) => {
      if (url.endsWith('/health')) {
        return Response.json({
          service: 'abutown-sim',
          world_id: 'abutown-main',
          ok: false,
          protocol_version: 1,
        });
      }

      return Response.json({
        protocol_version: 1,
        world_id: 'abutown-main',
        chunk_size: 32,
        loaded_chunks: [{ x: 4, y: 4 }],
      });
    });
    vi.stubGlobal('fetch', fetch);
    vi.stubGlobal('window', { setTimeout: vi.fn() });

    const states: ReturnType<typeof createInitialBackendOverlayState>[] = [];
    startBackendBridge({
      baseUrl: 'http://backend.test',
      onState: (state) => states.push(state),
      WebSocketCtor: TestWebSocket as unknown as typeof WebSocket,
    });

    await flushPromises();

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(fetch).toHaveBeenCalledWith('http://backend.test/health');
    expect(TestWebSocket.instances).toHaveLength(0);
    expect(states.at(-1)).toMatchObject({
      status: 'disconnected',
      warning: 'Backend health check failed',
    });
  });

  it('rejects malformed HTTP DTOs before opening websocket', async () => {
    const fetch = vi.fn(async () =>
      Response.json({
        service: 'abutown-sim',
        ok: true,
        protocol_version: 1,
      }),
    );
    vi.stubGlobal('fetch', fetch);
    vi.stubGlobal('window', { setTimeout: vi.fn() });

    const states: ReturnType<typeof createInitialBackendOverlayState>[] = [];
    startBackendBridge({
      baseUrl: 'http://backend.test',
      onState: (state) => states.push(state),
      WebSocketCtor: TestWebSocket as unknown as typeof WebSocket,
    });

    await flushPromises();

    expect(TestWebSocket.instances).toHaveLength(0);
    expect(states.at(-1)).toMatchObject({
      status: 'disconnected',
      warning: 'Invalid health response',
    });
  });

  it('reconnects after websocket error without scheduling duplicate timers', async () => {
    const fetch = vi.fn(async (url: string) => {
      if (url.endsWith('/health')) {
        return Response.json({
          service: 'abutown-sim',
          world_id: 'abutown-main',
          ok: true,
          protocol_version: 1,
        });
      }
      if (url.endsWith('/world')) {
        return Response.json({
          protocol_version: 1,
          world_id: 'abutown-main',
          chunk_size: 32,
          loaded_chunks: [{ x: 4, y: 4 }],
        });
      }
      return Response.json({
        protocol_version: 1,
        world_id: 'abutown-main',
        coord: { x: 4, y: 4 },
        chunk_state: 'active',
        chunk_version: 1,
        tile_count: 1024,
        dirty_tiles: [],
      });
    });
    vi.stubGlobal('fetch', fetch);

    const setTimeout = vi.fn(() => 456);
    vi.stubGlobal('window', {
      clearTimeout: vi.fn(),
      setTimeout,
    });

    const states: ReturnType<typeof createInitialBackendOverlayState>[] = [];
    startBackendBridge({
      baseUrl: 'http://backend.test',
      onState: (state) => states.push(state),
      WebSocketCtor: TestWebSocket as unknown as typeof WebSocket,
    });

    await flushPromises();

    expect(TestWebSocket.instances).toHaveLength(1);

    TestWebSocket.instances[0].dispatchEvent(new Event('error'));
    expect(states.at(-1)).toMatchObject({
      status: 'disconnected',
      warning: 'Backend websocket error',
    });

    TestWebSocket.instances[0].dispatchEvent(new Event('close'));

    expect(setTimeout).toHaveBeenCalledTimes(1);
    expect(setTimeout).toHaveBeenCalledWith(expect.any(Function), 2500);
  });

  it('ignores stale socket close events while reconnecting', async () => {
    const fetch = vi.fn(async (url: string) => {
      if (url.endsWith('/health')) {
        return Response.json({
          service: 'abutown-sim',
          world_id: 'abutown-main',
          ok: true,
          protocol_version: 1,
        });
      }
      if (url.endsWith('/world')) {
        return Response.json({
          protocol_version: 1,
          world_id: 'abutown-main',
          chunk_size: 32,
          loaded_chunks: [{ x: 4, y: 4 }],
        });
      }
      return Response.json({
        protocol_version: 1,
        world_id: 'abutown-main',
        coord: { x: 4, y: 4 },
        chunk_state: 'active',
        chunk_version: 1,
        tile_count: 1024,
        dirty_tiles: [],
      });
    });
    vi.stubGlobal('fetch', fetch);

    let reconnect: (() => void) | undefined;
    const setTimeout = vi.fn((callback: () => void) => {
      reconnect = callback;
      return 789;
    });
    vi.stubGlobal('window', {
      clearTimeout: vi.fn(),
      setTimeout,
    });

    startBackendBridge({
      baseUrl: 'http://backend.test',
      onState: vi.fn(),
      WebSocketCtor: TestWebSocket as unknown as typeof WebSocket,
    });

    await flushPromises();
    expect(TestWebSocket.instances).toHaveLength(1);

    TestWebSocket.instances[0].dispatchEvent(new Event('error'));
    expect(reconnect).toBeDefined();

    reconnect?.();
    await flushPromises();

    expect(TestWebSocket.instances).toHaveLength(2);
    expect(setTimeout).toHaveBeenCalledTimes(1);
  });

  it('clears a pending reconnect timer on stop', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => Response.json({ malformed: true })));

    const clearTimeout = vi.fn();
    vi.stubGlobal('window', {
      clearTimeout,
      setTimeout: vi.fn(() => 123),
    });

    const bridge = startBackendBridge({
      baseUrl: 'http://backend.test',
      onState: vi.fn(),
      WebSocketCtor: TestWebSocket as unknown as typeof WebSocket,
    });

    await flushPromises();
    bridge.stop();

    expect(clearTimeout).toHaveBeenCalledWith(123);
  });
});
