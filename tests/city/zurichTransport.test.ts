import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';
import { buildZurichTransport } from '../../src/city/zurichTransport';

describe('buildZurichTransport', () => {
  it('creates roads, rail, bridges, and intentional crossings without accidental overlap', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);

    expect(transport.roads.size).toBeGreaterThan(1200);
    expect(transport.rails.size).toBeGreaterThan(180);
    expect(transport.bridges.size).toBeGreaterThanOrEqual(3);
    expect(transport.bridges.size).toBeLessThanOrEqual(5);
    expect(transport.railCrossings.size).toBeGreaterThanOrEqual(1);

    const bridgeRoads = [...transport.roads.entries()].filter(([, road]) => road.kind === 'bridge');
    expect(bridgeRoads.length).toBeGreaterThanOrEqual(3);
    expect(bridgeRoads.length).toBeLessThanOrEqual(5);
    expect(bridgeRoads.length).toBe(transport.bridges.size);
    for (const [bridgeKey] of bridgeRoads) expect(transport.bridges.has(bridgeKey)).toBe(true);
    for (const bridgeKey of transport.bridges) expect(transport.roads.get(bridgeKey)?.kind).toBe('bridge');

    for (const crossingKey of transport.railCrossings) {
      expect(transport.roads.has(crossingKey)).toBe(true);
      expect(transport.rails.has(crossingKey)).toBe(true);
    }

    for (const coord of transport.arterialPaths.flat()) {
      expect(transport.roads.has(`${coord.x}:${coord.y}`)).toBe(true);
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
  });
});
