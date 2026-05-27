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
      'requireMobilitySnapshot',
      'mountCardHandView',
      'boot',
      'connectMobilityBackend',
      'addBeforeUnloadListener',
    ]);
    expect(dependencies.requireBackend).toHaveBeenCalledWith({ baseUrl: 'http://127.0.0.1:8080' });
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
    const mobilityState: MobilityOverlayState = { ...createMobilityOverlayState(), tick: 42 };
    const { dependencies } = createDependencies({
      requireBackend: vi.fn(async () => backendStatus),
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
