# M3 Tile-Streaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kamera-getriebenes LOD-Streaming der 273-Tile-Welt-Pyramide — Nahring volle Gebäude+Bäume, Mittelring Massing+Impostors, Horizont L0 — bei ≥85 fps und schnellerem Boot.

**Architecture:** Reine Ring-Policy (`desiredTiles`) + LRU + distanz-priorisierte Fetch-Queue in `tileStreamer.ts`; Materialisierung/Dispose pro Tile in `tileContent.ts` (bestehende Builder wiederverwendet); treeLayer bekommt per-Tile-Instanz-Pools. Tile-Proto wächst additiv um `t_family`, `bake-world.mjs` schreibt es aus der seit #136 family-attributierten `transformNature`.

**Tech Stack:** three/webgpu + TSL, @bufbuild/protobuf, vitest, plain-ESM bake libs, Playwright-Smoke.

## Global Constraints

- Determinismus: Bake byte-identisch bei gleichen Inputs; keine `Math.random`/`Date.now` in Bake/Policy (Spec).
- Perf: fps ≥ 85 im Fly-Through (Referenz 120); LRU-Kappe max. 80 materialisierte Tiles.
- Ringe: R2 = 800 m (L2), R1 = 2500 m (L1), Hysterese-Faktor 1.1; Fetch-Parallelität 4; 1 Retry pro Tile, dann `failed`.
- Familien nur `spreading | oval | tall | conic | slender` (kanonisch `treeArchetypes.ts`); `t_family`-Codes: 0=spreading, 1=oval, 2=tall, 3=conic, 4=slender (Reihenfolge = `[...BROAD_FAMILIES, ...CONIFER_FAMILIES]`).
- Platten-Inhalt (buildings/nature/roads.json) bleibt boot-geladen; Tiles überspringen Gebäude/Bäume im Platten-Rechteck aus `meta.json` (`plate: {cx, cz, w, d}` — Feldnamen in `src/diorama/ksw/geo/geoData.ts:31` verifizieren).
- Browser-Smoke ist Pflicht (CLAUDE.md); cargo NUR via `scripts/cargo-serial.sh`.
- Regressionsschutz: `scripts/smoke-trees.mjs` und `scripts/smoke-traffic.mjs` müssen am Ende grün sein.

---

### Task 1: Ring-Policy + LRU (pure Logik, `tileStreamer.ts`)

**Files:**
- Create: `src/diorama/ksw/geo/tileStreamer.ts`
- Test: `tests/geo/tileStreamer.test.ts`

**Interfaces:**
- Consumes: `TileRef` aus `../../../proto/world_pb` (Felder: `path: string`, `level: number`, `x: number`, `y: number`, plus Bounds — exakte Feldnamen in `src/proto/world_pb.ts` nachschlagen; falls TileRef keine Welt-Bounds trägt, Tile-Bounds aus Manifest-Projektion ableiten: `WorldManifest` in world.proto prüfen; wenn nötig `originX/originZ/sizeM` pro Ref aus dem zugehörigen Tile-Grid — dann stattdessen eine `TileBounds = {key, level, cx, cz}`-Liste als Input nehmen, die Task 6 aus dem Manifest baut).
- Produces (Task 2/6 verlassen sich exakt hierauf):
  - `type TileKey = string` (`` `L${level}/${x}_${y}` ``)
  - `type RingConfig = { r2: number; r1: number; hysteresis: number; maxLive: number }`
  - `const DEFAULT_RINGS: RingConfig = { r2: 800, r1: 2500, hysteresis: 1.1, maxLive: 80 }`
  - `desiredLevel(camX, camZ, tile: {level, cx, cz}, cfg): boolean` — Tile gehört zur Soll-Menge seines Levels (L2 wenn dist≤r2, L1 wenn dist≤r1, L0 immer)
  - `planStep(state: StreamerState, camX, camZ, all: TileMeta[], cfg): { load: TileMeta[]; unload: TileKey[] }` mit `type TileMeta = { key: TileKey; level: number; cx: number; cz: number }` und `type StreamerState = { live: Map<TileKey, { lastNear: number }>; tick: number }`
  - Regeln: laden bei dist ≤ R(level), entladen erst bei dist > R(level)·hysteresis; `load` distanz-aufsteigend sortiert; wenn `live.size + load.length > maxLive`, entlade zusätzlich die am längsten nicht-nahen Live-Tiles (kleinstes `lastNear`), aber NIE Tiles der aktuellen Soll-Menge; L0 nie entladen.

