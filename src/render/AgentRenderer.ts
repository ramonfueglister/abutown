import { agentColor, agentSize } from './agentSprites';
import type { AgentRenderPlan, AgentRenderStats } from './agentLod';

const EMPTY_STATS: AgentRenderStats = {
  simulatedAgents: 0,
  visibleAgents: 0,
  renderedSamples: 0,
  aggregatedAgents: 0,
  culledAgents: 0,
  budget: 0,
};

export class AgentRenderer {
  lastStats: AgentRenderStats = EMPTY_STATS;

  render(context: CanvasRenderingContext2D, plan: AgentRenderPlan): void {
    context.save();
    for (const sample of plan.samples) {
      const size = agentSize(sample.lod);
      const densityScale = sample.lod === 'density' ? Math.min(1.5, 0.65 + sample.density * 0.02) : 1;
      context.fillStyle = agentColor(sample.colorIndex);
      context.globalAlpha = sample.lod === 'density' ? 0.55 : 0.82;
      context.fillRect(sample.x - (size.width * densityScale) / 2, sample.y - size.height * densityScale, size.width * densityScale, size.height * densityScale);
      if (sample.lod === 'citizen') {
        context.fillStyle = '#c99a6a';
        context.globalAlpha = 0.86;
        context.fillRect(sample.x - 1.5, sample.y - size.height - 3, 3, 3);
      }
    }
    context.restore();
    this.lastStats = plan.stats;
  }

  clear(): void {
    this.lastStats = EMPTY_STATS;
  }
}
