// src/diorama/ksw/geo/tileStreamer.ts
// M3: deterministische Ring-Policy + Hysterese + LRU fürs Tile-Streaming.
// KEIN three, KEIN fetch — alles hier ist deterministisch unit-testbar, aber
// NICHT pure: planStep() mutiert das übergebene StreamerState (tick/lastNear).
// Die Queue/IO lebt in Task 2 (streamWorld), die Materialisierung in
// tileContent.ts.

export type TileKey = string;
export type TileMeta = { key: TileKey; level: number; cx: number; cz: number };
export type RingConfig = { r2: number; r1: number; hysteresis: number; maxLive: number };
export type StreamerState = { live: Map<TileKey, { lastNear: number }>; tick: number };

// r1=3600 deckt die L1-Halbdiagonale (5000·√2/2 ≈ 3536): mit dem alten 2500
// blieb ein L1-Tile unsichtbar, sobald die Kamera nahe einer Tile-Ecke stand —
// der Mittelring des Horizont-Shots zeigte dann nacktes L0-Backdrop statt
// Terrain-Relief + Baum-Impostors (Task-6-Finding, in Task 7 im Browser
// verifiziert). Kosten: ~1-3 zusätzliche live L1-Tiles (nur Terrain+Impostors),
// fps am Stadtrand blieb im Task-7-Smoke deutlich über dem 85er-Gate.
export const DEFAULT_RINGS: RingConfig = { r2: 800, r1: 3600, hysteresis: 1.1, maxLive: 80 };

function radiusFor(level: number, cfg: RingConfig): number {
  return level === 2 ? cfg.r2 : level === 1 ? cfg.r1 : Infinity;
}

/** Zentrum eines Tiles in Weltkoordinaten aus der Manifest-Quadtree-Wurzel
 * (minX/minZ/size) und einer Tile-Referenz (level/x/y). Pure Funktion —
 * Task 6 baut damit die TileMeta-Liste aus dem WorldManifest auf.
 *
 * Teilungsfaktor: der Bake (scripts/geo/lib/tiles.mjs, LEVEL_CELLS = [1, 4,
 * 16]) teilt JEDE Stufe in 4x4 — also 4**level Zellen pro Seite, NICHT
 * 2**level. Gegen den echten Bake verifiziert: L2/5_7 (origin -3391/-356,
 * Kantenlänge 1250 m = 20000/16) zentriert exakt auf
 * minX + (5 + 0.5) * size/16. */
export function tileCenter(
  manifest: { minX: number; minZ: number; size: number },
  ref: { level: number; x: number; y: number },
): [number, number] {
  const cell = manifest.size / 4 ** ref.level;
  return [manifest.minX + (ref.x + 0.5) * cell, manifest.minZ + (ref.y + 0.5) * cell];
}

export function desiredLevel(
  camX: number,
  camZ: number,
  tile: { level: number; cx: number; cz: number },
  cfg: RingConfig,
): boolean {
  return Math.hypot(tile.cx - camX, tile.cz - camZ) <= radiusFor(tile.level, cfg);
}

export function planStep(
  state: StreamerState,
  camX: number,
  camZ: number,
  all: TileMeta[],
  cfg: RingConfig,
): { load: TileMeta[]; unload: TileKey[] } {
  state.tick++;
  const desired = new Set<TileKey>();
  const load: TileMeta[] = [];
  const unload: TileKey[] = [];
  const dist = (m: TileMeta) => Math.hypot(m.cx - camX, m.cz - camZ);

  for (const m of all) {
    const d = dist(m);
    const r = radiusFor(m.level, cfg);
    if (d <= r) {
      desired.add(m.key);
      const rec = state.live.get(m.key);
      if (rec) rec.lastNear = state.tick;
      else load.push(m);
    } else if (state.live.has(m.key) && m.level !== 0 && d > r * cfg.hysteresis) {
      unload.push(m.key);
    }
  }
  load.sort((a, b) => dist(a) - dist(b));

  // LRU-Kappe: Über-Budget → älteste nicht-nahe, nicht-gewünschte, nicht-L0.
  // Bewusster Kontrakt: maxLive kann dennoch überschritten bleiben, wenn alle
  // verbleibenden Live-Tiles zur Soll-Menge (desired) gehören oder L0 sind —
  // beide Kategorien sind unantastbar, auch wenn das Budget dadurch reisst.
  const projected = state.live.size - unload.length + load.length;
  let excess = projected - cfg.maxLive;
  if (excess > 0) {
    const unloadSet = new Set(unload);
    const candidates = [...state.live.entries()]
      .filter(([k]) => !desired.has(k) && !unloadSet.has(k) && !k.startsWith('L0/'))
      .sort((a, b) => a[1].lastNear - b[1].lastNear);
    for (const [k] of candidates) {
      if (excess-- <= 0) break;
      unload.push(k);
    }
  }
  return { load, unload };
}

const MAX_PARALLEL_FETCHES = 4;
const MAX_ATTEMPTS = 2; // 1 initial try + 1 retry

export type TileStreamerOptions = {
  all: TileMeta[];
  cfg?: RingConfig;
  fetchTile: (meta: TileMeta) => Promise<unknown>;
  onReady: (meta: TileMeta, tile: unknown) => void;
  onUnload: (key: TileKey) => void;
  onError?: (meta: TileMeta, err: unknown) => void;
};

