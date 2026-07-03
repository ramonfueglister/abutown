# Winterthur endless horizon — ganze Gemeinde, echtes Terrain, Streaming, kein Sockel

**Datum:** 2026-07-03 · **Branch:** `geo/endless-horizon` (Basis `origin/main` @ 49af15c)

## Ziel

Die Diorama-Welt hört auf, ein schwebendes Rechteck zu sein. Heute ist die Stadt
ein flacher `cityPlate`-Slab (1187×1506 m aus `meta.json`) mit Edge-Mist-Ring —
ein Sockelmodell, das „irgendwo im Nebel liegt". Neu wird sie zum Ort:

1. **Detailzone = die ganze Gemeinde Winterthur** (~68 km², Grenzpolygon aus
   swissBOUNDARIES3D statt bbox) — alle swissBUILDINGS3D-Gebäude, alle
   OSM-Strassen bis zum Feldweg, ÖV, Natur, Landuse.
2. **Echtes Terrain trägt alles**: swissALTI3D-DEM, Gebäude auf realer
   Höhenkote, Strassen/Flüsse drapiert. Eulachtal, Eschenberg, Lindberg,
   Goldenberg, Brüelberg werden real sichtbar.
3. **Kartenrand = die Aussendörfer** (Wiesendangen, Elsau, Seuzach, Brütten,
   Kyburg, …) als grobe, echte Kulisse auf dem DEM.
4. **Horizont statt Kante**: aerial perspective löst das ferne Gelände in den
   Himmel auf, bevor es endet (Cities-Skylines-Prinzip). `cityPlate` und
   `addMistRing` entfallen ersatzlos.
5. **Streaming**: die Welt kommt als Kachel-Pyramide (Quadtree, binär,
   protobuf) über HTTP; der Start lädt ~1 MB Grobmodell, Detail streamt unter
   der Kamera nach.

