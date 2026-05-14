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

    let accidentalOverlap = 0;
    for (const roadKey of transport.roads.keys()) {
      if (transport.rails.has(roadKey) && !transport.railCrossings.has(roadKey)) accidentalOverlap += 1;
    }
    expect(accidentalOverlap).toBe(0);
  });

  it('places bridge road tiles only on water or riverbank terrain', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);

    for (const bridgeKey of transport.bridges) {
      const terrain = world.terrain.get(bridgeKey)?.kind;
      expect(['water', 'riverbank']).toContain(terrain);
    }
  });
});
