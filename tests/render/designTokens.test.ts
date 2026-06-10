import { describe, expect, it } from 'vitest';
import * as tokens from '../../src/render/designTokens';

describe('designTokens', () => {
  it('zoom bands are ordered inside the camera scale range (0.18..2.8)', () => {
    expect(tokens.ZOOM_ECONOMY_MAX).toBeGreaterThan(0.18);
    expect(tokens.ZOOM_CITY_MIN).toBeGreaterThan(tokens.ZOOM_ECONOMY_MAX);
    expect(tokens.ZOOM_CITY_MIN).toBeLessThan(2.8);
  });

  it('every good id used by the sim (1..5) has a flow color', () => {
    for (const id of [1, 2, 3, 4, 5]) {
      expect(tokens.GOOD_COLORS[id], `good ${id}`).toMatch(/^#[0-9a-f]{6}$/);
    }
    expect(tokens.GOOD_COLOR_FALLBACK).toMatch(/^#[0-9a-f]{6}$/);
  });

  it('opacity floors are sane', () => {
    expect(tokens.FLOW_MIN_OPACITY).toBeGreaterThan(0);
    expect(tokens.FLOW_MIN_OPACITY).toBeLessThan(1);
    expect(tokens.AGENT_SHIMMER_OPACITY).toBeGreaterThan(0);
    expect(tokens.AGENT_SHIMMER_OPACITY).toBeLessThan(1);
  });
});
