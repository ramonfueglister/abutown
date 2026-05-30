import { describe, expect, it } from 'vitest';
import { create, toBinary, fromBinary } from '@bufbuild/protobuf';
import {
  AgentMobilitySchema,
  AgentStateSchema,
  ChunkCoordSchema,
  ChunkSubscribeSchema,
  ClientMessageSchema,
  Direction,
  HelloSchema,
  MobilityChunkDeltaSchema,
  ServerMessageSchema,
  WalkingSchema,
  WorldCoordSchema,
} from './abutown_pb';

describe('proto roundtrip (TS @bufbuild/protobuf v2 API)', () => {
  it('roundtrips ServerMessage carrying Hello', () => {
    const hello = create(HelloSchema, {
      protocolVersion: 16,
      worldId: 'abutopia',
      chunkSize: 32,
    });
    const msg = create(ServerMessageSchema, {
      body: { case: 'hello', value: hello },
    });

    const bytes = toBinary(ServerMessageSchema, msg);
    const back = fromBinary(ServerMessageSchema, bytes);

    expect(back.body.case).toBe('hello');
    if (back.body.case !== 'hello') throw new Error('unreachable');
    expect(back.body.value.worldId).toBe('abutopia');
    expect(back.body.value.chunkSize).toBe(32);
    expect(back.body.value.protocolVersion).toBe(16);
  });

  it('roundtrips ClientMessage carrying ChunkSubscribe with two coords', () => {
    const subscribe = create(ChunkSubscribeSchema, {
      protocolVersion: 16,
      coords: [
        create(ChunkCoordSchema, { x: 0, y: 0 }),
        create(ChunkCoordSchema, { x: -3, y: 7 }),
      ],
    });
    const msg = create(ClientMessageSchema, {
      body: { case: 'chunkSubscribe', value: subscribe },
    });

    const bytes = toBinary(ClientMessageSchema, msg);
    const back = fromBinary(ClientMessageSchema, bytes);

    expect(back.body.case).toBe('chunkSubscribe');
    if (back.body.case !== 'chunkSubscribe') throw new Error('unreachable');
    expect(back.body.value.coords).toHaveLength(2);
    expect(back.body.value.coords[0].x).toBe(0);
    expect(back.body.value.coords[0].y).toBe(0);
    expect(back.body.value.coords[1].x).toBe(-3);
    expect(back.body.value.coords[1].y).toBe(7);
  });

  it('roundtrips ServerMessage carrying MobilityChunkDelta with one walking agent', () => {
    const walking = create(WalkingSchema, {
      linkId: 'link-42',
      progress: 0.5,
    });
    const state = create(AgentStateSchema, {
      state: { case: 'walking', value: walking },
    });
    const agent = create(AgentMobilitySchema, {
      id: 'agent-1',
      state,
      worldCoord: create(WorldCoordSchema, { x: 12.5, y: -4.25 }),
      direction: Direction.NE,
      spriteKey: 'pedestrian.basic',
      planCursor: 3,
    });
    const delta = create(MobilityChunkDeltaSchema, {
      protocolVersion: 16,
      worldId: 'abutopia',
      tick: 1234n,
      chunk: create(ChunkCoordSchema, { x: 1, y: -2 }),
      changedAgents: [agent],
      changedVehicles: [],
      leftAgents: ['agent-7', 'agent-9'],
      leftVehicles: [],
    });
    const msg = create(ServerMessageSchema, {
      body: { case: 'mobilityChunkDelta', value: delta },
    });

    const bytes = toBinary(ServerMessageSchema, msg);
    const back = fromBinary(ServerMessageSchema, bytes);

    expect(back.body.case).toBe('mobilityChunkDelta');
    if (back.body.case !== 'mobilityChunkDelta') throw new Error('unreachable');
    const d = back.body.value;
    expect(d.worldId).toBe('abutopia');
    expect(d.tick).toBe(1234n);
    expect(d.chunk?.x).toBe(1);
    expect(d.chunk?.y).toBe(-2);
    expect(d.changedAgents).toHaveLength(1);
    const a = d.changedAgents[0];
    expect(a.id).toBe('agent-1');
    expect(a.direction).toBe(Direction.NE);
    expect(a.spriteKey).toBe('pedestrian.basic');
    expect(a.planCursor).toBe(3);
    expect(a.worldCoord?.x).toBeCloseTo(12.5, 5);
    expect(a.worldCoord?.y).toBeCloseTo(-4.25, 5);
    expect(a.state?.state.case).toBe('walking');
    if (a.state?.state.case !== 'walking') throw new Error('unreachable');
    expect(a.state.state.value.linkId).toBe('link-42');
    expect(a.state.state.value.progress).toBeCloseTo(0.5, 5);
    expect(d.leftAgents).toEqual(['agent-7', 'agent-9']);
    expect(d.leftVehicles).toEqual([]);
  });

  it('encodes a one-agent MobilityChunkDelta in a sane byte budget', () => {
    // Guard against accidental regression to verbose encodings (e.g. JSON).
    // A single walking-agent delta with short ids should be ~100 bytes;
    // 200 bytes is a generous upper bound.
    const walking = create(WalkingSchema, { linkId: 'link-42', progress: 0.5 });
    const state = create(AgentStateSchema, {
      state: { case: 'walking', value: walking },
    });
    const agent = create(AgentMobilitySchema, {
      id: 'agent-1',
      state,
      worldCoord: create(WorldCoordSchema, { x: 12.5, y: -4.25 }),
      direction: Direction.NE,
      spriteKey: 'pedestrian.basic',
      planCursor: 3,
    });
    const delta = create(MobilityChunkDeltaSchema, {
      protocolVersion: 16,
      worldId: 'abutopia',
      tick: 1234n,
      chunk: create(ChunkCoordSchema, { x: 1, y: -2 }),
      changedAgents: [agent],
    });
    const msg = create(ServerMessageSchema, {
      body: { case: 'mobilityChunkDelta', value: delta },
    });
    const bytes = toBinary(ServerMessageSchema, msg);
    expect(bytes.byteLength).toBeLessThan(200);
    expect(bytes.byteLength).toBeGreaterThan(0);
  });
});
