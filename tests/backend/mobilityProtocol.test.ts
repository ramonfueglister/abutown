import { describe, expect, it } from 'vitest';
import { create, toBinary, fromBinary } from '@bufbuild/protobuf';
import {
  isMobilityChunkDeltaDto,
  isMobilityChunkSnapshotDto,
  isMobilitySnapshotDto,
  isWorldSummaryDto,
  mobilityChunkDeltaFromProto,
  mobilityChunkSnapshotFromProto,
  agentMobilityFromProto,
  vehicleMobilityFromProto,
  directionFromProto,
  type MobilitySnapshotDto,
} from '../../src/backend/mobilityProtocol';
import {
  AgentMobilitySchema,
  AgentStateSchema,
  ChunkCoordSchema,
  ChunkSubscribeSchema,
  ClientMessageSchema,
  Direction,
  MobilityChunkDeltaSchema,
  MobilityChunkSnapshotSchema,
  VehicleKind,
  VehicleMobilitySchema,
  WalkingSchema,
  WorldCoordSchema,
} from '../../src/backend/proto/abutown_pb';

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

  it('accepts a valid MobilityChunkDelta object', () => {
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
  });

  it('accepts a valid MobilityChunkSnapshot object', () => {
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
  });

  it('rejects malformed mobility snapshot payloads', () => {
    expect(
      isMobilitySnapshotDto({
        ...snapshot,
        agents: [{ id: 'agent:bad', state: { type: 'walking' }, plan_cursor: 0 }],
      }),
    ).toBe(false);
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

describe('proto ↔ DTO converters', () => {
  it('encodes a chunk_subscribe binary frame that round-trips through ClientMessage', () => {
    const msg = create(ClientMessageSchema, {
      body: {
        case: 'chunkSubscribe',
        value: create(ChunkSubscribeSchema, {
          protocolVersion: 16,
          coords: [create(ChunkCoordSchema, { x: 4, y: 4 }), create(ChunkCoordSchema, { x: 5, y: 4 })],
        }),
      },
    });
    const bytes = toBinary(ClientMessageSchema, msg);
    const decoded = fromBinary(ClientMessageSchema, bytes);
    expect(decoded.body.case).toBe('chunkSubscribe');
    if (decoded.body.case !== 'chunkSubscribe') throw new Error('unreachable');
    expect(decoded.body.value.coords.map((c) => ({ x: c.x, y: c.y }))).toEqual([
      { x: 4, y: 4 },
      { x: 5, y: 4 },
    ]);
  });

  it('directionFromProto maps every Direction enum value to its DTO string', () => {
    expect(directionFromProto(Direction.N)).toBe('n');
    expect(directionFromProto(Direction.E)).toBe('e');
    expect(directionFromProto(Direction.NW)).toBe('nw');
  });

  it('rejects missing direction', () => {
    expect(() => directionFromProto(Direction.UNSPECIFIED)).toThrow(/missing direction/);
    expect(() => directionFromProto(999 as Direction)).toThrow(/missing direction/);
  });

  it('agentMobilityFromProto converts proto AgentMobility → snake_case DTO', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'agent:proto:0',
      state: create(AgentStateSchema, {
        state: { case: 'walking', value: create(WalkingSchema, { linkId: 'link:A', progress: 0.5 }) },
      }),
      planCursor: 2,
      worldCoord: create(WorldCoordSchema, { x: 10, y: 20 }),
      direction: Direction.E,
      spriteKey: 'pedestrian:0',
    });
    const dto = agentMobilityFromProto(proto);
    expect(dto.id).toBe('agent:proto:0');
    expect(dto.world_coord).toEqual({ x: 10, y: 20 });
    expect(dto.direction).toBe('e');
    expect(dto.sprite_key).toBe('pedestrian:0');
    expect(dto.plan_cursor).toBe(2);
    expect(dto.state).toEqual({ type: 'walking', link_id: 'link:A', progress: 0.5 });
  });

  it('rejects missing agent state', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'agent:bad',
      worldCoord: { x: 1, y: 2 },
      direction: Direction.E,
      spriteKey: 'pedestrian:0',
      planCursor: 0,
    });

    expect(() => agentMobilityFromProto(proto)).toThrow(/missing AgentState/);
  });

  it('rejects missing world coord', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'agent:bad',
      state: create(AgentStateSchema, {
        state: { case: 'walking', value: create(WalkingSchema, { linkId: 'edge:7', progress: 0.5 }) },
      }),
      direction: Direction.E,
      spriteKey: 'pedestrian:0',
      planCursor: 0,
    });

    expect(() => agentMobilityFromProto(proto)).toThrow(/missing world_coord/);
  });

  it('rejects unspecified agent direction', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'agent:bad',
      state: create(AgentStateSchema, {
        state: { case: 'walking', value: create(WalkingSchema, { linkId: 'edge:7', progress: 0.5 }) },
      }),
      worldCoord: { x: 1, y: 2 },
      direction: Direction.UNSPECIFIED,
      spriteKey: 'pedestrian:0',
      planCursor: 0,
    });

    expect(() => agentMobilityFromProto(proto)).toThrow(/missing direction/);
  });

  it('accepts graph-native walking edge ids', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'agent:ok',
      state: create(AgentStateSchema, {
        state: { case: 'walking', value: create(WalkingSchema, { linkId: 'edge:7', progress: 0.5 }) },
      }),
      worldCoord: { x: 7, y: 8 },
      direction: Direction.E,
      spriteKey: 'pedestrian:0',
      planCursor: 0,
    });

    expect(agentMobilityFromProto(proto).state).toEqual({
      type: 'walking',
      link_id: 'edge:7',
      progress: 0.5,
    });
  });

  it('vehicleMobilityFromProto converts proto VehicleMobility → snake_case DTO', () => {
    const proto = create(VehicleMobilitySchema, {
      id: 'vehicle:proto:0',
      kind: VehicleKind.TRAM,
      routeId: 'route:0',
      linkIndex: 1,
      progress: 0.25,
      capacity: 4,
      occupants: ['agent:0'],
      dwellTicksRemaining: 3,
      worldCoord: create(WorldCoordSchema, { x: 30, y: 40 }),
      direction: Direction.S,
      spriteKey: 'tram:0',
    });
    const dto = vehicleMobilityFromProto(proto);
    expect(dto).toMatchObject({
      id: 'vehicle:proto:0',
      kind: 'tram',
      route_id: 'route:0',
      link_index: 1,
      progress: 0.25,
      capacity: 4,
      occupants: ['agent:0'],
      dwell_ticks_remaining: 3,
      world_coord: { x: 30, y: 40 },
      direction: 's',
      sprite_key: 'tram:0',
    });
  });

  it('rejects missing vehicle world coord', () => {
    const proto = create(VehicleMobilitySchema, {
      id: 'vehicle:bad',
      kind: VehicleKind.TRAM,
      routeId: 'route:0',
      linkIndex: 1,
      progress: 0.25,
      capacity: 4,
      occupants: [],
      dwellTicksRemaining: 3,
      direction: Direction.S,
      spriteKey: 'tram:0',
    });

    expect(() => vehicleMobilityFromProto(proto)).toThrow(/missing world_coord/);
  });

  it('mobilityChunkDeltaFromProto converts proto delta → snake_case DTO with bigint→number tick', () => {
    const proto = create(MobilityChunkDeltaSchema, {
      protocolVersion: 16,
      worldId: 'abutown-main',
      tick: 5n,
      chunk: create(ChunkCoordSchema, { x: 4, y: 4 }),
      changedAgents: [],
      changedVehicles: [],
      leftAgents: ['agent:gone'],
      leftVehicles: [],
    });
    const dto = mobilityChunkDeltaFromProto(proto);
    expect(dto.type).toBe('mobility_chunk_delta');
    expect(dto.world_id).toBe('abutown-main');
    expect(dto.tick).toBe(5);
    expect(dto.chunk).toEqual({ x: 4, y: 4 });
    expect(dto.left_agents).toEqual(['agent:gone']);
  });

  it('mobilityChunkSnapshotFromProto converts proto snapshot → snake_case DTO', () => {
    const proto = create(MobilityChunkSnapshotSchema, {
      protocolVersion: 16,
      worldId: 'abutown-main',
      tick: 7n,
      chunk: create(ChunkCoordSchema, { x: 1, y: 2 }),
      agents: [],
      vehicles: [],
    });
    const dto = mobilityChunkSnapshotFromProto(proto);
    expect(dto.type).toBe('mobility_chunk_snapshot');
    expect(dto.tick).toBe(7);
    expect(dto.chunk).toEqual({ x: 1, y: 2 });
  });
});
