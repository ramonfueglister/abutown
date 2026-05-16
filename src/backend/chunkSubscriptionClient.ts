import { encodeClientMessage, type ChunkCoordDto } from './mobilityProtocol';

export function computeInitialSubscriptionCoords(opts: {
  worldWidthTiles: number;
  worldHeightTiles: number;
  chunkSize: number;
}): ChunkCoordDto[] {
  const cs = opts.chunkSize;
  const cols = Math.ceil(opts.worldWidthTiles / cs);
  const rows = Math.ceil(opts.worldHeightTiles / cs);
  const out: ChunkCoordDto[] = [];
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < cols; x++) {
      out.push({ x, y });
    }
  }
  return out;
}

export type SubscriptionClient = {
  start(): void;
};

export function createSubscriptionClient(opts: {
  send: (text: string) => void;
  worldWidthTiles: number;
  worldHeightTiles: number;
  chunkSize: number;
}): SubscriptionClient {
  const initialCoords = computeInitialSubscriptionCoords(opts);
  return {
    start() {
      opts.send(encodeClientMessage({
        type: 'chunk_subscribe',
        protocol_version: 1,
        coords: initialCoords,
      }));
    },
  };
}
