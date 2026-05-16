import { describe, expect, it } from 'vitest';
import {
  computeInitialSubscriptionCoords,
  createSubscriptionClient,
} from '../../src/backend/chunkSubscriptionClient';

describe('chunkSubscriptionClient', () => {
  it('computeInitialSubscriptionCoords covers the entire world grid', () => {
    const coords = computeInitialSubscriptionCoords({
      worldWidthTiles: 256,
      worldHeightTiles: 256,
      chunkSize: 32,
    });
    expect(coords).toHaveLength(64);
    expect(coords).toContainEqual({ x: 0, y: 0 });
    expect(coords).toContainEqual({ x: 7, y: 7 });
  });

  it('sends a chunk_subscribe with the initial coords when start is called', () => {
    const sendCalls: string[] = [];
    const send = (s: string) => sendCalls.push(s);
    const client = createSubscriptionClient({
      send,
      worldWidthTiles: 64,
      worldHeightTiles: 64,
      chunkSize: 32,
    });
    client.start();
    expect(sendCalls).toHaveLength(1);
    const parsed = JSON.parse(sendCalls[0]);
    expect(parsed.type).toBe('chunk_subscribe');
    expect(parsed.coords).toHaveLength(4);
  });
});
