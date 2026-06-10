import type { MarketGoodDto, MarketLocationDto } from '../backend/mobilityProtocol';
import { GROUND, MARKET_ORANGE } from './designTokens';
import { screenStableWorldSize } from './minimalGlyphScale';

type Point = { x: number; y: number };
export type PriceTrend = 'up' | 'down' | 'flat';

const TREND_DEADBAND = 0.01; // ±1% of the EWMA reference counts as flat
const PULSE_DURATION_MS = 600;

export function marketActivity(goods: readonly MarketGoodDto[]): number {
  return goods.reduce((sum, g) => sum + g.tradedQtyLastTick, 0);
}

/** Screen-pixel node radius: floor 6, log growth, ceiling 14. */
export function marketNodeRadius(activity: number): number {
  if (activity <= 0) return 6;
  return Math.min(14, 6 + 2 * Math.log10(1 + activity));
}

export function satisfiedDemandFraction(goods: readonly MarketGoodDto[]): number {
  const traded = goods.reduce((s, g) => s + g.tradedQtyLastTick, 0);
  const unmet = goods.reduce((s, g) => s + g.unmetDemandLastTick, 0);
  if (traded + unmet === 0) return 1;
  return traded / (traded + unmet);
}

export function priceTrend(goods: readonly MarketGoodDto[]): PriceTrend {
  let score = 0;
  for (const g of goods) {
    if (g.ewmaReferencePrice === 0) continue;
    const deviation = (g.lastSettlementPrice - g.ewmaReferencePrice) / g.ewmaReferencePrice;
    if (deviation > TREND_DEADBAND) score += 1;
    else if (deviation < -TREND_DEADBAND) score -= 1;
  }
  if (score > 0) return 'up';
  if (score < 0) return 'down';
  return 'flat';
}

// Settlement-pulse bookkeeping: render-only wall-clock animation state.
// Keyed by marketId; restarted only when traded qty changes (a settlement happened).
const pulseState = new Map<number, { lastTradedQty: number; pulseStartMs: number }>();

export function pulseAlpha(nowMs: number, pulseStartMs: number): number {
  const t = (nowMs - pulseStartMs) / PULSE_DURATION_MS;
  if (t < 0 || t >= 1) return 0;
  return 0.5 * (1 - t);
}

export function drawMarketNodes(
  ctx: CanvasRenderingContext2D,
  project: (coord: Point) => Point,
  cameraScale: number,
  markets: ReadonlyMap<number, MarketLocationDto>,
  goodsByMarket: (marketId: number) => readonly MarketGoodDto[],
  nowMs: number,
): void {
  for (const market of markets.values()) {
    const goods = goodsByMarket(market.marketId);
    const point = project({ x: market.tileX, y: market.tileY });
    const radius = screenStableWorldSize(
      marketNodeRadius(marketActivity(goods)),
      cameraScale,
      { minWorld: 5, maxWorld: 16 },
    );

    const traded = marketActivity(goods);
    const tracked = pulseState.get(market.marketId);
    if (!tracked || tracked.lastTradedQty !== traded) {
      pulseState.set(market.marketId, {
        lastTradedQty: traded,
        pulseStartMs: tracked && traded > 0 ? nowMs : Number.NEGATIVE_INFINITY,
      });
    }
    const pulse = pulseAlpha(nowMs, pulseState.get(market.marketId)?.pulseStartMs ?? Number.NEGATIVE_INFINITY);

    ctx.save();
    if (pulse > 0) {
      ctx.globalAlpha = pulse;
      ctx.strokeStyle = MARKET_ORANGE;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(point.x, point.y, radius * 1.6, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    ctx.fillStyle = MARKET_ORANGE;
    ctx.strokeStyle = GROUND;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(point.x, point.y, radius, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    const fraction = satisfiedDemandFraction(goods);
    if (fraction < 1) {
      ctx.strokeStyle = MARKET_ORANGE;
      ctx.lineWidth = 2.4;
      ctx.beginPath();
      ctx.arc(point.x, point.y, radius + 3, -Math.PI / 2, -Math.PI / 2 + fraction * Math.PI * 2);
      ctx.stroke();
    }
    const trend = priceTrend(goods);
    if (trend !== 'flat') {
      const dir = trend === 'up' ? -1 : 1;
      ctx.fillStyle = GROUND;
      ctx.beginPath();
      ctx.moveTo(point.x, point.y + dir * (radius * 0.45) - dir * 2);
      ctx.lineTo(point.x - 3, point.y + dir * (radius * 0.45) + dir * 3);
      ctx.lineTo(point.x + 3, point.y + dir * (radius * 0.45) + dir * 3);
      ctx.closePath();
      ctx.fill();
    }
    ctx.restore();
  }
}
