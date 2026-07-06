# SOTA Trees Slice 2 (Fallback) + Look-Defekte Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Artenvielfalt + realistische Grössenspreizung für alle Plate-Bäume aus Landuse-Heuristik (User-Entscheid 2026-07-06: OHNE Baumkataster — offizieller WFS ist 401-geschützt, Drittanbieter-DEMO abgelehnt), plus Fix der vier diagnostizierten Look-Defekte: Impostor-Farb-Mismatch, Bäume-in-Fassaden, Pop-in-Härte, Tiny-Crown-Skelette.

**Architecture:** Bake-seitig wächst `TreeSpec` um `family` (deterministisch aus umgebendem Landuse + Hash) und Grössen aus Familien-Wachstumskurven mit Pseudo-Alter-Spreizung. Render-seitig wird `archetypeIndexFor` familien-bewusst, Bäume in Gebäude-Footprints werden im Bake gefiltert, und die Impostor-Quads erhalten einen Licht-Kalibrier-Uniform aus dem Env-System. Plate-scoped (`nature.json`-Rebake); Tile-Format-Extension/Welt-Rebake bleibt explizit deferred (Spec §6).

**Tech Stack:** plain-ESM bake libs (`scripts/geo/lib/`), three/webgpu + TSL, vitest.

## Global Constraints

- Determinismus: gleiche Inputs → byte-identische `nature.json`; alle Zufälligkeit über `h01`/`hash01`-Koordinaten-Hashes (Spec §1, bestehende Konvention).
- Perf-Budget: 100–120 fps mit 10k-Pipeline; Baumzahl ≥ aktueller Bake (Spec §5).
- Browser-Smoke ist Pflicht vor "fertig" (CLAUDE.md); Screenshots als Beweis.
- cargo nur via `scripts/cargo-serial.sh` (hier nicht erwartet — reine Frontend/Bake-Arbeit).
- Keine neuen Archetyp-Familien (YAGNI): nur die 5 bestehenden (`spreading`, `oval`, `tall`, `conic`, `slender`) aus `treeArchetypes.ts`.
- OSM-Positionen sind Ground Truth — Bäume werden NIE verschoben, nur gefiltert (Footprint-Konflikt) oder attributiert.

---

### Task 1: Familien-Heuristik + Wachstumskurven im Bake (`treeSpec`)

**Files:**
- Modify: `scripts/geo/lib/style.mjs` (treeSpec, forestFill, neue Tabellen)
- Test: `tests/geo/treeFamilies.test.ts` (neu)

**Interfaces:**
- Produces: `treeSpec(tags, x, z, context?) -> {x,z,kind,family,h,r}`; `familyFor(x, z, kind, context) -> family`; `sizeFor(family, x, z) -> {h,r}`; `forestFill(ring, existing, density?, context?)` reicht `context` durch. `context = { green?: 'wood'|'forest'|'park'|'garden'|'grass'|... , leafType?: 'needleleaved'|'broadleaved'|'mixed' }`.
- Consumes: bestehendes `h01(a,b)` (style.mjs) und `TREE_DEFAULTS`.

