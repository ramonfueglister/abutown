import { describe, expect, it } from 'vitest';
import {
  marketActivity,
  marketNodeRadius,
  priceTrend,
  satisfiedDemandFraction,
  stationShapeForMarket,
  trendTriangle,
} from '../../src/render/drawMarkets';
import type { MarketGoodDto } from '../../src/backend/mobilityProtocol';

const good = (overrides: Partial<MarketGoodDto>): MarketGoodDto => ({
  marketId: 1, goodId: 1, lastSettlementPrice: 0, ewmaReferencePrice: 0,
  tradedQtyLastTick: 0, unmetDemandLastTick: 0, unsoldSupplyLastTick: 0,
  ...overrides,
});

describe('marketActivity', () => {
  it('sums traded qty across goods', () => {
    expect(marketActivity([good({ tradedQtyLastTick: 3 }), good({ tradedQtyLastTick: 7 })])).toBe(10);
  });
});

describe('marketNodeRadius', () => {
  it('has a floor at 8, grows monotonically, clamps at 17', () => {
    expect(marketNodeRadius(0)).toBe(8);
    expect(marketNodeRadius(100)).toBeGreaterThan(marketNodeRadius(10));
    expect(marketNodeRadius(1e12)).toBe(17);
  });
});

describe('stationShapeForMarket', () => {
  it('assigns deterministic Mini-Metro station shapes', () => {
    expect(stationShapeForMarket(9001)).toBe(stationShapeForMarket(9001));
    expect(new Set([9001, 9002, 9003, 9004].map(stationShapeForMarket)).size).toBeGreaterThan(1);
  });
});

describe('satisfiedDemandFraction', () => {
  it('is traded/(traded+unmet)', () => {
    expect(satisfiedDemandFraction([good({ tradedQtyLastTick: 75, unmetDemandLastTick: 25 })])).toBeCloseTo(0.75);
  });
  it('is 1 when there was no demand at all (0/0)', () => {
    expect(satisfiedDemandFraction([good({})])).toBe(1);
    expect(satisfiedDemandFraction([])).toBe(1);
  });
});

describe('priceTrend', () => {
  it('is up/down when settlement deviates >1% from the EWMA reference', () => {
    expect(priceTrend([good({ lastSettlementPrice: 1100, ewmaReferencePrice: 1000 })])).toBe('up');
    expect(priceTrend([good({ lastSettlementPrice: 900, ewmaReferencePrice: 1000 })])).toBe('down');
  });
  it('is flat inside the deadband and for empty/zero-reference data', () => {
    expect(priceTrend([good({ lastSettlementPrice: 1005, ewmaReferencePrice: 1000 })])).toBe('flat');
    expect(priceTrend([good({ lastSettlementPrice: 50, ewmaReferencePrice: 0 })])).toBe('flat');
    expect(priceTrend([])).toBe('flat');
  });
});

describe('trendTriangle', () => {
  it('apex is above the base for up, below for down (canvas y grows downward)', () => {
    const [apexUp, b1Up, b2Up] = trendTriangle({ x: 0, y: 100 }, 10, -1);
    expect(apexUp.y).toBeLessThan(b1Up.y);
    expect(apexUp.y).toBeLessThan(b2Up.y);
    expect(b1Up.y).toBe(b2Up.y);
    const [apexDown, b1Down] = trendTriangle({ x: 0, y: 100 }, 10, 1);
    expect(apexDown.y).toBeGreaterThan(b1Down.y);
    // up-marker sits above the node center, down-marker below
    expect(apexUp.y).toBeLessThan(100);
    expect(apexDown.y).toBeGreaterThan(100);
  });
});
