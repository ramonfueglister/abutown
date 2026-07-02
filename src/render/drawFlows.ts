import type { EconomyFlowDto, MarketLocationDto } from '../backend/mobilityProtocol';
import { FLOW_CASING, GOOD_COLORS, GOOD_COLOR_FALLBACK } from './designTokens';
import type { LayerBlend } from './layerBlend';
import { drawCapsule } from './canvasPrimitives';
import { screenStableWorldSize } from './minimalGlyphScale';

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

/** Screen-pixel stroke width from an EWMA rate. 0 means "do not draw". */
export function flowStrokeWidth(rate: number): number {
  if (rate <= 0) return 0;
  return Math.min(10, 2 + 2 * Math.log10(1 + rate));
}

export function goodColor(goodId: number): string {
  return GOOD_COLORS[goodId] ?? GOOD_COLOR_FALLBACK;
}

/** Number of animated cargo dots riding a flow. More throughput, more cargo. */
export function cargoDotCount(rate: number): number {
  if (rate <= 0) return 0;
  return Math.min(8, 1 + Math.floor(1.8 * Math.log10(1 + rate)));
}

const CARGO_PERIOD_MS = 5200; // one full src→dst trip per dot

/** Evenly-phased curve parameters in [0,1) for the cargo dots at a wall-clock time. */
export function cargoDotPhases(count: number, nowMs: number, periodMs = CARGO_PERIOD_MS): number[] {
  const base = (nowMs % periodMs) / periodMs;
  return Array.from({ length: count }, (_, i) => (base + i / count) % 1);
}

export function pointOnQuadratic(a: Point, c: Point, b: Point, t: number): Point {
  const u = 1 - t;
  return {
    x: u * u * a.x + 2 * u * t * c.x + t * t * b.x,
    y: u * u * a.y + 2 * u * t * c.y + t * t * b.y,
  };
}

/** Tangent angle (radians) of the quadratic at t — orients cargo along the line. */
export function angleOnQuadratic(a: Point, c: Point, b: Point, t: number): number {
  const u = 1 - t;
  const dx = 2 * u * (c.x - a.x) + 2 * t * (b.x - c.x);
  const dy = 2 * u * (c.y - a.y) + 2 * t * (b.y - c.y);
  return Math.atan2(dy, dx);
}

/** Draw all flows. Returns the number of curves drawn (for diagnostics/smoke). */
export function drawFlows(
  ctx: CanvasRenderingContext2D,
  project: (coord: Point) => Point,
  markets: ReadonlyMap<number, MarketLocationDto>,
  flows: readonly EconomyFlowDto[],
  blend: LayerBlend,
  cameraScale = 1,
  nowMs = 0,
): number {
  if (blend.opacity <= 0) return 0;
  let drawn = 0;
  ctx.save();
  ctx.lineCap = 'round';
  for (const flow of flows) {
    const screenWidth = flowStrokeWidth(flow.rate);
    if (screenWidth === 0) continue;
    const src = markets.get(flow.srcMarketId);
    const dst = markets.get(flow.dstMarketId);
    if (!src || !dst) continue;
    const a = project({ x: src.tileX, y: src.tileY });
    const b = project({ x: dst.tileX, y: dst.tileY });
    const c = flowCurveControlPoint(a, b);
    const width = screenStableWorldSize(screenWidth, cameraScale, {
      minWorld: 1.2,
      maxWorld: screenWidth * 4.5,
    });

    // casing first, so crossing lines keep the Mini-Metro paper gap
    ctx.globalAlpha = 0.9 * blend.opacity;
    ctx.strokeStyle = FLOW_CASING;
    ctx.lineWidth = width + screenStableWorldSize(3, cameraScale, { minWorld: 1.2, maxWorld: 12 });
    strokeCurve(ctx, a, c, b);

    ctx.globalAlpha = 0.92 * blend.opacity;
    ctx.strokeStyle = goodColor(flow.goodId);
    ctx.lineWidth = width;
    strokeCurve(ctx, a, c, b);

    drawCargoDots(ctx, a, c, b, flow, blend, width, nowMs);
    drawn += 1;
  }
  ctx.restore();
  return drawn;
}

function strokeCurve(ctx: CanvasRenderingContext2D, a: Point, c: Point, b: Point): void {
  ctx.beginPath();
  ctx.moveTo(a.x, a.y);
  ctx.quadraticCurveTo(c.x, c.y, b.x, b.y);
  ctx.stroke();
}

/** Animated cargo riding src→dst: throughput made visible, direction implicit. */
function drawCargoDots(
  ctx: CanvasRenderingContext2D,
  a: Point,
  c: Point,
  b: Point,
  flow: EconomyFlowDto,
  blend: LayerBlend,
  lineWidth: number,
  nowMs: number,
): void {
  const phases = cargoDotPhases(cargoDotCount(flow.rate), nowMs);
  if (phases.length === 0) return;
  // Mini-Metro trains: capsules oriented along the line, riding src→dst;
  // direction reads from their motion.
  const width = Math.max(2.6, lineWidth * 1.15);
  const length = lineWidth * 2.6;
  ctx.globalAlpha = blend.opacity;
  for (const t of phases) {
    const p = pointOnQuadratic(a, c, b, t);
    const angle = angleOnQuadratic(a, c, b, t);
    drawCapsule(ctx, p, angle, length, width, goodColor(flow.goodId), FLOW_CASING);
  }
}
