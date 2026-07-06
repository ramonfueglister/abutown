import { describe, expect, it } from 'vitest';
import { DEFAULT_RINGS, desiredLevel, planStep, tileCenter, type StreamerState, type TileMeta } from '../../src/diorama/ksw/geo/tileStreamer';

const t = (level: number, cx: number, cz: number): TileMeta => ({ key: `L${level}/${cx}_${cz}`, level, cx, cz });
const fresh = (): StreamerState => ({ live: new Map(), tick: 0 });

describe('tileCenter', () => {
  const manifest = { minX: -100, minZ: -100, size: 200 };

  it('L0-Tile {0,0,0} liegt auf der Weltmitte', () => {
    expect(tileCenter(manifest, { level: 0, x: 0, y: 0 })).toEqual([0, 0]);
  });

  // Bake-Konvention (scripts/geo/lib/tiles.mjs LEVEL_CELLS = [1, 4, 16]):
  // 4x4-Teilung PRO Stufe, also cell = size / 4**level.
  it('L2-Tile {2,0,0} liegt im Zentrum der ersten 1/16-Zelle (cell=12.5)', () => {
    expect(tileCenter(manifest, { level: 2, x: 0, y: 0 })).toEqual([-93.75, -93.75]);
  });

  it('L1-Tile {1,1,0} liegt im Zentrum der zweiten Viertel-Zelle entlang x (cell=50)', () => {
    expect(tileCenter(manifest, { level: 1, x: 1, y: 0 })).toEqual([-25, -75]);
  });

  it('L2-Tile {2,15,15} liegt im Zentrum der letzten Zelle', () => {
    expect(tileCenter(manifest, { level: 2, x: 15, y: 15 })).toEqual([93.75, 93.75]);
  });
});

describe('desiredLevel', () => {
  it('L2 nur im Nahring, L1 im Mittelring, L0 immer', () => {
    expect(desiredLevel(0, 0, { level: 2, cx: 500, cz: 0 }, DEFAULT_RINGS)).toBe(true);
    expect(desiredLevel(0, 0, { level: 2, cx: 900, cz: 0 }, DEFAULT_RINGS)).toBe(false);
    expect(desiredLevel(0, 0, { level: 1, cx: 2000, cz: 0 }, DEFAULT_RINGS)).toBe(true);
    expect(desiredLevel(0, 0, { level: 1, cx: 2600, cz: 0 }, DEFAULT_RINGS)).toBe(false);
    expect(desiredLevel(0, 0, { level: 0, cx: 9999, cz: 0 }, DEFAULT_RINGS)).toBe(true);
  });
});

describe('planStep', () => {
  it('lädt distanz-sortiert und entlädt erst jenseits der Hysterese', () => {
    const all = [t(2, 100, 0), t(2, 700, 0), t(2, 850, 0)];
    const s = fresh();
    const p1 = planStep(s, 0, 0, all, DEFAULT_RINGS);
    expect(p1.load.map((m) => m.cx)).toEqual([100, 700]); // 850 > r2
    // Kamera rückt zu x=60: Tile 850 ist jetzt 790 entfernt → laden;
    // Tile 100 bleibt live. Kein Entladen (nichts > 880 = r2·1.1).
    for (const m of p1.load) s.live.set(m.key, { lastNear: s.tick });
    const p2 = planStep(s, 60, 0, all, DEFAULT_RINGS);
    expect(p2.load.map((m) => m.cx)).toEqual([850]);
    expect(p2.unload).toEqual([]);
    // Kamera springt weit weg: alles jenseits 880 → entladen.
    s.live.set('L2/850_0', { lastNear: s.tick });
    const p3 = planStep(s, 5000, 0, all, DEFAULT_RINGS);
    expect(new Set(p3.unload)).toEqual(new Set(['L2/100_0', 'L2/700_0', 'L2/850_0']));
  });

  it('flattert nicht an der Ringgrenze (Hysterese-Band)', () => {
    const all = [t(2, 800, 0)];
    const s = fresh();
    const p1 = planStep(s, 0, 0, all, DEFAULT_RINGS);
    expect(p1.load.length).toBe(1);
    s.live.set(all[0].key, { lastNear: 0 });
    // dist 810: > r2, aber < r2·1.1 → weder load noch unload
    const p2 = planStep(s, -10, 0, all, DEFAULT_RINGS);
    expect(p2.load).toEqual([]);
    expect(p2.unload).toEqual([]);
  });

  it('LRU-Kappe entlädt die ältesten nicht-nahen Tiles, nie die Soll-Menge, nie L0', () => {
    const cfg = { ...DEFAULT_RINGS, maxLive: 2 };
    const s = fresh();
    s.live.set('L2/9000_0', { lastNear: 1 }); // alt, fern
    s.live.set('L0/0_0', { lastNear: 0 });    // L0: unantastbar
    const all = [t(2, 9000, 0), t(0, 0, 0), t(2, 100, 0), t(2, 200, 0)];
    const p = planStep(s, 0, 0, all, cfg);
    expect(p.load.map((m) => m.cx)).toEqual([100, 200]);
    expect(p.unload).toContain('L2/9000_0');
    expect(p.unload).not.toContain('L0/0_0');
  });

  it('LRU-Kappe greift auch INNERHALB des Hysterese-Bands (kein Hysterese-Unload maskiert die Eviction)', () => {
    // dist(850,0) = 850: > r2=800 (nicht desired) aber <= r2*1.1=880
    // (kein Hysterese-Unload). Ohne LRU-Kappe bliebe das Tile also live —
    // dieser Test beweist, dass die LRU-Schleife selbst greift, nicht die
    // Hysterese-Logik (die im bestehenden Test bereits das Entladen erledigt).
    const cfg = { ...DEFAULT_RINGS, maxLive: 2 };
    const s = fresh();
    s.live.set('L2/850_0', { lastNear: 1 }); // live, im Hysterese-Band, alt
    const all = [t(2, 100, 0), t(2, 200, 0), t(2, 850, 0)];
    const p = planStep(s, 0, 0, all, cfg);
    expect(p.load.map((m) => m.cx)).toEqual([100, 200]);
    expect(p.unload).toContain('L2/850_0');
  });
});
