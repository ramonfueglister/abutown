import { describe, expect, it } from 'vitest';
import { generatePopulation } from '../../src/agents/generateAgents';
import { generateCity } from '../../src/city/generateCity';

describe('generatePopulation', () => {
  it('creates a deterministic 10,000 entity population without storing entities on the city', () => {
    const city = generateCity();
    const first = generatePopulation(city, { count: 10_000, seed: 9231 });
    const second = generatePopulation(city, { count: 10_000, seed: 9231 });

    expect(first.stats.totalEntities).toBe(10_000);
    expect(first.stats.people + first.stats.vehicles).toBe(10_000);
    expect(first.entities.slice(0, 12)).toEqual(second.entities.slice(0, 12));
    expect('agents' in city).toBe(false);
  });

  it('buckets population entities by road segment', () => {
    const city = generateCity();
    const population = generatePopulation(city, { count: 800, seed: 7 });

    expect(population.segmentBuckets.size).toBeGreaterThan(1);
    expect([...population.segmentBuckets.values()].flat()).toHaveLength(800);
  });
});