- [ ] **Step 1: Failing Tests schreiben** — `tests/geo/tileStreamer.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { DEFAULT_RINGS, desiredLevel, planStep, type StreamerState, type TileMeta } from '../../src/diorama/ksw/geo/tileStreamer';

const t = (level: number, cx: number, cz: number): TileMeta => ({ key: `L${level}/${cx}_${cz}`, level, cx, cz });
const fresh = (): StreamerState => ({ live: new Map(), tick: 0 });

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
});
```

- [ ] **Step 2: FAIL verifizieren**: `npx vitest run tests/geo/tileStreamer.test.ts` → „Cannot find module …tileStreamer"
- [ ] **Step 3: Implementation** — `src/diorama/ksw/geo/tileStreamer.ts`:

```ts
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
```

- [ ] **Step 4: Tests grün** + `npm run typecheck` grün.
- [ ] **Step 5: Commit**: `git add src/diorama/ksw/geo/tileStreamer.ts tests/geo/tileStreamer.test.ts && git commit -m "feat(m3): Ring-Policy + Hysterese + LRU als pure Streaming-Logik"`

### Task 2: Fetch-Queue + per-Tile-Fetch (worldData-Refactor)

**Files:**
- Modify: `src/diorama/ksw/geo/worldData.ts` (exportiere `fetchTileBin` + `decodeTileBin`; `loadWorld` bleibt für L0/Boot bestehen)
- Modify: `src/diorama/ksw/geo/tileStreamer.ts` (Klasse `TileStreamer` — Queue über der puren Logik)
- Test: `tests/geo/tileStreamerQueue.test.ts`

**Interfaces:**
- Consumes: Task-1-API (`planStep`, `TileMeta`, `RingConfig`, `StreamerState`); `WorldTileSchema`/`fromBinary` wie in worldData.ts vorhanden.
- Produces:
  - worldData: `export async function fetchTileBin(baseUrl: string, path: string): Promise<Uint8Array>` (dünner Wrapper um das existierende `fetchBinary`); `export function decodeTileBin(bin: Uint8Array): WorldTile` (`fromBinary(WorldTileSchema, bin)`).
  - tileStreamer: `class TileStreamer { constructor(opts: { all: TileMeta[]; cfg?: RingConfig; fetchTile: (meta: TileMeta) => Promise<unknown>; onReady: (meta: TileMeta, tile: unknown) => void; onUnload: (key: TileKey) => void; onError?: (meta: TileMeta, err: unknown) => void }); update(camX: number, camZ: number): void; readonly liveCount: number; readonly failed: ReadonlySet<TileKey>; }`
  - Verhalten: max. 4 parallele fetchTile; pro Tile 1 Retry, danach `failed` (kein weiterer Versuch); ein während des Fetchs wieder aus der Soll-Menge gefallenes Tile wird nach Ankunft NICHT materialisiert (onReady unterdrückt) und nicht in `live` aufgenommen; `update` ist idempotent bei unveränderter Kamera.

- [ ] **Step 1: Failing Tests** — `tests/geo/tileStreamerQueue.test.ts` (fetch als vi.fn mit steuerbaren Promises; Fake-Timer unnötig — Promises manuell auflösen):

```ts
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
```

- [ ] **Step 2: FAIL** → **Step 3: Implementation** in tileStreamer.ts (Queue-Klasse; Skeleton — der Implementer füllt die privaten Details, das VERHALTEN ist durch die Tests fixiert):

