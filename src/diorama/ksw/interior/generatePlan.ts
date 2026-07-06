// Zone-ladder interior generator (T17, S3b-2). Takes the rectilinear zones of
// the real KSW footprint (interior/zones.ts) plus the baked main-entrance door
// and lays an authored department "ladder" into every zone: two east-west
// corridor spines with room rows tiled between/around them, furnished from the
// kswPlan catalog (floorPlan.ts). The result is a valid FloorPlan — same types
// — so the existing building.ts / nav.ts vocabulary consumes it unchanged.
//
// Fully deterministic: no RNG, no Date. The same zones + door always yield the
// same plan. Department assignment is a fixed priority list keyed off zone role
// (door zone, largest zone, then the rest by descending area).
//
// Ladder geometry per zone (local to the zone rect x/z/w/d):
//   • Two horizontal corridor bands at ~1/4 and ~3/4 of the depth, each CW
//     deep — these are the two spines the test asserts (>=2 corridors/zone).
//   • Three room rows: North (above the north spine), Middle (between the two
//     spines) and South (below the south spine). Each row is split into a few
//     equal-width room columns. Rooms whose row is too thin (<MIN_ROOM) are
//     dropped; every surviving room gets a door on the wall facing its nearest
//     spine, so nav can reach it.
//   • Props/people are laid procedurally on a deterministic grid inside each
//     room (with a wall margin), reusing the template department's prop KINDS
//     and person ROLES — never the template's absolute coordinates, so
//     containment holds for any room size.

import type {
  FloorPlan,
  PersonPlacement,
  PropPlacement,
  Room,
  WallSide,
  Walker,
} from '../floorPlan';
import { kswPlan, type PersonRole } from '../floorPlan';
import type { Zone } from './zones';
import { storeyLayout } from './cutaway';

export type MainDoor = { x: number; z: number; yaw: number };

// Corridor band depth and the minimum a room row may be before it's dropped.
const CW = 3;
const MIN_ROOM = 4; // a room row/column shorter than this is skipped
const WALL_MARGIN = 0.75; // props/people stay this far from a room wall

// Department template: a room-type family pulled from the kswPlan catalog. We
// keep only what's portable to a generated rect — label, accent, the prop
// KINDS and the person ROLES. Positions are regenerated per room.
type Dept = {
  key: string;
  label: string;
  accent: number;
  propKinds: string[];
  roles: PersonRole[];
};

function deptFrom(templateId: string, label?: string, key?: string): Dept {
  const t = kswPlan.rooms.find((r) => r.id === templateId);
  if (!t) throw new Error(`unknown template room ${templateId}`);
  return {
    key: key ?? templateId,
    label: label ?? t.label,
    accent: t.accent,
    propKinds: t.props.map((p) => p.kind),
    roles: t.people.map((p) => p.role),
  };
}

// Fixed priority list of departments a zone's rooms cycle through, drawn from
// the authored kswPlan room types. Index 0 families are the marquee wards; the
// tail repeats generic bed-station wards for the many small annex zones.
function priorityDepts(): Dept[] {
  return [
    deptFrom('op1', 'Zentral-OP'),
    deptFrom('ips', 'Intensivstation IPS'),
    deptFrom('xray', 'Radiologie'),
    deptFrom('ct', 'Computertomographie'),
    deptFrom('mri', 'MRI'),
    deptFrom('lab', 'Zentrallabor'),
    deptFrom('cardio', 'Kardiologie'),
    deptFrom('endo', 'Endoskopie'),
    deptFrom('wardChirurgie', 'Bettenstation Chirurgie'),
    deptFrom('wardMedizin', 'Bettenstation Medizin'),
    deptFrom('physio', 'Physiotherapie'),
    deptFrom('onko', 'Onkologie Tagesklinik'),
    deptFrom('dialyse', 'Dialyse'),
    deptFrom('kinder', 'Kinderklinik'),
    deptFrom('geburt', 'Gebärsaal'),
    deptFrom('neo', 'Neonatologie'),
    deptFrom('apotheke', 'Spitalapotheke'),
    deptFrom('admin', 'Verwaltung'),
    deptFrom('cafeteria', 'Cafeteria'),
  ];
}

// Door-zone departments: reception + emergency lead the ladder.
function doorDepts(): Dept[] {
  return [deptFrom('empfang', 'Eingangshalle Empfang'), deptFrom('notfall', 'Interdisziplinäres Notfallzentrum')];
}

