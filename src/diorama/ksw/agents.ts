// Wandering agents: every bean person is a little state machine that dwells,
// picks a destination, walks the nav graph there, and dwells again.
// Deterministic (seeded mulberry32, no Math.random) and three.js-free, so the
// whole behavior is unit-testable.

import type { PersonRole } from './floorPlan';
import { pathLength, routePath, type NavGraph, type Pt } from './nav';

export type AgentKind = 'resident' | 'rounds' | 'outdoor';

export type AgentSpec = {
  role: PersonRole;
  home: Pt;
  homeRoomId: string | null;
  kind: AgentKind;
  seed: number;
  stationary?: boolean;
};

export type Agent = {
  spec: AgentSpec;
  pos: Pt;
  yaw: number;
  heading: number | null; // yaw while walking, null when idle
  phase: 'dwell' | 'walk';
  path: Pt[];
  seg: number;
  segDist: number;
  dwellLeft: number;
  speed: number;
  roomId: string | null; // where the agent currently is (for routing)
  awayFromHome: boolean;
  rngState: number;
};

function mulberry32(state: number): { next: () => number; state: () => number } {
  let s = state >>> 0;
  const next = (): number => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  return { next, state: () => s };
}

export function createAgent(spec: AgentSpec): Agent {
  return {
    spec,
    pos: [spec.home[0], spec.home[1]],
    yaw: 0,
    heading: null,
    phase: 'dwell',
    path: [],
    seg: 0,
    segDist: 0,
    dwellLeft: 1 + (spec.seed % 7),
    speed: 1.2,
    roomId: spec.homeRoomId,
    awayFromHome: false,
    rngState: (spec.seed * 2654435761) >>> 0,
  };
}

const dist = (p: Pt, q: Pt): number => Math.hypot(p[0] - q[0], p[1] - q[1]);

function pickTarget(agent: Agent, rnd: () => number, nav: NavGraph): { point: Pt; roomId: string | null } {
  const spec = agent.spec;
  const rooms = nav.plan.rooms;
  // away agents go home first — keeps residents anchored to their ward
  if (spec.kind === 'resident' && agent.awayFromHome) {
    return { point: spec.home, roomId: spec.homeRoomId };
  }
  if (spec.kind === 'outdoor') {
    // stroll the southern forecourt: plaza, apron, parking
    const slabs = nav.plan.outdoorSlabs;
    const s = slabs[Math.floor(rnd() * slabs.length) % slabs.length];
    return {
      point: [s.x + (rnd() - 0.5) * (s.w - 3), s.z + (rnd() - 0.5) * (s.d - 3)],
      roomId: null,
    };
  }
  if (spec.kind === 'rounds') {
    // staff on rounds: visit any room's doorway, sometimes pause in a corridor
    if (rnd() < 0.3) {
      const { a, b, xMin, xMax } = nav.lanes;
      return { point: [xMin + rnd() * (xMax - xMin), rnd() < 0.5 ? a : b], roomId: null };
    }
    const room = rooms[Math.floor(rnd() * rooms.length) % rooms.length];
    const door = nav.roomDoors[room.id]?.[0];
    return door ? { point: door.inside, roomId: room.id } : { point: [room.rect.x, room.rect.z], roomId: room.id };
  }
  // resident: mostly small moves inside the own room, sometimes a corridor
  // errand or a visit next door (then straight home again)
  const r = rnd();
  const homeRoom = rooms.find((rm) => rm.id === spec.homeRoomId);
  if (r < 0.55 && homeRoom) {
    const jx = homeRoom.rect.x + (rnd() - 0.5) * (homeRoom.rect.w - 2.4);
    const jz = homeRoom.rect.z + (rnd() - 0.5) * (homeRoom.rect.d - 2.4);
    return { point: [jx, jz], roomId: homeRoom.id };
  }
  if (r < 0.8) {
    const door = spec.homeRoomId ? nav.roomDoors[spec.homeRoomId]?.[0] : undefined;
    if (door) {
      const along = (rnd() - 0.5) * 12;
      const anchor = door.anchor;
      const horizontal = Math.abs(anchor[1] - nav.lanes.a) < 0.01 || Math.abs(anchor[1] - nav.lanes.b) < 0.01;
      const point: Pt = horizontal
        ? [Math.min(Math.max(anchor[0] + along, nav.lanes.xMin), nav.lanes.xMax), anchor[1]]
        : [anchor[0], Math.min(Math.max(anchor[1] + along, nav.lanes.zMin), nav.lanes.zMax)];
      return { point, roomId: null };
    }
  }
  const other = rooms[Math.floor(rnd() * rooms.length) % rooms.length];
  const door = nav.roomDoors[other.id]?.[0];
  return door ? { point: door.inside, roomId: other.id } : { point: spec.home, roomId: spec.homeRoomId };
}

export function updateAgent(agent: Agent, dt: number, nav: NavGraph): void {
  if (agent.spec.stationary) return;

  if (agent.phase === 'dwell') {
    agent.dwellLeft -= dt;
    agent.heading = null;
    if (agent.dwellLeft > 0) return;
    const rng = mulberry32(agent.rngState);
    const target = pickTarget(agent, rng.next, nav);
    agent.speed = 1.0 + rng.next() * 0.5;
    // staff on rounds keeps moving; residents and strollers linger
    const dwellBase = agent.spec.kind === 'rounds' ? 2 : agent.spec.kind === 'outdoor' ? 6 : 8;
    const nextDwell = dwellBase + rng.next() * dwellBase * 2.5;
    agent.rngState = rng.state();
    const path = routePath(nav, { point: agent.pos, roomId: agent.roomId }, target);
    if (pathLength(path) < 0.4) {
      agent.dwellLeft = nextDwell;
      return;
    }
    agent.path = path;
    agent.seg = 0;
    agent.segDist = 0;
    agent.phase = 'walk';
    agent.dwellLeft = nextDwell;
    // remember where this trip ends
    agent.roomId = target.roomId;
    agent.awayFromHome =
      agent.spec.kind === 'resident' &&
      !(target.roomId === agent.spec.homeRoomId && target.roomId !== null);
    return;
  }

  // walk phase: advance along the polyline
  let travel = agent.speed * dt;
  while (travel > 0 && agent.seg < agent.path.length - 1) {
    const p = agent.path[agent.seg];
    const q = agent.path[agent.seg + 1];
    const segLen = dist(p, q);
    const remaining = segLen - agent.segDist;
    if (segLen > 1e-6) {
      agent.heading = Math.atan2(q[0] - p[0], q[1] - p[1]);
      agent.yaw = agent.heading;
    }
    if (travel < remaining) {
      agent.segDist += travel;
      const t = agent.segDist / segLen;
      agent.pos = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
      return;
    }
    travel -= remaining;
    agent.seg += 1;
    agent.segDist = 0;
    agent.pos = [q[0], q[1]];
  }
  // arrived
  agent.phase = 'dwell';
  agent.heading = null;
}
