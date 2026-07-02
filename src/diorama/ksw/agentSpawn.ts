// Deterministic crowd spawning (Slice D of the 10k-perf design).
// The authored floor-plan people are always the first specs, in the exact
// order main.ts used to build them — so the default crowd (no ?agents param)
// is byte-identical to before. Extra agents beyond the plan are drawn from a
// seeded mulberry32 stream (no Math.random): role mix sampled from the plan's
// own distribution, kind mix ~40% rounds / 30% outdoor / 30% resident.
// Pure data + math — three.js-free and unit-testable.

import type { FloorPlan, PersonRole } from './floorPlan';
import type { NavGraph, Pt } from './nav';
import { mulberry32, type AgentSpec } from './agents';

export type SpawnSpec = { spec: AgentSpec; yaw: number };

// Fixed stream seed: the same requested count always yields the same crowd.
const SPAWN_SEED = 0x1d10ca7;

// The authored people: room residents, plaza people, corridor walkers —
// order and seeds identical to the pre-Slice-D main.ts loop.
export function planSpawnSpecs(plan: FloorPlan): SpawnSpec[] {
  const inBuilding = (x: number, z: number): boolean =>
    Math.abs(x - plan.building.x) < plan.building.w / 2 && Math.abs(z - plan.building.z) < plan.building.d / 2;
  const specs: SpawnSpec[] = [];
  const push = (spec: Omit<AgentSpec, 'seed'>, yaw: number): void => {
    specs.push({ spec: { ...spec, seed: specs.length + 1 }, yaw });
  };
  for (const room of plan.rooms) {
    for (const p of room.people) {
      push({ role: p.role, home: [p.x, p.z], homeRoomId: room.id, kind: 'resident', stationary: p.stationary }, p.yaw);
    }
  }
  for (const p of plan.outdoorPeople) {
    push({ role: p.role, home: [p.x, p.z], homeRoomId: null, kind: 'outdoor' }, p.yaw);
  }
  for (const w of plan.walkers) {
    const home: Pt = w.axis === 'x' ? [w.from, w.fixed] : [w.fixed, w.from];
    push({ role: w.role, home, homeRoomId: null, kind: inBuilding(home[0], home[1]) ? 'rounds' : 'outdoor' }, 0);
  }
  return specs;
}

export function buildSpawnSpecs(plan: FloorPlan, nav: NavGraph, count?: number): SpawnSpec[] {
  const base = planSpawnSpecs(plan);
  const n = count ?? base.length;
  if (n <= base.length) return base.slice(0, n);

  const specs = base.slice();
  const rnd = mulberry32(SPAWN_SEED).next;
  const rooms = plan.rooms;
  const slabs = plan.outdoorSlabs;
  const { a: laneA, b: laneB, xMin, xMax } = nav.lanes;

  while (specs.length < n) {
    const seed = specs.length + 1;
    // role mix: sampling a random authored spec's role IS the plan distribution
    const role: PersonRole = base[Math.floor(rnd() * base.length) % base.length].spec.role;
    const kindRoll = rnd();
    const yaw = (rnd() - 0.5) * Math.PI * 2;
    if (kindRoll < 0.4) {
      // staff on rounds: start on a corridor lane
      const lane = rnd() < 0.5 ? laneA : laneB;
      const home: Pt = [xMin + rnd() * (xMax - xMin), lane];
      specs.push({ spec: { role, home, homeRoomId: null, kind: 'rounds', seed }, yaw });
    } else if (kindRoll < 0.7) {
      // outdoor stroller: somewhere on the forecourt slabs
      const s = slabs[Math.floor(rnd() * slabs.length) % slabs.length];
      const home: Pt = [s.x + (rnd() - 0.5) * (s.w - 3), s.z + (rnd() - 0.5) * (s.d - 3)];
      specs.push({ spec: { role, home, homeRoomId: null, kind: 'outdoor', seed }, yaw });
    } else {
      // resident: a random room is home, jittered inside its rect (same
      // 2.4 m wall margin as agents.pickTarget)
      const room = rooms[Math.floor(rnd() * rooms.length) % rooms.length];
      const home: Pt = [
        room.rect.x + (rnd() - 0.5) * Math.max(room.rect.w - 2.4, 0),
        room.rect.z + (rnd() - 0.5) * Math.max(room.rect.d - 2.4, 0),
      ];
      specs.push({ spec: { role, home, homeRoomId: room.id, kind: 'resident', seed }, yaw });
    }
  }
  return specs;
}
