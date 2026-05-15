import { describe, expect, it } from 'vitest';
import {
  connectMobilityBackend,
  requireMobilitySnapshot,
  resolveMobilityBackendBaseUrl,
} from '../../src/backend/mobilityClient';
import { mobilityDiagnostics } from '../../src/backend/mobilityState';
import type { MobilitySnapshotDto } from '../../src/backend/mobilityProtocol';

const snapshot: MobilitySnapshotDto = {
  protocol_version: 1,
  world_id: 'abutown-main',
  tick: 42,
  agents: [
    {
      id: 'agent-1',
      state: { type: 'walking', link_id: 'link-1', progress: 0.25 },
      plan_cursor: 2,
    },
  ],
  vehicles: [
    {
      id: 'vehicle-1',
      route_id: 'route-1',
      link_index: 1,
      progress: 0.5,
      capacity: 20,
      occupants: ['agent-1'],
      dwell_ticks_remaining: 0,
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
      fetchImpl: async () => new Response('{}', { status: 503 }),
    })).rejects.toThrow('Mobility snapshot HTTP 503');
  });

  it('requires a valid initial snapshot payload', async () => {
    await expect(requireMobilitySnapshot({
      fetchImpl: async () => Response.json({ world_id: 'abutown-main', tick: 1 }),
    })).rejects.toThrow('Invalid mobility snapshot payload');
  });

  it('returns a connected state from the initial snapshot', async () => {
    const state = await requireMobilitySnapshot({
      fetchImpl: async (input) => {
        expect(String(input)).toBe('http://127.0.0.1:8080/mobility');
        return Response.json(snapshot);
      },
      now: () => 123,
    });

    expect(mobilityDiagnostics(state)).toEqual({
      status: 'connected',
      tick: 42,
      agents: 1,
      vehicles: 1,
      stops: 1,
      invalidMessages: 0,
      lastError: null,
    });
    expect(state.lastUpdatedAt).toBe(123);
  });

  it('connects websocket streaming to the same backend', async () => {
    let requested = '';
    const sockets: string[] = [];
    const WebSocketImpl = class {
      readyState = 1;
      onclose: ((event: CloseEvent) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      onmessage: ((event: MessageEvent<string>) => void) | null = null;

      constructor(url: string) {
        sockets.push(url);
      }

      close(): void {
        this.readyState = 3;
      }
    } as unknown as typeof WebSocket;

    const bridge = connectMobilityBackend({
      fetchImpl: async (input) => {
        requested = String(input);
        return Response.json(snapshot);
      },
      WebSocketImpl,
      now: () => 123,
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    bridge.stop();

    expect(requested).toBe('http://127.0.0.1:8080/mobility');
    expect(sockets).toEqual(['ws://127.0.0.1:8080/ws']);
  });
});