```ts
export class TileStreamer {
  // privat: state: StreamerState, inflight: Map<TileKey, {meta, attempt}>,
  // queue: TileMeta[], failed: Set<TileKey>, lastCam: [number, number] | null.
  // update(cam): planStep → unload sofort (onUnload + live.delete);
  //   load-Menge in queue mergen (dedupe gegen live/inflight/failed/queue);
  //   pump(): solange inflight.size < 4 und queue nicht leer → fetchTile;
  //   bei resolve: wenn Tile noch gewünscht (desiredLevel mit lastCam) →
  //   live.set + onReady, sonst verwerfen; bei reject: attempt<1 → requeue
  //   vorne, sonst failed.add + onError. Jede Ankunft ruft pump() erneut.
}
```

- [ ] **Step 4: Tests grün, typecheck grün, `npx vitest run tests/geo/` komplett grün** (Task-1-Tests unverändert).
- [ ] **Step 5: worldData-Exports ergänzen** (fetchTileBin/decodeTileBin, 6 Zeilen, kein Verhaltensbruch — bestehende worldData/geoData-Tests grün).
- [ ] **Step 6: Commit**: `feat(m3): TileStreamer-Queue (4-parallel, 1 Retry, Stale-Drop) + per-Tile-Fetch-API`

### Task 3: `t_family` durch Proto + Bake + Welt-Rebake

**Files:**
- Modify: `backend/crates/protocol/proto/world.proto` (nach `t_kind = 54`: `repeated uint32 t_family = 55; // 0 spreading,1 oval,2 tall,3 conic,4 slender; leer = Alt-Bake`)
- Modify: `scripts/geo/bake-world.mjs` (`treesNum`-Mapping ~Zeile 349: family-String → Code; `assignToTiles` reicht durch — prüfen, wo Trees serialisiert werden, Feld ergänzen)
- Modify: `src/diorama/ksw/geo/worldData.ts` oder Verbrauchsstelle: Decoder-Helper `tileTreeSpecs(tile: WorldTile): TreeSpec[]` (x/z/h/r/kind/family; family nur wenn `t_family.length > 0`)
- Test: `tests/geo/worldProto.test.ts` erweitern (Roundtrip mit t_family) + `tests/geo/tileTreeSpecs.test.ts`
- Daten: voller Welt-Rebake (gitignored) — `npm run geo:bake-world` (braucht scratch; Symlinks liegen in diesem Worktree, sonst aus run-app/kind-bose verlinken)

**Interfaces:**
- Produces: `FAMILY_CODES = ['spreading','oval','tall','conic','slender'] as const` (Export in tileContent.ts ODER treeArchetypes.ts — WICHTIG: identisch zur Reihenfolge `[...BROAD_FAMILIES, ...CONIFER_FAMILIES]`, per Test verankert); `tileTreeSpecs(tile) -> TreeSpec[]` für Task 5.
- Rust-Check: `grep -rn "WorldTile\|t_family" backend/crates --include='*.rs'` — wenn das Backend WorldTile dekodiert, `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p <crate>` zur Prost-Regeneration + Test; proto3-additiv bricht nichts.

- [ ] **Step 1: Failing Proto-Roundtrip-Test** (worldProto.test.ts: Tile mit `tFamily: [0,3]` bauen → toBinary → fromBinary → erwartet identisch; PLUS Ordnungs-Anker-Test: `FAMILY_CODES` === `[...BROAD_FAMILIES, ...CONIFER_FAMILIES]`).
- [ ] **Step 2: FAIL** (Feld existiert nicht) → proto ändern → `npm run generate:proto` → **Step 3: grün**.
- [ ] **Step 4: bake-world.mjs**: `treesNum = nature.trees.map(t => ({ …, family: FAMILY_CODES.indexOf(t.family) }))` mit Fail-fast bei unbekannter family (`indexOf === -1 → throw`); Serialisierung des Feldes an der Stelle, wo t_kind geschrieben wird (im Script suchen: `tKind`).
- [ ] **Step 5: `tileTreeSpecs`-Decoder + Test** (leeres t_family → family undefined; gefüllt → korrekt gemappt; kind aus t_kind bleibt massgeblich für den Konsistenz-Check `['conic','slender'].includes(family) === (kind==='conifer')` — bei Widerspruch kind aus family ableiten und zählen/loggen).
- [ ] **Step 6: Welt-Rebake laufen lassen** (`npm run geo:bake-world`, dauert Minuten, --max-old-space-size steht im npm-Script). Danach Stichprobe: Koniferen-Anteil in Wald-Tiles > 0 (node-Einzeiler über 3 Tiles), Gesamt-Baumzahl ≥ 350k. Pyramide ist gitignored — NICHTS davon committen; `git status` sauber ausser Code.
- [ ] **Step 7: Voller `npx vitest run tests/geo/` + typecheck + Commit**: `feat(m3): t_family im Tile-Proto + bake-world schreibt Familien (Aussenwälder erben den Familie-zuerst-Fix)`

