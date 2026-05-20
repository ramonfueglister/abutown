// src/backend/chunkSubscriptionClient.ts
import { create, toBinary } from '@bufbuild/protobuf';
import {
  ChunkCoordSchema,
  ChunkSubscribeSchema,
  ChunkUnsubscribeSchema,
  ClientMessageSchema,
} from './proto/abutown_pb';
import type { ChunkCoordDto } from './mobilityProtocol';

export type SubscriptionClient = {
  update(visible: ChunkCoordDto[]): void;
  reset(): void;
};

function key(coord: ChunkCoordDto): string {
  return `${coord.x},${coord.y}`;
}

const PROTOCOL_VERSION = 16;

function encodeSubscribe(coords: ChunkCoordDto[]): Uint8Array {
  const msg = create(ClientMessageSchema, {
    body: {
      case: 'chunkSubscribe',
      value: create(ChunkSubscribeSchema, {
        protocolVersion: PROTOCOL_VERSION,
        coords: coords.map((c) => create(ChunkCoordSchema, { x: c.x, y: c.y })),
      }),
    },
  });
  return toBinary(ClientMessageSchema, msg);
}

function encodeUnsubscribe(coords: ChunkCoordDto[]): Uint8Array {
  const msg = create(ClientMessageSchema, {
    body: {
      case: 'chunkUnsubscribe',
      value: create(ChunkUnsubscribeSchema, {
        protocolVersion: PROTOCOL_VERSION,
        coords: coords.map((c) => create(ChunkCoordSchema, { x: c.x, y: c.y })),
      }),
    },
  });
  return toBinary(ClientMessageSchema, msg);
}

export function createSubscriptionClient(opts: {
  send: (bytes: Uint8Array) => void;
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
        opts.send(encodeSubscribe(added));
      }
      if (removed.length > 0) {
        opts.send(encodeUnsubscribe(removed));
      }
      current = next;
    },
    reset() {
      current = new Map();
    },
  };
}
