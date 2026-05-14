import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';

describe('buildZurichWorld', () => {
  it('builds a deterministic flat 256 by 256 city region', () => {
    const first = buildZurichWorld({ seed: 1848 });
    const second = buildZurichWorld({ seed: 1848 });

    expect(first).toEqual(second);
    expect(first.width).toBe(256);
    expect(first.height).toBe(256);
    expect(first.chunkSize).toBe(32);
    expect(first.terrain.size).toBe(256 * 256);
    expect(first.zones.length).toBeGreaterThanOrEqual(10);
  });

  it('contains river, old town, rail center, forest, industry, residential, and reserve zones', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const zoneKinds = new Set(world.zones.map((zone) => zone.kind));

    expect(zoneKinds.has('river')).toBe(true);
    expect(zoneKinds.has('old-town')).toBe(true);
    expect(zoneKinds.has('rail-center')).toBe(true);
    expect(zoneKinds.has('forest')).toBe(true);
    expect(zoneKinds.has('industry')).toBe(true);
    expect(zoneKinds.has('residential')).toBe(true);
    expect(zoneKinds.has('reserve')).toBe(true);
  });

  it('keeps the terrain flat while reserving meaningful water and forest coverage', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const terrainValues = [...world.terrain.values()];

    expect(new Set(terrainValues.map((tile) => tile.elevation))).toEqual(new Set([0]));
    expect(terrainValues.filter((tile) => tile.kind === 'water').length).toBeGreaterThan(1800);
    expect(terrainValues.filter((tile) => tile.kind === 'forest').length).toBeGreaterThan(4500);
    expect(terrainValues.filter((tile) => tile.kind === 'reserve').length).toBeGreaterThan(2500);
  });
});
