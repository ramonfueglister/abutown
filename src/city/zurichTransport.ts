import { inside, key, parseKey, type Coord, type ZurichRailTile, type ZurichRoadKind, type ZurichRoadTile, type ZurichWorld } from './worldTypes';
import { removeAdjacentParallelRoadRuns } from './roadParallelCleanup';

const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;

export type ZurichTransport = {
  roads: Map<string, ZurichRoadTile>;
  rails: Map<string, ZurichRailTile>;
  bridges: Set<string>;
  railCrossings: Set<string>;
  arterialPaths: Coord[][];
  railPaths: Coord[][];
};

export function buildZurichTransport(world: ZurichWorld): ZurichTransport {
  const railPaths = buildRailPaths(world);
  const railPoints = new Set(railPaths.flatMap((path) => path.map(key)));
  const railCrossings = new Set(['118:154']);
  const roadKinds = new Map<string, ZurichRoadKind>();
  const bridgeKeys = new Set<string>();
  const arterialPaths = buildArterialPaths(world);

  const addRoad = (coord: Coord, allowBridge = false) => {
    if (!inside(coord, world.width, world.height)) return;
    const tileKey = key(coord);
    if (railPoints.has(tileKey) && !railCrossings.has(tileKey)) return;
    const terrain = world.terrain.get(tileKey)?.kind;
    const isBridge = allowBridge && (terrain === 'water' || terrain === 'riverbank');
    if (terrain === 'water' && !isBridge) return;
    if (terrain === 'riverbank' && !isBridge) return;
    const kind: ZurichRoadKind = isBridge ? 'bridge' : 'street';
    if (roadKinds.get(tileKey) === 'bridge' && kind === 'street') return;
    roadKinds.set(tileKey, kind);
    if (kind === 'bridge') bridgeKeys.add(tileKey);
  };

  for (const path of arterialPaths) for (const coord of path) addRoad(coord, true);
  for (const zone of world.zones) {
    if (zone.kind === 'forest' || zone.kind === 'river') continue;
    addDistrictStreetPattern(world, addRoad, zone.center, zone.radius, zone.density);
  }

  removeAdjacentParallelRoadRuns(roadKinds, new Set([
    ...arterialPaths.flat().map(key),
    ...bridgeKeys,
    ...railCrossings,
  ]));

  const roads = new Map<string, ZurichRoadTile>();
  for (const [tileKey, kind] of roadKinds) {
    const coord = parseKey(tileKey);
    roads.set(tileKey, { coord, kind, mask: maskFor(roadKinds, coord) });
  }

  const rails = new Map<string, ZurichRailTile>();
  for (const tileKey of railPoints) {
    const coord = parseKey(tileKey);
    rails.set(tileKey, { coord, mask: maskForRail(railPoints, coord) });
  }

  const roadBackedArterialPaths = splitRoadBackedPaths(arterialPaths, roadKinds);

  return { roads, rails, bridges: bridgeKeys, railCrossings, arterialPaths: roadBackedArterialPaths, railPaths };
}

function splitRoadBackedPaths(paths: Coord[][], roadKinds: ReadonlyMap<string, ZurichRoadKind>): Coord[][] {
  const roadBackedPaths: Coord[][] = [];
  for (const path of paths) {
    let currentPath: Coord[] = [];
    for (const coord of path) {
      if (!roadKinds.has(key(coord))) {
        if (currentPath.length > 0) roadBackedPaths.push(currentPath);
        currentPath = [];
        continue;
      }

      const previous = currentPath[currentPath.length - 1];
      if (previous && Math.abs(coord.x - previous.x) + Math.abs(coord.y - previous.y) !== 1) {
        roadBackedPaths.push(currentPath);
        currentPath = [];
      }
      currentPath.push(coord);
    }
    if (currentPath.length > 0) roadBackedPaths.push(currentPath);
  }
  return roadBackedPaths;
}

function buildArterialPaths(world: ZurichWorld): Coord[][] {
  return [
    route([{ x: 0, y: 128 }, { x: 73, y: 124 }, { x: 112, y: 112 }, { x: 139, y: 112 }, { x: 206, y: 116 }, { x: world.width - 1, y: 121 }]),
    route([{ x: 104, y: 0 }, { x: 111, y: 78 }, { x: 118, y: 145 }, { x: 101, y: 196 }, { x: 88, y: world.height - 1 }]),
    route([{ x: 43, y: 184 }, { x: 100, y: 196 }, { x: 151, y: 180 }, { x: 175, y: 184 }, { x: world.width - 1, y: 198 }]),
    route([{ x: 20, y: 74 }, { x: 74, y: 125 }, { x: 118, y: 145 }, { x: 151, y: 143 }, { x: 220, y: 160 }]),
    route([{ x: 58, y: 92 }, { x: 105, y: 92 }, { x: 152, y: 92 }, { x: 201, y: 98 }]),
  ];
}

