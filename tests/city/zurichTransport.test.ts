import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';
import { buildZurichTransport } from '../../src/city/zurichTransport';
import { countAdjacentParallelRoadRuns } from '../../src/city/roadParallelCleanup';

describe('buildZurichTransport', () => {
  it('creates roads, rail, bridges, and intentional crossings without accidental overlap', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);

    expect(transport.roads.size).toBeGreaterThan(1200);
    expect(transport.rails.size).toBeGreaterThan(180);
    expect(transport.bridges.size).toBeGreaterThanOrEqual(18);
    expect(transport.railCrossings.size).toBeGreaterThanOrEqual(1);

    const bridgeRoads = [...transport.roads.entries()].filter(([, road]) => road.kind === 'bridge');
    expect(bridgeRoads.length).toBe(transport.bridges.size);
    for (const [bridgeKey] of bridgeRoads) expect(transport.bridges.has(bridgeKey)).toBe(true);
    for (const bridgeKey of transport.bridges) expect(transport.roads.get(bridgeKey)?.kind).toBe('bridge');
    const bridgeSpans = connectedBridgeSpans(transport.bridges);
    expect(bridgeSpans.length).toBeGreaterThanOrEqual(3);
    expect(bridgeSpans.filter((span) => span.length >= 5).length).toBeGreaterThanOrEqual(3);

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

  it('places bridge road tiles only on water or riverbank terrain', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);

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

  it('keeps non-bridge district streets out of the raw riverbank corridor', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const nonBridgeRiverbankRoads = [...transport.roads.entries()].filter(([roadKey, road]) =>
      road.kind !== 'bridge' && world.terrain.get(roadKey)?.kind === 'riverbank'
    );

    expect(nonBridgeRiverbankRoads).toEqual([]);
  });

  it('keeps district roads from reading as a rigid parallel grid', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);

    expect(countAdjacentParallelRoadRuns(transport.roads)).toBeLessThanOrEqual(2);
  });
});

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
