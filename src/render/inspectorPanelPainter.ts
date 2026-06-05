import type { EntityInspector } from './entityInspector';
import { roundedRectPath } from './canvasPrimitives';
import type { MarketLocationDto, MarketGoodDto } from '../backend/mobilityProtocol';

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

export const MARKET_INSPECTOR_PANEL: InspectorPanelTheme = {
  x: 12,
  y: 244,
  accent: '#f0a85a',
  stroke: 'rgba(240,168,90,0.8)',
};

/** Divisor for converting internal money units (integer * 1000) to display values. */
export const MONEY_DISPLAY_SCALE = 1000;

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

/** Maps well-known economy good IDs to human-readable labels. */
const GOOD_LABELS: Readonly<Record<number, string>> = {
  1: 'FOOD',
  4: 'TOOLS',
  5: 'RAW',
};

function goodLabel(goodId: number): string {
  return GOOD_LABELS[goodId] ?? `good ${goodId}`;
}

function formatMoney(raw: number): string {
  return (raw / MONEY_DISPLAY_SCALE).toFixed(2);
}

/**
 * Pure formatter: returns the market panel title as the first element, then one
 * row string per good, then a wages line.
 *
 * Row format: "<GOOD>  p=<settlement/MONEY_DISPLAY_SCALE>  short=<unmet>  glut=<unsold>"
 * Wages line: "wages=<wagePaidLastTick/MONEY_DISPLAY_SCALE>"
 */
export function marketInspectorRows(market: MarketLocationDto, goods: MarketGoodDto[]): string[] {
  const rows: string[] = [market.name];
  for (const g of goods) {
    rows.push(
      `${goodLabel(g.goodId)}  p=${formatMoney(g.lastSettlementPrice)}  short=${g.unmetDemandLastTick}  glut=${g.unsoldSupplyLastTick}`,
    );
  }
  rows.push(`wages=${formatMoney(market.wagePaidLastTick)}`);
  return rows;
}

/**
 * Draws the read-only market inspector panel using the existing HUD idiom
 * (setTransform pixelRatio + inspectorPanelLayout).
 */
export function drawMarketInspectorPanel(
  ctx: CanvasRenderingContext2D,
  market: MarketLocationDto,
  goods: MarketGoodDto[],
  theme: InspectorPanelTheme,
  pixelRatio: number,
): void {
  const rows = marketInspectorRows(market, goods);
  // rows[0] is the title; rows[1..] are content rows (no label/value split — single string per row).
  const [title, ...contentRows] = rows;
  const inspector: NonNullable<EntityInspector> = {
    title: title ?? market.name,
    rows: contentRows.map((row) => ({ label: row, value: '' })),
  };
  drawInspectorPanel(ctx, inspector, theme, pixelRatio);
}
