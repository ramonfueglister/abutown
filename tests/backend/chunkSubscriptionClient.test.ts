// tests/backend/chunkSubscriptionClient.test.ts
import { describe, it, expect, vi } from 'vitest';
import { createSubscriptionClient } from '../../src/backend/chunkSubscriptionClient';

describe('createSubscriptionClient', () => {
  function setup() {
    const send = vi.fn<(text: string) => void>();
    const client = createSubscriptionClient({ send });
    return { client, send };
  }

  it('sends a single chunk_subscribe on the first update with non-empty visible set', () => {
    const { client, send } = setup();
    client.update([{ x: 1, y: 1 }, { x: 2, y: 1 }]);
    expect(send).toHaveBeenCalledTimes(1);
    const msg = JSON.parse(send.mock.calls[0][0]);
    expect(msg.type).toBe('chunk_subscribe');
    expect(msg.coords).toEqual(expect.arrayContaining([{ x: 1, y: 1 }, { x: 2, y: 1 }]));
    expect(msg.coords).toHaveLength(2);
  });

  it('sends nothing when called twice with the same visible set', () => {
    const { client, send } = setup();
    const coords = [{ x: 0, y: 0 }];
    client.update(coords);
    expect(send).toHaveBeenCalledTimes(1);
    client.update(coords);
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('sends subscribe-for-added + unsubscribe-for-removed on partial overlap', () => {
    const { client, send } = setup();
    client.update([{ x: 0, y: 0 }, { x: 1, y: 0 }]);
    send.mockClear();
    client.update([{ x: 1, y: 0 }, { x: 2, y: 0 }]);
    const messages = send.mock.calls.map((c) => JSON.parse(c[0]));
    const subscribe = messages.find((m) => m.type === 'chunk_subscribe');
    const unsubscribe = messages.find((m) => m.type === 'chunk_unsubscribe');
    expect(subscribe).toBeDefined();
    expect(subscribe.coords).toEqual([{ x: 2, y: 0 }]);
    expect(unsubscribe).toBeDefined();
    expect(unsubscribe.coords).toEqual([{ x: 0, y: 0 }]);
  });

  it('sends nothing when called with an empty visible set after an empty set', () => {
    const { client, send } = setup();
    client.update([]);
    expect(send).not.toHaveBeenCalled();
  });

  it('reset() clears internal state so the next update re-subscribes from scratch', () => {
    const { client, send } = setup();
    client.update([{ x: 5, y: 5 }]);
    expect(send).toHaveBeenCalledTimes(1);
    send.mockClear();
    client.reset();
    client.update([{ x: 5, y: 5 }]);
    expect(send).toHaveBeenCalledTimes(1);
    const msg = JSON.parse(send.mock.calls[0][0]);
    expect(msg.type).toBe('chunk_subscribe');
  });
});
