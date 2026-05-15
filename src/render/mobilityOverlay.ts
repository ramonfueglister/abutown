import { mobilityMarkers, type MobilityCoord, type MobilityMarker, type MobilityOverlayState } from '../backend/mobilityState';

export type MobilityOverlayDrawItem = {
  id: string;
  kind: MobilityMarker['kind'];
  x: number;
  y: number;
  radius: number;
  color: string;
  label: string;
};

export type MobilityOverlayBuildOptions = {
  project: (coord: MobilityCoord) => { x: number; y: number };
  isVisible?: (coord: MobilityCoord) => boolean;
};

export function buildMobilityOverlayDrawItems(state: MobilityOverlayState, options: MobilityOverlayBuildOptions): MobilityOverlayDrawItem[] {
  return mobilityMarkers(state)
    .filter((marker) => options.isVisible?.(marker.coord) ?? true)
    .map((marker) => {
      const projected = options.project(marker.coord);
      return {
        id: marker.id,
        kind: marker.kind,
        x: projected.x,
        y: projected.y,
        radius: marker.kind === 'agent' ? 4 : marker.kind === 'vehicle' ? 5 : 3,
        color: markerColor(marker),
        label: marker.label,
      };
    });
}

export function drawMobilityOverlay(context: CanvasRenderingContext2D, state: MobilityOverlayState, options: MobilityOverlayBuildOptions): void {
  const items = buildMobilityOverlayDrawItems(state, options);
  if (items.length === 0) return;

  context.save();
  context.lineWidth = 1;
  context.font = '6px sans-serif';
  context.textAlign = 'center';
  context.textBaseline = 'middle';

  for (const item of items) {
    context.globalAlpha = item.kind === 'stop' ? 0.52 : 0.88;
    context.fillStyle = item.color;
    context.strokeStyle = 'rgba(8, 10, 8, 0.72)';
    context.beginPath();
    if (item.kind === 'vehicle') {
      context.rect(item.x - item.radius, item.y - item.radius * 0.75, item.radius * 2, item.radius * 1.5);
    } else {
      context.arc(item.x, item.y, item.radius, 0, Math.PI * 2);
    }
    context.fill();
    context.stroke();

    if (item.kind === 'agent') {
      context.globalAlpha = 0.72;
      context.fillStyle = '#1b1f1b';
      context.fillText('A', item.x, item.y + 0.4);
    }
  }

  context.restore();
}

function markerColor(marker: MobilityMarker): string {
  if (marker.kind === 'agent') {
    if (marker.state === 'in_vehicle') return '#8fd5ff';
    if (marker.state === 'waiting_at_stop') return '#ffdf8a';
    return '#f7d76a';
  }
  if (marker.kind === 'vehicle') return '#6ad7a8';
  return '#d9eef2';
}
