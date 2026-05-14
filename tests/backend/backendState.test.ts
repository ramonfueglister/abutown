import { afterEach, describe, expect, it, vi } from 'vitest';
import { startBackendBridge } from '../../src/backend/backendClient';
import {
  applyChunkSnapshot,
  applyHealth,
  applyServerMessage,
  applyWorldSummary,
  createInitialBackendOverlayState,
} from '../../src/backend/backendState';
import { parseServerMessage } from '../../src/backend/protocol';

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
