import type { EconomyFlowDto, MarketLocationDto } from '../backend/mobilityProtocol';
import { GOOD_COLORS, GOOD_COLOR_FALLBACK } from './designTokens';
import type { LayerBlend } from './layerBlend';

type Point = { x: number; y: number };

/** Quadratic-curve control point: segment midpoint displaced perpendicular,
 *  bulge proportional to length but capped so long edges stay tame. */
export function flowCurveControlPoint(a: Point, b: Point): Point {
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2;
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  const bulge = Math.min(40, len * 0.18);
  return { x: mx - (dy / len) * bulge, y: my + (dx / len) * bulge };
}

/** World-unit stroke width from an EWMA rate. 0 means "do not draw". */
export function flowStrokeWidth(rate: number): number {
  if (rate <= 0) return 0;
  return Math.min(10, 2 + 2 * Math.log10(1 + rate));
}

export function goodColor(goodId: number): string {
  return GOOD_COLORS[goodId] ?? GOOD_COLOR_FALLBACK;
}

/** Draw all flows. Returns the number of curves drawn (for diagnostics/smoke). */
export function drawFlows(
  ctx: CanvasRenderingContext2D,
  project: (coord: Point) => Point,
  markets: ReadonlyMap<number, MarketLocationDto>,
  flows: readonly EconomyFlowDto[],
  blend: LayerBlend,
): number {
  if (blend.opacity <= 0) return 0;
  let drawn = 0;
  ctx.save();
  ctx.lineCap = 'round';
  for (const flow of flows) {
    const width = flowStrokeWidth(flow.rate);
    if (width === 0) continue;
    const src = markets.get(flow.srcMarketId);
    const dst = markets.get(flow.dstMarketId);
    if (!src || !dst) continue;
    const a = project({ x: src.tileX, y: src.tileY });
    const b = project({ x: dst.tileX, y: dst.tileY });
    const c = flowCurveControlPoint(a, b);
    ctx.globalAlpha = 0.85 * blend.opacity;
    ctx.strokeStyle = goodColor(flow.goodId);
    ctx.lineWidth = width;
    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.quadraticCurveTo(c.x, c.y, b.x, b.y);
    ctx.stroke();
    if (blend.detail === 'individual') drawChevron(ctx, a, c, b);
    drawn += 1;
  }
  ctx.restore();
  return drawn;
}

/** Direction marker at the curve's t=0.5 point, oriented along the tangent. */
function drawChevron(ctx: CanvasRenderingContext2D, a: Point, c: Point, b: Point): void {
  const mid = { x: 0.25 * a.x + 0.5 * c.x + 0.25 * b.x, y: 0.25 * a.y + 0.5 * c.y + 0.25 * b.y };
  const tangent = { x: b.x - a.x, y: b.y - a.y };
  const len = Math.hypot(tangent.x, tangent.y) || 1;
  const tx = tangent.x / len;
  const ty = tangent.y / len;
  const size = 5;
  ctx.beginPath();
  ctx.moveTo(mid.x - size * tx - size * 0.7 * ty, mid.y - size * ty + size * 0.7 * tx);
  ctx.lineTo(mid.x + size * tx, mid.y + size * ty);
  ctx.lineTo(mid.x - size * tx + size * 0.7 * ty, mid.y - size * ty - size * 0.7 * tx);
  ctx.lineWidth = 2;
  ctx.stroke();
}
