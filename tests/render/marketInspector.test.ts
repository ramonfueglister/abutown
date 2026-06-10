import { describe, expect, it } from 'vitest';
import { marketInspectorRows, MONEY_DISPLAY_SCALE } from '../../src/render/inspectorPanelPainter';
import type { MarketLocationDto, MarketGoodDto, EconomyProducerDto } from '../../src/backend/mobilityProtocol';

function makeMarket(overrides?: Partial<MarketLocationDto>): MarketLocationDto {
  return {
    marketId: 9001,
    name: 'Demo A',
    tileX: 2,
    tileY: 3,
    wagePaidLastTick: 3000,
    ...overrides,
  };
}

function makeGood(overrides?: Partial<MarketGoodDto>): MarketGoodDto {
  return {
    marketId: 9001,
    goodId: 4,
    lastSettlementPrice: 5000,
    ewmaReferencePrice: 5100,
    tradedQtyLastTick: 10,
    unmetDemandLastTick: 2,
    unsoldSupplyLastTick: 0,
    ...overrides,
  };
}

function makeProducer(overrides?: Partial<EconomyProducerDto>): EconomyProducerDto {
  return {
    actorId: 8031,
    marketId: 9001,
    inGood: 2,
    outGood: 4,
    retainedEarnings: 30_000_000,
    wcTarget: 240,
    maxBid: 400,
    inQty: 10,
    outQty: 10,
    ...overrides,
  };
}

describe('marketInspectorRows', () => {
  it('MONEY_DISPLAY_SCALE is 1000', () => {
    expect(MONEY_DISPLAY_SCALE).toBe(1000);
  });

  it('first element is the market name (title)', () => {
    const rows = marketInspectorRows(makeMarket({ name: 'Demo A' }), [], []);
    expect(rows[0]).toBe('Demo A');
  });

  it('divides settlement price by MONEY_DISPLAY_SCALE: 5000 → "5.00"', () => {
    const rows = marketInspectorRows(makeMarket(), [makeGood({ lastSettlementPrice: 5000 })], []);
    const goodRow = rows[1];
    expect(goodRow).toContain('p=5.00');
  });

  it('formats one row per good with correct field names', () => {
    const rows = marketInspectorRows(makeMarket(), [
      makeGood({ goodId: 4, lastSettlementPrice: 1000, unmetDemandLastTick: 3, unsoldSupplyLastTick: 7 }),
    ], []);
    // row index 0 is title, index 1 is the good row
    expect(rows[1]).toBe('TOOLS  p=1.00  short=3  glut=7');
  });

  it('labels known good IDs: 1→FOOD, 4→TOOLS, 5→RAW', () => {
    const rows = marketInspectorRows(makeMarket(), [
      makeGood({ goodId: 1, lastSettlementPrice: 1000, unmetDemandLastTick: 0, unsoldSupplyLastTick: 0 }),
      makeGood({ goodId: 4, lastSettlementPrice: 2000, unmetDemandLastTick: 0, unsoldSupplyLastTick: 0 }),
      makeGood({ goodId: 5, lastSettlementPrice: 500, unmetDemandLastTick: 0, unsoldSupplyLastTick: 0 }),
    ], []);
    expect(rows[1]).toContain('FOOD');
    expect(rows[2]).toContain('TOOLS');
    expect(rows[3]).toContain('RAW');
  });

  it('falls back to "good <id>" for unknown good IDs', () => {
    const rows = marketInspectorRows(makeMarket(), [
      makeGood({ goodId: 99, lastSettlementPrice: 1000, unmetDemandLastTick: 0, unsoldSupplyLastTick: 0 }),
    ], []);
    expect(rows[1]).toContain('good 99');
  });

  it('appends a wages line at the end dividing by MONEY_DISPLAY_SCALE', () => {
    const rows = marketInspectorRows(makeMarket({ wagePaidLastTick: 3000 }), [], []);
    const wagesLine = rows[rows.length - 1];
    expect(wagesLine).toBe('wages=3.00');
  });

  it('returns title + goods rows + wages for multiple goods', () => {
    const goods = [
      makeGood({ goodId: 1, lastSettlementPrice: 1000, unmetDemandLastTick: 0, unsoldSupplyLastTick: 0 }),
      makeGood({ goodId: 4, lastSettlementPrice: 2000, unmetDemandLastTick: 1, unsoldSupplyLastTick: 2 }),
    ];
    const rows = marketInspectorRows(makeMarket({ name: 'Test Market', wagePaidLastTick: 500 }), goods, []);
    // title + 2 good rows + 1 wages line = 4 total
    expect(rows).toHaveLength(4);
    expect(rows[0]).toBe('Test Market');
    expect(rows[3]).toBe('wages=0.50');
  });

  it('zero wages formats as "wages=0.00"', () => {
    const rows = marketInspectorRows(makeMarket({ wagePaidLastTick: 0 }), [], []);
    expect(rows[rows.length - 1]).toBe('wages=0.00');
  });

  it('renders three producer rows between the goods and the wages line', () => {
    const rows = marketInspectorRows(makeMarket(), [makeGood()], [makeProducer()]);
    // title, good, recipe, cash/target, max bid, wages
    expect(rows).toHaveLength(6);
    expect(rows[2]).toBe('recipe: 10 WOOD → 10 TOOLS');
    expect(rows[3]).toBe('cash/target=30000.00/0.24');
    expect(rows[4]).toBe('max bid=0.40');
    expect(rows[5]).toBe('wages=3.00');
  });

  it('an unpriced producer pool shows zero target and zero bid', () => {
    const rows = marketInspectorRows(makeMarket(), [], [makeProducer({ wcTarget: 0, maxBid: 0 })]);
    expect(rows[2]).toBe('cash/target=30000.00/0.00');
    expect(rows[3]).toBe('max bid=0.00');
  });
});
