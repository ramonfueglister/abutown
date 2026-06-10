import { describe, expect, it } from 'vitest';
import { create } from '@bufbuild/protobuf';
import {
  EconomyMarketGoodSchema,
  EconomyMarketSchema,
  EconomySnapshotSchema,
  HelloSchema,
  ServerMessageSchema,
} from './proto/abutown_pb';
import {
  applyEconomyServerMessage,
  createEconomyOverlayState,
} from './economyState';

describe('economyState reducer', () => {
  it('applying an economySnapshot populates markets and goods and sets tick', () => {
    const market = create(EconomyMarketSchema, {
      marketId: 7,
      name: 'Downtown Market',
      tileX: 10,
      tileY: 20,
      wagePaidLastTick: 42n,
    });
    const good = create(EconomyMarketGoodSchema, {
      marketId: 7,
      goodId: 3,
      lastSettlementPrice: 100n,
      ewmaReferencePrice: 99n,
      tradedQtyLastTick: 5n,
      unmetDemandLastTick: 1n,
      unsoldSupplyLastTick: 0n,
    });
    const snapshot = create(EconomySnapshotSchema, {
      tick: 1234n,
      markets: [market],
      goods: [good],
    });
    const msg = create(ServerMessageSchema, {
      body: { case: 'economySnapshot', value: snapshot },
    });

    const initial = createEconomyOverlayState();
    const next = applyEconomyServerMessage(initial, msg);

    expect(next.tick).toBe(1234);
    expect(next.markets.size).toBe(1);
    expect(next.markets.get(7)).toMatchObject({ marketId: 7, name: 'Downtown Market', tileX: 10, tileY: 20, wagePaidLastTick: 42 });
    expect(next.goods.size).toBe(1);
    expect(next.goods.get('7:3')).toMatchObject({ marketId: 7, goodId: 3, lastSettlementPrice: 100, tradedQtyLastTick: 5 });
  });

  it('applying a non-economy message (hello) returns the exact same state reference', () => {
    const hello = create(HelloSchema, {
      protocolVersion: 16,
      worldId: 'abutopia',
      chunkSize: 32,
    });
    const msg = create(ServerMessageSchema, {
      body: { case: 'hello', value: hello },
    });

    const initial = createEconomyOverlayState();
    const next = applyEconomyServerMessage(initial, msg);

    expect(next).toBe(initial); // same reference — no allocation
  });
});
