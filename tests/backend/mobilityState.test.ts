import { describe, expect, it } from 'vitest';
import { create } from '@bufbuild/protobuf';
import {
  applyMobilityChunkDelta,
  applyMobilityChunkSnapshot,
  applyMobilitySnapshot,
  applyServerMessage,
  createMobilityOverlayState,
  interpolatedAgents,
  mobilityDiagnostics,
  trafficDiagnostics,
} from '../../src/backend/mobilityState';
import type { AgentMobilityDto, MobilityChunkDeltaDto, MobilityChunkSnapshotDto, MobilitySnapshotDto } from '../../src/backend/mobilityProtocol';
import {
  ChunkCoordSchema,
  Direction,
  MobilityChunkDeltaSchema,
  MobilityChunkSnapshotSchema,
  ServerMessageSchema,
  AgentMobilitySchema,
  AgentStateSchema,
  WalkingSchema,
  WorldCoordSchema,
} from '../../src/backend/proto/abutown_pb';

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
      id: 'vehicle:car:0:0',
      kind: 'car',
      route_id: 'route:arterial:0',
      link_index: 0,
      progress: 0,
      capacity: 1,
      occupants: [],
      dwell_ticks_remaining: 0,
      world_coord: { x: 0, y: 0 },
      direction: 'e',
      sprite_key: 'vehicle:0',
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

  it('applies chunk deltas by replacing changed agents and vehicles', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 100);
    const next = applyMobilityChunkDelta(
      state,
      {
        type: 'mobility_chunk_delta',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 3,
        chunk: { x: 0, y: 0 },
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
        left_agents: [],
        left_vehicles: [],
      },
      200
    );

    expect(mobilityDiagnostics(next)).toMatchObject({ tick: 3, agents: 1, vehicles: 1 });
  });

  it('counts invalid messages without dropping known records', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 100);
    // An empty ServerMessage envelope (no oneof body set) — represents a
    // proto frame that decoded successfully but carries no recognized
    // payload (e.g. backend added a new variant we haven't taught the
    // client to handle yet).
    const emptyEnvelope = create(ServerMessageSchema, {});
    const next = applyServerMessage(state, emptyEnvelope, 200);

    expect(mobilityDiagnostics(next)).toMatchObject({ agents: 1, invalidMessages: 1 });
  });

  it('applyServerMessage routes proto MobilityChunkDelta into the reducer', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 100);
    const delta = create(MobilityChunkDeltaSchema, {
      protocolVersion: 16,
      worldId: 'abutown-main',
      tick: 3n,
      chunk: create(ChunkCoordSchema, { x: 0, y: 0 }),
      changedAgents: [
        create(AgentMobilitySchema, {
          id: 'agent:pedestrian:0',
          state: create(AgentStateSchema, {
            state: { case: 'walking', value: create(WalkingSchema, { linkId: 'link:next', progress: 0.1 }) },
          }),
          planCursor: 1,
          worldCoord: create(WorldCoordSchema, { x: 5, y: 5 }),
          direction: Direction.E,
          spriteKey: 'pedestrian:0',
        }),
      ],
      changedVehicles: [],
      leftAgents: [],
      leftVehicles: [],
    });
    const envelope = create(ServerMessageSchema, {
      body: { case: 'mobilityChunkDelta', value: delta },
    });
    const next = applyServerMessage(state, envelope, 200);
    expect(next.tick).toBe(3);
    expect(next.agents.get('agent:pedestrian:0')?.current.world_coord).toEqual({ x: 5, y: 5 });
  });

  it('applyServerMessage routes proto MobilityChunkSnapshot into the reducer', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 100);
    const snap = create(MobilityChunkSnapshotSchema, {
      protocolVersion: 16,
      worldId: 'abutown-main',
      tick: 4n,
      chunk: create(ChunkCoordSchema, { x: 0, y: 0 }),
      agents: [],
      vehicles: [],
    });
    const envelope = create(ServerMessageSchema, {
      body: { case: 'mobilityChunkSnapshot', value: snap },
    });
    const next = applyServerMessage(state, envelope, 200);
    expect(next.tick).toBe(4);
    // Snapshot for chunk (0,0) wiped the agent originally there.
    expect(next.agents.has('agent:pedestrian:0')).toBe(false);
  });

  it('applyMobilityChunkDelta drops entities listed in left_agents and left_vehicles', () => {
    const state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'w',
        tick: 0,
        agents: [{
          id: 'agent:walk:1',
          state: { type: 'walking', link_id: 'l', progress: 0 },
          plan_cursor: 0,
          world_coord: { x: 0, y: 0 },
          direction: 'e',
          sprite_key: 'p:0',
        }],
        vehicles: [{
          id: 'vehicle:car:0:0',
          kind: 'car',
          route_id: 'r',
          link_index: 0,
          progress: 0,
          capacity: 1,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 0, y: 0 },
          direction: 'e',
          sprite_key: 'c:0',
        }],
        stops: [],
      },
      0,
    );
    expect(state.agents.size).toBe(1);
    expect(state.vehicles.size).toBe(1);

    const after = applyMobilityChunkDelta(state, {
      type: 'mobility_chunk_delta',
      protocol_version: 1,
      world_id: 'w',
      tick: 1,
      chunk: { x: 0, y: 0 },
      changed_agents: [],
      changed_vehicles: [],
      left_agents: ['agent:walk:1'],
      left_vehicles: ['vehicle:car:0:0'],
    }, 100);
    expect(after.agents.size).toBe(0);
    expect(after.vehicles.size).toBe(0);
  });

  it('drops cars when they leave a subscribed chunk', () => {
    const state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'w',
        tick: 0,
        agents: [],
        vehicles: [{
          id: 'vehicle:car:outside-viewport',
          kind: 'car',
          route_id: 'route:arterial:0',
          link_index: 0,
          progress: 0,
          capacity: 1,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 150, y: 224 },
          direction: 's',
          sprite_key: 'vehicle:0',
        }],
        stops: [],
      },
      0,
    );

    const after = applyMobilityChunkDelta(state, {
      type: 'mobility_chunk_delta',
      protocol_version: 1,
      world_id: 'w',
      tick: 1,
      chunk: { x: 4, y: 7 },
      changed_agents: [],
      changed_vehicles: [],
      left_agents: [],
      left_vehicles: ['vehicle:car:outside-viewport'],
    }, 100);

    expect(after.vehicles.has('vehicle:car:outside-viewport')).toBe(false);
  });

  it('applyMobilityChunkSnapshot replaces entities for that chunk only', () => {
    // agent in chunk (0,0): world_coord x in [0,31], y in [0,31]
    // agent in chunk (1,0): world_coord x in [32,63], y in [0,31]
    const seedState = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'w',
        tick: 0,
        agents: [
          {
            id: 'agent:chunk00',
            state: { type: 'walking', link_id: 'l', progress: 0 },
            plan_cursor: 0,
            world_coord: { x: 0, y: 0 },
            direction: 'e',
            sprite_key: 'p:0',
          },
          {
            id: 'agent:chunk10',
            state: { type: 'walking', link_id: 'l', progress: 0 },
            plan_cursor: 0,
            world_coord: { x: 32, y: 0 },
            direction: 'e',
            sprite_key: 'p:0',
          },
        ],
        vehicles: [],
        stops: [],
      },
      0,
    );
    expect(seedState.agents.size).toBe(2);

    // Apply snapshot for chunk (0,0) with a different agent
    const chunkSnap: MobilityChunkSnapshotDto = {
      type: 'mobility_chunk_snapshot',
      protocol_version: 1,
      world_id: 'w',
      tick: 1,
      chunk: { x: 0, y: 0 },
      agents: [{
        id: 'agent:chunk00:new',
        state: { type: 'walking', link_id: 'l', progress: 0 },
        plan_cursor: 0,
        world_coord: { x: 10, y: 10 },
        direction: 'e',
        sprite_key: 'p:0',
      }],
      vehicles: [],
    };

    const after = applyMobilityChunkSnapshot(seedState, chunkSnap, 100);
    // Old chunk (0,0) agent gone, new snapshot agent present
    expect(after.agents.has('agent:chunk00')).toBe(false);
    expect(after.agents.has('agent:chunk00:new')).toBe(true);
    // chunk (1,0) agent untouched
    expect(after.agents.has('agent:chunk10')).toBe(true);
    expect(after.agents.size).toBe(2);
  });

  it('ignores stale chunk snapshots and deltas', () => {
    const seedState = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'w',
        tick: 10,
        agents: [],
        vehicles: [{
          id: 'vehicle:car:0',
          kind: 'car',
          route_id: 'route:arterial:0',
          link_index: 0,
          progress: 0.25,
          capacity: 1,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 150, y: 224 },
          direction: 's',
          sprite_key: 'vehicle:0',
        }],
        stops: [],
      },
      0,
    );

    const staleSnapshot = applyMobilityChunkSnapshot(seedState, {
      type: 'mobility_chunk_snapshot',
      protocol_version: 1,
      world_id: 'w',
      tick: 9,
      chunk: { x: 4, y: 7 },
      agents: [],
      vehicles: [],
    }, 100);
    expect(staleSnapshot).toBe(seedState);
    expect(staleSnapshot.vehicles.has('vehicle:car:0')).toBe(true);

    const staleDelta = applyMobilityChunkDelta(seedState, {
      type: 'mobility_chunk_delta',
      protocol_version: 1,
      world_id: 'w',
      tick: 9,
      chunk: { x: 4, y: 7 },
      changed_agents: [],
      changed_vehicles: [],
      left_agents: [],
      left_vehicles: ['vehicle:car:0'],
    }, 100);
    expect(staleDelta).toBe(seedState);
    expect(staleDelta.vehicles.has('vehicle:car:0')).toBe(true);
  });

  it('reports traffic diagnostics for car movement and invalid route messages', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'w',
        tick: 0,
        agents: [],
        vehicles: [
          {
            id: 'vehicle:car:moving',
            kind: 'car',
            route_id: 'route:arterial:0',
            link_index: 0,
            progress: 0,
            capacity: 1,
            occupants: [],
            dwell_ticks_remaining: 0,
            world_coord: { x: 0, y: 0 },
            direction: 'e',
            sprite_key: 'vehicle:0',
          },
          {
            id: 'vehicle:car:stuck',
            kind: 'car',
            route_id: 'route:arterial:1',
            link_index: 0,
            progress: 0,
            capacity: 1,
            occupants: [],
            dwell_ticks_remaining: 0,
            world_coord: { x: 10, y: 10 },
            direction: 'e',
            sprite_key: 'vehicle:1',
          },
        ],
        stops: [],
      },
      0,
    );

    state = applyMobilityChunkDelta(state, {
      type: 'mobility_chunk_delta',
      protocol_version: 1,
      world_id: 'w',
      tick: 1,
      chunk: { x: 0, y: 0 },
      changed_agents: [],
      changed_vehicles: [
        {
          id: 'vehicle:car:moving',
          kind: 'car',
          route_id: 'route:arterial:0',
          link_index: 0,
          progress: 0.5,
          capacity: 1,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 5, y: 0 },
          direction: 'e',
          sprite_key: 'vehicle:0',
        },
        {
          id: 'vehicle:car:stuck',
          kind: 'car',
          route_id: 'route:arterial:1',
          link_index: 0,
          progress: 0,
          capacity: 1,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 10, y: 10 },
          direction: 'e',
          sprite_key: 'vehicle:1',
        },
      ],
      left_agents: [],
      left_vehicles: [],
    }, 100);

    state = applyServerMessage(state, create(ServerMessageSchema, {}), 200);

    expect(trafficDiagnostics(state)).toEqual({
      routes: 2,
      cars: 2,
      movingCars: 1,
      stuckCars: 1,
      invalidRouteCars: 1,
    });
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

  it('chunk delta moves prev<-current and sets current=new dto', () => {
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
    const delta: MobilityChunkDeltaDto = {
      type: 'mobility_chunk_delta',
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 2,
      chunk: { x: 3, y: 6 },
      changed_agents: [agentAt('agent:seed:0', 110, 200)],
      changed_vehicles: [],
      left_agents: [],
      left_vehicles: [],
    };
    state = applyMobilityChunkDelta(state, delta, 1100);
    const entry = state.agents.get('agent:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.current.world_coord).toEqual({ x: 110, y: 200 });
    expect(entry.lastTickAt).toBe(1100);
  });

  it('chunk delta for a new agent sets prev == current', () => {
    let state = createMobilityOverlayState();
    state = applyMobilityChunkDelta(
      state,
      {
        type: 'mobility_chunk_delta',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        chunk: { x: 1, y: 1 },
        changed_agents: [agentAt('agent:seed:0', 50, 60)],
        changed_vehicles: [],
        left_agents: [],
        left_vehicles: [],
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
    state = applyMobilityChunkDelta(
      state,
      {
        type: 'mobility_chunk_delta',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        chunk: { x: 3, y: 6 },
        changed_agents: [agentAt('agent:seed:0', 110, 200)],
        changed_vehicles: [],
        left_agents: [],
        left_vehicles: [],
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
    state = applyMobilityChunkDelta(
      state,
      {
        type: 'mobility_chunk_delta',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        chunk: { x: 0, y: 0 },
        changed_agents: [agentAt('agent:seed:0', 100, 0)],
        changed_vehicles: [],
        left_agents: [],
        left_vehicles: [],
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
