import { inside, key, parseKey, type Coord, type ZurichRailTile, type ZurichRoadKind, type ZurichRoadTile, type ZurichWorld } from './worldTypes';
import { removeAdjacentParallelRoadRuns } from './roadParallelCleanup';
import { pruneInvalidRoadDeadEnds } from './roadTopology';

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
  const roadKinds = new Map<string, ZurichRoadKind>();
  const bridgeKeys = new Set<string>();
  const arterialPaths = buildArterialPaths(world);
  const railCrossings = new Set(
    arterialPaths
      .flat()
      .map(key)
      .filter((tileKey) => railPoints.has(tileKey)),
  );

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
    if (zone.kind === 'forest' || zone.kind === 'river' || zone.kind === 'reserve') continue;
    addDistrictStreetPattern(world, addRoad, zone.center, zone.radius, zone.density);
  }

  removeAdjacentParallelRoadRuns(roadKinds, new Set([
    ...arterialPaths.flat().map(key),
    ...bridgeKeys,
    ...railCrossings,
  ]));
  pruneInvalidRoadDeadEnds(roadKinds, { width: world.width, height: world.height });
  const postPruneConnectorPaths = buildPostPruneConnectorPaths(world);
  for (const path of postPruneConnectorPaths) for (const coord of path) addRoad(coord, true);
  removeStreetsTouchingOpenWater(world, roadKinds);
  pruneInvalidRoadDeadEnds(roadKinds, { width: world.width, height: world.height });
  removeDisconnectedRoadComponents(roadKinds);
  for (const bridgeKey of [...bridgeKeys]) {
    if (!roadKinds.has(bridgeKey)) bridgeKeys.delete(bridgeKey);
  }

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

  const roadBackedArterialPaths = splitRoadBackedPaths(arterialPaths.filter((path) => isThroughTrafficPath(path, world)), roadKinds);

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
    route([{ x: 94, y: 0 }, { x: 101, y: 78 }, { x: 105, y: 145 }, { x: 101, y: 196 }, { x: 88, y: world.height - 1 }]),
    route([{ x: 0, y: 176 }, { x: 43, y: 184 }, { x: 100, y: 196 }, { x: 151, y: 180 }, { x: 175, y: 184 }, { x: world.width - 1, y: 198 }]),
    route([{ x: 139, y: 112 }, { x: 139, y: 143 }, { x: 151, y: 143 }]),
    route([{ x: 101, y: 78 }, { x: 112, y: 78 }]),
    route([{ x: 61, y: 184 }, { x: 61, y: 190 }]),
    route([{ x: 142, y: 196 }, { x: 142, y: 216 }]),
  ];
}

function isThroughTrafficPath(path: readonly Coord[], world: ZurichWorld): boolean {
  return path.length >= 48 && isMapEdge(path[0], world) && isMapEdge(path[path.length - 1], world);
}

function isMapEdge(coord: Coord | undefined, world: ZurichWorld): boolean {
  if (!coord) return false;
  return coord.x === 0 || coord.y === 0 || coord.x === world.width - 1 || coord.y === world.height - 1;
}

function buildRailPaths(world: ZurichWorld): Coord[][] {
  return [
    route([{ x: 150, y: 0 }, { x: 150, y: world.height - 1 }]),
  ];
}

function buildPostPruneConnectorPaths(world: ZurichWorld): Coord[][] {
  return [
    route([{ x: 101, y: 145 }, { x: 101, y: 183 }]),
    route([{ x: 101, y: 78 }, { x: 112, y: 78 }]),
    route([{ x: 61, y: 184 }, { x: 61, y: 190 }]),
    route([{ x: 142, y: 196 }, { x: 142, y: 216 }]),
  ].filter((path) => path.length > 0 && path.every((coord) => inside(coord, world.width, world.height)));
}

