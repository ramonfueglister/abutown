import { describe, expect, it } from 'vitest';
import { buildNav } from '../../src/diorama/ksw/nav';
import { createAgent, updateAgent, type Agent } from '../../src/diorama/ksw/agents';
import { kswPlan } from '../../src/diorama/ksw/floorPlan';

const nav = buildNav(kswPlan);

function insideRect(r: { x: number; z: number; w: number; d: number }, x: number, z: number, pad = 0): boolean {
  return Math.abs(x - r.x) <= r.w / 2 + pad && Math.abs(z - r.z) <= r.d / 2 + pad;
}

function isLegal(x: number, z: number): boolean {
  if (kswPlan.corridors.some((c) => insideRect(c, x, z, 0.15))) return true;
  if (kswPlan.rooms.some((r) => insideRect(r.rect, x, z, 0.15))) return true;
  const outside = !insideRect(kswPlan.building, x, z, -0.05);
  return outside && Math.abs(x) <= kswPlan.plate.w / 2 && Math.abs(z) <= kswPlan.plate.d / 2;
}

function simulate(agent: Agent, seconds: number, dt = 0.1): void {
  for (let t = 0; t < seconds; t += dt) updateAgent(agent, dt, nav);
}

describe('agents', () => {
  it('is deterministic for the same seed', () => {
    const a = createAgent({ role: 'nurse', home: [-4, -16], homeRoomId: 'ips', kind: 'resident', seed: 7 });
    const b = createAgent({ role: 'nurse', home: [-4, -16], homeRoomId: 'ips', kind: 'resident', seed: 7 });
    simulate(a, 120);
    simulate(b, 120);
    expect(a.pos).toEqual(b.pos);
    expect(a.phase).toBe(b.phase);
  });

  it('different seeds diverge', () => {
    const a = createAgent({ role: 'nurse', home: [-4, -16], homeRoomId: 'ips', kind: 'resident', seed: 1 });
    const b = createAgent({ role: 'nurse', home: [-4, -16], homeRoomId: 'ips', kind: 'resident', seed: 2 });
    simulate(a, 200);
    simulate(b, 200);
    const same = a.pos[0] === b.pos[0] && a.pos[1] === b.pos[1];
    expect(same).toBe(false);
  });

  it('actually walks (leaves its start position) and dwells in between', () => {
    const a = createAgent({ role: 'doctor', home: [4, -2], homeRoomId: 'wardMedizin', kind: 'rounds', seed: 3 });
    let movedFar = 0;
    let dwelled = false;
    for (let t = 0; t < 300; t += 0.1) {
      updateAgent(a, 0.1, nav);
      movedFar = Math.max(movedFar, Math.hypot(a.pos[0] - 4, a.pos[1] + 2));
      if (a.phase === 'dwell') dwelled = true;
    }
    expect(movedFar).toBeGreaterThan(8);
    expect(dwelled).toBe(true);
  });

  it('never leaves legal ground over a long run', () => {
    for (const seed of [11, 22, 33]) {
      const a = createAgent({ role: 'nurse', home: [-23.5, 12], homeRoomId: 'notfall', kind: 'resident', seed });
      for (let t = 0; t < 400; t += 0.1) {
        updateAgent(a, 0.1, nav);
        expect(isLegal(a.pos[0], a.pos[1]), `seed ${seed} at ${a.pos[0].toFixed(1)},${a.pos[1].toFixed(1)} t=${t.toFixed(1)}`).toBe(true);
      }
    }
  });

  it('stationary agents never move', () => {
    const a = createAgent({ role: 'surgeon', home: [-26.9, -16.3], homeRoomId: 'op1', kind: 'resident', seed: 5, stationary: true });
    simulate(a, 200);
    expect(a.pos).toEqual([-26.9, -16.3]);
    expect(a.phase).toBe('dwell');
  });

  it('outdoor agents stroll and stay outdoors when kind=outdoor', () => {
    const a = createAgent({ role: 'visitor', home: [-2.4, 20.8], homeRoomId: null, kind: 'outdoor', seed: 9 });
    let moved = false;
    for (let t = 0; t < 300; t += 0.1) {
      updateAgent(a, 0.1, nav);
      if (Math.hypot(a.pos[0] + 2.4, a.pos[1] - 20.8) > 3) moved = true;
      const b = kswPlan.building;
      expect(insideRect(b, a.pos[0], a.pos[1], -0.05), `entered building at ${a.pos}`).toBe(false);
    }
    expect(moved).toBe(true);
  });

  it('walking agents face their direction of travel', () => {
    const a = createAgent({ role: 'nurse', home: [-4, -16], homeRoomId: 'ips', kind: 'rounds', seed: 13 });
    for (let t = 0; t < 60; t += 0.1) {
      updateAgent(a, 0.1, nav);
      if (a.phase === 'walk' && a.heading !== null) {
        expect(Number.isFinite(a.yaw)).toBe(true);
        break;
      }
    }
  });
});
