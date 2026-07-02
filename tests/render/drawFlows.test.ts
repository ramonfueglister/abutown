import { describe, expect, it } from 'vitest';
import {
  cargoDotCount,
  cargoDotPhases,
  drawFlows,
  flowCurveControlPoint,
  flowStrokeWidth,
  goodColor,
  pointOnQuadratic,
} from '../../src/render/drawFlows';
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

describe('cargoDotCount', () => {
  it('is zero for non-positive rates', () => {
    expect(cargoDotCount(0)).toBe(0);
    expect(cargoDotCount(-3)).toBe(0);
  });
  it('grows with rate and clamps at 8', () => {
    expect(cargoDotCount(100)).toBeGreaterThan(cargoDotCount(1));
    expect(cargoDotCount(1e9)).toBe(8);
  });
});

describe('cargoDotPhases', () => {
  it('spaces dots evenly and keeps every phase in [0,1)', () => {
    const phases = cargoDotPhases(4, 1300, 5200);
    expect(phases).toHaveLength(4);
    for (const t of phases) {
      expect(t).toBeGreaterThanOrEqual(0);
      expect(t).toBeLessThan(1);
    }
    const sorted = [...phases].sort((a, b) => a - b);
    expect(sorted[1] - sorted[0]).toBeCloseTo(0.25);
  });
  it('advances with wall-clock time (animation moves)', () => {
    const [t0] = cargoDotPhases(1, 0, 5200);
    const [t1] = cargoDotPhases(1, 2600, 5200);
    expect(t1).not.toBeCloseTo(t0);
  });
});

describe('pointOnQuadratic', () => {
  it('hits the endpoints at t=0 and t=1', () => {
    const a = { x: 0, y: 0 };
    const c = { x: 50, y: 40 };
    const b = { x: 100, y: 0 };
    expect(pointOnQuadratic(a, c, b, 0)).toEqual(a);
    expect(pointOnQuadratic(a, c, b, 1)).toEqual(b);
    expect(pointOnQuadratic(a, c, b, 0.5).y).toBeCloseTo(20);
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

  it('strokes no curves and draws no cargo at zero layer opacity', () => {
    const operations: string[] = [];
    const ctx = new Proxy({} as CanvasRenderingContext2D, {
      get: (_target, prop) => {
        if (prop === 'canvas') return undefined;
        if (prop === 'stroke') return () => operations.push('stroke');
        if (prop === 'fill') return () => operations.push('fill');
        if (prop === 'beginPath') return () => operations.push('beginPath');
        return () => undefined;
      },
      set: () => true,
    });

    const drawn = drawFlows(
      ctx,
      project,
      markets,
      [flow(9003, 9004, 250)],
      { opacity: 0, detail: 'aggregate' },
    );

    expect(drawn).toBe(0);
    expect(operations).toEqual([]);
  });
});
