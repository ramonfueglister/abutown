import { describe, expect, it, vi } from 'vitest';
import { TileStreamer, type TileMeta } from '../../src/diorama/ksw/geo/tileStreamer';

const t = (level: number, cx: number, cz: number): TileMeta => ({ key: `L${level}/${cx}_${cz}`, level, cx, cz });

function deferredFetch() {
  const pending = new Map<string, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  const fetchTile = vi.fn((m: TileMeta) =>
    new Promise((resolve, reject) => pending.set(m.key, { resolve, reject })));
  return { fetchTile, pending };
}

describe('TileStreamer queue', () => {
  it('max 4 parallele Fetches, Nachschub beim Auflösen', async () => {
    const all = Array.from({ length: 8 }, (_, i) => t(2, i * 10, 0));
    const { fetchTile, pending } = deferredFetch();
    const s = new TileStreamer({ all, fetchTile, onReady: () => {}, onUnload: () => {} });
    s.update(0, 0);
    expect(fetchTile).toHaveBeenCalledTimes(4);
    pending.get('L2/0_0')!.resolve({});
    await Promise.resolve(); await Promise.resolve();
    expect(fetchTile).toHaveBeenCalledTimes(5);
  });

  it('1 Retry, dann failed', async () => {
    const all = [t(2, 0, 0)];
    const { fetchTile, pending } = deferredFetch();
    const errs: unknown[] = [];
    const s = new TileStreamer({ all, fetchTile, onReady: () => {}, onUnload: () => {}, onError: (_m, e) => errs.push(e) });
    s.update(0, 0);
    pending.get('L2/0_0')!.reject(new Error('net'));
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(fetchTile).toHaveBeenCalledTimes(2); // Retry
    pending.get('L2/0_0')!.reject(new Error('net2'));
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(s.failed.has('L2/0_0')).toBe(true);
    expect(errs.length).toBe(1);
    s.update(0, 0);
    expect(fetchTile).toHaveBeenCalledTimes(2); // failed wird nicht erneut versucht
  });

  it('verwirft Ankünfte, die nicht mehr gewünscht sind', async () => {
    const all = [t(2, 0, 0)];
    const { fetchTile, pending } = deferredFetch();
    const ready = vi.fn();
    const s = new TileStreamer({ all, fetchTile, onReady: ready, onUnload: () => {} });
    s.update(0, 0);
    s.update(99999, 0); // Kamera weg, Tile nicht mehr gewünscht
    pending.get('L2/0_0')!.resolve({});
    await Promise.resolve(); await Promise.resolve();
    expect(ready).not.toHaveBeenCalled();
    expect(s.liveCount).toBe(0);
  });
});
