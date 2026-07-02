// Agent + vehicle glyphs for the schematic renderer (L2).
// Spec: docs/superpowers/specs/2026-06-10-schematic-map-renderer-design.md §1
import type { BackendCar, BackendPedestrian, Coord } from './backendMobilityDrawables';
import type { AgentMobilityDto } from '../backend/mobilityProtocol';
import type { MinimalMapRendererState } from './minimalMapRenderer';
import {
  AGENT_INK,
  SELECTION_HALO_AGENT,
  SELECTION_HALO_VEHICLE,
  STATION_FILL,
  TRADER_RED,
  VEHICLE_COLORS,
} from './designTokens';
import type { LayerBlend } from './layerBlend';
import { drawCapsule } from './canvasPrimitives';
import { carRenderStyle, carVisualWorldPoint, pedestrianRenderStyle } from './entityRenderStyle';
import { stableHash as hash } from './gridMath';
import { mapProject } from './minimalMapProjection';

export type AgentGlyph = { shape: 'dot' | 'ring'; color: string; radiusScale: number };

const iso = (state: MinimalMapRendererState, coord: Coord): Coord => mapProject(coord, state.tileSize);

export function agentGlyph(
  stateType: AgentMobilityDto['state']['type'],
  kind: BackendPedestrian['kind'],
): AgentGlyph {
  if (kind === 'trader') return { shape: 'dot', color: TRADER_RED, radiusScale: 1.5 };
  if (stateType === 'at_activity' || stateType === 'waiting_at_stop') {
    return { shape: 'ring', color: AGENT_INK, radiusScale: 1 };
  }
  return { shape: 'dot', color: AGENT_INK, radiusScale: 1 };
}

export function pedestrianOpacity(kind: BackendPedestrian['kind'], blend: LayerBlend): number {
  if (kind === 'trader') return Math.max(0.95, blend.opacity);
  if (blend.detail === 'individual') return Math.min(0.72, blend.opacity);
  return blend.opacity;
}

export function pedestrianRadiusScale(kind: BackendPedestrian['kind'], blend: LayerBlend): number {
  if (kind === 'trader') return 1.35;
  if (blend.detail === 'individual') return 0.95;
  return 1;
}

export function pedestrianFinalRadius(
  baseRadius: number,
  glyph: AgentGlyph,
  kind: BackendPedestrian['kind'],
  blend: LayerBlend,
): number {
  return baseRadius * glyph.radiusScale * pedestrianRadiusScale(kind, blend);
}

export function traderHaloRadius(finalRadius: number, baseRadius: number): number {
  return finalRadius + Math.max(1, baseRadius * 0.45);
}

export function drawPedestrian(
  state: MinimalMapRendererState,
  pedestrian: BackendPedestrian,
  selected: boolean,
  blend: LayerBlend,
): void {
  const { ctx, camera } = state;
  const current = pedestrian.path[0];
  const next = pedestrian.path[1] ?? current;
  const pos = current;
  const point = iso(state, pos);
  const currentPoint = iso(state, current);
  const nextPoint = iso(state, next);
  const style = pedestrianRenderStyle(currentPoint, nextPoint, camera.scale, pedestrian.laneOffset);
  ctx.save();
  ctx.translate(point.x + style.lane.x, point.y + style.lane.y);
  if (selected) {
    ctx.globalAlpha = 0.92;
    ctx.strokeStyle = SELECTION_HALO_AGENT;
    ctx.lineWidth = 2 / Math.max(0.75, camera.scale);
    ctx.beginPath();
    ctx.ellipse(0, 0, style.selectedRadius, style.selectedRadius, 0, 0, Math.PI * 2);
    ctx.stroke();
  }
  const resolved = agentGlyph(pedestrian.stateType, pedestrian.kind);
  const glyph = blend.detail === 'aggregate' ? { ...resolved, shape: 'dot' as const } : resolved;
  const radiusScale = pedestrianRadiusScale(pedestrian.kind, blend);
  const finalRadius = style.radius * glyph.radiusScale * radiusScale;
  ctx.globalAlpha *= pedestrianOpacity(pedestrian.kind, blend);
  if (pedestrian.kind === 'trader') {
    ctx.strokeStyle = STATION_FILL;
    ctx.lineWidth = Math.max(1, style.radius * 0.55);
    ctx.beginPath();
    ctx.arc(0, 0, traderHaloRadius(finalRadius, style.radius), 0, Math.PI * 2);
    ctx.stroke();
  }
  if (glyph.shape === 'ring') {
    ctx.strokeStyle = glyph.color;
    ctx.lineWidth = Math.max(1.2, style.radius * 0.45);
    ctx.beginPath();
    ctx.arc(0, 0, finalRadius, 0, Math.PI * 2);
    ctx.stroke();
  } else {
    ctx.fillStyle = glyph.color;
    ctx.beginPath();
    ctx.arc(0, 0, finalRadius, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.restore();
}

export function drawCar(state: MinimalMapRendererState, car: BackendCar, selected: boolean, blend: LayerBlend): void {
  if (blend.opacity <= 0) return;
  const { ctx, camera, tileSize } = state;
  const point = carVisualWorldPoint(car, camera.scale, tileSize);
  const currentPoint = iso(state, car.path[0]);
  const nextPoint = iso(state, car.path[1] ?? car.path[0]);
  const style = carRenderStyle(currentPoint, nextPoint, camera.scale);
  ctx.save();
  ctx.globalAlpha *= blend.opacity;
  ctx.translate(point.x, point.y);
  if (selected) {
    ctx.globalAlpha = 0.94;
    ctx.strokeStyle = SELECTION_HALO_VEHICLE;
    ctx.lineWidth = 2 / Math.max(0.75, camera.scale);
    ctx.beginPath();
    ctx.ellipse(0, 0, style.selection.x, style.selection.y, 0, 0, Math.PI * 2);
    ctx.stroke();
  }
  drawCapsule(ctx, { x: 0, y: 0 }, style.angle, style.capsule.length, style.capsule.width, vehicleVectorColor(car.id));
  ctx.restore();
}

function vehicleVectorColor(id: string): string {
  return VEHICLE_COLORS[hash(`vehicle-color:${id}`) % VEHICLE_COLORS.length];
}
