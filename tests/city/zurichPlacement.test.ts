import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';
import { buildZurichTransport } from '../../src/city/zurichTransport';
import { buildZurichPlacement } from '../../src/city/zurichPlacement';
import { validateZurichCity } from '../../src/city/zurichValidation';
import { key } from '../../src/city/worldTypes';

describe('buildZurichPlacement', () => {
  it('places varied buildings, forests, and reserves without hard-rule conflicts', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(true);
    expect(validation.errors).toEqual([]);
    expect(placement.buildings.length).toBeGreaterThan(1800);
    expect(placement.trees.length).toBeGreaterThan(3200);
    expect(placement.details.length).toBeGreaterThan(120);
    expect(placement.reserveTiles.size).toBeGreaterThan(2500);
    expect(new Set(placement.buildings.map((building) => building.sheet)).size).toBeGreaterThanOrEqual(8);
  });

  it('keeps buildings off water, roads, and rails', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);

    for (const building of placement.buildings) {
      const tileKey = key(building.coord);
      expect(world.terrain.get(tileKey)?.kind).not.toBe('water');
      expect(transport.roads.has(tileKey)).toBe(false);
      expect(transport.rails.has(tileKey)).toBe(false);
    }
  });
});
