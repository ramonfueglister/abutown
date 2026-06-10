import type { ServerMessage } from './proto/abutown_pb';
import {
  economySnapshotFromProto,
  type EconomyVitalsDto,
  type MarketLocationDto,
  type MarketGoodDto,
  type EconomyFlowDto,
} from './mobilityProtocol';

export type EconomyOverlayState = {
  tick: number;
  markets: Map<number, MarketLocationDto>; // by marketId
  goods: Map<string, MarketGoodDto>; // key `${marketId}:${goodId}`
  vitals?: EconomyVitalsDto;
  flows: EconomyFlowDto[];
};

export function createEconomyOverlayState(): EconomyOverlayState {
  return { tick: 0, markets: new Map(), goods: new Map(), flows: [] };
}

export function applyEconomyServerMessage(
  state: EconomyOverlayState,
  message: ServerMessage,
): EconomyOverlayState {
  if (message.body.case !== 'economySnapshot') return state;
  const dto = economySnapshotFromProto(message.body.value);
  const markets = new Map(dto.markets.map((m) => [m.marketId, m]));
  const goods = new Map(dto.goods.map((g) => [`${g.marketId}:${g.goodId}`, g]));
  return { tick: dto.tick, markets, goods, vitals: dto.vitals, flows: dto.flows };
}
