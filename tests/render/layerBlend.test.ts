import { describe, expect, it } from 'vitest';
import { layerBlend } from '../../src/render/layerBlend';
import {
  AGENT_SHIMMER_OPACITY,
  FLOW_MIN_OPACITY,
  ZOOM_CITY_MIN,
  ZOOM_ECONOMY_MAX,
} from '../../src/render/designTokens';

describe('layerBlend', () => {
  it('network and markets are always fully visible', () => {
    for (const scale of [0.18, 0.6, 1.0, 2.8]) {
      expect(layerBlend('network', scale)).toEqual({ opacity: 1, detail: 'individual' });
      expect(layerBlend('markets', scale)).toEqual({ opacity: 1, detail: 'individual' });
    }
  });

  it('agents: shimmer in the economy band, full in the city band, monotone between', () => {
    expect(layerBlend('agents', 0.18)).toEqual({ opacity: AGENT_SHIMMER_OPACITY, detail: 'aggregate' });
    expect(layerBlend('agents', ZOOM_ECONOMY_MAX).opacity).toBeCloseTo(AGENT_SHIMMER_OPACITY);
    expect(layerBlend('agents', ZOOM_CITY_MIN)).toEqual({ opacity: 1, detail: 'individual' });
    expect(layerBlend('agents', 2.8)).toEqual({ opacity: 1, detail: 'individual' });
    const mid = layerBlend('agents', (ZOOM_ECONOMY_MAX + ZOOM_CITY_MIN) / 2).opacity;
    expect(mid).toBeGreaterThan(AGENT_SHIMMER_OPACITY);
    expect(mid).toBeLessThan(1);
    expect(layerBlend('agents', ZOOM_ECONOMY_MAX + 0.01).detail).toBe('individual');
  });

  it('flows: full in the economy band, hint in the city band, monotone between', () => {
    expect(layerBlend('flows', 0.18)).toEqual({ opacity: 1, detail: 'individual' });
    expect(layerBlend('flows', ZOOM_CITY_MIN).opacity).toBeCloseTo(FLOW_MIN_OPACITY);
    expect(layerBlend('flows', 2.8)).toEqual({ opacity: FLOW_MIN_OPACITY, detail: 'aggregate' });
    const mid = layerBlend('flows', (ZOOM_ECONOMY_MAX + ZOOM_CITY_MIN) / 2).opacity;
    expect(mid).toBeLessThan(1);
    expect(mid).toBeGreaterThan(FLOW_MIN_OPACITY);
    expect(layerBlend('flows', ZOOM_CITY_MIN - 0.01).detail).toBe('individual');
  });

  it('clamps outside the camera range', () => {
    expect(layerBlend('agents', 0.0001).opacity).toBeCloseTo(AGENT_SHIMMER_OPACITY);
    expect(layerBlend('flows', 100).opacity).toBeCloseTo(FLOW_MIN_OPACITY);
  });
});
