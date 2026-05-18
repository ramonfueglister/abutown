// src/backend/chunkSubscriptionClient.ts
import { encodeClientMessage, type ChunkCoordDto } from './mobilityProtocol';

export type SubscriptionClient = {
  update(visible: ChunkCoordDto[]): void;
  reset(): void;
};

function key(coord: ChunkCoordDto): string {
  return `${coord.x},${coord.y}`;
}

export function createSubscriptionClient(opts: {
  send: (text: string) => void;
}): SubscriptionClient {
  let current = new Map<string, ChunkCoordDto>();

  return {
    update(visible) {
      const next = new Map(visible.map((coord) => [key(coord), coord]));
      const added: ChunkCoordDto[] = [];
      const removed: ChunkCoordDto[] = [];
      for (const [k, coord] of next) {
        if (!current.has(k)) added.push(coord);
      }
      for (const [k, coord] of current) {
        if (!next.has(k)) removed.push(coord);
      }
      if (added.length > 0) {
        opts.send(encodeClientMessage({
          type: 'chunk_subscribe',
          protocol_version: 1,
          coords: added,
        }));
      }
      if (removed.length > 0) {
        opts.send(encodeClientMessage({
          type: 'chunk_unsubscribe',
          protocol_version: 1,
          coords: removed,
        }));
      }
      current = next;
    },
    reset() {
      current = new Map();
    },
  };
}