- [ ] **Step 1: Failing Test schreiben** — `tests/geo/treeFamilies.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
// @ts-ignore — plain-ESM bake lib
import { familyFor, sizeFor, treeSpec, GROWTH } from '../../scripts/geo/lib/style.mjs';

describe('familyFor', () => {
  it('Nadelwald → überwiegend conic', () => {
    let conic = 0;
    for (let i = 0; i < 200; i++) {
      const f = familyFor(i * 13.7, i * 7.1, 'conifer', { green: 'wood', leafType: 'needleleaved' });
      if (f === 'conic') conic++;
      expect(['conic', 'slender']).toContain(f);
    }
    expect(conic).toBeGreaterThan(120); // ~70%
  });
  it('Laubwald → oval/spreading/tall, nie conifer-Familien', () => {
    for (let i = 0; i < 100; i++) {
      expect(['oval', 'spreading', 'tall']).toContain(
        familyFor(i * 3.3, i * 9.9, 'broad', { green: 'forest', leafType: 'broadleaved' }),
      );
    }
  });
  it('deterministisch: gleiche Koordinate → gleiche Familie', () => {
    expect(familyFor(101.5, -33.25, 'broad', {})).toBe(familyFor(101.5, -33.25, 'broad', {}));
  });
});

describe('sizeFor', () => {
  it('liefert Spreizung statt Uniform-Defaults (≥3 m Höhen-Spanne über 100 Bäume)', () => {
    const hs = [];
    for (let i = 0; i < 100; i++) hs.push(sizeFor('oval', i * 17.3, i * 5.7).h);
    expect(Math.max(...hs) - Math.min(...hs)).toBeGreaterThan(3);
  });
  it('bleibt unter h∞ und über Sapling-Minimum', () => {
    for (let i = 0; i < 100; i++) {
      const { h, r } = sizeFor('spreading', i * 7.7, i * 3.1);
      expect(h).toBeGreaterThan(3);
      expect(h).toBeLessThan(GROWTH.spreading.hInf);
      expect(r).toBeGreaterThan(1);
    }
  });
});

describe('treeSpec', () => {
  it('explizite OSM-Tags gewinnen weiterhin', () => {
    const s = treeSpec({ height: '17', diameter_crown: '8' }, 5, 5, { green: 'park' });
    expect(s.h).toBe(17);
    expect(s.r).toBe(4);
  });
  it('trägt family', () => {
    expect(treeSpec({}, 5, 5, { green: 'park' }).family).toBeDefined();
  });
});
```

- [ ] **Step 2: Test laufen lassen — erwartet FAIL** (`familyFor` existiert nicht):
`npx vitest run tests/geo/treeFamilies.test.ts`

- [ ] **Step 3: Implementation in `style.mjs`** — nach `TREE_DEFAULTS` einfügen:

```js
// Slice 2 (Fallback-Variante): Familien + Grössen aus Landuse-Heuristik.
// Wachstumskurven h(age) = h∞ · age/(age + t½) (saturierend, Spec §1) mit
// deterministischem Pseudo-Alter, da ohne Kataster kein Pflanzjahr existiert.
// Konstanten: grobe mitteleuropäische Stadtbaum-/Waldbaum-Endgrössen
// (Platane/Linde/Pappel/Fichte-Klassen), dokumentiert hier als die "einmal
// recherchierte Tabelle" der Spec.
export const GROWTH = {
  spreading: { hInf: 28, rInf: 9.0, tHalf: 35 },
  oval:      { hInf: 30, rInf: 7.0, tHalf: 40 },
  tall:      { hInf: 32, rInf: 5.0, tHalf: 25 },
  conic:     { hInf: 34, rInf: 4.5, tHalf: 45 },
  slender:   { hInf: 20, rInf: 3.0, tHalf: 30 },
};

// Gewichtete Familienwahl je Kontext. Reihenfolge/Schwellen deterministisch.
const FAMILY_MIX = {
  needlewood:  [['conic', 0.7], ['slender', 1.0]],
  broadwood:   [['oval', 0.6], ['spreading', 0.9], ['tall', 1.0]],
  mixedwood:   [['oval', 0.4], ['spreading', 0.6], ['conic', 0.9], ['slender', 1.0]],
  park:        [['spreading', 0.4], ['oval', 0.8], ['tall', 1.0]],
  street:      [['spreading', 0.5], ['oval', 1.0]],
};

export function familyFor(x, z, kind, context = {}) {
  const inWood = context.green === 'wood' || context.green === 'forest';
  let mix;
  if (inWood) {
    if (context.leafType === 'needleleaved') mix = FAMILY_MIX.needlewood;
    else if (context.leafType === 'broadleaved') mix = FAMILY_MIX.broadwood;
    else mix = FAMILY_MIX.mixedwood;
  } else if (context.green) {
    mix = FAMILY_MIX.park;
  } else {
    mix = FAMILY_MIX.street;
  }
  // kind ist OSM-Ground-Truth: conifer erzwingt Nadel-Familien, broad Laub.
  const allowed = kind === 'conifer' ? ['conic', 'slender'] : ['spreading', 'oval', 'tall'];
  const filtered = mix.filter(([f]) => allowed.includes(f));
  const pool = filtered.length ? filtered : allowed.map((f, i) => [f, (i + 1) / allowed.length]);
  const hv = h01(x * 12.9898 + 4.1414, z * 78.233);
  const top = pool[pool.length - 1][1];
  for (const [f, cum] of pool) if (hv <= cum / top) return f;
  return pool[pool.length - 1][0];
}

export function sizeFor(family, x, z) {
  const g = GROWTH[family];
  // Pseudo-Alter 8..88 Jahre, quadratisch Richtung jung geschoben (Städte
  // pflanzen nach) — deterministisch pro Koordinate.
  const a01 = h01(x * 3.7 + 11.1, z * 9.3 - 2.2);
  const age = 8 + 80 * a01 * a01;
  const grow = age / (age + g.tHalf);
  const jitter = 0.9 + 0.2 * h01(x * 5.1, z * 13.7);
  return {
    h: Math.round(g.hInf * grow * jitter * 10) / 10,
    r: Math.round(g.rInf * grow * (0.9 + 0.2 * h01(x * 7.9, z * 3.3)) * 10) / 10,
  };
}
```

