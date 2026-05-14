import { describe, expect, it } from 'vitest';
import { AGENT_PALETTE, agentColor, agentSize, normalizeColorIndex } from '../../src/render/agentSprites';

describe('agent sprite helpers', () => {
  it('normalizes palette indices', () => {
    expect(normalizeColorIndex(-1)).toBe(AGENT_PALETTE.length - 1);
    expect(normalizeColorIndex(AGENT_PALETTE.length + 1)).toBe(1);
  });

  it('keeps LOD marker sizes tiny for the city scale', () => {
    expect(agentSize('density')).toEqual({ width: 2, height: 2 });
    expect(agentSize('pixel').height).toBeLessThan(agentSize('citizen').height);
    expect(agentColor(0)).toMatch(/^#[0-9a-f]{6}$/);
  });
});
