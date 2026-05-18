// src/backend/chunkSubscriptionClient.ts
import { encodeClientMessage, type ChunkCoordDto } from './mobilityProtocol';

export type SubscriptionClient = {
  update(visible: ChunkCoordDto[]): void;
  reset(): void;
};

function key(coord: ChunkCoordDto): string {
  return `${coord.x},${coord.y}`;
}

function unkey(k: string): ChunkCoordDto {
  const [x, y] = k.split(',').map((s) => Number.parseInt(s, 10));
  return { x, y };
}

export function createSubscriptionClient(opts: {
  send: (text: string) => void;
}): SubscriptionClient {
  let current = new Set<string>();

  return {
    update(visible) {
      const next = new Set(visible.map(key));
      const added: ChunkCoordDto[] = [];
      const removed: ChunkCoordDto[] = [];
      for (const k of next) {
        if (!current.has(k)) added.push(unkey(k));
      }
      for (const k of current) {
        if (!next.has(k)) removed.push(unkey(k));
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
      current = new Set();
    },
  };
}
