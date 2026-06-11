/**
 * Read-only sim vitals HUD (bottom-left). Idempotent like the persistence
 * banner: re-render updates the single element in place.
 */
import type { EconomyVitalsDto, MarketGoodDto } from '../backend/mobilityProtocol';
import { GOOD_COLORS, GOOD_COLOR_FALLBACK } from '../render/designTokens';

// MONEY_DISPLAY_SCALE is also used in inspectorPanelPainter.ts as a local
// constant; there is no shared module for it yet so we define it here too.
const MONEY_DISPLAY_SCALE = 1000;

const GOOD_LABELS: Readonly<Record<number, string>> = {
  1: 'Food',
  2: 'Wood',
  3: 'Iron',
  4: 'Tools',
  5: 'Raw',
};

export type GoodPriceChip = {
  goodId: number;
  label: string;
  color: string;
  price: number; // display-scaled mean settlement price across markets
  trend: 'up' | 'down' | 'flat';
};

const CHIP_TREND_DEADBAND = 0.01;

/** Cross-market mean settlement price per good, with an EWMA-relative trend. */
export function goodPriceChips(goods: readonly MarketGoodDto[]): GoodPriceChip[] {
  const byGood = new Map<number, { last: number[]; ewma: number[] }>();
  for (const g of goods) {
    if (g.lastSettlementPrice <= 0) continue;
    const bucket = byGood.get(g.goodId) ?? { last: [], ewma: [] };
    bucket.last.push(g.lastSettlementPrice);
    bucket.ewma.push(g.ewmaReferencePrice);
    byGood.set(g.goodId, bucket);
  }
  const mean = (xs: number[]) => xs.reduce((a, b) => a + b, 0) / xs.length;
  return [...byGood.entries()]
    .sort(([a], [b]) => a - b)
    .map(([goodId, bucket]) => {
      const last = mean(bucket.last);
      const ewma = mean(bucket.ewma);
      let trend: GoodPriceChip['trend'] = 'flat';
      if (ewma > 0) {
        const deviation = (last - ewma) / ewma;
        if (deviation > CHIP_TREND_DEADBAND) trend = 'up';
        else if (deviation < -CHIP_TREND_DEADBAND) trend = 'down';
      }
      return {
        goodId,
        label: GOOD_LABELS[goodId] ?? `Good ${goodId}`,
        color: GOOD_COLORS[goodId] ?? GOOD_COLOR_FALLBACK,
        price: last / MONEY_DISPLAY_SCALE,
        trend,
      };
    });
}

const TREND_GLYPH: Record<GoodPriceChip['trend'], string> = { up: '▲', down: '▼', flat: '' };

export function setVitalsHud(
  doc: Document,
  vitals: EconomyVitalsDto | undefined,
  goods: readonly MarketGoodDto[] = [],
): void {
  const existing = doc.querySelector('[data-vitals-hud]');
  if (!vitals) {
    existing?.remove();
    return;
  }
  const el = (existing as HTMLElement) ?? doc.createElement('div');
  el.setAttribute('data-vitals-hud', 'true');
  el.className = 'vitals-hud';
  const chips = goodPriceChips(goods)
    .map(
      (chip) =>
        `<span class="vitals-chip"><span class="vitals-chip-dot" style="background:${chip.color}"></span>` +
        `${chip.label} ${chip.price.toFixed(2)}` +
        `<span class="vitals-chip-trend vitals-chip-trend-${chip.trend}">${TREND_GLYPH[chip.trend]}</span></span>`,
    )
    .join('');
  el.innerHTML =
    `<div class="vitals-row">` +
    `<span class="vitals-stat"><span class="vitals-key">pop</span> ${vitals.population}</span>` +
    `<span class="vitals-stat"><span class="vitals-key">routed</span> ${vitals.routedCitizens}</span>` +
    `<span class="vitals-stat"><span class="vitals-key">money</span> ${(vitals.totalMoney / MONEY_DISPLAY_SCALE).toFixed(2)}</span>` +
    `<span class="vitals-stat"><span class="vitals-key">routes</span> ${vitals.routesAssigned}✓ ${vitals.routesFailed}✗</span>` +
    `</div>` +
    (chips ? `<div class="vitals-row vitals-chips">${chips}</div>` : '');
  if (!existing) doc.body.appendChild(el);
}
