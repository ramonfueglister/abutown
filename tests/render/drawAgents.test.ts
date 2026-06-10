import { describe, expect, it } from 'vitest';
import { agentGlyph } from '../../src/render/drawAgents';
import { AGENT_INK, TRADER_RED } from '../../src/render/designTokens';

describe('agentGlyph', () => {
  it('walking and in_vehicle render as filled ink dots', () => {
    expect(agentGlyph('walking', 'pedestrian')).toEqual({ shape: 'dot', color: AGENT_INK, radiusScale: 1 });
    expect(agentGlyph('in_vehicle', 'pedestrian')).toEqual({ shape: 'dot', color: AGENT_INK, radiusScale: 1 });
  });
  it('at_activity and waiting_at_stop render as rings', () => {
    expect(agentGlyph('at_activity', 'pedestrian').shape).toBe('ring');
    expect(agentGlyph('waiting_at_stop', 'pedestrian').shape).toBe('ring');
  });
  it('traders are larger red dots regardless of state', () => {
    expect(agentGlyph('at_activity', 'trader')).toEqual({ shape: 'dot', color: TRADER_RED, radiusScale: 1.5 });
  });
});
