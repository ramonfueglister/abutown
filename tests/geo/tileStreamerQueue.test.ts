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

  it('#143 M3: ein endgültig gescheitertes Tile wird nach Cooldown-Ablauf erneut versucht (kein Session-Loch)', async () => {
    const all = [t(2, 0, 0)];
    const { fetchTile, pending } = deferredFetch();
    // Kleiner Cooldown, damit der Test deterministisch ohne 120 update()-Ticks auskommt.
    const s = new TileStreamer({ all, fetchTile, onReady: () => {}, onUnload: () => {}, failedCooldownTicks: 2 });
    s.update(0, 0); // tick 1, attempt 1
    pending.get('L2/0_0')!.reject(new Error('net'));
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    pending.get('L2/0_0')!.reject(new Error('net2')); // attempt 2 → failed, failedUntil = tick1 + 2 = 3
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(fetchTile).toHaveBeenCalledTimes(2);
    expect(s.failed.has('L2/0_0')).toBe(true);
    s.update(0, 0); // tick 2 < 3 → weiter im Cooldown, kein Refetch
    expect(fetchTile).toHaveBeenCalledTimes(2);
    s.update(0, 0); // tick 3 ≥ 3 → Cooldown abgelaufen → neu eingereiht + gefetcht
    expect(fetchTile).toHaveBeenCalledTimes(3);
    expect(s.failed.has('L2/0_0')).toBe(false); // nicht mehr als gescheitert geführt
    pending.get('L2/0_0')!.resolve({}); // Neuversuch gelingt
    await Promise.resolve(); await Promise.resolve();
    expect(s.liveCount).toBe(1);
    expect(s.failed.has('L2/0_0')).toBe(false);
  });

  it('#143 M3: ein gescheitertes Tile wird sofort zurückgesetzt, sobald es nicht mehr gewünscht ist (Reset bei Verlassen)', async () => {
    const all = [t(2, 0, 0)];
    const { fetchTile, pending } = deferredFetch();
    // Grosser Cooldown: ohne den Un-desired-Reset gäbe es KEINEN Refetch im Testfenster.
    const s = new TileStreamer({ all, fetchTile, onReady: () => {}, onUnload: () => {}, failedCooldownTicks: 10_000 });
    s.update(0, 0);
    pending.get('L2/0_0')!.reject(new Error('net'));
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    pending.get('L2/0_0')!.reject(new Error('net2'));
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(s.failed.has('L2/0_0')).toBe(true);
    s.update(99999, 0); // Kamera weg → Tile nicht mehr gewünscht → Fail-Status verfällt
    expect(s.failed.has('L2/0_0')).toBe(false);
    s.update(0, 0); // Rückkehr → sofortiger Neuversuch trotz noch nicht abgelaufenem Cooldown
    expect(fetchTile).toHaveBeenCalledTimes(3);
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

  it('Retry geht vorne in die Queue statt die 4-Slot-Kappe zu umgehen', async () => {
    // 6 Tiles: 4 belegen sofort die Slots, 2 (E, F) warten in der Queue.
    const all = Array.from({ length: 6 }, (_, i) => t(2, i * 10, 0));
    const { fetchTile, pending } = deferredFetch();
    const s = new TileStreamer({ all, fetchTile, onReady: () => {}, onUnload: () => {} });
    // Kontrakt-Sonde: private pump() muss der EINZIGE Ort sein, der startFetch()
    // aufruft. Wir zählen, wie oft pump() vs. startFetch() (jeweils privat)
    // während des Retries aufgerufen werden, um zu erzwingen, dass der Retry
    // über pump() läuft statt startFetch() direkt zu callen.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const proto = TileStreamer.prototype as any;
    const pumpSpy = vi.spyOn(proto, 'pump');
    const startFetchSpy = vi.spyOn(proto, 'startFetch');

    s.update(0, 0);
    expect(fetchTile).toHaveBeenCalledTimes(4);
    expect(s.queuedOrder).toEqual(['L2/40_0', 'L2/50_0']);

    const pumpCallsBeforeReject = pumpSpy.mock.calls.length;

    // Erstes inflight-Tile (L2/0_0) schlägt fehl -> Retry-Pflicht.
    pending.get('L2/0_0')!.reject(new Error('net'));
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();

    // Vertrag (Finding 1): der Retry wird VORNE in die Queue gestellt
    // (this.queue.unshift(meta)) und läuft über den normalen pump()-Pfad —
    // er darf NICHT per direktem startFetch()-Aufruf an Queue/Kappe
    // vorbeigeschleust werden. Beweis: onFetchRejected muss pump() ein
    // weiteres Mal aufgerufen haben (statt startFetch() direkt zu callen,
    // OHNE dass zuvor pump() dafür lief).
    expect(pumpSpy.mock.calls.length).toBeGreaterThan(pumpCallsBeforeReject);
    // Jeder startFetch()-Aufruf, der NACH dem Reject passiert, muss aus
    // einem pump()-Aufruf stammen — d.h. die Reihenfolge der Spy-Aufrufe
    // zeigt: pump() wurde aufgerufen, BEVOR der Retry-startFetch() feuert.
    // (Ein direkter startFetch()-Bypass würde startFetch() aufrufen, OHNE
    // dass pump() zwischen dem Reject und diesem startFetch()-Call steht.)
    const startFetchCallOrder = startFetchSpy.mock.invocationCallOrder;
    const pumpCallOrder = pumpSpy.mock.invocationCallOrder;
    const retryStartFetchOrder = startFetchCallOrder[startFetchCallOrder.length - 1];
    const lastPumpBeforeRetry = Math.max(...pumpCallOrder.filter((o) => o < retryStartFetchOrder));
    expect(lastPumpBeforeRetry).toBeLessThan(retryStartFetchOrder);
    // ...und zwischen diesem pump()-Aufruf und dem Retry-startFetch() darf
    // kein ANDERER startFetch()-Aufruf mehr liegen (sonst wäre der Retry
    // nicht der, den dieser konkrete pump()-Aufruf ausgelöst hat).
    const startFetchesBetween = startFetchCallOrder.filter(
      (o) => o > lastPumpBeforeRetry && o <= retryStartFetchOrder,
    );
    expect(startFetchesBetween).toEqual([retryStartFetchOrder]);

    // Funktionale Endkontrolle: Retry kommt vor E dran (FIFO-Queue-Vorrang),
    // Parallelität bleibt <= 4, E/F unangetastet.
    expect(fetchTile).toHaveBeenCalledTimes(5);
    const fifthCallMeta = fetchTile.mock.calls[4][0] as TileMeta;
    expect(fifthCallMeta.key).toBe('L2/0_0');
    expect(s.queuedOrder).toEqual(['L2/40_0', 'L2/50_0']);
    expect(pending.has('L2/40_0')).toBe(false);
    expect(pending.has('L2/50_0')).toBe(false);
    expect(pending.size).toBe(4);

    pumpSpy.mockRestore();
    startFetchSpy.mockRestore();
  });

  it('kein onUnload / kein live-Eintrag, wenn Tile-Ankunft verworfen wird (Kamera nie live)', async () => {
    const all = [t(2, 0, 0)];
    const { fetchTile, pending } = deferredFetch();
    const unload = vi.fn();
    const s = new TileStreamer({ all, fetchTile, onReady: () => {}, onUnload: unload });
    s.update(0, 0);
    s.update(99999, 0); // Kamera weg, bevor der Fetch aufgelöst wird
    pending.get('L2/0_0')!.resolve({});
    await Promise.resolve(); await Promise.resolve();
    expect(unload).not.toHaveBeenCalled();
    expect(s.liveCount).toBe(0);
  });

  it('zwei synchrone update(0,0)-Aufrufe erzeugen keine Doppel-Fetches', () => {
    const all = [t(2, 0, 0)];
    const { fetchTile } = deferredFetch();
    const s = new TileStreamer({ all, fetchTile, onReady: () => {}, onUnload: () => {} });
    s.update(0, 0);
    expect(fetchTile).toHaveBeenCalledTimes(1);
    s.update(0, 0);
    expect(fetchTile).toHaveBeenCalledTimes(1);
  });
});
