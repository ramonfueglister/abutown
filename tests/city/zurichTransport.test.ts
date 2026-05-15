import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';
import { buildZurichTransport } from '../../src/city/zurichTransport';
import { countAdjacentParallelRoadRuns } from '../../src/city/roadParallelCleanup';
import { countInvalidRoadDeadEnds, countRoadNetworkComponents } from '../../src/city/roadTopology';

describe('buildZurichTransport', () => {
  it('creates roads, rail, bridges, and intentional crossings without accidental overlap', () => {
    const { world, transport } = transportFixture();

    expect(transport.roads.size).toBeGreaterThan(1200);
    expect(transport.rails.size).toBe(world.height);
    expect(transport.bridges.size).toBeGreaterThanOrEqual(6);
    expect(transport.bridges.size).toBeLessThanOrEqual(12);
    expect(transport.railCrossings.size).toBeGreaterThanOrEqual(1);
    expect(transport.railPaths).toHaveLength(1);
    expect(transport.railPaths[0]).toHaveLength(world.height);

    const railXs = new Set([...transport.rails.values()].map((rail) => rail.coord.x));
    expect(railXs.size).toBe(1);
    const [railX] = [...railXs];
    expect(railX).toBe(150);
    for (let y = 0; y < world.height; y += 1) expect(transport.rails.has(`${railX}:${y}`)).toBe(true);

    const bridgeRoads = [...transport.roads.entries()].filter(([, road]) => road.kind === 'bridge');
    expect(bridgeRoads.length).toBe(transport.bridges.size);
    for (const [bridgeKey] of bridgeRoads) expect(transport.bridges.has(bridgeKey)).toBe(true);
    for (const bridgeKey of transport.bridges) expect(transport.roads.get(bridgeKey)?.kind).toBe('bridge');
    const bridgeSpans = connectedBridgeSpans(transport.bridges);
    expect(bridgeSpans.length).toBeGreaterThanOrEqual(2);
    expect(bridgeSpans.length).toBeLessThanOrEqual(3);
    expect(bridgeSpans.filter((span) => span.length >= 5).length).toBeGreaterThanOrEqual(1);
    expect(Math.max(...bridgeSpans.map((span) => span.length))).toBeLessThanOrEqual(5);

    for (const crossingKey of transport.railCrossings) {
      expect(transport.roads.has(crossingKey)).toBe(true);
      expect(transport.rails.has(crossingKey)).toBe(true);
    }

    for (const coord of transport.arterialPaths.flat()) {
      expect(transport.roads.has(`${coord.x}:${coord.y}`)).toBe(true);
    }
    for (const path of transport.arterialPaths) {
      for (let index = 1; index < path.length; index += 1) {
        const previous = path[index - 1];
        const current = path[index];
        expect(Math.abs(current.x - previous.x) + Math.abs(current.y - previous.y)).toBe(1);
      }
    }

    let accidentalOverlap = 0;
    for (const roadKey of transport.roads.keys()) {
      if (transport.rails.has(roadKey) && !transport.railCrossings.has(roadKey)) accidentalOverlap += 1;
    }
    expect(accidentalOverlap).toBe(0);
  });

  it('keeps a single vertical rail corridor off water without rail-yard ladders', () => {
    const { world, transport } = transportFixture();
    const railOnWater = [...transport.rails.entries()].filter(([railKey]) =>
      ['water', 'riverbank'].includes(world.terrain.get(railKey)?.kind ?? '')
    );

    expect(railOnWater).toEqual([]);
    expect(longRailRows(transport.rails, 40)).toHaveLength(0);
    expect(longRailColumns(transport.rails, world.height)).toHaveLength(1);
  });

  it('places bridge road tiles only on water or riverbank terrain', () => {
    const { world, transport } = transportFixture();

    const bridgeRoads = [...transport.roads.entries()].filter(([, road]) => road.kind === 'bridge');
    for (const [bridgeKey] of bridgeRoads) {
      const terrain = world.terrain.get(bridgeKey)?.kind;
      expect(['water', 'riverbank']).toContain(terrain);
    }

    for (const [roadKey, road] of transport.roads) {
      const terrain = world.terrain.get(roadKey)?.kind;
      if (terrain === 'water') expect(road.kind).toBe('bridge');
    }
  });

  it('keeps bridge spans straight and connected to street on both ends', () => {
    const { transport } = transportFixture();

    for (const span of connectedBridgeSpans(transport.bridges)) {
      expect(isStraightSpan(span)).toBe(true);
      expect(hasStreetEndpoint(span, transport.roads)).toBe(true);
    }
  });

  it('keeps non-bridge district streets out of the raw riverbank corridor', () => {
    const { world, transport } = transportFixture();
    const nonBridgeRiverbankRoads = [...transport.roads.entries()].filter(([roadKey, road]) =>
      road.kind !== 'bridge' && world.terrain.get(roadKey)?.kind === 'riverbank'
    );

    expect(nonBridgeRiverbankRoads).toEqual([]);
  });

  it('keeps district roads from reading as a rigid parallel grid', () => {
    const { transport } = transportFixture();

    expect(countAdjacentParallelRoadRuns(transport.roads)).toBeLessThanOrEqual(2);
  });

  it('only allows street dead-ends as straight map-edge exits', () => {
    const { world, transport } = transportFixture();

    expect(countInvalidRoadDeadEnds(transport.roads, { width: world.width, height: world.height })).toBe(0);
  });

  it('connects every village and district road into one street network', () => {
    const { transport } = transportFixture();

    expect(countRoadNetworkComponents(transport.roads)).toBe(1);
  });

  it('exports only through corridors for vehicle traffic, not short connector stubs', () => {
    const { world, transport } = transportFixture();
    const localTrafficStubs = transport.arterialPaths.filter((path) =>
      path.length < 48 ||
      !isMapEdge(path[0], world.width, world.height) ||
      !isMapEdge(path[path.length - 1], world.width, world.height)
    );

    expect(localTrafficStubs).toEqual([]);
  });
});

