import { describe, expect, it } from 'vitest';
import {
  agentGlyph,
  drawCar,
  pedestrianFinalRadius,
  pedestrianOpacity,
  pedestrianRadiusScale,
  traderHaloRadius,
} from '../../src/render/drawAgents';
import { AGENT_INK, TRADER_RED } from '../../src/render/designTokens';
import type { MinimalMapRendererState } from '../../src/render/minimalMapRenderer';
import { MINIMAL_MAP_TILE_SIZE } from '../../src/render/minimalMapProjection';
import type { BackendCar } from '../../src/render/backendMobilityDrawables';

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

describe('pedestrian visual policy', () => {
  it('renders ordinary city agents as readable citizens in the default city view', () => {
    expect(pedestrianOpacity('pedestrian', { opacity: 1, detail: 'individual' })).toBeCloseTo(0.72);
    expect(pedestrianRadiusScale('pedestrian', { opacity: 1, detail: 'individual' })).toBeCloseTo(0.95);
  });

  it('keeps traders readable because they explain economy motion', () => {
    expect(pedestrianOpacity('trader', { opacity: 0.9, detail: 'individual' })).toBeCloseTo(0.95);
    expect(pedestrianOpacity('trader', { opacity: 1, detail: 'individual' })).toBeCloseTo(1);
    expect(pedestrianRadiusScale('trader', { opacity: 1, detail: 'individual' })).toBeCloseTo(1.35);
  });

  it('keeps aggregate economy agents visible but secondary', () => {
    expect(pedestrianOpacity('pedestrian', { opacity: 0.55, detail: 'aggregate' })).toBeCloseTo(0.55);
    expect(pedestrianRadiusScale('pedestrian', { opacity: 0.55, detail: 'aggregate' })).toBeCloseTo(1);
  });
});

describe('pedestrian radius policy', () => {
  it('draws the trader paper halo outside the final trader dot radius', () => {
    const baseRadius = 4;
    const glyph = agentGlyph('walking', 'trader');
    const finalRadius = pedestrianFinalRadius(baseRadius, glyph, 'trader', { opacity: 1, detail: 'individual' });
    const haloRadius = traderHaloRadius(finalRadius, baseRadius);

    expect(finalRadius).toBeCloseTo(baseRadius * glyph.radiusScale * 1.35);
    expect(haloRadius).toBeGreaterThan(finalRadius);
    expect(haloRadius - finalRadius).toBeGreaterThanOrEqual(1);
  });
});

describe('vehicle visual policy', () => {
  it('draws no vehicle operations when the vehicle layer is hidden', () => {
    const operations: string[] = [];
    const ctx = new Proxy({} as CanvasRenderingContext2D, {
      get: (_target, prop) => {
        if (prop === 'canvas') return undefined;
        if (prop === 'save') return () => operations.push('save');
        if (prop === 'translate') return () => operations.push('translate');
        if (prop === 'rotate') return () => operations.push('rotate');
        if (prop === 'beginPath') return () => operations.push('beginPath');
        if (prop === 'moveTo') return () => operations.push('moveTo');
        if (prop === 'lineTo') return () => operations.push('lineTo');
        if (prop === 'stroke') return () => operations.push('stroke');
        if (prop === 'restore') return () => operations.push('restore');
        return () => undefined;
      },
      set: () => true,
    });
    const state = {
      ctx,
      camera: { scale: 1 },
      tileSize: MINIMAL_MAP_TILE_SIZE,
    } as MinimalMapRendererState;
    const car: BackendCar = {
      id: 'car:1',
      path: [{ x: 1, y: 1 }, { x: 2, y: 1 }],
      offset: 0,
      speed: 0,
      sprite: { sheet: 'car', role: 'car' },
      direction: 'e',
    };

    drawCar(state, car, false, { opacity: 0, detail: 'aggregate' });

    expect(operations).toEqual([]);
  });
});
