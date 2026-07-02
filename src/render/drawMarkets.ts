import type { MarketGoodDto, MarketLocationDto } from '../backend/mobilityProtocol';
import { AGENT_INK, MARKET_ORANGE, STATION_FILL, TRADER_RED } from './designTokens';
import { roundedRectPath } from './canvasPrimitives';
import { screenStableWorldSize } from './minimalGlyphScale';

type Point = { x: number; y: number };

/** Triangle vertices for the price-trend marker. Apex points toward the trend. */
export function trendTriangle(point: { x: number; y: number }, radius: number, dir: -1 | 1): [Point, Point, Point] {
  const baseY = point.y + dir * (radius + 5);
  return [
    { x: point.x, y: baseY + dir * 4 },
    { x: point.x - 3.4, y: baseY - dir * 2 },
    { x: point.x + 3.4, y: baseY - dir * 2 },
  ];
}
export type PriceTrend = 'up' | 'down' | 'flat';

const TREND_DEADBAND = 0.01; // ±1% of the EWMA reference counts as flat
const PULSE_DURATION_MS = 600;

export function marketActivity(goods: readonly MarketGoodDto[]): number {
  return goods.reduce((sum, g) => sum + g.tradedQtyLastTick, 0);
}

/** Screen-pixel node radius: floor 8, log growth, ceiling 17. */
export function marketNodeRadius(activity: number): number {
  if (activity <= 0) return 8;
  return Math.min(17, 8 + 2.2 * Math.log10(1 + activity));
}

export type StationShape = 'circle' | 'square' | 'diamond' | 'triangle';

export function stationShapeForMarket(marketId: number): StationShape {
  const shapes: readonly StationShape[] = ['circle', 'square', 'diamond', 'triangle'];
  return shapes[Math.abs(marketId) % shapes.length] ?? 'circle';
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
      { minWorld: 6, maxWorld: 30 },
    );
    const ringWidth = Math.max(2.4, radius * 0.3);

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
      ctx.lineWidth = ringWidth;
      ctx.beginPath();
      ctx.arc(point.x, point.y, radius * 1.7, 0, Math.PI * 2);
      ctx.stroke();
    }

    // Mini-Metro station: paper-white interchange glyph with a heavy ink ring.
    ctx.globalAlpha = 1;
    ctx.fillStyle = STATION_FILL;
    ctx.strokeStyle = AGENT_INK;
    ctx.lineWidth = ringWidth;
    stationShapePath(ctx, point, radius, stationShapeForMarket(market.marketId));
    ctx.fill();
    ctx.stroke();

    // Unmet demand: the ink ring stays open — an orange (or red when starved)
    // arc closes around the node proportional to satisfied demand.
    const fraction = satisfiedDemandFraction(goods);
    if (fraction < 1) {
      ctx.strokeStyle = fraction < 0.5 ? TRADER_RED : MARKET_ORANGE;
      ctx.lineWidth = ringWidth;
      ctx.beginPath();
      ctx.arc(point.x, point.y, radius + ringWidth * 1.4, -Math.PI / 2, -Math.PI / 2 + fraction * Math.PI * 2);
      ctx.stroke();
    }

    const trend = priceTrend(goods);
    if (trend !== 'flat') {
      const dir = trend === 'up' ? -1 : 1;
      const [apex, bl, br] = trendTriangle(point, radius, dir);
      ctx.fillStyle = trend === 'up' ? TRADER_RED : AGENT_INK;
      ctx.beginPath();
      ctx.moveTo(apex.x, apex.y);
      ctx.lineTo(bl.x, bl.y);
      ctx.lineTo(br.x, br.y);
      ctx.closePath();
      ctx.fill();
    }

    drawMarketLabel(ctx, market.name, point, radius, cameraScale);
    ctx.restore();
  }
}

/** Station name in screen-stable small caps under the node, with a paper halo. */
function drawMarketLabel(
  ctx: CanvasRenderingContext2D,
  name: string,
  point: Point,
  radius: number,
  cameraScale: number,
): void {
  if (!name) return;
  const fontSize = screenStableWorldSize(11, cameraScale, { minWorld: 8, maxWorld: 36 });
  const padX = screenStableWorldSize(5, cameraScale, { minWorld: 4, maxWorld: 18 });
  const padY = screenStableWorldSize(2.2, cameraScale, { minWorld: 2, maxWorld: 10 });
  ctx.font = `600 ${fontSize.toFixed(1)}px system-ui, -apple-system, BlinkMacSystemFont, sans-serif`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  const y = point.y + radius * 1.85 + fontSize * 0.5;
  const textWidth = ctx.measureText(name).width;
  const boxWidth = textWidth + padX * 2;
  const boxHeight = fontSize + padY * 2;
  ctx.globalAlpha = 0.92;
  ctx.fillStyle = STATION_FILL;
  roundedRectPath(ctx, point.x - boxWidth / 2, y - boxHeight / 2, boxWidth, boxHeight, boxHeight * 0.42);
  ctx.fill();
  ctx.globalAlpha = 0.42;
  ctx.strokeStyle = 'rgba(46, 52, 64, 0.28)';
  ctx.lineWidth = screenStableWorldSize(0.8, cameraScale, { minWorld: 0.6, maxWorld: 3 });
  roundedRectPath(ctx, point.x - boxWidth / 2, y - boxHeight / 2, boxWidth, boxHeight, boxHeight * 0.42);
  ctx.stroke();
  ctx.globalAlpha = 1;
  ctx.fillStyle = AGENT_INK;
  ctx.fillText(name, point.x, y);
}

function stationShapePath(ctx: CanvasRenderingContext2D, point: Point, radius: number, shape: StationShape): void {
  ctx.beginPath();
  if (shape === 'circle') {
    ctx.arc(point.x, point.y, radius, 0, Math.PI * 2);
    return;
  }
  if (shape === 'square') {
    roundedRectPath(ctx, point.x - radius, point.y - radius, radius * 2, radius * 2, radius * 0.22);
    return;
  }
  if (shape === 'diamond') {
    ctx.moveTo(point.x, point.y - radius * 1.15);
    ctx.lineTo(point.x + radius * 1.15, point.y);
    ctx.lineTo(point.x, point.y + radius * 1.15);
    ctx.lineTo(point.x - radius * 1.15, point.y);
    ctx.closePath();
    return;
  }
  ctx.moveTo(point.x, point.y - radius * 1.12);
  ctx.lineTo(point.x + radius * 1.08, point.y + radius * 0.72);
  ctx.lineTo(point.x - radius * 1.08, point.y + radius * 0.72);
  ctx.closePath();
}
