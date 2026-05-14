import type { BackendOverlayState, BackendPulse } from '../backend/backendState';
import type { Coord } from '../city/worldTypes';

export const BACKEND_PULSE_LIFETIME_MS = 1400;

type ProjectIso = (coord: Coord) => Coord;

export function localIndexToWorldCoord(chunk: Coord, chunkSize: number, localIndex: number): Coord {
  return {
    x: chunk.x * chunkSize + (localIndex % chunkSize),
    y: chunk.y * chunkSize + Math.floor(localIndex / chunkSize),
  };
}

export function activeBackendPulses(pulses: readonly BackendPulse[], nowMs: number): BackendPulse[] {
  return pulses.filter((pulse) => nowMs - pulse.receivedAtMs < BACKEND_PULSE_LIFETIME_MS);
}

export function drawBackendWorldOverlay(
  context: CanvasRenderingContext2D,
  state: BackendOverlayState,
  projectIso: ProjectIso,
  tileWidth: number,
  tileHeight: number,
  nowMs: number,
): void {
  if (!state.loadedChunk || !state.chunkSize) return;

  drawChunkOutline(context, state, projectIso, tileWidth);
  drawPulseMarkers(context, state, projectIso, tileHeight, nowMs);
}

export function drawBackendStatusBadge(
  context: CanvasRenderingContext2D,
  state: BackendOverlayState,
  viewport: { width: number; height: number },
): void {
  const x = 14;
  const y = Math.max(14, viewport.height - 86);
  const statusColor = state.status === 'live' ? '#7df2b2' : state.status === 'snapshot' ? '#f3d37a' : '#ff8f8f';
  const lines = [
    `RUST ${state.status.toUpperCase()}`,
    state.worldId ? `world ${state.worldId}` : 'world offline',
    state.loadedChunk ? `chunk ${state.loadedChunk.coord.x}:${state.loadedChunk.coord.y} ${state.loadedChunk.state}` : 'chunk none',
    `tick ${state.latestTick ?? '-'} v${state.latestVersion ?? '-'}`,
  ];

  context.save();
  context.font = '12px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
  context.textBaseline = 'top';
  context.fillStyle = 'rgba(4, 12, 9, 0.78)';
  roundRect(context, x, y, 186, 64, 5);
  context.fill();
  context.strokeStyle = statusColor;
  context.lineWidth = 1;
  context.stroke();

  lines.forEach((line, index) => {
    context.fillStyle = index === 0 ? statusColor : 'rgba(222, 255, 235, 0.9)';
    context.fillText(line, x + 10, y + 8 + index * 14);
  });
  context.restore();
}

function drawChunkOutline(
  context: CanvasRenderingContext2D,
  state: BackendOverlayState,
  projectIso: ProjectIso,
  tileWidth: number,
): void {
  const chunk = state.loadedChunk;
  const chunkSize = state.chunkSize;
  if (!chunk || !chunkSize) return;

  const start = { x: chunk.coord.x * chunkSize, y: chunk.coord.y * chunkSize };
  const points = [
    projectIso(start),
    projectIso({ x: start.x + chunkSize, y: start.y }),
    projectIso({ x: start.x + chunkSize, y: start.y + chunkSize }),
    projectIso({ x: start.x, y: start.y + chunkSize }),
  ];

  context.save();
  context.strokeStyle = state.status === 'live' ? 'rgba(125, 242, 178, 0.85)' : 'rgba(243, 211, 122, 0.72)';
  context.lineWidth = Math.max(2, tileWidth / 18);
  context.setLineDash([10, 8]);
  context.beginPath();
  context.moveTo(points[0].x, points[0].y);
  for (const point of points.slice(1)) context.lineTo(point.x, point.y);
  context.closePath();
  context.stroke();
  context.restore();
}

function drawPulseMarkers(
  context: CanvasRenderingContext2D,
  state: BackendOverlayState,
  projectIso: ProjectIso,
  tileHeight: number,
  nowMs: number,
): void {
  const chunkSize = state.chunkSize;
  if (!chunkSize) return;

  for (const pulse of activeBackendPulses(state.pulses, nowMs)) {
    const age = nowMs - pulse.receivedAtMs;
    const t = Math.max(0, Math.min(1, age / BACKEND_PULSE_LIFETIME_MS));
    const coord = localIndexToWorldCoord(pulse.coord, chunkSize, pulse.localIndex);
    const point = projectIso(coord);
    const radius = 9 + t * 26;
    const alpha = 1 - t;

    context.save();
    context.globalAlpha = alpha;
    context.strokeStyle = '#ffd166';
    context.lineWidth = 3;
    context.beginPath();
    context.ellipse(point.x, point.y - tileHeight * 0.4, radius, radius * 0.48, 0, 0, Math.PI * 2);
    context.stroke();
    context.fillStyle = 'rgba(255, 209, 102, 0.28)';
    context.beginPath();
    context.arc(point.x, point.y - tileHeight * 0.4, 4, 0, Math.PI * 2);
    context.fill();
    context.restore();
  }
}

function roundRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
): void {
  context.beginPath();
  context.moveTo(x + radius, y);
  context.lineTo(x + width - radius, y);
  context.quadraticCurveTo(x + width, y, x + width, y + radius);
  context.lineTo(x + width, y + height - radius);
  context.quadraticCurveTo(x + width, y + height, x + width - radius, y + height);
  context.lineTo(x + radius, y + height);
  context.quadraticCurveTo(x, y + height, x, y + height - radius);
  context.lineTo(x, y + radius);
  context.quadraticCurveTo(x, y, x + radius, y);
  context.closePath();
}