### Task 4: `tileContent.ts` — Materialisierung + Dispose + Platten-Exclusion

**Files:**
- Create: `src/diorama/ksw/geo/tileContent.ts`
- Modify: `src/diorama/ksw/geo/terrain.ts` (extrahiere `buildTerrainTileMesh(dec: DecodedTile): THREE.Mesh` aus der Schleife von `buildTerrainTiles` — `buildTerrainTiles` ruft sie fortan auf; Verhalten identisch, bestehende terrain-Tests grün)
- Test: `tests/geo/tileContent.test.ts`

**Interfaces:**
- Consumes: `buildTerrainTileMesh` (neu), `mergeTinted`/`tintedClay` aus cityMassing.ts (Massing-Prismen aus Footprints — die Tile-Gebäude haben KEINE Mesh-Verts: Prisma = Footprint-Ring extrudiert auf bHeight, wie `ringBandParts`+Deckel; konkret die einfachste vorhandene Prisma-Route in cityMassing nachschlagen und wiederverwenden statt neu erfinden), `tileTreeSpecs` (Task 3).
- Produces (Task 5/6 verlassen sich hierauf):
  - `type TileContent = { key: TileKey; group: THREE.Group; treeKey: string | null; dispose(): void }`
  - `materializeTile(dec: DecodedTile, ctx: MaterializeCtx): TileContent` mit `type MaterializeCtx = { plateRect: { x: number; z: number; w: number; d: number } | null; groundShiftY: number; buildings: boolean; trees: boolean }` — `buildings`/`trees` steuern L2 (beides) vs L1 (nur trees→Impostors, Task 5) vs L0 (beides false).
  - Gebäude/Bäume, deren (x,z) im `plateRect` liegen, werden übersprungen (`insideRect`-Prädikat wie treeLayer.ts:138).
  - `dispose()` disposed Geometrien, NICHT die geteilten Materialien (Materialien aus tintedClay-Cache).

- [ ] **Step 1: Failing Tests** (ohne WebGPU — three-Objekte sind konstruierbar; Tests prüfen Struktur):

```ts
import { describe, expect, it } from 'vitest';
import { materializeTile } from '../../src/diorama/ksw/geo/tileContent';
// Fixture: minimaler DecodedTile mit 2 Gebäuden (eines im plateRect) und
// 3 Bäumen (einer im plateRect) — WorldTile via create(WorldTileSchema, {...})
// bauen (Muster: tests/geo/worldProto.test.ts).

it('überspringt Platten-Inhalt und baut den Rest', () => {
  const content = materializeTile(dec, { plateRect: { x: 0, z: 0, w: 100, d: 100 }, groundShiftY: 0, buildings: true, trees: true });
  const meshes: string[] = [];
  content.group.traverse((o) => { if ((o as { isMesh?: boolean }).isMesh) meshes.push(o.name); });
  expect(meshes.some((n) => n.startsWith('tileTerrain'))).toBe(true);
  expect(meshes.some((n) => n.startsWith('tileBuildings'))).toBe(true);
  // Gebäude-Zählung über eine exponierte Debug-Zahl:
  expect(content.group.userData.buildingCount).toBe(1); // 1 von 2 (Platte gefiltert)
});

it('dispose gibt Geometrien frei (dispose-Spy) und lässt geteilte Materialien leben', () => { /* vi.spyOn(geometry,'dispose') über traverse */ });

it('buildings:false lässt Gebäude weg (L1-Modus)', () => { /* … */ });
```