// The ward department reused for zones past the priority list (Bettenstation
// Nord/Süd/…). Cycles the two authored ward templates.
const WARD_DIRS = ['Nord', 'Süd', 'Ost', 'West', 'Zentrum'];
function wardDept(templateId: 'wardChirurgie' | 'wardMedizin', dir: string): Dept {
  return deptFrom(templateId, `Bettenstation ${dir}`, `ward-${dir}`);
}

// Zone → the ordered department queue that fills its rooms. Deterministic:
//   • the door zone starts with Empfang+Notfall then priority families,
//   • the largest zone starts with OP+ICU (priority[0..1]),
//   • other zones consume the priority list by (already-sorted) area rank,
//   • zones whose rank runs past the priority list become wards.
function zoneDeptQueue(rank: number, isDoorZone: boolean, isLargest: boolean): Dept[] {
  const prio = priorityDepts();
  if (isDoorZone) return [...doorDepts(), ...prio];
  if (isLargest) return prio; // OP+ICU already lead the priority list
  // rank here is the index among the remaining zones (door + largest removed).
  // Offset into the priority list so each zone gets distinct marquee types
  // before falling back to wards.
  const offset = rank % prio.length;
  const rotated = [...prio.slice(offset), ...prio.slice(0, offset)];
  const dir = WARD_DIRS[rank % WARD_DIRS.length];
  const wardTemplate = rank % 2 === 0 ? 'wardChirurgie' : 'wardMedizin';
  return [...rotated, wardDept(wardTemplate, dir)];
}

// A door center offset (along the wall) clamped so the opening fits the wall
// with the 0.3 m end margin the floorPlan invariant checks.
function fitDoorCenter(center: number, wallLen: number, width: number): number {
  const max = wallLen / 2 - width / 2 - 0.35;
  if (max <= 0) return 0;
  return Math.max(-max, Math.min(max, center));
}

// Lay a department's props on a deterministic grid inside the room rect, with a
// wall margin. Reuses the template's prop KINDS in order, tiling row-major.
function layProps(rect: Room['rect'], kinds: string[]): PropPlacement[] {
  const innerW = rect.w - 2 * WALL_MARGIN;
  const innerD = rect.d - 2 * WALL_MARGIN;
  if (innerW <= 0 || innerD <= 0 || kinds.length === 0) return [];
  const cols = Math.max(1, Math.min(kinds.length, Math.round(Math.sqrt(kinds.length * (innerW / Math.max(innerD, 0.1))))));
  const rows = Math.max(1, Math.ceil(kinds.length / cols));
  const props: PropPlacement[] = [];
  for (let i = 0; i < kinds.length; i++) {
    const c = i % cols;
    const r = Math.floor(i / cols);
    // cell centers spread evenly across the inner rect
    const fx = cols === 1 ? 0.5 : c / (cols - 1);
    const fz = rows === 1 ? 0.5 : r / (rows - 1);
    const x = rect.x - innerW / 2 + fx * innerW;
    const z = rect.z - innerD / 2 + fz * innerD;
    props.push({ kind: kinds[i], x, z });
  }
  return props;
}

// Lay people on a deterministic inner grid (offset from the prop grid so they
// don't all stack on props), reusing the template roles.
function layPeople(rect: Room['rect'], roles: PersonRole[]): PersonPlacement[] {
  const innerW = rect.w - 2 * WALL_MARGIN;
  const innerD = rect.d - 2 * WALL_MARGIN;
  if (innerW <= 0 || innerD <= 0 || roles.length === 0) return [];
  const n = roles.length;
  const people: PersonPlacement[] = [];
  for (let i = 0; i < n; i++) {
    const fx = (i + 0.5) / n;
    const fz = 0.5 + 0.28 * Math.sin(i * 2.399);
    const x = rect.x - innerW / 2 + fx * innerW;
    const z = rect.z - innerD / 2 + Math.min(Math.max(fz, 0), 1) * innerD;
    people.push({ role: roles[i], x, z, yaw: (i * 1.7) % (Math.PI * 2) });
  }
  return people;
}

type RowSpec = { z: number; d: number; spine: 's' | 'n' }; // door faces this spine

