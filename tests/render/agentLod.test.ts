import { describe, expect, it } from 'vitest';
import { generateAgents } from '../../src/agents/generateAgents';
import { generateCity } from '../../src/city/generateCity';
import { buildAgentRenderPlan } from '../../src/render/agentLod';

describe('buildAgentRenderPlan', () => {
  it('culls, budgets, and aggregates a large simulated population', () => {
    const city = generateCity();
    const population = generateAgents(city, { count: 10_000, seed: 9231 });
    const plan = buildAgentRenderPlan(city, population, {
      stageX: 420,
      stageY: -160,
      stageScale: 1.25,
      viewportWidth: 1280,
      viewportHeight: 800,
      quality: 'standard',
    });

    expect(plan.stats.simulatedAgents).toBe(10_000);
    expect(plan.stats.renderedSamples).toBeLessThanOrEqual(plan.stats.budget);
    expect(plan.stats.visibleAgents + plan.stats.culledAgents).toBe(10_000);
    expect(plan.stats.aggregatedAgents).toBe(Math.max(0, plan.stats.visibleAgents - plan.stats.renderedSamples));
  });

  it('drops to density LOD at far zoom', () => {
    const city = generateCity();
    const population = generateAgents(city, { count: 10_000, seed: 9231 });
    const plan = buildAgentRenderPlan(city, population, {
      stageX: 420,
      stageY: -160,
      stageScale: 0.55,
      viewportWidth: 1280,
      viewportHeight: 800,
      quality: 'ultra-low',
    });

    expect(plan.stats.budget).toBe(72);
    expect(new Set(plan.samples.map((sample) => sample.lod))).toEqual(new Set(['density']));
  });
});
