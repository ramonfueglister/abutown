# M3 — Kamera-getriebenes LOD-Streaming der World-Tile-Pyramide

Date: 2026-07-06
Status: approved design (User: „Volle Stadt bis zum Horizont", M3 vorgezogen), pre-implementation

## Problem

Seit #133/#136 sind Strassen, Verkehr und Platten-Bäume gemeindeweit konsistent,
aber ausserhalb der Stadtplatte rendert `ksw.html` weder Gebäude noch Bäume —
obwohl die #119-World-Pyramide beides trägt (29'450 Gebäude als
Footprint+Höhe, 350'259 Bäume, dupliziert auf L1+L2; L0 = reines
Übersichts-Terrain; 64.7 MB gesamt, gitignored/deterministisch). Heute lädt der
Boot ALLE 273 Tiles und rendert davon nur das L2-Terrain — 41.5 MB für eine
statische Fläche, nichts fürs Fernfeld.

## Ziel

Cities-Skylines-Gefühl bis zum Horizont: Nahring voll (Gebäude + Bäume),
Mittelring Massing + Baum-Impostors, Horizont L0-Terrain — kamera-getrieben
gestreamt, Boot schneller als heute, 100–120-fps-Budget gehalten, sauberes
Entladen (kein Leak). Architektur = die in M1 §M3 vorgesehene Ausbaustufe
(loadWorld-`keep`-Hook, AOI-Partitionierung).

Nicht-Ziele: CDN/Serving (bleibt same-origin static bis zum Deploy),
Traffic-AOI (eigenes Streaming), Neubau/Abriss (M4), 3D-Tiles/Quadtree-SSE
(Overkill für 3 Levels).

## Entscheid: Ring-basierter Tile-Streamer

Kamera-(x,z) → Soll-Menge pro Level:
- **L2 (Nahring, R2 ≈ 800 m):** Terrain fein + volle Gebäude-Massing + Bäume in
  den treeLayer (voll + per-Tile-Impostors).
- **L1 (Mittelring, R1 ≈ 2500 m):** Terrain mittel + Massing (dieselben
  Gebäude, gröber getintet ok) + NUR Baum-Impostors.
- **L0 (immer):** Horizont-Terrain, boot-geladen.
- Hysterese ±10 % auf die Radien (Laden bei R, Entladen erst bei 1.1·R).
- Fetch-Queue distanz-priorisiert, max. N parallele Fetches (Start: 4).
- LRU-Entladen mit vollständigem `dispose()` (Geometrien, Materialien werden
  geteilt/nicht disposed, Instanz-Pools freigegeben).
- Level-Überlappung: wo ein L2-Tile materialisiert ist, wird das darunter
  liegende L1-Terrain-Tile versteckt (kein Z-Fighting); Gebäude/Bäume sind
  exklusiv EINEM Level zugeordnet (L2-Tile aktiv → dessen L1-Inhalt aus).

## Komponenten

1. **`src/diorama/ksw/geo/tileStreamer.ts`** (neu, pure Logik + dünner
   Fetch-Wrapper): `desiredTiles(camX, camZ, manifest) -> {level,x,y}[]`,
   Hysterese-/LRU-Zustand, Fetch-Queue. Reine Funktionen unit-testbar ohne
   three/Netz. `loadWorld` wird in `fetchTile(baseUrl, ref)` + `decodeTile`
   refaktoriert (decodeWorld bleibt für Tests/Boot-L0).
2. **`src/diorama/ksw/geo/tileContent.ts`** (neu): `materialize(tile, ctx) ->
   TileContent` und `disposeContent(TileContent)`. Terrain über den
   bestehenden Builder (Einzeltile-Variante von `buildTerrainTiles`),
   Gebäude über `mergeTinted`/`mergeWalls`-Bausteine aus cityMassing.ts
   (Footprint-Prismen — Tiles tragen keine Mesh-Verts), Bäume via neuer
   treeLayer-Pool-API.
3. **treeLayer-Erweiterung:** `addTileTrees(tileKey, TreeSpec[])` /
   `removeTileTrees(tileKey)` — dynamische Instanz-Pools statt einmaliger
   Bau; Impostor-Quads pro Tile als eigenes kleines Mesh (16 L1-Tiles + Nahring
   → wenige Draw-Calls). Kompaktierung (compactNear) läuft über alle Pools.
4. **Platten-Interplay:** Stadtkern (buildings/nature/roads.json) bleibt
   boot-geladen; `materialize` überspringt Gebäude/Bäume innerhalb des
   Platten-Rechtecks aus meta.json (Exclusion-Muster wie heroRect).
5. **Proto-/Bake-Extension (Tile-Format, aus Trees-Spec §1 vorgezogen):**
   `WorldTile` erhält `repeated uint32 t_family` (Index in die kanonische
   Familien-Liste; proto3-additiv, Rust-Seite wire-kompatibel).
   `bake-world.mjs` schreibt family aus `transformNature` (seit #136
   vorhanden) — damit bekommen die Aussenwälder auch den Familie-zuerst-Fix
   (der alte Bake stammt von davor: Koniferen fehlen dort). Voller
   Welt-Rebake: deterministisch, gitignored, kein Repo-Churn.
6. **Boot-Sequenz:** L0 sofort + Nahring-Tiles der Boot-Kamera;
   `__LOOK_READY` sobald L0 + sichtbarer Nahring materialisiert sind.
   Erwartung: Boot-Bytes sinken (kein 41.5-MB-Alles-Laden).

## Budgets & Verifikation

- fps ≥ 85 im Fly-Through (Referenz 120 nach #136), Speicher: max ~80
  materialisierte Tiles (LRU-Kappe), Fetch-Fehler = sichtbarer Konsolen-Error,
  kein Retry-Sturm (ein Retry, dann Tile als failed markiert).
- Unit-Tests: Ring-Policy (Soll-Mengen, Hysterese-Flattern), LRU-Entlade-
  Reihenfolge, Proto-Roundtrip t_family, Platten-Exclusion.
- Browser-Smoke `scripts/smoke-streaming.mjs`: Kamerafahrt Zentrum → Stadtrand
  → Wald → zurück; Assertions: Tile-Zähler steigen/fallen, dispose-Zähler > 0,
  fps-Probe ≥ 85, keine unbehandelten Fetch-Fehler, Horizont-Screenshot zeigt
  Gebäude-Massing im Mittelfeld.
- Screenshots als Beweis (CLAUDE.md): Horizont-Shot, Stadtrand-Shot,
  Wald-Shot vorher/nachher.

## Risiken

- treeLayer-Pool-Refactor berührt die Task-6-Fixes (#136) — Regressionsschutz:
  smoke-trees.mjs muss grün bleiben, Handoff-Screenshot re-verifizieren.
- Terrain-Naht L2↔L1 (T-Junctions an Ringgrenzen): akzeptiert für v1 (Skirts
  existieren im Terrain-Builder? prüfen; sonst kleine Überlappung/Skirt).
- Rust-Seite liest WorldTile? proto3-additives Feld ist wire-kompatibel;
  cargo-Regeneration nur falls das Backend das Tile-Message nutzt (prüfen,
  via cargo-serial).