// Build the rooms of one zone ladder. Returns rooms + the zone's 2 corridors.
function buildZoneLadder(
  zone: Zone,
  depts: Dept[],
  deptCursor: { i: number },
  withPeople: boolean,
): { rooms: Room[]; corridors: Array<{ x: number; z: number; w: number; d: number }> } {
  const { x: zx, z: zz, w: zw, d: zd } = zone;
  const zNorth = zz - zd / 2;
  const zSouth = zz + zd / 2;
  const zWest = zx - zw / 2;
  const zEast = zx + zw / 2;

  // Two horizontal corridor spines. Place them so three room rows fit; if the
  // zone is too shallow for three rows, the rows simply collapse (dropped
  // below MIN_ROOM) but the two corridors are always emitted.
  const spineDepth = Math.min(CW, Math.max(1.5, zd / 6));
  const laneA = zNorth + zd * 0.28; // north spine centerline
  const laneB = zNorth + zd * 0.72; // south spine centerline
  // Two horizontal spines (the ladder rails the test counts) + two short
  // vertical end-connectors joining them, so the whole ladder is one connected
  // corridor network (mirrors kswPlan's west/east connectors).
  const connDepth = laneB - laneA + spineDepth;
  const endInset = Math.min(spineDepth, zw / 6);
  const corridors = [
    { x: zx, z: laneA, w: zw, d: spineDepth },
    { x: zx, z: laneB, w: zw, d: spineDepth },
    { x: zWest + endInset / 2, z: (laneA + laneB) / 2, w: endInset, d: connDepth },
    { x: zEast - endInset / 2, z: (laneA + laneB) / 2, w: endInset, d: connDepth },
  ];

  const rows: RowSpec[] = [
    { z: (zNorth + (laneA - spineDepth / 2)) / 2, d: laneA - spineDepth / 2 - zNorth, spine: 's' }, // N row, door on south wall -> lane A
    { z: laneA + spineDepth / 2 + (laneB - spineDepth / 2 - (laneA + spineDepth / 2)) / 2, d: laneB - spineDepth / 2 - (laneA + spineDepth / 2), spine: 'n' }, // M row
    { z: (laneB + spineDepth / 2 + zSouth) / 2, d: zSouth - (laneB + spineDepth / 2), spine: 'n' }, // S row, door on north wall -> lane B
  ];

  const rooms: Room[] = [];
  for (const row of rows) {
    if (row.d < MIN_ROOM) continue;
    // split the row width into as many rooms as fit at a comfortable ~10 m,
    // at least one, capped so tiny zones don't shatter into slivers.
    const cols = Math.max(1, Math.min(6, Math.floor(zw / 10)));
    const colW = zw / cols;
    if (colW < MIN_ROOM) continue;
    for (let c = 0; c < cols; c++) {
      const dept = depts[deptCursor.i % depts.length];
      deptCursor.i += 1;
      const rx = zWest + (c + 0.5) * colW;
      const rect = { x: rx, z: row.z, w: colW - 0.2, d: row.d - 0.2 };
      const doorWall: WallSide = row.spine;
      const doorWidth = Math.min(1.8, rect.w - 1.2 > 0 ? rect.w - 1.2 : 1.0);
      const room: Room = {
        id: `${zone.id}-r${rooms.length}`,
        label: dept.label,
        accent: dept.accent,
        rect,
        doors: [{ wall: doorWall, center: fitDoorCenter(0, doorWall === 'n' || doorWall === 's' ? rect.w : rect.d, doorWidth), width: doorWidth }],
        windows: [],
        props: layProps(rect, dept.propKinds),
        people: withPeople ? layPeople(rect, dept.roles) : [],
      };
      rooms.push(room);
    }
  }

  return { rooms, corridors };
}

const CONNECTOR_W = 3;

// A cross-zone connector is an L-shaped pair of axis-aligned corridor rects
// running from the center of zone a to the center of zone b. Because each
// zone's ladder has vertical end-connectors + horizontal spines passing through
// its central band, a corridor that reaches a zone's center overlaps that
// zone's own corridors — so the two ladders become one connected network.
// (An L instead of a diagonal keeps every corridor axis-aligned, matching the
// building.ts / floorPlan rect vocabulary.)
function connectorRects(a: Zone, b: Zone): FloorPlan['corridors'] {
  const dx = b.x - a.x;
  const dz = b.z - a.z;
  // horizontal leg at a.z from a.x to b.x, then vertical leg at b.x from a.z to b.z
  const rects: FloorPlan['corridors'] = [];
  if (Math.abs(dx) > 0.1) {
    rects.push({ x: (a.x + b.x) / 2, z: a.z, w: Math.abs(dx) + CONNECTOR_W, d: CONNECTOR_W });
  }
  if (Math.abs(dz) > 0.1) {
    rects.push({ x: b.x, z: (a.z + b.z) / 2, w: CONNECTOR_W, d: Math.abs(dz) + CONNECTOR_W });
  }
  if (rects.length === 0) rects.push({ x: a.x, z: a.z, w: CONNECTOR_W, d: CONNECTOR_W });
  return rects;
}

