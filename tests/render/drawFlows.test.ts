import { describe, expect, it } from 'vitest';
import { drawFlows, flowCurveControlPoint, flowStrokeWidth, goodColor } from '../../src/render/drawFlows';
import { GOOD_COLORS, GOOD_COLOR_FALLBACK } from '../../src/render/designTokens';
import type { EconomyFlowDto, MarketLocationDto } from '../../src/backend/mobilityProtocol';

describe('flowCurveControlPoint', () => {
  it('bulges perpendicular to the segment midpoint', () => {
    const c = flowCurveControlPoint({ x: 0, y: 0 }, { x: 100, y: 0 });
    expect(c.x).toBeCloseTo(50);
    expect(c.y).not.toBeCloseTo(0); // displaced off the segment
  });
  it('is antisymmetric in direction (A→B bulges opposite to B→A)', () => {
    const ab = flowCurveControlPoint({ x: 0, y: 0 }, { x: 100, y: 0 });
    const ba = flowCurveControlPoint({ x: 100, y: 0 }, { x: 0, y: 0 });
    expect(ab.y).toBeCloseTo(-ba.y);
  });
});

describe('flowStrokeWidth', () => {
  it('is zero for non-positive rates', () => {
    expect(flowStrokeWidth(0)).toBe(0);
    expect(flowStrokeWidth(-5)).toBe(0);
  });
  it('grows monotonically and clamps at 10', () => {
    expect(flowStrokeWidth(10)).toBeGreaterThan(flowStrokeWidth(1));
    expect(flowStrokeWidth(1e9)).toBe(10);
  });
});

describe('goodColor', () => {
  it('maps known goods and falls back for unknown ids', () => {
    expect(goodColor(1)).toBe(GOOD_COLORS[1]);
    expect(goodColor(999)).toBe(GOOD_COLOR_FALLBACK);
  });
});

describe('drawFlows', () => {
  const market = (marketId: number, tileX: number, tileY: number): MarketLocationDto =>
    ({ marketId, name: `m${marketId}`, tileX, tileY, wagePaidLastTick: 0 });
  const flow = (src: number, dst: number, rate: number): EconomyFlowDto =>
    ({ srcMarketId: src, dstMarketId: dst, goodId: 1, rate });
  const fakeCtx = (): CanvasRenderingContext2D =>
    new Proxy({} as CanvasRenderingContext2D, {
      get: (target, prop) => {
        void target;
        if (prop === 'canvas') return undefined;
        return () => undefined;
      },
      set: () => true,
    });
  const project = (c: { x: number; y: number }) => ({ x: c.x * 18 + 9, y: c.y * 18 + 9 });
  const markets = new Map([[9003, market(9003, 16, 48)], [9004, market(9004, 208, 48)]]);

  it('draws one curve per positive flow with known endpoints and reports the count', () => {
    const drawn = drawFlows(fakeCtx(), project, markets, [flow(9003, 9004, 250)], { opacity: 1, detail: 'individual' });
    expect(drawn).toBe(1);
  });

  it('skips zero-rate flows and flows with unknown markets', () => {
    expect(drawFlows(fakeCtx(), project, markets, [flow(9003, 9004, 0)], { opacity: 1, detail: 'individual' })).toBe(0);
    expect(drawFlows(fakeCtx(), project, markets, [flow(1, 9004, 50)], { opacity: 1, detail: 'individual' })).toBe(0);
  });

  it('draws nothing at zero layer opacity', () => {
    expect(drawFlows(fakeCtx(), project, markets, [flow(9003, 9004, 250)], { opacity: 0, detail: 'aggregate' })).toBe(0);
  });
});
