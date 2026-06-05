import { describe, expect, it } from 'vitest';
import { create } from '@bufbuild/protobuf';
import {
  EconomyMarketGoodSchema,
  EconomyMarketSchema,
  EconomySnapshotSchema,
} from './proto/abutown_pb';
import { economySnapshotFromProto } from './mobilityProtocol';

describe('economySnapshotFromProto', () => {
  it('converts an EconomySnapshot proto to a plain DTO', () => {
    const proto = create(EconomySnapshotSchema, {
      tick: 42n,
      markets: [
        create(EconomyMarketSchema, {
          marketId: 9001,
          name: 'Demo A',
          tileX: 2,
          tileY: 3,
          wagePaidLastTick: 320n,
        }),
      ],
      goods: [
        create(EconomyMarketGoodSchema, {
          marketId: 9002,
          goodId: 4,
          lastSettlementPrice: 5000n,
          ewmaReferencePrice: 5100n,
          tradedQtyLastTick: 10n,
          unmetDemandLastTick: 0n,
          unsoldSupplyLastTick: 0n,
        }),
      ],
    });

    const dto = economySnapshotFromProto(proto);

    // tick: bigint → number
    expect(dto.tick).toBe(42);
    expect(typeof dto.tick).toBe('number');

    // markets
    expect(dto.markets).toHaveLength(1);
    const market = dto.markets[0];
    expect(market.marketId).toBe(9001);
    expect(market.name).toBe('Demo A');
    expect(market.tileX).toBe(2);
    expect(market.tileY).toBe(3);
    // wagePaidLastTick: bigint → number
    expect(market.wagePaidLastTick).toBe(320);
    expect(typeof market.wagePaidLastTick).toBe('number');

    // goods
    expect(dto.goods).toHaveLength(1);
    const good = dto.goods[0];
    expect(good.marketId).toBe(9002);
    expect(good.goodId).toBe(4);
    // all int64 fields: bigint → number
    expect(good.lastSettlementPrice).toBe(5000);
    expect(typeof good.lastSettlementPrice).toBe('number');
    expect(good.ewmaReferencePrice).toBe(5100);
    expect(typeof good.ewmaReferencePrice).toBe('number');
    expect(good.tradedQtyLastTick).toBe(10);
    expect(typeof good.tradedQtyLastTick).toBe('number');
    expect(good.unmetDemandLastTick).toBe(0);
    expect(typeof good.unmetDemandLastTick).toBe('number');
    expect(good.unsoldSupplyLastTick).toBe(0);
    expect(typeof good.unsoldSupplyLastTick).toBe('number');
  });

  it('converts an empty EconomySnapshot (no markets or goods)', () => {
    const proto = create(EconomySnapshotSchema, { tick: 0n });
    const dto = economySnapshotFromProto(proto);
    expect(dto.tick).toBe(0);
    expect(dto.markets).toEqual([]);
    expect(dto.goods).toEqual([]);
  });
});