`treeSpec` ersetzen (Signatur wächst um `context`, OSM-Tags gewinnen weiterhin):

```js
export function treeSpec(tags, x, z, context = {}) {
  const kind = tags.leaf_type === 'needleleaved' ? 'conifer' : 'broad';
  const family = familyFor(x, z, kind, context);
  const sized = sizeFor(family, x, z);
  const tagH = Number.parseFloat(tags.height ?? '');
  const tagCrown = Number.parseFloat(tags.diameter_crown ?? '');
  return {
    x, z, kind, family,
    h: tagH > 0 ? tagH : sized.h,
    // Tiny-Crown-Guard (Skelett-Defekt): Krone nie unter 30% der Familien-
    // Erwartung — ein OSM diameter_crown=0.5 erzeugte trunk-only-Skelette.
    r: tagCrown > 0 ? Math.max(tagCrown / 2, sized.r * 0.3) : sized.r,
  };
}
```

In `forestFill` die Kandidaten-Zeile ersetzen (Familie+Grösse statt Uniform-Broad; `context` als 4. Parameter mit `{green:'wood', leafType}` vom Aufrufer):

```js
export function forestFill(ring, existingTrees, density = 1 / 60, context = { green: 'wood' }) {
  // … (Grid/Exclude unverändert) …
      const family = familyFor(x, z, context.leafType === 'needleleaved' ? 'conifer' : 'broad', context);
      const kind = ['conic', 'slender'].includes(family) ? 'conifer' : 'broad';
      const sized = sizeFor(family, x, z);
      out.push({ x: Math.round(x * 100) / 100, z: Math.round(z * 100) / 100, kind, family, h: sized.h, r: sized.r });
```

- [ ] **Step 4: Tests grün**: `npx vitest run tests/geo/treeFamilies.test.ts` → PASS
- [ ] **Step 5: Commit**: `git add scripts/geo/lib/style.mjs tests/geo/treeFamilies.test.ts && git commit -m "feat(trees): Landuse-Familien-Heuristik + Wachstumskurven im Bake (Slice-2-Fallback)"`

### Task 2: transformNature verdrahten + Footprint-Exklusion + nature.json rebaken

**Files:**
- Modify: `scripts/geo/lib/transform.mjs` (`transformNature`: greens-Kontext an treeSpec/forestFill; neuer Param `buildings` für Footprint-Filter)
- Modify: `scripts/geo/bake-winterthur.mjs` (buildings an transformNature übergeben)
- Test: `tests/geo/landuse.test.ts` erweitern
- Output: `data/winterthur/nature.json` (Rebake, committed)