// Minimum spanning tree over the zones (by center-to-center distance) — the
// deterministic set of connector edges that links EVERY zone into one network
// (Prim's algorithm; ties broken by zone index, so the result is stable).
function zoneMstEdges(zones: Zone[]): Array<[number, number]> {
  const n = zones.length;
  if (n <= 1) return [];
  const inTree = new Array<boolean>(n).fill(false);
  const edges: Array<[number, number]> = [];
  inTree[0] = true;
  for (let added = 1; added < n; added++) {
    let bestFrom = -1;
    let bestTo = -1;
    let bestD = Infinity;
    for (let i = 0; i < n; i++) {
      if (!inTree[i]) continue;
      for (let j = 0; j < n; j++) {
        if (inTree[j]) continue;
        const d = Math.hypot(zones[i].x - zones[j].x, zones[i].z - zones[j].z);
        if (d < bestD - 1e-9) {
          bestD = d;
          bestFrom = i;
          bestTo = j;
        }
      }
    }
    if (bestTo === -1) break;
    inTree[bestTo] = true;
    edges.push([bestFrom, bestTo]);
  }
  return edges;
}

function generatePlanWithQueues(
  zones: Zone[],
  mainDoor: MainDoor,
  queueFor: (rank: number, isDoorZone: boolean, isLargest: boolean) => Dept[],
  withPeople: boolean,
): FloorPlan {
  if (zones.length === 0) {
    return {
      plate: { w: 1, d: 1 },
      building: { x: 0, z: 0, w: 1, d: 1 },
      corridors: [],
      rooms: [],
      corridorProps: [],
      outdoorSlabs: [],
      outdoorProps: [],
      outdoorPeople: [],
      walkers: [],
    };
  }

  // Rank zones by area (desc) so department assignment is deterministic and the
  // "largest zone" is unambiguous. zones.ts already returns them largest-first,
  // but re-sort defensively (stable by id on ties).
  const ranked = [...zones].sort((p, q) => q.w * q.d - p.w * p.d || (p.id < q.id ? -1 : 1));

  // Door zone = the zone whose rect contains (or is nearest to) the main door.
  const doorZone = nearestZone(ranked, mainDoor);
  const largestZone = ranked[0];

  const allRooms: Room[] = [];
  const allCorridors: FloorPlan['corridors'] = [];
  const cursor = { i: 0 };
  let otherRank = 0;
  for (const zone of ranked) {
    const isDoorZone = zone.id === doorZone.id;
    const isLargest = zone.id === largestZone.id && !isDoorZone;
    const rank = isDoorZone || isLargest ? 0 : otherRank++;
    const depts = queueFor(rank, isDoorZone, isLargest);
    const local = { i: 0 };
    const { rooms, corridors } = buildZoneLadder(zone, depts, local, withPeople);
    allRooms.push(...rooms);
    allCorridors.push(...corridors);
  }

  // Cross-corridors: a spanning tree over the zones guarantees every zone's
  // ladder is reachable from every other (and thus from the main door). Each
  // MST edge is an L of axis-aligned corridor rects between the zone centers.
  for (const [i, j] of zoneMstEdges(ranked)) {
    allCorridors.push(...connectorRects(ranked[i], ranked[j]));
  }

  // Bounding box of all zones = plate + building extents.
  let minX = Infinity;
  let maxX = -Infinity;
  let minZ = Infinity;
  let maxZ = -Infinity;
  for (const z of zones) {
    minX = Math.min(minX, z.x - z.w / 2);
    maxX = Math.max(maxX, z.x + z.w / 2);
    minZ = Math.min(minZ, z.z - z.d / 2);
    maxZ = Math.max(maxZ, z.z + z.d / 2);
  }
  const bx = (minX + maxX) / 2;
  const bz = (minZ + maxZ) / 2;
  const bw = maxX - minX;
  const bd = maxZ - minZ;

  const walkers: Walker[] = [];

  return {
    plate: { w: bw, d: bd },
    building: { x: bx, z: bz, w: bw, d: bd },
    corridors: allCorridors,
    rooms: allRooms,
    corridorProps: [],
    outdoorSlabs: [],
    outdoorProps: [],
    outdoorPeople: [],
    walkers,
  };
}

export function generateInteriorPlan(zones: Zone[], mainDoor: MainDoor): FloorPlan {
  return generatePlanWithQueues(zones, mainDoor, zoneDeptQueue, true);
}

// ── Vertical hospital zoning (Phase A, spec §5 'clinic') ────────────────────
// Which department families fill which storey. Level 0 keeps the door-zone
// logic (Empfang + Notfall lead); imaging is heavy machinery → ground floor.
type LevelBand = 'ground' | 'treatment' | 'ward' | 'technik';