/**
 * IO layer over the pure ring policy (`planStep`): owns the live tile
 * bookkeeping, an in-order fetch queue capped at 4 concurrent requests, a
 * single retry per tile before giving up (`failed`), and stale-drop handling
 * for tiles that fall out of the desired set while their fetch is in flight.
 */
export class TileStreamer {
  private readonly all: TileMeta[];
  private readonly cfg: RingConfig;
  private readonly fetchTile: (meta: TileMeta) => Promise<unknown>;
  private readonly onReadyCb: (meta: TileMeta, tile: unknown) => void;
  private readonly onUnloadCb: (key: TileKey) => void;
  private readonly onErrorCb?: (meta: TileMeta, err: unknown) => void;

  private readonly state: StreamerState = { live: new Map(), tick: 0 };
  private readonly metaByKey = new Map<TileKey, TileMeta>();
  private readonly inflight = new Map<TileKey, { meta: TileMeta; attempt: number }>();
  private readonly queue: TileMeta[] = [];
  private readonly queuedKeys = new Set<TileKey>();
  private readonly failedSet = new Set<TileKey>();
  private readonly retryAttempt = new Map<TileKey, number>();
  private lastCam: [number, number] | null = null;

  constructor(opts: TileStreamerOptions) {
    this.all = opts.all;
    this.cfg = opts.cfg ?? DEFAULT_RINGS;
    this.fetchTile = opts.fetchTile;
    this.onReadyCb = opts.onReady;
    this.onUnloadCb = opts.onUnload;
    this.onErrorCb = opts.onError;
    for (const m of this.all) this.metaByKey.set(m.key, m);
  }

  get liveCount(): number {
    return this.state.live.size;
  }

  get failed(): ReadonlySet<TileKey> {
    return this.failedSet;
  }

  /** Anzahl noch nicht abgeschlossener Ladevorgänge (gequeued + in flight).
   * 0 nach einem update() heisst: der aktuelle Soll-Ring ist fertig
   * materialisiert (oder endgültig gescheitert) — main.ts gated darauf
   * `__LOOK_READY`. */
  get pendingCount(): number {
    return this.queue.length + this.inflight.size;
  }

  /** Test-Sonde: FIFO-Reihenfolge der aktuell gequeuten (noch nicht gestarteten)
   * Tile-Keys. Kein Produktionscode liest dies — dient ausschliesslich dazu, den
   * Queue-Vertrag (Retry via unshift(), kein direkter startFetch()-Bypass) von
   * aussen zu verifizieren. */
  get queuedOrder(): readonly TileKey[] {
    return this.queue.map((m) => m.key);
  }

  update(camX: number, camZ: number): void {
    this.lastCam = [camX, camZ];
    const { load, unload } = planStep(this.state, camX, camZ, this.all, this.cfg);

    for (const key of unload) {
      this.state.live.delete(key);
      this.onUnloadCb(key);
    }

    for (const meta of load) {
      if (this.failedSet.has(meta.key)) continue;
      if (this.state.live.has(meta.key)) continue;
      if (this.inflight.has(meta.key)) continue;
      if (this.queuedKeys.has(meta.key)) continue;
      this.queue.push(meta);
      this.queuedKeys.add(meta.key);
    }

    this.pump();
  }

  private pump(): void {
    while (this.inflight.size < MAX_PARALLEL_FETCHES && this.queue.length > 0) {
      const meta = this.queue.shift()!;
      this.queuedKeys.delete(meta.key);
      const attempt = this.retryAttempt.get(meta.key) ?? 1;
      this.retryAttempt.delete(meta.key);
      this.startFetch(meta, attempt);
    }
  }

  private startFetch(meta: TileMeta, attempt: number): void {
    this.inflight.set(meta.key, { meta, attempt });
    this.fetchTile(meta).then(
      (tile) => this.onFetchResolved(meta, tile),
      (err) => this.onFetchRejected(meta, attempt, err),
    );
  }

  private isStillDesired(meta: TileMeta): boolean {
    if (!this.lastCam) return false;
    const [camX, camZ] = this.lastCam;
    return desiredLevel(camX, camZ, meta, this.cfg);
  }

  private onFetchResolved(meta: TileMeta, tile: unknown): void {
    this.inflight.delete(meta.key);
    if (this.isStillDesired(meta)) {
      this.state.live.set(meta.key, { lastNear: this.state.tick });
      this.onReadyCb(meta, tile);
    }
    this.pump();
  }

  private onFetchRejected(meta: TileMeta, attempt: number, err: unknown): void {
    this.inflight.delete(meta.key);
    if (attempt < MAX_ATTEMPTS) {
      // Retry läuft über die normale Queue/pump()-Fairness statt die 4-Slot-
      // Kappe per direktem startFetch()-Aufruf zu umgehen — sonst verhungern
      // entferntere, bereits gequeute Tiles hinter einem flackernden Tile.
      this.retryAttempt.set(meta.key, attempt + 1);
      this.queue.unshift(meta);
      this.queuedKeys.add(meta.key);
      this.pump();
    } else {
      this.failedSet.add(meta.key);
      this.onErrorCb?.(meta, err);
      this.pump();
    }
  }
}