**Nicht-Ziel / hart garantiert:** Der Look bleibt Clay-Diorama — Materialien,
designTokens, Echtzeit-Environment (Sonne/Mond/Wetter, #115/#116), Post-Stack
und die 10k-Perf-Pipeline (#111) werden wiederverwendet, nicht ersetzt. Neue
Geometrie geht durch dieselben Builder-Familien (`clayMat`,
`mergeTinted`-Muster, instanzierte Bäume).

**Endvision (Kontext, nicht Scope):** Browser-„Cities-Skylines-2"-Simulation —
Rust-Server-Sim (ECS; Kandidaten `bevy_ecs` standalone oder SpacetimeDB, eigener
späterer Spec) + MMORPG-artige Betrachtung durch bis zu ~2000 gleichzeitige
Clients. Dieser Spec baut ausschliesslich das Welt-Fundament, aber so, dass ihm
das nicht im Weg steht (§ Zukunftsverträge).

## Datenlage (Quellen, alle gratis OGD via `data.geo.admin.ch` STAC + Overpass)

| Quelle | Inhalt | Neu/bestehend |
|---|---|---|
| swissBOUNDARIES3D `TLM_HOHEITSGEBIET` | Gemeindegrenz-Polygon Winterthur + Nachbargemeinden (Ausgabe 2026) | neu |
| swissALTI3D (GeoTIFF, 2 m, 1-km²-Kacheln, STAC `ch.swisstopo.swissalti3d`) | Terrainmodell für Gemeinde + Dörfer-Ring | neu |
| swissBUILDINGS3D 3.0 (GDB-Kacheln) | LoD2-Gebäude; **mehrere Kacheln** (~4–8, Liste aus dem Grenzpolygon berechnet), pro Kachel den **neuesten Jahrgang** wählen (bisherige `1072-14` ist 2019) | erweitert |
| OSM/Overpass `highway` (alle Klassen inkl. `track`/`path`/`footway`) | Strassennetz bis zum Feldweg; Tags `oneway`, `maxspeed`, `lanes`, `surface`; `traffic_signals`-Nodes, Fussgängerübergänge; **turn-restriction-Relations** | erweitert |
| OSM route-Relations (`bus`, `tram`, `train`) + `public_transport`-Stops | ÖV-Linien und Haltestellen | neu |
| OSM `landuse`/`natural`/`leisure` (voll, nicht nur Grün/Wasser) | Landnutzung: Wohnen/Industrie/Landwirtschaft/Wald/Wasser | erweitert |
| OSM Gebäude-Tags + swisstopo-Attribute | Nutzung (Wohnen/Gewerbe/Industrie/öffentlich), Namen; Parkplätze/POIs als optionale Metadaten | erweitert |

Projektion: wie bisher lokale Meter um den Anker (`meta.json`); ~10 km
Ausdehnung → float32 mm-genau, kein Origin-Shifting. Die Projektions-Definition
(Anker, Achsen) wird Teil des Artefakt-Manifests — sie ist künftig
**Client-Server-Vertrag**.

## Zonenmodell

```
Horizont (aerial perspective, kein Geometrie-Ende sichtbar)
   ↑ … DEM + Landcover läuft weiter, löst im Dunst auf
Ring: Aussendörfer — DEM ~25–50 m Grid, Gebäude als getintete Massing-Klötze
   (keine Fassaden), Wald/Feld als Vertex-Tint, Hauptstrassen als Bänder
   ↑ Schnitt: Gemeindegrenz-Polygon (swissBOUNDARIES3D)
Detailzone: Gemeinde Winterthur — DEM ~5–10 m Grid, LoD2-Gebäude mit
   Fassaden/Fenstern (kameranah), alle Strassen, Bäume, Lampen, ÖV
```

Die Gemeindegrenze ist damit erstmals ein echtes, datengetragenes Feature;
optional als dezente Bodenlinie darstellbar (kein physischer Rand).

## Bake: Kachel-Pyramide (Slice 1 — das Schema ist die zentrale Entscheidung)

### Format

- **Quadtree über die Gemeinde + Ring**, Zellen an der 1-km²-swisstopo-Kachelung
  ausgerichtet. Stabile, adressierbare Tile-IDs (`L{level}/{x}/{y}`).
- **Level 0**: ganze Welt ultra-grob (DEM 50 m, Gebäude-Massing verschmolzen) —
  wenige hundert KB, lädt beim Start.
- **Level 1**: Quadranten (DEM ~10 m, Gebäude einzeln grob, Hauptstrassen).
- **Level 2**: Detailkacheln (Fassaden/Fenster-Attribute, alle Wege, Bäume
  instanziert, Lampen). Dörfer-Ring existiert nur bis Level 1.
- **Binär + protobuf**: Schemas in `proto/` (buf-Toolchain besteht). Kein
  Riesen-JSON — die heutigen 7,6 MB `buildings.json` für 846 Gebäude würden
  naiv auf 150–250 MB wachsen; Ziel-Budget **< 40 MB gesamt**, Startladung
  **~1 MB**. Payload = fertige Vertex-/Index-Buffer bzw. SoA-Spalten (s. u.),
  keine Laufzeit-Triangulation.
- **Deterministisch + versioniert**: Bake byte-reproduzierbar, `bake_version`
  im Manifest; spätere Sim-Snapshots referenzieren „Welt-Version X".

### Inhalte pro Kachel (statisch — bewegte Objekte sind hart ausgeschlossen)

1. **Terrain-Patch**: dezimiertes Heightfield-Mesh, Landcover als Vertex-Farbe.
2. **Gebäude**: Geometrie auf realer Höhenkote (Z aus swissBUILDINGS3D;
   Höhen aus Geometrie ableiten — Attributfelder sind leer, verifiziert
   2026-07-02), plus **Metadaten pro Gebäude**: Nutzung, Grundfläche,
   Höhe/Volumen (→ spätere Einwohner-/Arbeitsplatz-Schätzung),
   **Zugangspunkt** = nächste befahrbare/begehbare Graph-Kante + Offset
   (Lektion BLOCKER-1: ohne Anbindung bindet nichts).
3. **Strassen-Render-Bänder**: aus dem Graph generiert (eine Quelle der
   Wahrheit — Lektion Phase 7a), aufs DEM drapiert.
4. **Natur**: Bäume (instanziert), Grünflächen, Wasser.

### Weltweite Artefakte (nicht gekachelt, klein)

- **`roadGraph`** (protobuf, SoA): Knoten (echte OSM-Kreuzungen, geteilte
  IDs) + Kanten mit Klasse, Breite, `oneway`, `maxspeed`, `lanes`, `surface`,
  Höhenprofil (aus DEM), Ampel-Flags, Abbiegeverbote. Spaltenlayout: ids,
  positions, edge-endpoints, Attribute je als eigener flacher Buffer —
  lädt direkt in ECS-Resources.
- **`transit`**: Linien (Route-Relations) + Haltestellen, an Graph-Kanten
  gebunden.
- **`boundary`**: Gemeindegrenz-Polygon.
- **Manifest**: Projektion, Kachel-Index, Versionen, Attribution.

### Pipeline

- `scripts/geo/fetch-winterthur.mjs` erweitert (oder Schwester-Skripte):
  STAC-Loops für swissALTI3D + Multi-Tile swissBUILDINGS3D + swissBOUNDARIES3D,
  breitere Overpass-Queries (Gemeinde + Ring; bestehende Mirror-/Retry-Logik
  wiederverwenden). Netz-Schritt bleibt von der offline-Bake getrennt.
- `scripts/geo/bake-winterthur.mjs` wird zum Pyramiden-Bake: clip an
  Grenzpolygon, Multi-Res-Dezimierung, Graph-Bau (Topologie erhalten!),
  Drapierung, Zugangspunkte, protobuf-Encoding.

## Renderer (Slice 1 minimal + Slice 3 Horizont)

- `cityPlate` und `addMistRing` in `src/diorama/ksw/main.ts` entfallen; die
  Welt steht auf den Terrain-Patches. Die Wetter-Kopplung von `mistOpacity`
  bleibt als echter Dunst erhalten, nur nicht mehr als Rand-Ring.
- **KSW-Hero-Campus zieht aufs Terrain um** (bisher y=0 auf der Platte):
  Campus-Plateau aus dem DEM, Interior-Plan und Agent-y-Offsets folgen der
  Plateau-Kote. Grösster Einzelposten neben dem Bake.
- **LOD wird 4-ringig** (Erweiterung `cityLodState`/`applyCityLod`): nah =
  Fassaden/Fenster/Lampen · mittel = merged Clay · fern = Impostor-Billboards
  (Bäume, ferne Bebauung) · Ring/Horizont = nur DEM + Massing. Fernring:
  `castShadow` aus, statischer BatchedMesh, **explizit aus der Hero-GI-Probe
  ausgenommen** (die „city stays in GI probe"-Regel gilt der Kernstadt, nicht
  dem Backdrop).
- **Aerial perspective statt Fog-Klippe**: distanzabhängiger Tint des fernen
  Geländes zur Himmelsfarbe (Erweiterung des Environment-Stacks; Keyframe-
  Werte in designTokens). Ring gross genug (~Radius 6–8 km um die Gemeinde),
  dass seine Aussenkante jenseits `fogFar` liegt. `domeRadius` ≥ Ring-Radius.

## Streaming-Runtime (Slice 2)

Neues `src/diorama/ksw/geo/tileStream.ts`:

- **Screen-Space-Error-Auswahl** (3D-Tiles-Prinzip): pro Kachel projizierter
  Fehler → Kinder laden (nah) oder Parent behalten (fern).
- **Fetch-Priorisierung** nach Blickrichtung; HTTP, immutable/cacheable —
  N Clients kosten den Sim-Server null.
- **LRU-Cache mit Budget**; Eviction disposed GPU-Buffer.
- **Crossfade beim LOD-Swap** — der Diorama-Look verzeiht kein Popping.
- Slice 1 lädt übergangsweise stumpf alle Kacheln (funktional wie heute);
  Slice 2 ersetzt nur den Loader, das Format steht schon.

## Zukunftsverträge (MMORPG/Sim — festgezurrt, nicht gebaut)

1. **Statisch/dynamisch-Grenze**: Kacheln enthalten nie bewegte Objekte.
   Entities (Spieler, Fahrzeuge, Agenten) kommen ausschliesslich über den
   WS-Kanal.
2. **Tile-IDs = AOI-Zellen**: Interest Management der späteren Server-Sim
   abonniert dieselben stabilen Kachel-IDs, die das Rendering streamt.
3. **Graph + Projektion = Client-Server-Vertrag**: Sim routet auf dem Graph,
   Positionen wandern als (Kanten-ID, Offset) über die Leitung; Anker/Achsen
   im Manifest sind verbindlich.
4. **Artefakte ECS-ready**: protobuf-definierte SoA-Spalten, von Rust ohne
   Umformatierung ladbar (`bevy_ecs` standalone oder SpacetimeDB — Entscheid
   im späteren Sim-Spec).

Nicht in diesem Spec: Netcode, Sharding, Tick-Raten, Prediction, Accounts,
Wassersim, editierbares Terrain/Bauen.

## Slices

1. **Bake-Umbau**: Fetch (DEM/Boundaries/Multi-Tile/OSM-voll) + Pyramiden-Bake
   (Terrain, Gebäude+Metadaten+Anbindung, Graph inkl. Restrictions/Ampeln,
   ÖV, Landuse, protobuf/SoA) + Renderer-Minimalumbau (Terrain trägt, Platte
   raus, KSW auf Plateau, alle Kacheln stumpf geladen).
2. **Streaming**: Tile-Manager (SSE, Priorität, LRU, Crossfade).
3. **Horizont**: Dörfer-Ring-Kacheln, aerial perspective, Dome geweitet,
   Mist-Ring endgültig raus.

## Verifikation

- **Slice 1**: Bake-Determinismus (zweimal baken → byte-identisch);
  Graph-Invarianten (zusammenhängend im Hauptnetz, jede Kante mit Höhenprofil,
  jedes Gebäude mit Zugangspunkt); Render-Smoke via bestehendem
  `capture-env.mjs`-Harness (Stadt steht, KSW auf Plateau, fps-Budget der
  10k-Pipeline gehalten).
- **Slice 2**: Netzwerk-Trace (Startladung ~1 MB, Nachladen folgt der Kamera),
  Speicher-Budget (Eviction greift), kein sichtbares Popping in Captures.
- **Slice 3**: Horizont-Shots Tag/Nacht/Nebel × nah/fern — keine harte Kante,
  reale Hügelsilhouette, Auflösung in den Himmel.
- Browser-Smoke ist gemäss CLAUDE.md für alles Frontend-Wirksame Pflicht.

## Risiken

1. **Perf** (25–30k Gebäude + Terrain): Mitigation = LOD im Bake vorverlagert
   + bestehende BatchedMesh/GI-Caching-Pipeline; Budget-Gate in Slice 1.
2. **Bake-Komplexität** ist der eigentliche Brocken (Multi-Tile-GDB, Graph-
   Topologie, Drapierung); deshalb eigener Slice mit eigener Verifikation.
3. **Datenvolumen** swissALTI3D (68 km² + Ring à 1-km²-Kacheln): 2-m-Grid
   laden, im Bake dezimieren; Scratch-Bedarf einige GB, Artefakt-Budget
   < 40 MB.
4. **KSW-Umzug aufs Terrain** kann Interior/Agents/Cutaway-Annahmen (y=0)
   brechen — im Plan als eigene Task-Gruppe mit Render-Vergleich.