function buildRailPaths(world: ZurichWorld): Coord[][] {
  return [
    route([{ x: 0, y: 154 }, { x: 118, y: 154 }, { x: 175, y: 184 }, { x: world.width - 1, y: 191 }]),
    route([{ x: 118, y: 154 }, { x: 126, y: world.height - 1 }]),
    ...parallelStationTracks(86, 154, 66, [-8, -5, -2, 2, 5, 8]),
    ...parallelStationTracks(136, 178, 58, [-6, -3, 0, 3, 6]),
  ];
}

function parallelStationTracks(startX: number, y: number, length: number, offsets: number[]): Coord[][] {
  return offsets.map((offset) => route([{ x: startX, y: y + offset }, { x: startX + length, y: y + offset }]));
}

function addDistrictStreetPattern(world: ZurichWorld, addRoad: (coord: Coord) => void, center: Coord, radius: number, density: number): void {
  const arm = Math.max(8, Math.round(radius * (density > 0.8 ? 0.95 : 0.65)));
  const curve = Math.max(2, Math.round(radius * 0.12));
  const eastWest = route([
    { x: center.x - arm, y: center.y },
    { x: center.x - Math.round(arm * 0.35), y: center.y + curve },
    { x: center.x + Math.round(arm * 0.35), y: center.y - curve },
    { x: center.x + arm, y: center.y },
  ]);
  const northSouth = route([
    { x: center.x, y: center.y - Math.round(arm * 0.55) },
    { x: center.x + curve, y: center.y - Math.round(arm * 0.18) },
    { x: center.x - curve, y: center.y + Math.round(arm * 0.18) },
    { x: center.x, y: center.y + Math.round(arm * 0.55) },
  ]);
  for (const coord of eastWest) addRoad(coord);
  for (const coord of northSouth) addRoad(coord);

  if (density > 0.72) {
    for (const offset of [-9, 9]) {
      for (const coord of route([
        { x: center.x - arm + 4, y: center.y + offset },
        { x: center.x, y: center.y + offset + Math.sign(offset) * 2 },
        { x: center.x + arm - 4, y: center.y + offset },
      ])) addRoad(coord);
      for (const coord of route([
        { x: center.x + offset, y: center.y - arm + 6 },
        { x: center.x + offset - Math.sign(offset) * 2, y: center.y },
        { x: center.x + offset, y: center.y + arm - 6 },
      ])) addRoad(coord);
    }
  }
}

function route(points: Coord[]): Coord[] {
  const result: Coord[] = [];
  for (let index = 1; index < points.length; index += 1) {
    const segment = cardinalLinePath(points[index - 1], points[index]);
    result.push(...(index === 1 ? segment : segment.slice(1)));
  }
  return result;
}

function cardinalLinePath(from: Coord, to: Coord): Coord[] {
  const result: Coord[] = [];
  let x = from.x;
  let y = from.y;
  result.push({ x, y });
  while (x !== to.x) {
    x += Math.sign(to.x - x);
    result.push({ x, y });
  }
  while (y !== to.y) {
    y += Math.sign(to.y - y);
    result.push({ x, y });
  }
  return result;
}

function maskFor(points: ReadonlyMap<string, unknown>, coord: Coord): number {
  return (
    (points.has(key({ x: coord.x, y: coord.y - 1 })) ? NORTH : 0) |
    (points.has(key({ x: coord.x + 1, y: coord.y })) ? EAST : 0) |
    (points.has(key({ x: coord.x, y: coord.y + 1 })) ? SOUTH : 0) |
    (points.has(key({ x: coord.x - 1, y: coord.y })) ? WEST : 0)
  );
}

function maskForRail(points: ReadonlySet<string>, coord: Coord): number {
  return (
    (points.has(key({ x: coord.x, y: coord.y - 1 })) ? NORTH : 0) |
    (points.has(key({ x: coord.x + 1, y: coord.y })) ? EAST : 0) |
    (points.has(key({ x: coord.x, y: coord.y + 1 })) ? SOUTH : 0) |
    (points.has(key({ x: coord.x - 1, y: coord.y })) ? WEST : 0)
  );
}