**Interfaces:**
- Produces: `transformNature({ osmNature, projector, buildingFootprints? })`; Trees tragen `family`; Bäume in Footprint+1 m Marge werden gedroppt und gezählt geloggt.
- Consumes: Task 1 (`treeSpec(tags,x,z,context)`, `forestFill(ring,existing,density,context)`), bestehendes `pointInRing`.

- [ ] **Step 1: Failing Test** (in `tests/geo/landuse.test.ts` anhängen):

```ts
describe('transformNature Slice-2', () => {
  it('Baum im Gebäude-Footprint (+1 m) wird gedroppt', () => {
    const osmNature = { elements: [
      { type: 'node', lat: 0.00001, lon: 0.00001, tags: { natural: 'tree' } },
    ]};
    const projector = { toLocal: (lon: number, lat: number) => [lon * 111320, -lat * 111320] };
    const fp = [[0, 0], [5, 0], [5, -5], [0, -5]]; // enthält den Baum (~1.1, -1.1)
    const { trees } = transformNature({ osmNature, projector, buildingFootprints: [fp] });
    expect(trees.length).toBe(0);
  });
  it('Baum im Park erhält family', () => {
    const osmNature = { elements: [
      { type: 'way', tags: { leisure: 'park' }, geometry: [
        { lon: 0, lat: 0 }, { lon: 0.001, lat: 0 }, { lon: 0.001, lat: -0.001 }, { lon: 0, lat: -0.001 }, { lon: 0, lat: 0 } ] },
      { type: 'node', lat: -0.0005, lon: 0.0005, tags: { natural: 'tree' } },
    ]};
    const projector = { toLocal: (lon: number, lat: number) => [lon * 111320, -lat * 111320] };
    const { trees } = transformNature({ osmNature, projector });
    expect(trees.length).toBe(1);
    expect(['spreading', 'oval', 'tall']).toContain(trees[0].family);
  });
});
```

- [ ] **Step 2: FAIL verifizieren** (`buildingFootprints` unbekannt / family fehlt).
- [ ] **Step 3: Implementation** — in `transformNature`:
  1. Greens ZUERST sammeln (passiert schon), dann für jeden Einzelbaum `context` bestimmen: erstes Green dessen Ring den Punkt enthält (`pointInRing`), `leafType` aus dessen `leaf_type`-Tag; Aufruf `treeSpec(t.tags, x, z, context)`.
  2. `forestFill(g.ring, trees, undefined, { green: g.kind, leafType: g.leafType })`.
  3. Footprint-Filter am Ende: 8 m-Spatial-Hash über `buildingFootprints`; Baum gedroppt wenn `pointInRing` in einem Footprint (Ring um 1 m nach aussen gepuffert via Vertex-Normalen ODER einfacher: `pointInRing` + Distanz-zu-Kante < 1). Gedroppte zählen: `console.log('nature: dropped N trees inside building footprints')`.
  4. `bake-winterthur.mjs`: `transformNature({ …, buildingFootprints: buildingsOut.map(b => b.footprint) })` — Zeile ~90, buildings werden vor nature gebakt.