- [ ] **Step 2: FAIL** → **Step 3: Implementation** (Terrain-Mesh + ein gemergtes Prisma-Mesh pro Tile via mergeTinted-Vokabular; `group.userData.buildingCount`/`treeCount` als Test-/Smoke-Oberfläche; Bäume werden NICHT hier instanziert — `materializeTile` liefert die gefilterten `TreeSpec[]` an Task 5 weiter: ergänze ins Produces: `content.trees: TreeSpec[]`).
- [ ] **Step 4: Tests + typecheck + bestehende terrain-Tests grün** → **Step 5: Commit** `feat(m3): per-Tile-Materialisierung mit Platten-Exclusion + sauberem Dispose`

### Task 5: treeLayer-Pools + per-Tile-Impostors (Regressions-sensibel!)

**Files:**
- Modify: `src/diorama/ksw/geo/treeLayer.ts` (`addTileTrees(key: string, specs: TreeSpec[]): void`, `removeTileTrees(key: string): void` auf `TreeLayer`; interne Pools pro Tile-Key; `instances`/Grid/compactNear über alle Pools)
- Modify: `src/diorama/ksw/geo/treeImpostors.ts` (`buildImpostorMeshFor(instances: TreeInstance[], atlas, archCount): THREE.InstancedMesh` — heutige `buildImpostorMesh` wird zum Aufrufer für den Boot-Bestand; per-Tile-Meshes nutzen dieselbe Funktion)
- Test: `tests/diorama/treeLayerPools.test.ts`

**Interfaces:**
- Consumes: `TreeSpec[]` aus Task 4 (`content.trees`), Atlas/archetypes wie heute (main.ts baut den Atlas einmal beim Boot — per-Tile-Meshes referenzieren dieselbe Textur).
- Produces: obige zwei Methoden + `TreeLayer.tileKeys(): string[]` (Smoke-Assertion). Kapazitäts-Kontrakt: Full-Detail-Meshes bleiben global (Kompaktierung zieht aus ALLEN Pools die nahen); nur Impostor-Quads sind per-Tile.
- HARTE Regressions-Gates: `npx vitest run tests/diorama/` grün; `node scripts/smoke-trees.mjs` SMOKE OK; ein Handoff-Re-Capture (scripts/capture-trees.mjs, Szene handoff) visuell unverändert — die #136-Task-6-Fixes (positionGeometry, Atlas, Tint-Ratio) dürfen NICHT angefasst werden; wenn eine Änderung dort nötig scheint: STOPP, BLOCKED melden.

- [ ] **Step 1: Failing Tests** (Pools: add → instances wachsen deterministisch; remove → Impostor-Mesh disposed + instances schrumpfen; doppelter add auf gleichen Key wirft; compactNear nach add/remove konsistent — Zähl-Assertions über fullMeshes-counts).
- [ ] **Step 2: FAIL** → **Step 3: Implementation** → **Step 4: alle Gates aus „Produces" grün** → **Step 5: Commit** `feat(m3): treeLayer-Tile-Pools + per-Tile-Impostor-Meshes`

### Task 6: Boot-Integration in main.ts

**Files:**
- Modify: `src/diorama/ksw/main.ts` (~Zeile 596-660: statt `loadWorld()` alles-laden → L0 via `loadWorld(base, ref => ref.path.startsWith('tiles/L0'))`; TileMeta-Liste aus `manifest.tiles` (Center aus Tile-Grid: `originX + gridN·cellSize/2` — beim Fetch des Manifests sind die Tiles noch nicht da → Bounds müssen aus TileRef kommen; TileRef-Felder prüfen und ggf. Task-1-Anmerkung nutzen: die Bounds-Liste wird hier aus einem einmaligen leichten HEAD-Load…; PRAGMATISCH: `manifest`-Proto prüfen — wenn TileRef keine Bounds hat, erweitere das Manifest-Proto in Task 3 gleich mit `repeated double tile_cx/tile_cz` (bake-world schreibt sie) statt zur Laufzeit zu raten); `TileStreamer` mit `fetchTile = fetchTileBin+decodeTileBin`, `onReady = materializeTile + addTileTrees + terrainRoot.add`, `onUnload = dispose + removeTileTrees`; `streamer.update(camera.position.x, camera.position.z)` throttled ~2 Hz im bestehenden compactNear-Rhythmus (Anker: treeLayer.compactNear-Aufruf in animate); `__LOOK_READY` erst wenn L0 steht UND der initiale Nahring leer-gefetcht ist (`liveCount` stabil ODER Queue leer))
- Modify: `src/diorama/ksw/geo/lod.ts`/Aufrufer nur falls `getObjectByName('terrainL2/…')`-Kopplungen existieren (grep!) — Namen bleiben stabil via buildTerrainTileMesh.
- Test: kein neuer Unit-Test (Integration) — Gate ist Task 7.

