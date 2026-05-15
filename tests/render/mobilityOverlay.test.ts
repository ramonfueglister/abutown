import { describe, expect, it } from 'vitest';
import { applyMobilitySnapshot, createMobilityOverlayState } from '../../src/backend/mobilityState';
import type { MobilitySnapshotDto } from '../../src/backend/mobilityProtocol';
import { buildMobilityOverlayDrawItems } from '../../src/render/mobilityOverlay';

const snapshot: MobilitySnapshotDto = {
  protocol_version: 1,
  world_id: 'abutown-main',
  tick: 2,
  agents: [
    {
      id: 'agent:seed:0',
      state: { type: 'walking', link_id: 'link:home-to-old-town-stop', progress: 0.5 },
      plan_cursor: 0,
    },
  ],
  vehicles: [
    {
      id: 'vehicle:shuttle:0',
      route_id: 'route:old-town-loop',
      link_index: 0,
      progress: 0,
      capacity: 4,
      occupants: [],
      dwell_ticks_remaining: 0,
    },
  ],
  stops: [
    {
      id: 'stop:old-town',
      route_id: 'route:old-town-loop',
      link_index: 0,
      progress: 0,
      waiting_agents: [],
    },
  ],
};

describe('mobility overlay', () => {
  it('builds draw items for server mobility markers', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 100);
    const items = buildMobilityOverlayDrawItems(state, {
      project: (coord) => ({ x: coord.x * 2, y: coord.y * 2 }),
    });

    expect(items.map((item) => item.kind).sort()).toEqual(['agent', 'stop', 'vehicle']);
    expect(items.find((item) => item.id === 'agent:seed:0')).toMatchObject({
      x: 250,
      y: 262,
      radius: 10,
      color: '#f7d76a',
    });
  });

  it('respects the visibility predicate before projecting', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 100);
    const items = buildMobilityOverlayDrawItems(state, {
      project: (coord) => ({ x: coord.x, y: coord.y }),
      isVisible: (coord) => coord.x > 124,
    });

    expect(items.map((item) => item.id).sort()).toEqual(['agent:seed:0', 'stop:old-town', 'vehicle:shuttle:0']);
  });
});