function addDistrictStreetPattern(world: ZurichWorld, addRoad: (coord: Coord) => void, center: Coord, radius: number, density: number): void {
  const halfWidth = Math.max(8, Math.round(radius * (density > 0.8 ? 0.78 : 0.58)));
  const halfHeight = Math.max(6, Math.round(radius * (density > 0.8 ? 0.55 : 0.42)));
  const left = center.x - halfWidth;
  const right = center.x + halfWidth;
  const top = center.y - halfHeight;
  const bottom = center.y + halfHeight;

  addRoadLoop(addRoad, left, right, top, bottom);
  addRoadLine(addRoad, { x: left, y: center.y }, { x: right, y: center.y });
  addRoadLine(addRoad, { x: center.x, y: top }, { x: center.x, y: bottom });

  const horizontalOffsets = density > 0.72
    ? blockOffsets(halfHeight, 4)
    : density > 0.45
      ? blockOffsets(halfHeight, 6)
      : symmetricOffsets(Math.max(5, Math.round(halfHeight * 0.5)));
  const verticalOffsets = density > 0.72
    ? blockOffsets(halfWidth, 5)
    : density > 0.45
      ? blockOffsets(halfWidth, 7)
      : symmetricOffsets(Math.max(7, Math.round(halfWidth * 0.38)));

  for (const offset of horizontalOffsets) {
    const y = center.y + offset;
    if (y > top && y < bottom) addRoadLine(addRoad, { x: left, y }, { x: right, y });
  }
  for (const offset of verticalOffsets) {
    const x = center.x + offset;
    if (x > left && x < right) addRoadLine(addRoad, { x, y: top }, { x, y: bottom });
  }
}

function removeDisconnectedRoadComponents(roads: Map<string, ZurichRoadKind>): void {
  const components: string[][] = [];
  const remaining = new Set(roads.keys());

  for (const start of roads.keys()) {
    if (!remaining.has(start)) continue;
    const component: string[] = [];
    const queue = [start];
    remaining.delete(start);

    while (queue.length > 0) {
      const current = queue.shift()!;
      component.push(current);
      for (const neighbor of cardinal(parseKey(current))) {
        const neighborKey = key(neighbor);
        if (!remaining.has(neighborKey)) continue;
        remaining.delete(neighborKey);
        queue.push(neighborKey);
      }
    }

    components.push(component);
  }

  const largest = components.sort((a, b) => b.length - a.length)[0];
  if (!largest) return;
  const connectedKeys = new Set(largest);
  for (const tileKey of roads.keys()) {
    if (!connectedKeys.has(tileKey)) roads.delete(tileKey);
  }
}

function removeStreetsTouchingOpenWater(world: ZurichWorld, roads: Map<string, ZurichRoadKind>): void {
  const removable: string[] = [];
  for (const [tileKey, kind] of roads) {
    if (kind !== 'street') continue;
    const coord = parseKey(tileKey);
    const touchesOpenWater = cardinal(coord).some((neighbor) => {
      const neighborKey = key(neighbor);
      const terrain = world.terrain.get(neighborKey)?.kind;
      return (terrain === 'water' || terrain === 'riverbank') && roads.get(neighborKey) !== 'bridge';
    });
    if (touchesOpenWater) removable.push(tileKey);
  }
  for (const tileKey of removable) roads.delete(tileKey);
}

function blockOffsets(halfSize: number, spacing: number): number[] {
  const inner = Math.max(spacing + 1, halfSize - 3);
  const offsets: number[] = [];
  for (let offset = -inner; offset <= inner; offset += spacing) {
    if (Math.abs(offset) < 2) continue;
    offsets.push(offset);
  }
  return offsets;
}

function symmetricOffsets(offset: number): number[] {
  return [-offset, offset];
}

function addRoadLoop(addRoad: (coord: Coord) => void, left: number, right: number, top: number, bottom: number): void {
  for (const coord of route([
    { x: left, y: top },
    { x: right, y: top },
    { x: right, y: bottom },
    { x: left, y: bottom },
    { x: left, y: top },
  ])) addRoad(coord);
}

function addRoadLine(addRoad: (coord: Coord) => void, from: Coord, to: Coord): void {
  for (const coord of route([from, to])) addRoad(coord);
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

function cardinal(coord: Coord): Coord[] {
  return [
    { x: coord.x, y: coord.y - 1 },
    { x: coord.x + 1, y: coord.y },
    { x: coord.x, y: coord.y + 1 },
    { x: coord.x - 1, y: coord.y },
  ];
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
