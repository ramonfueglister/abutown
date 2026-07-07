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
// r2=900 deckt die L2-Halbdiagonale (Kante 20000/16 = 1250 m, Halbdiagonale
// 1250·√2/2 ≈ 884): mit dem alten 800 blieb an Tile-Ecken KEIN L2-Tile
// desired, die Hysterese (880) rettete das nicht — Eck-Loch im Nahring
// (Gebäude + Vollbäume fehlten dort). Analog zu r1 (Task 7). Kosten: minimal,
// nur 1-2 zusätzliche live L2-Tiles an Ecken, fps-Gate bleibt unberührt.
export const DEFAULT_RINGS: RingConfig = { r2: 900, r1: 3600, hysteresis: 1.1, maxLive: 80 };

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
// #143 M3: how many update() ticks a tile stays "failed" before it becomes
// eligible for a fresh attempt. Without an expiry a tile that fails its 2
// attempts was a permanent session hole (skipped forever until a reload); with
// it a transient blip self-heals. update() advances the tick only when the
// camera moves (~2 Hz), so this is "ticks of active navigation", not seconds —
// enough to avoid hammering a genuinely-missing tile, short enough to recover.
const DEFAULT_FAILED_COOLDOWN_TICKS = 120;

export type TileStreamerOptions = {
  all: TileMeta[];
  cfg?: RingConfig;
  fetchTile: (meta: TileMeta) => Promise<unknown>;
  onReady: (meta: TileMeta, tile: unknown) => void;
  onUnload: (key: TileKey) => void;
  onError?: (meta: TileMeta, err: unknown) => void;
  /** #143 M3: ticks a tile stays failed before a fresh attempt (default 120). */
  failedCooldownTicks?: number;
};

/**
 * IO layer over the pure ring policy (`planStep`): owns the live tile
 * bookkeeping, an in-order fetch queue capped at 4 concurrent requests, a
 * single retry per tile before giving up, and stale-drop handling for tiles
 * that fall out of the desired set while their fetch is in flight.
 *
 * A tile that exhausts its attempts is NOT failed forever (#143 M3): it enters
 * a cooldown (`failedUntil`, keyed on the streamer tick) and is retried once
 * the cooldown lapses, or immediately if the camera leaves and re-approaches
 * it — so a transient error is a momentary gap, never a permanent hole.
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
  // #143 M3: TileKey → streamer tick at which the cooldown lapses. A tile in
  // this map with `state.tick < until` is currently failed (skipped from the
  // load set); once `state.tick >= until` it becomes eligible again.
  private readonly failedUntil = new Map<TileKey, number>();
  private readonly failedCooldownTicks: number;
  private readonly retryAttempt = new Map<TileKey, number>();
  private lastCam: [number, number] | null = null;

  constructor(opts: TileStreamerOptions) {
    this.all = opts.all;
    this.cfg = opts.cfg ?? DEFAULT_RINGS;
    this.fetchTile = opts.fetchTile;
    this.onReadyCb = opts.onReady;
    this.onUnloadCb = opts.onUnload;
    this.onErrorCb = opts.onError;
    this.failedCooldownTicks = opts.failedCooldownTicks ?? DEFAULT_FAILED_COOLDOWN_TICKS;
    for (const m of this.all) this.metaByKey.set(m.key, m);
  }

  get liveCount(): number {
    return this.state.live.size;
  }

  /** Keys currently in failure cooldown (tick < until) — a debug/telemetry
   * probe (`__stream`). Expired-but-not-yet-swept entries are excluded. */
  get failed(): ReadonlySet<TileKey> {
    const now = this.state.tick;
    const out = new Set<TileKey>();
    for (const [k, until] of this.failedUntil) if (now < until) out.add(k);
    return out;
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

    // #143 M3 reset: a failed tile that is no longer desired (camera moved
    // away) clears its cooldown immediately, so re-approaching it retries at
    // once instead of waiting out the timer — a transient failure never
    // outlives the view that hit it.
    for (const key of this.failedUntil.keys()) {
      const meta = this.metaByKey.get(key);
      if (!meta || !desiredLevel(camX, camZ, meta, this.cfg)) this.failedUntil.delete(key);
    }

    for (const meta of load) {
      // #143 M3 expiry: still in cooldown → skip; lapsed → sweep and re-queue.
      const until = this.failedUntil.get(meta.key);
      if (until !== undefined) {
        if (this.state.tick < until) continue;
        this.failedUntil.delete(meta.key);
      }
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
    this.failedUntil.delete(meta.key); // a success clears any lingering cooldown
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
      // #143 M3: enter cooldown rather than fail permanently — see failedUntil.
      this.failedUntil.set(meta.key, this.state.tick + this.failedCooldownTicks);
      this.onErrorCb?.(meta, err);
      this.pump();
    }
  }
}
