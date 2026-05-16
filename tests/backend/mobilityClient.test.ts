import { describe, expect, it } from 'vitest';
import {
  connectMobilityBackend,
  requireMobilitySnapshot,
  resolveMobilityBackendBaseUrl,
} from '../../src/backend/mobilityClient';
import { mobilityDiagnostics } from '../../src/backend/mobilityState';
import type { MobilitySnapshotDto, WorldSummaryDto } from '../../src/backend/mobilityProtocol';

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
        requested.push(String(input));
        return snapshotFetch(input);
      },
      WebSocketImpl,
      now: () => 123,
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    bridge.stop();

    expect(requested).toEqual([
      'http://127.0.0.1:8080/mobility',
    ]);
    expect(sockets).toEqual(['ws://127.0.0.1:8080/ws']);
  });
});