function transportFixture(): {
  world: ReturnType<typeof buildZurichWorld>;
  transport: ReturnType<typeof buildZurichTransport>;
} {
  if (!cachedTransportFixture) {
    const world = buildZurichWorld({ seed: 1848 });
    cachedTransportFixture = { world, transport: buildZurichTransport(world) };
  }
  return cachedTransportFixture;
}

let cachedTransportFixture:
  | { world: ReturnType<typeof buildZurichWorld>; transport: ReturnType<typeof buildZurichTransport> }
  | undefined;

function connectedBridgeSpans(bridgeKeys: ReadonlySet<string>): string[][] {
  const remaining = new Set(bridgeKeys);
  const spans: string[][] = [];
  for (const start of bridgeKeys) {
    if (!remaining.has(start)) continue;
    const span: string[] = [];
    const queue = [start];
    remaining.delete(start);
    while (queue.length > 0) {
      const current = queue.shift()!;
      span.push(current);
      const [x, y] = current.split(':').map(Number);
      for (const neighbor of [`${x + 1}:${y}`, `${x - 1}:${y}`, `${x}:${y + 1}`, `${x}:${y - 1}`]) {
        if (!remaining.has(neighbor)) continue;
        remaining.delete(neighbor);
        queue.push(neighbor);
      }
    }
    spans.push(span);
  }
  return spans;
}

function isStraightSpan(span: string[]): boolean {
  const coords = span.map((value) => value.split(':').map(Number));
  const xs = new Set(coords.map(([x]) => x));
  const ys = new Set(coords.map(([, y]) => y));
  return xs.size === 1 || ys.size === 1;
}

function hasStreetEndpoint(
  span: string[],
  roads: ReadonlyMap<string, { kind: string }>,
): boolean {
  const spanKeys = new Set(span);
  const endpoints = span
    .map((value) => {
      const [x, y] = value.split(':').map(Number);
      return { x, y };
    })
    .filter((coord) => cardinalKeys(coord).filter((neighbor) => spanKeys.has(neighbor)).length <= 1);

  return endpoints.length >= 2 && endpoints.every((coord) =>
    cardinalKeys(coord).some((neighbor) => roads.get(neighbor)?.kind === 'street')
  );
}

function cardinalKeys(coord: { x: number; y: number }): string[] {
  return [
    `${coord.x}:${coord.y - 1}`,
    `${coord.x + 1}:${coord.y}`,
    `${coord.x}:${coord.y + 1}`,
    `${coord.x - 1}:${coord.y}`,
  ];
}

function isMapEdge(coord: { x: number; y: number }, width: number, height: number): boolean {
  return coord.x === 0 || coord.y === 0 || coord.x === width - 1 || coord.y === height - 1;
}

function longRailRows(rails: ReadonlyMap<string, { coord: { x: number; y: number } }>, minTiles: number): Array<[number, number]> {
  const rowCounts = new Map<number, number>();
  for (const rail of rails.values()) rowCounts.set(rail.coord.y, (rowCounts.get(rail.coord.y) ?? 0) + 1);
  return [...rowCounts].filter(([, count]) => count >= minTiles);
}

function longRailColumns(rails: ReadonlyMap<string, { coord: { x: number; y: number } }>, minTiles: number): Array<[number, number]> {
  const columnCounts = new Map<number, number>();
  for (const rail of rails.values()) columnCounts.set(rail.coord.x, (columnCounts.get(rail.coord.x) ?? 0) + 1);
  return [...columnCounts].filter(([, count]) => count >= minTiles);
}
