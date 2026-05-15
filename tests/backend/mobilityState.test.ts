import { describe, expect, it } from 'vitest';
import {
  applyMobilityDelta,
  applyMobilitySnapshot,
  applyServerMessage,
  createMobilityOverlayState,
  mobilityDiagnostics,
} from '../../src/backend/mobilityState';
import type { MobilitySnapshotDto } from '../../src/backend/mobilityProtocol';

const snapshot: MobilitySnapshotDto = {
  protocol_version: 1,
  world_id: 'abutown-main',
  tick: 2,
  agents: [
    {
      id: 'agent:pedestrian:0',
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

describe('mobility state reducer', () => {
  it('starts disconnected with empty records', () => {
    const state = createMobilityOverlayState();

    expect(state.status).toBe('disconnected');
    expect(mobilityDiagnostics(state)).toEqual({
      status: 'disconnected',
      tick: 0,
      agents: 0,
      vehicles: 0,
      stops: 0,
      invalidMessages: 0,
      lastError: null,
    });
  });

  it('stores snapshot records without projecting demo map markers', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 100);
    const diagnostics = mobilityDiagnostics(state);

    expect(state.status).toBe('connected');
    expect(diagnostics).toMatchObject({ tick: 2, agents: 1, vehicles: 1, stops: 1 });
    expect(diagnostics).not.toHaveProperty('seededAgentState');
  });

  it('applies mobility deltas by replacing changed agents and vehicles', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 100);
    const next = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 3,
        changed_agents: [
          {
            id: 'agent:pedestrian:0',
            state: { type: 'waiting_at_stop', stop_id: 'stop:old-town' },
            plan_cursor: 1,
          },
        ],
        changed_vehicles: [
          {
            ...snapshot.vehicles[0],
            dwell_ticks_remaining: 1,
          },
        ],
      },
      200
    );

    expect(mobilityDiagnostics(next)).toMatchObject({ tick: 3, agents: 1, vehicles: 1 });
  });

  it('counts invalid messages without dropping known records', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 100);
    const next = applyServerMessage(state, { type: 'mobility_delta', tick: 3 }, 200);

    expect(mobilityDiagnostics(next)).toMatchObject({ agents: 1, invalidMessages: 1 });
  });

});
