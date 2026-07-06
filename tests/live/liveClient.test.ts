// tests/live/liveClient.test.ts
//
// Task 14: pure frame-handling tests for the live-channel client core
// (WS/DOM-free, mirroring tests/traffic/trafficClient.test.ts's testable-core
// split). Frames are hand-constructed LiveServerMsg binaries via
// @bufbuild/protobuf create + toBinary, then fed through the SAME decode path
// the WebSocket handler uses (LiveClientCore.handleBinary):
//
//   (a) vitals: the onVitals callback receives converted values (bigints kept
//       as bigints, audit_ok uint -> boolean, prices mapped 1:1);
//   (b) keyframe REPLACES the full membership of its cell (stale citizens the
//       keyframe no longer lists are gone) and positions decode x_dm/z_dm
//       (decimetres, sint32 — negatives survive) -> metres (/10);
//   (c) delta upserts `citizens` and removes `departed`.

import { describe, expect, it } from 'vitest';
import { create, toBinary, type MessageInitShape } from '@bufbuild/protobuf';
import { LiveServerMsgSchema } from '../../src/proto/live_pb';
import { LiveClientCore, type LiveCitizen, type LiveVitals } from '../../src/diorama/live/liveClient';

function bin(msg: MessageInitShape<typeof LiveServerMsgSchema>): Uint8Array {
  return toBinary(LiveServerMsgSchema, create(LiveServerMsgSchema, msg));
}

interface CitizenEvent {
  cell: number;
  citizens: LiveCitizen[];
  departed: number[];
  keyframe: boolean;
}

function makeCore(): {
  core: LiveClientCore;
  vitals: LiveVitals[];
  citizenEvents: CitizenEvent[];
} {
  const vitals: LiveVitals[] = [];
  const citizenEvents: CitizenEvent[] = [];
  const core = new LiveClientCore({
    onVitals: (v) => vitals.push(v),
    onCitizens: (cell, citizens, departed, keyframe) =>
      citizenEvents.push({ cell, citizens, departed, keyframe }),
  });
  return { core, vitals, citizenEvents };
}

describe('LiveClientCore — vitals conversion', () => {
  it('delivers converted vitals to onVitals', () => {
    const { core, vitals } = makeCore();
    core.handleBinary(
      bin({
        vitals: {
          worldTick: 1234n,
          sOfWorldDay: 27_000, // 07:30 on the 4h world day scale (seconds of world day)
          population: 8_500n,
          totalMoney: 123_456_789n, // raw ×1000
          auditOk: 1,
          prices: [
            { marketId: 7, goodId: 2, ewmaPrice: 4_200n, marketName: 'Altstadt' },
            { marketId: 9, goodId: 3, ewmaPrice: 900n, marketName: 'Toess' },
          ],
          tripsActive: 42n,
        },
      }),
    );

    expect(vitals).toHaveLength(1);
    const v = vitals[0];
    expect(v.worldTick).toBe(1234n);
    expect(v.sOfWorldDay).toBe(27_000);
    expect(v.population).toBe(8_500n);
    expect(v.totalMoney).toBe(123_456_789n);
    expect(v.auditOk).toBe(true);
    expect(v.tripsActive).toBe(42n);
    expect(v.prices).toEqual([
      { marketId: 7, goodId: 2, ewmaPrice: 4_200n, marketName: 'Altstadt' },
      { marketId: 9, goodId: 3, ewmaPrice: 900n, marketName: 'Toess' },
    ]);
  });

  it('converts audit_ok 0 to false and does not fire onVitals without vitals', () => {
    const { core, vitals } = makeCore();
    core.handleBinary(bin({})); // empty server msg — no vitals present
    expect(vitals).toHaveLength(0);
    core.handleBinary(bin({ vitals: { worldTick: 1n, auditOk: 0 } }));
    expect(vitals).toHaveLength(1);
    expect(vitals[0].auditOk).toBe(false);
  });
});

describe('LiveClientCore — citizen cell frames', () => {
  it('decodes x_dm/z_dm (sint32 decimetres) to metres (/10), negatives included', () => {
    const { core, citizenEvents } = makeCore();
    core.handleBinary(
      bin({
        cells: [
          {
            cell: 11,
            worldTick: 100n,
            keyframe: true,
            citizens: [{ id: 1, xDm: -12345, zDm: 6789, activity: 3 }],
            departed: [],
          },
        ],
      }),
    );
    expect(citizenEvents).toHaveLength(1);
    const ev = citizenEvents[0];
    expect(ev.cell).toBe(11);
    expect(ev.keyframe).toBe(true);
    expect(ev.citizens).toEqual([{ id: 1, x: -1234.5, z: 678.9, activity: 3 }]);
    // core state mirrors the frame
    expect([...core.citizensInCell(11)].map((c) => c.id)).toEqual([1]);
  });

  it('keyframe REPLACES the full membership of its cell', () => {
    const { core, citizenEvents } = makeCore();
    // Keyframe 1: citizens A(1) and B(2) in cell 5.
    core.handleBinary(
      bin({
        cells: [
          {
            cell: 5,
            worldTick: 10n,
            keyframe: true,
            citizens: [
              { id: 1, xDm: 100, zDm: 200, activity: 0 },
              { id: 2, xDm: 300, zDm: 400, activity: 1 },
            ],
            departed: [],
          },
        ],
      }),
    );
    expect([...core.citizensInCell(5)].map((c) => c.id).sort()).toEqual([1, 2]);

    // Keyframe 2 lists ONLY C(3): A and B are ghosts and must be healed away.
    core.handleBinary(
      bin({
        cells: [
          {
            cell: 5,
            worldTick: 20n,
            keyframe: true,
            citizens: [{ id: 3, xDm: 500, zDm: 600, activity: 4 }],
            departed: [],
          },
        ],
      }),
    );
    expect([...core.citizensInCell(5)].map((c) => c.id)).toEqual([3]);
    expect(citizenEvents).toHaveLength(2);
    expect(citizenEvents[1].citizens.map((c) => c.id)).toEqual([3]);
  });

  it('delta upserts citizens and removes departed', () => {
    const { core } = makeCore();
    core.handleBinary(
      bin({
        cells: [
          {
            cell: 5,
            worldTick: 10n,
            keyframe: true,
            citizens: [
              { id: 1, xDm: 100, zDm: 200, activity: 0 },
              { id: 2, xDm: 300, zDm: 400, activity: 1 },
            ],
            departed: [],
          },
        ],
      }),
    );
    // Delta: citizen 1 moves, citizen 2 departs, citizen 4 arrives.
    core.handleBinary(
      bin({
        cells: [
          {
            cell: 5,
            worldTick: 11n,
            keyframe: false,
            citizens: [
              { id: 1, xDm: 110, zDm: 210, activity: 3 },
              { id: 4, xDm: 700, zDm: 800, activity: 3 },
            ],
            departed: [2],
          },
        ],
      }),
    );
    const byId = new Map([...core.citizensInCell(5)].map((c) => [c.id, c]));
    expect([...byId.keys()].sort()).toEqual([1, 4]);
    expect(byId.get(1)).toEqual({ id: 1, x: 11, z: 21, activity: 3 });
    expect(byId.get(2)).toBeUndefined();
  });

  it('tracks the newest world tick across frames', () => {
    const { core } = makeCore();
    core.handleBinary(
      bin({
        cells: [
          { cell: 1, worldTick: 50n, keyframe: true, citizens: [], departed: [] },
          { cell: 2, worldTick: 49n, keyframe: true, citizens: [], departed: [] },
        ],
      }),
    );
    expect(core.worldTick).toBe(50);
  });
});
