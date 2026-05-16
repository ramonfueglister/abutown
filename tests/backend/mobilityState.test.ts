import { describe, expect, it } from 'vitest';
import {
  applyMobilityDelta,
  applyMobilitySnapshot,
  applyServerMessage,
  createMobilityOverlayState,
  interpolatedAgents,
  mobilityDiagnostics,
} from '../../src/backend/mobilityState';
import type { AgentMobilityDto, MobilityDeltaDto, MobilitySnapshotDto } from '../../src/backend/mobilityProtocol';

const snapshot: MobilitySnapshotDto = {
  protocol_version: 1,
  world_id: 'abutown-main',
  tick: 2,
  agents: [
    {
      id: 'agent:pedestrian:0',
      state: { type: 'walking', link_id: 'link:home-to-old-town-stop', progress: 0.5 },
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
      progress: 0,
      capacity: 4,
      occupants: [],
      dwell_ticks_remaining: 0,
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
      roadVehicles: 0,
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
            world_coord: { x: 0, y: 0 },
            direction: 'e',
            sprite_key: 'pedestrian:0',
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

  it('applies road_vehicle_delta messages into embedded road vehicle state', () => {
    let state = createMobilityOverlayState();
    state = applyServerMessage(state, {
      type: 'road_vehicle_delta',
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 5,
      changed: [
        {
          id: 'road_vehicle:seed:0',
          world_coord: { x: 10, y: 20 },
          direction: 'n',
          sprite_key: 'vehicle:0',
        },
      ],
    });
    expect(state.roadVehicles.tick).toBe(5);
    expect(state.roadVehicles.vehicles.get('road_vehicle:seed:0')?.current.world_coord.y).toBe(20);
  });
});

function agentAt(id: string, x: number, y: number): AgentMobilityDto {
  return {
    id,
    state: { type: 'walking', link_id: 'link:walk:default', progress: 0.0 },
    plan_cursor: 0,
    world_coord: { x, y },
    direction: 'e',
    sprite_key: 'pedestrian:0',
  };
}

describe('mobility state interpolation buffer', () => {
  it('initial snapshot sets prev == current for each agent', () => {
    const snapshot: MobilitySnapshotDto = {
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 1,
      agents: [agentAt('agent:seed:0', 100, 200)],
      vehicles: [],
      stops: [],
    };
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 1000);
    const entry = state.agents.get('agent:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.current.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.lastTickAt).toBe(1000);
  });

  it('delta moves prev<-current and sets current=new dto', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [agentAt('agent:seed:0', 100, 200)],
        vehicles: [],
        stops: [],
      },
      1000,
    );
    const delta: MobilityDeltaDto = {
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 2,
      changed_agents: [agentAt('agent:seed:0', 110, 200)],
      changed_vehicles: [],
    };
    state = applyMobilityDelta(state, delta, 1100);
    const entry = state.agents.get('agent:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.current.world_coord).toEqual({ x: 110, y: 200 });
    expect(entry.lastTickAt).toBe(1100);
  });

  it('delta for a new agent sets prev == current', () => {
    let state = createMobilityOverlayState();
    state = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        changed_agents: [agentAt('agent:seed:0', 50, 60)],
        changed_vehicles: [],
      },
      500,
    );
    const entry = state.agents.get('agent:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 50, y: 60 });
    expect(entry.current.world_coord).toEqual({ x: 50, y: 60 });
  });

  it('interpolatedAgents lerps world_coord by t = (now - lastTickAt) / tickPeriodMs', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [agentAt('agent:seed:0', 100, 200)],
        vehicles: [],
        stops: [],
      },
      1000,
    );
    state = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed_agents: [agentAt('agent:seed:0', 110, 200)],
        changed_vehicles: [],
      },
      1100,
    );
    const agents = interpolatedAgents(state, 1150, 100);
    expect(agents).toHaveLength(1);
    expect(agents[0].world_coord.x).toBeCloseTo(105.0, 5);
    expect(agents[0].world_coord.y).toBeCloseTo(200.0, 5);
  });

  it('interpolatedAgents clamps t to [0, 1]', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [agentAt('agent:seed:0', 0, 0)],
        vehicles: [],
        stops: [],
      },
      0,
    );
    state = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed_agents: [agentAt('agent:seed:0', 100, 0)],
        changed_vehicles: [],
      },
      1000,
    );
    // now < lastTickAt → t clamps to 0 → prev coord
    const earlyAgents = interpolatedAgents(state, 500, 100);
    expect(earlyAgents[0].world_coord.x).toBeCloseTo(0, 5);
    // now >> lastTickAt + tickPeriodMs → t clamps to 1 → current coord
    const lateAgents = interpolatedAgents(state, 5000, 100);
    expect(lateAgents[0].world_coord.x).toBeCloseTo(100, 5);
  });
});
