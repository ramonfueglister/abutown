import { afterEach, describe, expect, it, vi } from 'vitest';
import { goodPriceChips, setVitalsHud } from '../../src/app/vitalsHud';
import type { MarketGoodDto } from '../../src/backend/mobilityProtocol';

type FakeHudElement = {
  tagName: string;
  className: string;
  textContent: string;
  innerHTML: string;
  setAttribute: (name: string, value: string) => void;
  getAttribute: (name: string) => string | null;
  remove: () => void;
  dataset: Record<string, string>;
};

function installFakeDom(existingHud = false) {
  let hud: FakeHudElement | null = null;

  if (existingHud) {
    hud = makeFakeHudElement(() => {
      hud = null;
    });
  }

  function makeFakeHudElement(onRemove: () => void): FakeHudElement {
    const attrs: Record<string, string> = {};
    const el: FakeHudElement = {
      tagName: 'DIV',
      className: '',
      textContent: '',
      innerHTML: '',
      dataset: {},
      setAttribute: vi.fn((name: string, value: string) => { attrs[name] = value; }),
      getAttribute: vi.fn((name: string) => attrs[name] ?? null),
      remove: vi.fn(onRemove),
    };
    return el;
  }

  const doc = {
    body: {
      appendChild: vi.fn((element: FakeHudElement) => {
        hud = element;
      }),
    },
    createElement: vi.fn(() => makeFakeHudElement(() => { hud = null; })),
    querySelector: vi.fn((_selector: string) => hud),
  } as unknown as Document;

  return { doc, getHud: () => hud };
}

afterEach(() => {
  vi.restoreAllMocks();
});

const good = (overrides: Partial<MarketGoodDto>): MarketGoodDto => ({
  marketId: 1, goodId: 1, lastSettlementPrice: 0, ewmaReferencePrice: 0,
  tradedQtyLastTick: 0, unmetDemandLastTick: 0, unsoldSupplyLastTick: 0,
  ...overrides,
});

describe('goodPriceChips', () => {
  it('averages settlement prices across markets per good, display-scaled', () => {
    const chips = goodPriceChips([
      good({ marketId: 1, goodId: 1, lastSettlementPrice: 1000, ewmaReferencePrice: 1000 }),
      good({ marketId: 2, goodId: 1, lastSettlementPrice: 3000, ewmaReferencePrice: 3000 }),
    ]);
    expect(chips).toHaveLength(1);
    expect(chips[0].price).toBeCloseTo(2);
    expect(chips[0].label).toBe('Food');
    expect(chips[0].trend).toBe('flat');
  });

  it('reports an EWMA-relative trend and sorts by good id', () => {
    const chips = goodPriceChips([
      good({ goodId: 4, lastSettlementPrice: 1100, ewmaReferencePrice: 1000 }),
      good({ goodId: 1, lastSettlementPrice: 900, ewmaReferencePrice: 1000 }),
    ]);
    expect(chips.map((c) => c.goodId)).toEqual([1, 4]);
    expect(chips[0].trend).toBe('down');
    expect(chips[1].trend).toBe('up');
  });

  it('skips goods that have never settled', () => {
    expect(goodPriceChips([good({ lastSettlementPrice: 0 })])).toHaveLength(0);
  });
});

describe('setVitalsHud', () => {
  it('renders population, routed, money, and price chips; idempotent single element', () => {
    const { doc, getHud } = installFakeDom(false);
    const vitals = { population: 348, routedCitizens: 13, totalMoney: 3_000_000, routesAssigned: 5, routesFailed: 1 };
    const goods = [good({ goodId: 1, lastSettlementPrice: 1570, ewmaReferencePrice: 1570 })];
    setVitalsHud(doc, vitals, goods);
    setVitalsHud(doc, vitals, goods);
    const hud = getHud();
    expect(hud).not.toBeNull();
    expect((doc.body.appendChild as ReturnType<typeof vi.fn>).mock.calls).toHaveLength(1);
    expect(hud?.innerHTML).toContain('348');
    expect(hud?.innerHTML).toContain('routed');
    expect(hud?.innerHTML).toContain('3000.00');
    expect(hud?.innerHTML).toContain('Food 1.57');
  });

  it('removes the HUD when vitals are undefined', () => {
    const { doc, getHud } = installFakeDom(true);
    expect(getHud()).not.toBeNull();
    setVitalsHud(doc, undefined);
    expect(getHud()).toBeNull();
  });
});
