import { describe, expect, test } from 'vitest';
import { defaultCitySeed } from '../../src/city/defaultSeed';
import { generateCity } from '../../src/city/generateCity';
import { generateAgents } from '../../src/agents/generateAgents';

describe('generateAgents', () => {
  test('generates a deterministic 10000-agent population without adding display objects to the city', () => {
    const city = generateCity(defaultCitySeed);
    const first = generateAgents(city, { count: 10_000, seed: 9231 });
    const second = generateAgents(city, { count: 10_000, seed: 9231 });

    expect(first).toEqual(second);
    expect(first.agents).toHaveLength(10_000);
    expect(first.segmentBuckets.size).toBeGreaterThan(10);
    expect(first.stats.totalAgents).toBe(10_000);
    expect(first.stats.pedestrians).toBeGreaterThan(7000);
    expect(first.stats.vehicles).toBeGreaterThan(500);
    expect('agents' in city).toBe(false);
  });

  test('assigns every generated agent to a real road edge and normalized progress', () => {
    const city = generateCity(defaultCitySeed);
    const population = generateAgents(city, { count: 10_000, seed: 9231 });
    const roadEdgeIds = new Set(city.roadEdges.map((edge) => edge.id));

    for (const agent of population.agents) {
      expect(roadEdgeIds.has(agent.roadEdgeId)).toBe(true);
      expect(agent.progress).toBeGreaterThanOrEqual(0);
      expect(agent.progress).toBeLessThan(1);
      expect(agent.speedTilesPerSecond).toBeGreaterThan(0);
    }
  });
});