- [ ] **Step 1:** Grep-Vorflug: `grep -rn "world.tiles\|buildTerrainTiles\|loadWorld" src/` — alle Aufrufstellen listen und im Report dokumentieren.
- [ ] **Step 2:** Umbau wie oben; L1-Terrain unter aktiven L2-Tiles: `visible=false` über eine Kinder-Lookup-Map (`terrainByKey: Map<TileKey, THREE.Mesh>`), Regel: L2-Key `L2/x_y` deckt L1-Key `L1/${x>>2}_${y>>2}` ab — Faktor aus `gridN·cellSize`-Verhältnis der Levels VERIFIZIEREN (nicht raten; im Manifest/Tile nachsehen, 16 L1 / 256 L2 ⇒ 4×4 L2 pro L1 ⇒ >>2 stimmt nur bei 4er-Teilung).
- [ ] **Step 3:** `npm run typecheck` + `npm test` + `npm run build` grün; Boot-Probe: `node scripts/capture-ksw.mjs stream-boot morning city` → CAPTURE OK.
- [ ] **Step 4: Commit** `feat(m3): kamera-getriebenes Tile-Streaming im Boot- und Frame-Pfad`

### Task 7: Fly-Through-Smoke + Screenshots + Perf

**Files:**
- Create: `scripts/smoke-streaming.mjs` (Muster: scripts/smoke-trees.mjs für Flags/READY-Gate; Ports env-übersteuerbar wie SMOKE_TRAFFIC_PORT-Konvention)
- Verify: Screenshots `scratch/streaming/{horizon,edge,forest}.png`

**Assertions (alle PASS/FAIL geloggt, Exit ≠ 0 bei FAIL):**
1. Boot: `__LOOK_READY` < 30 s; initiale Live-Tile-Zahl > 0 und < 80.
2. Fly-Through Zentrum → [4000, 0] → [1500, 3000] → zurück (je via `__traffic.lookAt`, 3 s Settle): Live-Tile-Menge ÄNDERT sich pro Etappe (Zähler-Hook: `window.__stream = { live: () => streamer.liveCount, disposed: () => n }` in main.ts, Task 6 legt ihn an — in Task 6 „Produces" ergänzen), `disposed() > 0` nach der Rückkehr.
3. Keine unbehandelten Fetch-Fehler (pageerror-Listener + `streamer.failed.size === 0`).
4. fps-Probe (rAF 5 s) an der Stadtrand-Etappe ≥ 85; vor Messung `pgrep Chromium`-Orphan-Check (Lesson #136).
5. Horizont-Screenshot (r3000 über Zentrum): Gebäude-Massing im Mittelfeld sichtbar (manueller Bild-Check im Report, PNG angehängt).
- [ ] Smoke schreiben → laufen lassen → PASS; `smoke-trees.mjs` + `smoke-traffic.mjs` (Ports 8793/5186-Konvention) ebenfalls SMOKE OK.
- [ ] Commit `test(m3): Fly-Through-Streaming-Smoke + Debug-Hooks`

### Task 8: Gate + PR (Controller)

- [ ] Voll-Gate (typecheck, vitest komplett, build; Rust nur falls Task 3 Rust berührte — dann fmt/clippy/test via cargo-serial).
- [ ] PR mit Vorher/Nachher (Horizont), CI ALLE Checks PASS, Squash-Merge, Branch-Cleanup, Memory-Update.
