// src/diorama/ksw/geo/tileStreamer.ts
// M3: pure Ring-Policy + Hysterese + LRU fürs Tile-Streaming. KEIN three,
// KEIN fetch — alles hier ist deterministisch unit-testbar. Die Queue/IO
// lebt in Task 2 (streamWorld), die Materialisierung in tileContent.ts.

export type TileKey = string;
export type TileMeta = { key: TileKey; level: number; cx: number; cz: number };
export type RingConfig = { r2: number; r1: number; hysteresis: number; maxLive: number };
export type StreamerState = { live: Map<TileKey, { lastNear: number }>; tick: number };

export const DEFAULT_RINGS: RingConfig = { r2: 800, r1: 2500, hysteresis: 1.1, maxLive: 80 };

function radiusFor(level: number, cfg: RingConfig): number {
  return level === 2 ? cfg.r2 : level === 1 ? cfg.r1 : Infinity;
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
