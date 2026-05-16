import { describe, expect, it } from 'vitest';
import {
  isMobilityDeltaDto,
  isMobilitySnapshotDto,
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

  it('accepts a valid mobility delta server message', () => {
    const message = {
      type: 'mobility_delta',
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 4,
      changed_agents: snapshot.agents,
      changed_vehicles: snapshot.vehicles,
    };

    expect(isMobilityDeltaDto(message)).toBe(true);
    expect(parseServerMessage(message)).toEqual(message);
  });

  it('keeps non-mobility server messages parseable but rejects malformed mobility payloads', () => {
    expect(parseServerMessage({ type: 'hello', protocol_version: 1, world_id: 'abutown-main', chunk_size: 32 })).toEqual({
      type: 'hello',
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
    });
    expect(parseServerMessage({ type: 'mobility_delta', tick: 1 })).toBeNull();
    expect(isMobilitySnapshotDto({ ...snapshot, agents: [{ id: 'agent:bad', state: { type: 'walking' }, plan_cursor: 0 }] })).toBe(false);
  });
});
