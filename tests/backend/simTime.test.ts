import { describe, expect, it } from 'vitest';
import { create, toBinary } from '@bufbuild/protobuf';
import {
  WorldSummarySchema,
  AgentMobilitySchema,
  AgentStateSchema,
  WalkingSchema,
  WorldCoordSchema,
  Direction,
} from '../../src/backend/proto/abutown_pb';
import { formatSimDate } from '../../src/backend/simTime';
import { worldSummaryFromProto, agentMobilityFromProto } from '../../src/backend/mobilityProtocol';
import { requireMobilitySnapshot } from '../../src/backend/mobilityClient';
import { buildBackendPedestrianInspector } from '../../src/render/entityInspector';
import type { MobilitySnapshotDto, WorldSummaryDto } from '../../src/backend/mobilityProtocol';
import { MobilitySnapshotSchema } from '../../src/backend/proto/abutown_pb';

// ---------------------------------------------------------------------------
// formatSimDate helper
// ---------------------------------------------------------------------------
describe('formatSimDate', () => {
  it('formats year 0 day 0', () => {
    expect(formatSimDate(0)).toBe('Year 0, Day 0');
  });

  it('formats a partial day (less than one day elapsed)', () => {
    expect(formatSimDate(3600)).toBe('Year 0, Day 0');
  });

  it('formats exactly one day', () => {
    expect(formatSimDate(86_400)).toBe('Year 0, Day 1');
  });

  it('formats exactly one year', () => {
    expect(formatSimDate(31_536_000)).toBe('Year 1, Day 0');
  });

  it('formats year 1 day 5', () => {
    expect(formatSimDate(31_536_000 + 5 * 86_400)).toBe('Year 1, Day 5');
  });

  it('formats a large sim time', () => {
    // year 10, day 100
    expect(formatSimDate(10 * 31_536_000 + 100 * 86_400)).toBe('Year 10, Day 100');
  });
});

// ---------------------------------------------------------------------------
// worldSummaryFromProto surfaces simTime
// ---------------------------------------------------------------------------
describe('worldSummaryFromProto', () => {
  it('parses simTime from the proto', () => {
    const proto = create(WorldSummarySchema, {
      protocolVersion: 1,
      worldId: 'w1',
      chunkSize: 32,
      loadedChunks: [],
      tickPeriodMs: 100,
      simTime: BigInt(31_536_000 + 86_400), // year 1, day 1
    });
    const dto = worldSummaryFromProto(proto);
    expect(dto.sim_time).toBe(31_536_000 + 86_400);
  });

  it('defaults simTime to 0 when field is absent', () => {
    const proto = create(WorldSummarySchema, {
      protocolVersion: 1,
      worldId: 'w1',
      chunkSize: 32,
      loadedChunks: [],
      tickPeriodMs: 100,
    });
    const dto = worldSummaryFromProto(proto);
    expect(dto.sim_time).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// agentMobilityFromProto surfaces ageSeconds
// ---------------------------------------------------------------------------
describe('agentMobilityFromProto', () => {
  it('parses ageSeconds from the proto', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'a1',
      planCursor: 0,
      worldCoord: create(WorldCoordSchema, { x: 0, y: 0 }),
      direction: Direction.E,
      spriteKey: 'p:0',
      state: create(AgentStateSchema, {
        state: {
          case: 'walking',
          value: create(WalkingSchema, { linkId: 'l1', progress: 0.5 }),
        },
      }),
      ageSeconds: BigInt(3 * 31_536_000), // 3 years
    });
    const dto = agentMobilityFromProto(proto);
    expect(dto.age_seconds).toBe(3 * 31_536_000);
  });

  it('defaults ageSeconds to 0 when field is absent', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'a1',
      planCursor: 0,
      worldCoord: create(WorldCoordSchema, { x: 0, y: 0 }),
      direction: Direction.E,
      spriteKey: 'p:0',
      state: create(AgentStateSchema, {
        state: {
          case: 'walking',
          value: create(WalkingSchema, { linkId: 'l1', progress: 0.5 }),
        },
      }),
    });
    const dto = agentMobilityFromProto(proto);
    expect(dto.age_seconds).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// requireMobilitySnapshot surfaces simTime in RequiredMobility
// ---------------------------------------------------------------------------
describe('requireMobilitySnapshot simTime', () => {
  function makeWorldSummaryResponse(simTime: number): Response {
    const message = create(WorldSummarySchema, {
      protocolVersion: 1,
      worldId: 'w1',
      chunkSize: 32,
      loadedChunks: [],
      tickPeriodMs: 100,
      simTime: BigInt(simTime),
    });
    return new Response(toBinary(WorldSummarySchema, message), {
      status: 200,
      headers: { 'content-type': 'application/x-protobuf' },
    });
  }

  function makeSnapshotResponse(): Response {
    const message = create(MobilitySnapshotSchema, {
      protocolVersion: 1,
      worldId: 'w1',
      tick: BigInt(0),
      agents: [],
      vehicles: [],
      stops: [],
    });
    return new Response(toBinary(MobilitySnapshotSchema, message), {
      status: 200,
      headers: { 'content-type': 'application/x-protobuf' },
    });
  }

  it('exposes simTime from /world in the returned RequiredMobility', async () => {
    const expectedSimTime = 31_536_000 + 5 * 86_400; // year 1, day 5

    const fetchImpl = ((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes('/world')) return Promise.resolve(makeWorldSummaryResponse(expectedSimTime));
      return Promise.resolve(makeSnapshotResponse());
    }) as typeof fetch;

    const result = await requireMobilitySnapshot({ fetchImpl });
    expect(result.simTime).toBe(expectedSimTime);
  });
});

// ---------------------------------------------------------------------------
// buildBackendPedestrianInspector includes Age row
// ---------------------------------------------------------------------------
describe('buildBackendPedestrianInspector Age row', () => {
  it('includes an Age row for an agent with ageSeconds', () => {
    const inspector = buildBackendPedestrianInspector({
      id: 'agent:1',
      path: [{ x: 10, y: 20 }, { x: 11, y: 20 }],
      offset: 0,
      speed: 0,
      laneOffset: 0,
      direction: 'e',
      ageSeconds: 2 * 31_536_000, // 2 years
      sprite: { sheet: 'minimal-peds.0', frameWidth: 16, frameHeight: 32 },
    });
    expect(inspector).not.toBeNull();
    const ageRow = inspector?.rows.find((r) => r.label === 'Age');
    expect(ageRow).toBeDefined();
    expect(ageRow?.value).toBe('2.0 yr');
  });

  it('shows 0.0 yr for an agent with ageSeconds = 0', () => {
    const inspector = buildBackendPedestrianInspector({
      id: 'agent:2',
      path: [{ x: 0, y: 0 }],
      offset: 0,
      speed: 0,
      laneOffset: 0,
      direction: 'n',
      ageSeconds: 0,
      sprite: { sheet: 'minimal-peds.0' },
    });
    const ageRow = inspector?.rows.find((r) => r.label === 'Age');
    expect(ageRow?.value).toBe('0.0 yr');
  });
});
