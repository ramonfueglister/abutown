import type { EntityInspector } from './entityInspector';
import { roundedRectPath } from './canvasPrimitives';

export type InspectorPanelTheme = {
  x: number;
  y: number;
  accent: string;
  stroke: string;
};

export type InspectorPanelLayout = {
  x: number;
  y: number;
  width: number;
  height: number;
  radius: number;
  padding: number;
  title: { x: number; y: number };
  rows: Array<{ label: string; value: string; labelX: number; valueX: number; y: number }>;
};

export const AGENT_INSPECTOR_PANEL: InspectorPanelTheme = {
  x: 12,
  y: 12,
  accent: '#f7d76a',
  stroke: 'rgba(247, 215, 106, 0.8)',
};

export const VEHICLE_INSPECTOR_PANEL: InspectorPanelTheme = {
  x: 12,
  y: 128,
  accent: '#75d7ff',
  stroke: 'rgba(117, 215, 255, 0.8)',
};

type InspectorPanelContent = NonNullable<EntityInspector>;

export function inspectorPanelLayout(inspector: InspectorPanelContent, theme: InspectorPanelTheme): InspectorPanelLayout {
  const width = 232;
  const padding = 10;
  const rowHeight = 17;
  const titleHeight = 20;
  return {
    x: theme.x,
    y: theme.y,
    width,
    height: padding * 2 + titleHeight + inspector.rows.length * rowHeight,
    radius: 6,
    padding,
    title: { x: theme.x + padding, y: theme.y + padding },
    rows: inspector.rows.map((row, index) => ({
      label: row.label,
      value: row.value,
      labelX: theme.x + padding,
      valueX: theme.x + 70,
      y: theme.y + padding + titleHeight + index * rowHeight,
    })),
  };
}

export function drawInspectorPanel(
  ctx: CanvasRenderingContext2D,
  inspector: EntityInspector,
  theme: InspectorPanelTheme,
  pixelRatio: number,
): void {
  if (!inspector) return;
  const layout = inspectorPanelLayout(inspector, theme);

  ctx.save();
  ctx.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
  ctx.fillStyle = 'rgba(7, 10, 9, 0.82)';
  ctx.strokeStyle = theme.stroke;
  ctx.lineWidth = 1;
  roundedRectPath(ctx, layout.x, layout.y, layout.width, layout.height, layout.radius);
  ctx.fill();
  ctx.stroke();

  ctx.font = '600 12px system-ui, -apple-system, BlinkMacSystemFont, sans-serif';
  ctx.fillStyle = theme.accent;
  ctx.textBaseline = 'top';
  ctx.fillText(inspector.title, layout.title.x, layout.title.y);

  ctx.font = '11px system-ui, -apple-system, BlinkMacSystemFont, sans-serif';
  for (const row of layout.rows) {
    ctx.fillStyle = 'rgba(231, 236, 224, 0.72)';
    ctx.fillText(row.label, row.labelX, row.y);
    ctx.fillStyle = '#f7f7e8';
    ctx.fillText(row.value, row.valueX, row.y);
  }
  ctx.restore();
}