export function levelBand(level: number, storeyCount: number): LevelBand {
  if (level === 0) return 'ground';
  if (storeyCount >= 4 && level === storeyCount - 1) return 'technik';
  const lastTreatment = Math.max(1, Math.floor((storeyCount - 1) / 2));
  return level <= lastTreatment ? 'treatment' : 'ward';
}

function bandDepts(band: LevelBand): Dept[] {
  switch (band) {
    case 'ground':
      return [
        deptFrom('xray', 'Radiologie'),
        deptFrom('ct', 'Computertomographie'),
        deptFrom('mri', 'MRI'),
        deptFrom('apotheke', 'Spitalapotheke'),
        deptFrom('cafeteria', 'Cafeteria'),
        deptFrom('admin', 'Verwaltung'),
      ];
    case 'treatment':
      return [
        deptFrom('op1', 'Zentral-OP'),
        deptFrom('ips', 'Intensivstation IPS'),
        deptFrom('lab', 'Zentrallabor'),
        deptFrom('endo', 'Endoskopie'),
        deptFrom('cardio', 'Kardiologie'),
        deptFrom('geburt', 'Gebärsaal'),
        deptFrom('neo', 'Neonatologie'),
      ];
    case 'ward':
      return [
        deptFrom('wardChirurgie', 'Bettenstation Chirurgie'),
        deptFrom('wardMedizin', 'Bettenstation Medizin'),
        deptFrom('physio', 'Physiotherapie'),
        deptFrom('onko', 'Onkologie Tagesklinik'),
        deptFrom('dialyse', 'Dialyse'),
        deptFrom('kinder', 'Kinderklinik'),
      ];
    case 'technik':
      return [deptFrom('admin', 'Technikgeschoss'), deptFrom('lab', 'Gebäudetechnik')];
  }
}

export type BuildingPlan = { storeyCount: number; storeyH: number; storeys: FloorPlan[] };

export function generateBuildingPlan(zones: Zone[], mainDoor: MainDoor, eaveH: number): BuildingPlan {
  const { storeyCount, storeyH } = storeyLayout(eaveH);
  const storeys: FloorPlan[] = [];
  for (let level = 0; level < storeyCount; level++) {
    const band = levelBand(level, storeyCount);
    if (band === 'ground') {
      // level 0 keeps the authored door behavior: Empfang+Notfall lead the
      // door zone, then the ground-floor families (imaging, Apotheke, …).
      const groundQueue = (rank: number, isDoorZone: boolean, isLargest: boolean): Dept[] => {
        const base = bandDepts('ground');
        if (isDoorZone) return [...doorDepts(), ...base];
        const offset = (isLargest ? 0 : rank + 1) % base.length;
        return [...base.slice(offset), ...base.slice(0, offset)];
      };
      storeys.push(generatePlanWithQueues(zones, mainDoor, groundQueue, true));
    } else {
      const base = bandDepts(band);
      const levelQueue = (rank: number, _isDoorZone: boolean, isLargest: boolean): Dept[] => {
        const offset = ((isLargest ? 0 : rank + 1) + level) % base.length;
        return [...base.slice(offset), ...base.slice(0, offset)];
      };
      storeys.push(generatePlanWithQueues(zones, mainDoor, levelQueue, false));
    }
  }
  return { storeyCount, storeyH, storeys };
}

// The zone containing the door, or the nearest by center distance if the door
// (baked on the outer shell) sits just outside every zone rect.
function nearestZone(zones: Zone[], door: MainDoor): Zone {
  let best = zones[0];
  let bestScore = Infinity;
  for (const z of zones) {
    const inside =
      door.x >= z.x - z.w / 2 && door.x <= z.x + z.w / 2 && door.z >= z.z - z.d / 2 && door.z <= z.z + z.d / 2;
    const d = Math.hypot(door.x - z.x, door.z - z.z);
    const score = inside ? d - 1e6 : d; // strongly prefer a containing zone
    if (score < bestScore) {
      bestScore = score;
      best = z;
    }
  }
  return best;
}

// Exported for T18/T19: the center of the emergency (Notfall) and OP zones, so
// the cutaway presets can aim at them. Falls back to the plan's building center.
export function departmentCenter(plan: FloorPlan, labelFragment: string): [number, number] {
  const room = plan.rooms.find((r) => r.label.includes(labelFragment));
  if (room) return [room.rect.x, room.rect.z];
  return [plan.building.x, plan.building.z];
}