- [ ] **Step 4: Tests grün** + `npx vitest run tests/geo/` komplett.
- [ ] **Step 5: Rebake**: `node scripts/geo/bake-winterthur.mjs` (scratch-Symlinks: `osm-landuse.json` ✓, prüfe `osm-nature`/Inputs im Script-Kopf; NUR `nature.json` committen — buildings/roads/meta per `git checkout --` zurücksetzen, roads.json ist seit #133 gemeindeweit und darf NICHT vom Plate-Bake überschrieben werden!). Guard: Tree-Count ≥ 3000 (existiert), Familien-Verteilung loggen.
- [ ] **Step 6: Commit** (`nature.json` + transform/bake): `git commit -m "feat(trees): family-attributierte Plate-Bäume + Footprint-Exklusion (nature.json rebaked)"`

### Task 3: Renderer familien-bewusst (`archetypeIndexFor`) 

**Files:**
- Modify: `src/diorama/ksw/geo/treeArchetypes.ts` (`archetypeIndexFor(x, z, kind, family?)`)
- Modify: `src/diorama/ksw/geo/geoData.ts` (`TreeSpec` += `family?: TreeFamily`)
- Modify: `src/diorama/ksw/geo/treeLayer.ts` (Aufrufstelle reicht `spec.family` durch)
- Test: `tests/geo/treeArchetypes.test.ts` (existierende Datei erweitern; sonst neu)

**Interfaces:**
- Produces: `archetypeIndexFor(x: number, z: number, kind: 'broad'|'conifer', family?: TreeFamily): number` — mit family: deterministischer Seed-Pick innerhalb der Familie; ohne: bisheriges Verhalten (Rückwärtskompatibel für Alt-Daten/Tests).
- Consumes: `BROAD_FAMILIES`, `CONIFER_FAMILIES`, `SEEDS_PER_FAMILY`, `hash01`.

- [ ] **Step 1: Failing Test**:

```ts
it('family-pick landet im Familien-Block', () => {
  for (let i = 0; i < 50; i++) {
    const idx = archetypeIndexFor(i * 3.1, i * 7.7, 'broad', 'oval');
    const ovalStart = BROAD_FAMILIES.indexOf('oval') * SEEDS_PER_FAMILY;
    expect(idx).toBeGreaterThanOrEqual(ovalStart);
    expect(idx).toBeLessThan(ovalStart + SEEDS_PER_FAMILY);
  }
});
it('ohne family: bisheriges Verhalten (Bereichs-Check)', () => {
  expect(archetypeIndexFor(10, 10, 'conifer')).toBeGreaterThanOrEqual(BROAD_FAMILIES.length * SEEDS_PER_FAMILY);
});
```

- [ ] **Step 2: FAIL** → **Step 3: Implementation**:

```ts
export function archetypeIndexFor(x: number, z: number, kind: 'broad' | 'conifer', family?: TreeFamily): number {
  const h = hash01(Math.round(x * 8) * 92837111 + Math.round(z * 8) * 689287499);
  const broadN = BROAD_FAMILIES.length * SEEDS_PER_FAMILY;
  if (family) {
    const bi = (BROAD_FAMILIES as readonly TreeFamily[]).indexOf(family);
    if (bi >= 0) return bi * SEEDS_PER_FAMILY + Math.floor(h * SEEDS_PER_FAMILY);
    const ci = (CONIFER_FAMILIES as readonly TreeFamily[]).indexOf(family);
    if (ci >= 0) return broadN + ci * SEEDS_PER_FAMILY + Math.floor(h * SEEDS_PER_FAMILY);
  }
  const conifN = CONIFER_FAMILIES.length * SEEDS_PER_FAMILY;
  return kind === 'broad' ? Math.floor(h * broadN) : broadN + Math.floor(h * conifN);
}
```

`treeLayer.ts` Aufrufstelle: `archetypeIndexFor(spec.x, spec.z, spec.kind, spec.family)`.
- [ ] **Step 4: Tests grün + typecheck** → **Step 5: Commit** `feat(trees): familien-bewusste Archetyp-Zuordnung`

### Task 4: Impostor-Licht-Kalibrierung (Farb-Mismatch)

**Files:**
- Modify: `src/diorama/ksw/geo/treeImpostors.ts` (Draw-Material: `impostorLightU`-Uniform in colorNode)
- Modify: `src/diorama/ksw/main.ts` (Env-Tick speist Uniform aus Sonnen-/Himmel-Zustand)
- Verify: Screenshot-Handoff (kein Unit-Test — visuelles Artefakt; Abnahme in Task 6)

**Interfaces:**
- Produces: `export const impostorLightU = uniform(new THREE.Color(1,1,1))` (treeImpostors.ts); main.ts setzt pro Env-Update `impostorLightU.value.copy(sunColor).multiplyScalar(sunIntensityFactor).add(ambientColor)` — konkrete Quelle: dieselben Werte, mit denen der Env-Code `DirectionalLight`/Hemisphere speist (in `applyCityEnvironment`/Env-Tick nachschlagen, dort wo `sun.intensity`/`hemi` gesetzt werden).
- Consumes: bestehendes Env-Uniform-Muster (`windAmpU` analog).

- [ ] **Step 1:** `impostorLightU` deklarieren + `material.colorNode = sampled.rgb.mul(aTint).mul(uniform(impostorLightU))` (TSL `uniform(color)`), Default weiss = Verhalten unverändert.
- [ ] **Step 2:** main.ts: im selben Block, der Sonne/Hemi pro Env-Tick setzt, Uniform aktualisieren; Normierung so wählen, dass Mittags-Klarwetter ≈ Ist-Zustand der vollen Bäume (Startwert: `sunColor*0.6 + hemiSky*0.4`, dann Task 6 kalibriert am Screenshot).
- [ ] **Step 3:** typecheck + vitest komplett grün.
- [ ] **Step 4: Commit** `fix(trees): Impostor-Quads folgen dem Szenenlicht (Farb-Mismatch am 150m-Handoff)`

### Task 5: Gate + Rebake-Konsistenz

- [ ] `npm run typecheck` grün; `npm test` grün; `npm run build` grün.
- [ ] `pgrep -f cargo` leer (kein Rust berührt).
- [ ] Commit-Stand sauber (`git status`).

### Task 6: Slice-3-Polish-Loop (Screenshot-getrieben, Abnahme-Schritt)

**Files:**
- Create: `scripts/capture-trees.mjs` (4 Szenen: Establishing r600 Park, Strassen-Allee r80, Waldrand r150, Handoff-Ring r160 quer über die 150m-Grenze; Muster von `capture-traffic.mjs` mit `__traffic.lookAt`, Ports env-übersteuerbar)
- Verify: Screenshots iterativ; Ziel-Kriterien unten

**Abnahme-Kriterien (jede Szene als PNG belegt):**
- Handoff-Shot: Impostor- und Voll-Bäume farblich nicht unterscheidbar (kein Mint-vs-Dunkelgrün-Sprung).
- Allee-Shot: sichtbare Formen-Varianz (≥3 Silhouetten unterscheidbar), keine Bäume auf Fahrbahn, keine in Fassaden.
- Waldrand: Nadel/Laub-Mix gemäss leaf_type, Grössenspreizung sichtbar.
- Kein nacktes Skelett in keinem Shot (Tiny-Crown-Guard wirkt); falls doch: Instanz einkreisen (aTint-Debug-Färbung) und Root Cause VOR Weiterarbeit klären (systematic-debugging).
- Perf-Probe: fps-Messung (rAF-Zähler 5 s) ≥ Ist-Stand (88/113 gemessen 2026-07-06).
- [ ] Iterieren bis Kriterien erfüllt; jede Iteration = gezielte Konstanten-Änderung + Re-Capture (KEINE Shotgun-Änderungen).
- [ ] Commit `polish(trees): Slice-3-Screenshot-Loop — [konkrete Änderungen]`

### Task 7: Browser-Smoke + PR

- [ ] `SMOKE_TRAFFIC_PORT=8792 SMOKE_VITE_PORT=5187 node scripts/smoke-traffic.mjs` → SMOKE OK (Regressionsschutz Traffic).
- [ ] Falls `scripts/smoke-trees.mjs` existiert: laufen lassen; Assertions ggf. an family-API anpassen.
- [ ] TEMP-DEBUG `__dbgScene`-Hook aus main.ts entfernen (Session-Forensik-Rest).
- [ ] PR gegen main: Diagnose-Zusammenfassung + Vorher/Nachher-Screenshots; CI ALLE Checks PASS abwarten; Squash-Merge; Branch löschen; Memory aktualisieren.
