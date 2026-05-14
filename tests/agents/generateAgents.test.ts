import { describe, expect, it } from 'vitest';
import { generateAgents } from '../../src/agents/generateAgents';
import { generateCity } from '../../src/city/generateCity';

describe('generateAgents', () => {
  it('creates a deterministic 10,000 agent population without storing agents on the city', () => {
    const city = generateCity();
    const first = generateAgents(city, { count: 10_000, seed: 9231 });
    const second = generateAgents(city, { count: 10_000, seed: 9231 });

    expect(first.stats.totalAgents).toBe(10_000);
    expect(first.stats.pedestrians + first.stats.vehicles).toBe(10_000);
    expect(first.agents.slice(0, 12)).toEqual(second.agents.slice(0, 12));
    expect('agents' in city).toBe(false);
  });

  it('buckets agents by road segment', () => {
    const city = generateCity();
    const population = generateAgents(city, { count: 800, seed: 7 });

    expect(population.segmentBuckets.size).toBeGreaterThan(1);
    expect([...population.segmentBuckets.values()].flat()).toHaveLength(800);
  });
});
