import { describe, expect, it } from 'vitest';
import {
  encodeClientMessage,
  isMobilityChunkDeltaDto,
  isMobilityChunkSnapshotDto,
  isMobilitySnapshotDto,
  isWorldSummaryDto,
  parseServerMessage,
  type MobilitySnapshotDto,
} from '../../src/backend/mobilityProtocol';

const snapshot: MobilitySnapshotDto = {
  protocol_version: 1,
  world_id: 'abutown-main',
  tick: 3,
  agents: [
    {
      id: 'agent:pedestrian:0',
      state: {
        type: 'walking',
        link_id: 'link:home-to-old-town-stop',
        progress: 0.5,
      },
      plan_cursor: 0,
      world_coord: { x: 0, y: 0 },
      direction: 'e',
      sprite_key: 'pedestrian:0',
    },
  ],
  vehicles: [
    {
      id: 'vehicle:shuttle:0',
      kind: 'tram' as const,
      route_id: 'route:old-town-loop',
      link_index: 0,
      progress: 0.25,
      capacity: 4,
      occupants: [],
      dwell_ticks_remaining: 1,
      world_coord: { x: 0, y: 0 },
      direction: 'e',
      sprite_key: 'tram:0',
    },
  ],
  stops: [
    {
      id: 'stop:old-town',
      route_id: 'route:old-town-loop',
      link_index: 0,
      progress: 0,
      waiting_agents: ['agent:pedestrian:0'],
    },
  ],
};

describe('mobility protocol guards', () => {
  it('accepts a valid mobility snapshot', () => {
    expect(isMobilitySnapshotDto(snapshot)).toBe(true);
  });

  it('accepts a valid MobilityChunkDelta message', () => {
    const message = {
      type: 'mobility_chunk_delta',
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 4,
      chunk: { x: 4, y: 4 },
      changed_agents: snapshot.agents,
      changed_vehicles: snapshot.vehicles,
      left_agents: [] as string[],
      left_vehicles: [] as string[],
    };

    expect(isMobilityChunkDeltaDto(message)).toBe(true);
    expect(parseServerMessage(message)).toEqual(message);
  });

  it('accepts a valid MobilityChunkSnapshot message', () => {
    const message = {
      type: 'mobility_chunk_snapshot',
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 4,
      chunk: { x: 4, y: 4 },
      agents: snapshot.agents,
      vehicles: snapshot.vehicles,
    };

    expect(isMobilityChunkSnapshotDto(message)).toBe(true);
    expect(parseServerMessage(message)).toEqual(message);
  });

  it('keeps non-mobility server messages parseable but rejects malformed mobility payloads', () => {
    expect(parseServerMessage({ type: 'hello', protocol_version: 1, world_id: 'abutown-main', chunk_size: 32 })).toEqual({
      type: 'hello',
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
    });
    expect(parseServerMessage({ type: 'mobility_chunk_delta', tick: 1 })).toBeNull();
    expect(isMobilitySnapshotDto({ ...snapshot, agents: [{ id: 'agent:bad', state: { type: 'walking' }, plan_cursor: 0 }] })).toBe(false);
  });
});

describe('isWorldSummaryDto', () => {
  it('accepts a valid payload with tick_period_ms', () => {
    const payload = {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [{ x: 4, y: 4 }, { x: 5, y: 4 }],
      tick_period_ms: 100,
    };
    expect(isWorldSummaryDto(payload)).toBe(true);
  });

  it('rejects payloads missing tick_period_ms', () => {
    const payload = {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [],
    };
    expect(isWorldSummaryDto(payload)).toBe(false);
  });

  it('rejects payloads with non-positive tick_period_ms', () => {
    const payload = {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [],
      tick_period_ms: 0,
    };
    expect(isWorldSummaryDto(payload)).toBe(false);
  });

  it('rejects payloads with malformed loaded_chunks entries', () => {
    const payload = {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [{ x: 'nope', y: 4 }],
      tick_period_ms: 100,
    };
    expect(isWorldSummaryDto(payload)).toBe(false);
  });
});

it('encodes chunk_subscribe message with snake_case discriminator', () => {
  const wire = encodeClientMessage({
    type: 'chunk_subscribe',
    protocol_version: 1,
    coords: [{ x: 4, y: 4 }, { x: 5, y: 4 }],
  });
  const json = JSON.parse(wire);
  expect(json.type).toBe('chunk_subscribe');
  expect(json.coords).toHaveLength(2);
});

it('encodes chunk_unsubscribe message', () => {
  const wire = encodeClientMessage({
    type: 'chunk_unsubscribe',
    protocol_version: 1,
    coords: [{ x: 4, y: 4 }],
  });
  expect(JSON.parse(wire).type).toBe('chunk_unsubscribe');
});

it('parses a MobilityChunkDelta server message', () => {
  const raw = {
    type: 'mobility_chunk_delta',
    protocol_version: 1,
    world_id: 'abutown-main',
    tick: 5,
    chunk: { x: 4, y: 4 },
    changed_agents: [],
    changed_vehicles: [],
    left_agents: [],
    left_vehicles: [],
  };
  const msg = parseServerMessage(raw);
  expect(msg?.type).toBe('mobility_chunk_delta');
});

it('parses a MobilityChunkSnapshot server message', () => {
  const raw = {
    type: 'mobility_chunk_snapshot',
    protocol_version: 1,
    world_id: 'abutown-main',
    tick: 5,
    chunk: { x: 4, y: 4 },
    agents: [],
    vehicles: [],
  };
  const msg = parseServerMessage(raw);
  expect(msg?.type).toBe('mobility_chunk_snapshot');
});
