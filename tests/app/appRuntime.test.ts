import { describe, expect, it, vi } from 'vitest';
import { createMobilityOverlayState, type MobilityOverlayState } from '../../src/backend/mobilityState';
import { startAppRuntime, type AppRuntimeDependencies } from '../../src/app/appRuntime';

function createBackendStatus() {
  return {
    service: 'abutown-sim' as const,
    world_id: 'test-world',
    ok: true as const,
    protocol_version: 1,
  };
}

function createBaseWorld() {
  return {
    schema_version: 1,
    world_id: 'zurich-river-city-v1',
    chunk_size: 32,
    world_tiles: { width: 256, height: 256 },
    terrain: { tiles: [{ x: 1, y: 1, kind: 'water' as const }] },
    transport: {
      roads: Array.from({ length: 1800 }, (_, index) => ({ x: index % 256, y: Math.floor(index / 256), kind: 'street' as const, mask: 3 })),
      rails: Array.from({ length: 256 }, (_, y) => ({ x: 150, y, mask: 5 })),
      arterial_paths: [0, 1, 2].map((index) => ({ id: `arterial:${index}`, points: [{ x: 0, y: index }, { x: 255, y: index }] })),
      rail_paths: [{ id: 'rail:0', points: [{ x: 150, y: 0 }, { x: 150, y: 255 }] }],
      pedestrian_corridors: Array.from({ length: 160 }, (_, index) => ({ id: `pedestrian:${index}`, points: [{ x: 0, y: index }, { x: 10, y: index }] })),
    },
    buildings: { footprints: Array.from({ length: 2250 }, (_, index) => ({ id: `building:${index}`, tiles: [{ x: index % 256, y: Math.floor(index / 256) }], sheet: 'houses', frame: 0, district: 'test' })) },
    decorations: {
      trees: Array.from({ length: 3000 }, (_, index) => ({ x: index % 256, y: Math.floor(index / 256) })),
      details: [],
    },
  };
}

function createDependencies(overrides: Partial<AppRuntimeDependencies> = {}) {
  const order: string[] = [];
  const bridge = {
    state: vi.fn(() => createMobilityOverlayState()),
    reconnect: vi.fn(),
    stop: vi.fn(),
  };
  const dependencies: AppRuntimeDependencies = {
    requireBackend: vi.fn(async () => {
      order.push('requireBackend');
      return createBackendStatus();
    }),
    requireBaseWorld: vi.fn(async () => {
      order.push('requireBaseWorld');
      return createBaseWorld();
    }),
    requireMobilitySnapshot: vi.fn(async () => {
      order.push('requireMobilitySnapshot');
      return { state: createMobilityOverlayState(), tickPeriodMs: 250 };
    }),
    mountCardHandView: vi.fn(() => {
      order.push('mountCardHandView');
    }),
    boot: vi.fn(async () => {
      order.push('boot');
    }),
    connectMobilityBackend: vi.fn(() => {
      order.push('connectMobilityBackend');
      return bridge;
    }),
    renderBackendRequired: vi.fn((error: unknown) => {
      order.push(`renderBackendRequired:${error instanceof Error ? error.message : String(error)}`);
    }),
    addBeforeUnloadListener: vi.fn((listener: () => void) => {
      order.push('addBeforeUnloadListener');
      return listener;
    }),
    ...overrides,
  };
  return { dependencies, order, bridge };
}

describe('startAppRuntime', () => {
  it('starts runtime dependencies in order and wires the mobility backend', async () => {
    const { dependencies, order, bridge } = createDependencies();
    const viewport = {
      getScreenToTile: vi.fn(() => (screen: { x: number; y: number }) => screen),
      getViewport: vi.fn(() => ({ width: 800, height: 600 })),
      getWorldDims: vi.fn(() => ({ widthTiles: 10, heightTiles: 12, chunkSize: 4 })),
    };
    const onInitialState = vi.fn();
    const onMobilityState = vi.fn();

    const handle = await startAppRuntime({
      backendBaseUrl: 'http://127.0.0.1:8080',
      viewport,
      onInitialState,
      onMobilityState,
      dependencies,
    });

    expect(order).toEqual([
      'requireBackend',
      'requireBaseWorld',
      'requireMobilitySnapshot',
      'mountCardHandView',
      'boot',
      'connectMobilityBackend',
      'addBeforeUnloadListener',
    ]);
    expect(dependencies.requireBackend).toHaveBeenCalledWith({ baseUrl: 'http://127.0.0.1:8080' });
    expect(dependencies.requireBaseWorld).toHaveBeenCalledWith({ baseUrl: 'http://127.0.0.1:8080' });
    expect(dependencies.requireMobilitySnapshot).toHaveBeenCalledWith({ baseUrl: 'http://127.0.0.1:8080' });
    expect(dependencies.mountCardHandView).toHaveBeenCalledWith({ baseUrl: 'http://127.0.0.1:8080' });
    expect(dependencies.boot).toHaveBeenCalledWith(onInitialState.mock.calls[0][0]);
    expect(dependencies.connectMobilityBackend).toHaveBeenCalledWith({
      baseUrl: 'http://127.0.0.1:8080',
      initialState: onInitialState.mock.calls[0][0].mobilityState,
      onState: onMobilityState,
      viewport,
    });
    expect(handle.mobilityBackendBridge).toBe(bridge);
  });

  it('passes backend status, mobility state, and tick period to onInitialState', async () => {
    const backendStatus = createBackendStatus();
    const baseWorld = createBaseWorld();
    const mobilityState: MobilityOverlayState = { ...createMobilityOverlayState(), tick: 42 };
    const { dependencies } = createDependencies({
      requireBackend: vi.fn(async () => backendStatus),
      requireBaseWorld: vi.fn(async () => baseWorld),
      requireMobilitySnapshot: vi.fn(async () => ({ state: mobilityState, tickPeriodMs: 125 })),
    });
    const onInitialState = vi.fn();

    await startAppRuntime({
      backendBaseUrl: 'http://127.0.0.1:8080',
      viewport: {
        getScreenToTile: () => null,
        getViewport: () => null,
        getWorldDims: () => ({ widthTiles: 1, heightTiles: 1, chunkSize: 1 }),
      },
      onInitialState,
      onMobilityState: vi.fn(),
      dependencies,
    });

    expect(onInitialState).toHaveBeenCalledWith({
      backendStatus,
      baseWorld,
      mobilityState,
      mobilityTickPeriodMs: 125,
    });
  });

  it('fails closed when backend startup fails', async () => {
    const startupError = new Error('backend offline');
    const { dependencies } = createDependencies({
      requireBackend: vi.fn(async () => {
        throw startupError;
      }),
    });

    const handle = await startAppRuntime({
      backendBaseUrl: 'http://127.0.0.1:8080',
      viewport: {
        getScreenToTile: () => null,
        getViewport: () => null,
        getWorldDims: () => ({ widthTiles: 1, heightTiles: 1, chunkSize: 1 }),
      },
      onInitialState: vi.fn(),
      onMobilityState: vi.fn(),
      dependencies,
    });

    expect(dependencies.renderBackendRequired).toHaveBeenCalledWith(startupError);
    expect(dependencies.mountCardHandView).not.toHaveBeenCalled();
    expect(dependencies.boot).not.toHaveBeenCalled();
    expect(dependencies.connectMobilityBackend).not.toHaveBeenCalled();
    expect(handle.mobilityBackendBridge).toBeNull();
    expect(() => handle.stop()).not.toThrow();
  });
});
