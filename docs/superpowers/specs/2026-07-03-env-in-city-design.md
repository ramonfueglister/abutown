# Echtzeit-Wetter & echte Sonnenzeiten in der Winterthur-Stadt-App (Design)

**Datum:** 2026-07-03
**Status:** Design aus Brainstorming, bereit zur Review
**Branch:** `klinik/env-in-city` (ab origin/main `318cfd0`)
**Vorgänger-Spec:** `2026-07-03-echtzeit-wetter-sonne-design.md` (Wetter-Feature im Look-Prototyp, gemergt als PR #115 → `klinik/look-prototype` @ `981064d`)

## Ziel in einem Satz

Die Winterthur-Stadt-App (`/`, `src/diorama/ksw/`) übernimmt das komplette Echtzeit-Environment des Prototyps — echte Sonne/Mond/Sterne + Live-Wetter über der **echten Karte**: volle Parität inkl. Niederschlag, `?preset=`/`?cycle=` → `?at=`/`?wx=`.

## Entscheidungen aus dem Brainstorming

| Frage | Entscheid |
|---|---|
| Scope | **Volle Parität**: Sonne/Mond/Sterne, Live-Bewölkung auf die Stadt-Wolkenkuppel, Wind-Drift, Hochnebel-Fog, Regen/Schnee-Partikel. |
| Ansatz | **A — Branch-Merge, dann Stadt-Verdrahtung** (ein Modul, eine Historie; kein Copy-Fork). |

## Schlüssel-Fakt (verifiziert)

Die Stadt-Geodaten nutzen dieselbe Szenen-Konvention wie `solar.ts`: **+x = Ost, +z = Süd, +y = oben** (`scripts/geo/lib/project.mjs`). Echte Sonnenazimute werfen damit ohne Anpassung geometrisch korrekte Schatten über die echte Karte.

## Phase 1 — Integrations-Merge

`git merge klinik/look-prototype` auf `klinik/env-in-city`, Konfliktauflösung:

- `src/diorama/look.ts` → **Wetter-Version übernehmen** (main hält byte-identisch den alten Prototyp-Stand `3e24f87`; nichts geht verloren).
- `src/diorama/designTokens.ts` → **Vereinigung**: Wetter-Seite (`envKeyframes`, `envAnchors`, `weatherLook`, `cloudLook`, `precipLook`, `moonDisc`, `nightGlow.boost`, angepasste `sunArcCfg`/`moonLight`/`post`) + ksw-Seite (`kswScene`, `kswPost`, `cloudCfg`, `kswCityStyle`, …). Die alten `lightPresets`/`skyPhys` bleiben in Phase 1 erhalten (ksw/main.ts braucht sie noch) und werden in Phase 2 gelöscht.
- `package.json`/`package-lock.json` → Vereinigung (suncalc + @types/suncalc).
- `tests/`, `scripts/smoke-environment.mjs`, `scripts/capture-env.mjs`, Spec/Plan-Dokumente → konfliktfrei übernehmen.

**Gate Phase 1:** alle Unit-Tests grün, Prototyp-Smoke (`smoke-environment.mjs`, look.html) 10/10, `npm run build` grün. Die Stadt (`/`) rendert unverändert (Preset-Architektur noch aktiv).

## Phase 2 — Stadt-Verdrahtung

### Treiber

`ksw/main.ts` konsumiert pro Frame `computeEnvironment(now(), currentWeather())` — identisches Muster wie der Prototyp (`startWeatherLoop`, `sampleWeather`, `CLEAR_SKY`-Default, `WX_OVERRIDES`). URL: `?preset=`/`?cycle=` **entfernt**, `?at=<ISO>`/`?wx=clear|overcast|rain|snow|fog` neu; die bestehenden Stadt-Kamera-Parameter bleiben. `window.__ENV_STATE` wird auch auf `/` gesetzt.

### `src/diorama/ksw/applyCityEnvironment.ts` (neu)

Stadt-Gegenstück zu `environment/applyEnvironment.ts`, mappt `EnvironmentState` auf die Stadt-Handles:

- **Sonne/Hemi/Exposure/Godrays:** direkt (Farben/Intensitäten aus env; Nacht = Mond als Key-Light wie im Prototyp).
- **Fog:** env-Werte × bestehendes `kswScene.fogScale`; das per-Preset-`skyUnfogged` entfällt — SkyMesh generell `fog = false` (Prototyp-Lösung).
- **Wolken:** die Zwei-Ebenen-Kuppel (inkl. Kamera-abhängigem `cloudMix`-Swap) bleibt; Coverage/Drift(2D)/Lit-/Shadow-Farben kommen aus env statt aus `cloudCfg.coverage[preset]`.
- **Nacht-Stadt:** `env.lampOn01` steuert kontinuierlich die bestehenden Strassenlampen (`geo/lamps.ts`) und Fenster-Glüh-Shader (`geo/windows.ts`) — beim echten Sonnenuntergang gehen Lampen und Fenster an.
- **Nachthimmel:** Vollkugel-Sternenfeld + Mondscheibe mit Phasen-Terminator aus dem Modul, aber **Grössen parametrisiert** (Radius, Quad-Grösse, Mond-Distanz als Parameter statt Konstanten; Prototyp- und Stadt-Wertesätze in designTokens). Siderische Rotation/Phasen-Mathematik unverändert.
- **Niederschlag:** `createPrecipitation(options)` — Box-Masse und Partikelzahl als Optionen statt Konstanten; Raum behält seine Werte, Stadt bekommt eine grössere kamerazentrierte Box (`precipLook`-Erweiterung).

### Aufräumen (No-Cruft, Ende Phase 2)

`lightPresets`, `skyPhys` und `cloudCfg.coverage[preset]` sind danach unreferenziert → löschen. Kein `?preset=`-Rest in Code oder Scripts.

## Nicht-Ziele

- Kein Umbau der ksw-Szenenstruktur über das Wiring hinaus (kein main.ts-Gross-Refactor).
- Keine neuen Wetteraspekte (nasse Strassen, Schneedecke auf Dächern — spätere Slice).
- Kein Backend, keine Persistenz.

## Harnesse & Tests

- **Purer Kern:** unverändert, durch die bestehenden 51 Tests abgedeckt; Parametrisierung von `createPrecipitation`/Sternenfeld darf die Prototyp-Smoke-Ergebnisse nicht ändern.
- **`smoke-environment.mjs`:** prüft neu **beide Seiten** — `look.html` und `/` — je mit Open-Meteo-Wiring-Beweis + `?at`/`?wx`-Zustandsproben gegen `__ENV_STATE` (CLAUDE.md-Pflicht-Gate; die Stadt kreuzt dieselbe Wire).
- **`capture-env.mjs`:** Seiten-Parameter; rendert die 9-Zustands-Matrix zusätzlich für die Stadt nach `artifacts/env-city/`; Look-Review mit Nachkuration **nur in designTokens** (gleiches Verfahren wie beim Prototyp; Kalibrierung: Stadt-Dawn/Noon/DREDGE-Dusk/Sternen-Nacht/Regen/Schnee/Hochnebel/Winternacht).
- **Finaler Gate:** `npm test && npm run typecheck && npm run build && node scripts/smoke-environment.mjs`.

## Abschluss

Worktree `.worktrees/winterthur-main` → PR gegen `main` (origin) → CI grün abwarten → Merge.
