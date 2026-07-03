# Winterthur-Stadt im Diorama-Stil: Fassaden, Dächer, Strassen, Licht, Perf

**Datum:** 2026-07-02 · **Branch:** `geo/winterthur-map` (nach 272d11a, Natur-Layer) · **Folge-Spec zu** `2026-07-02-winterthur-geodata-design.md`

## Ziel

Die geodätische Stadt (S1+S2 + Natur-Layer) sieht nach Rohbau aus; das
Original-Hero-Diorama ist die Qualitäts-Referenz („SOTA 2026"). Diese Slice
bringt die Stadt auf denselben Stil- und Licht-Stand — und behebt die vier
benannten Mängel: Häuser ohne Unterbau, Dächer-Problematik, müllige Strassen,
Schwebe-Geometrie — plus das Ruckeln (Perf-Regression).

**Harte Regeln (unverändert):**
- **Geodäsie:** keine Geometrie ohne Datenquelle. Die Stil-Schicht (Sockel,
  Bänder, Fenster, Trim) ist eine **deterministische Funktion der echten Form**
  (Footprint, Fassadenlänge, Höhe, Dachfläche). Nichts wird „wild platziert".
- **Hero pixel-treu:** bestehende `designTokens`-Werte, `look.ts`,
  `staticBatch.ts` unangetastet; nur additive Token. Jeder Task endet mit
  einem **Screenshot-Gate** (Hero `overview`/`er` + Stadt `city`/`bahnhof`),
  bewertet gegen das Original-Diorama.

## Diagnose der Mängel (verifiziert an Live-Screenshots)

| Mangel | Ursache |
|---|---|
| Häuser ohne Unterbau | Wand-Extrusion beginnt exakt bei y=0, kein Sockel, keine Basis — Häuser wirken aufgesetzt |
| Dächer-Problematik | LoD2-Dachflächen sind papierdünne, **einseitige** Planes (von unten/Seite unsichtbar); **Lücke zwischen Wandoberkante (Traufe=min-Dach-Z) und aufsteigendem Dach** (offene Giebel); Schattenseiten kippen grau (Tint-Spreizung ±14% L zu hart, kein Sonnenschatten) |
| Strassen müllig | Bänder ohne Gehrungs-Joints (Keillücken), überlappende Quads an Kreuzungen auf gleicher Höhe (Flackern), Einheitsfarbe/-höhe für alle Klassen inkl. Fusswege |
| Schwebende Dinge | Wenn der Wand-Unterkanten-Trace scheitert, fällt der Footprint auf ein einzelnes Facet zurück → Mini-Wandprisma unter grossem echtem Dach = schwebendes Dach |
| Ruckeln | Perf-Regression seit Stadt+Bäume. Hypothesen (im Plan ZUERST messen, dann fixen): (a) Shadow-Map-/GI-Caching aus #111 wird durch die neuen Objekte pro Frame invalidiert, (b) GI-Probe rendert jetzt die ganze Stadt (88k Tris × 6 Faces × Kadenz), (c) 4127 Baum-Instanzen als PCSS-Caster verteuern die Shadow-Pass |

## Design

### 1. Bake-Härtung: kein Gebäude ohne tragende Hülle (Schwebe-Fix)

- Gate im Bake: `bbox(roof) ⊆ bbox(footprint) + 1.5 m` UND
  `area(footprint) ≥ 0.5 × area(roof-projektion)`. Wenn verletzt →
  Footprint-Fallback = **projizierter Umriss der Dachflächen** (konvexe Hülle
  der Dach-XZ-Punkte — datengetreu: das Dach IST swisstopo-Geometrie).
- Report: Anzahl Trace-Erfolge / Dach-Fallbacks; 0 Gebäude ohne Hülle.

### 2. Gebäude-Stil (Diorama-Formsprache, alles aus echter Form abgeleitet)

- **Sockel:** Basisband 0.5 m hoch, 0.12 m ausgestellt, entlang des Footprints;
  Wand beginnt 0.3 m unter Plattenoberkante (nie schwebend). Farbton wie die
  Original-Basis (helles `white`-Band).
- **Traufband + Dachtrim:** helles Band (Original `kswPalette.roofTrim`-Familie)
  entlang der echten Traufkante; Dächer erhalten **Dicke** (0.22 m Extrusion der
  echten Dachfläche nach unten + Untersichtfläche) → keine Papier-Planes,
  sichtbare Dachkante wie beim Original (`roofThickness`-Look).
- **Giebel-Schliessung:** Wandprisma wird pro Fassadensegment bis zur lokalen
  Dachhöhe hochgezogen (Sampling der Dachfläche über dem Segment) statt global
  bis zur Traufe — keine offenen Dreiecks-Lücken unterm Dach.
- **Fenster (Voll-Variante, approved):** pro Fassadensegment Raster aus echter
  Segmentlänge × echten Stockwerken (`floors = clamp(round(h/3), 1, 24)`);
  Fensterproportionen/Sill/Head aus `kswScene`-Verhältnissen skaliert.
  Instanziert: 1× weisse Rahmen, 1× Glas-Panes, Night-Glow-Anteil deterministisch
  (`nightWindowHash`, `NIGHT_WINDOW_SHARE` — dieselben Funktionen wie das
  Original). Budget ~60–120k Instanzen, 2–3 Draw-Calls; Distanz-Cull, falls
  Perf-Gate reisst.
- **Tint zähmen:** Helligkeits-Spreizung ±14% → ±6%, Hue-Drift halbiert; Ziel:
  warme Clay-Schatten statt Grau-Kippen.
- **Türen/Eingänge:** ein Eingang pro Gebäude auf der strassenzugewandten
  Fassade (kürzeste Distanz Fassadensegment ↔ echte Strassen-Polyline; wo OSM
  `entrance`-Nodes existieren, gewinnen die). Tür-Instanz im Original-Stil
  (Rahmen + dunkleres Blatt), instanziert wie die Fenster.

### 2b. Strassenlaternen + Nacht-Stadt

- Laternen deterministisch entlang echter Strassen-Polylines: Abstand nach
  Klasse (primary 25 m, residential 35 m, Fusswege keine), Position = echter
  Polyline-Lauf + halbe Breite Versatz, abwechselnde Seite. Instanziert
  (Mast + Kopf); nachts warmes Glühen wie die Original-Lampen
  (`glowNight`-Bucket-Logik), morgens unauffällig.
- **Nacht-Gate:** `preset=night` wird in jedem Screenshot-Gate mitgeprüft —
  Stadt liest sich nachts wie das Original: warme Glühfenster
  (`NIGHT_WINDOW_SHARE`) + Laternenketten entlang der echten Strassenzüge.

### 2c. Semantisches LOD (SOTA-Baustein statt Ad-hoc-Culls)

Drei Ringe um die Kamera (Radius-basiert, Übergang via vorhandener
Fade-Mechanik; Instanz-Culling per Distanz wie die Agent-Pipeline):

| Ring | sichtbar |
|---|---|
| fern (> 600 m) | Massing + Dächer + Trauf-/Sockelband, Fahrbahnen; keine Fenster-/Tür-/Laternen-Instanzen, keine Fusswege |
| mittel (150–600 m) | + Fensterraster, Dachtrim, Fusswege, Laternenmasten |
| nah (< 150 m) | + Türen, Laternen-Detail, volle Baum-Schatten |

Der Fusswege-Cull aus §3 ist damit Teil des LOD-Systems, kein Sonderfall.

### 2d. Bäume v2 (geodätisch strenger + Original-Formsprache)

- **Grösse aus Daten, Faktenlage deklariert:** gemessene Tag-Abdeckung im
  Ausschnitt: `height` 2/4127, `leaf_type` 540/4127 (13%), `genus`/`species`
  ≈ 0. Priorität: (1) echte `height`/`diameter_crown` wo vorhanden, (2)
  `leaf_type`-Default (Laubbaum ≈ 9 m Höhe/6 m Krone, Nadelbaum ≈ 14 m/4 m),
  (3) ohne Tag: Laubbaum-Default (städtischer Regelfall). Auf die Defaults
  kommt eine **deklarierte** deterministische Varianz von ±15% (reine
  Darstellung — uniforme Klone wären falscher als die Varianz). Der Bake
  schreibt pro Baum `[x, z, h, r, kind]`.
- **Form wie das Original:** instanzierte Varianten der Hero-Baumgeometrie
  aus `props.ts` (chunky Clay-Krone), Nadelbäume als Kegel-Variante im selben
  Vokabular. LOD: nah = volle Form, fern = Lowpoly-Impostor (§2c-Ringe).
- **Waldfüllung:** die echten Waldpolygone werden mit deterministisch
  gesampelten Bäumen gefüllt (Poisson-artiges Hash-Gitter, Dichte ~1/60 m²,
  nur innerhalb des Polygons). Deklarierte flächentreue Darstellung: das
  Polygon ist das Datum, die Einzelstämme seine Visualisierung — wie in jeder
  Kartografie. Einzeln gemappte OSM-Bäume behalten Vorrang (kein Doppel im
  Umkreis 4 m).

### 3. Strassen v2

- **Miter-Joints:** Polyline-Offsetting mit Gehrung (Kappung bei >60°-Knick) —
  keine Keillücken.
- **Klassen-Hierarchie:** eigene Y-Ebene + Farbe pro Klasse (Fahrbahnen warmes
  Asphalt-Beige, Fusswege heller schmaler, Gleise dunkler auf Schotterband) —
  additive `kswCity`-Token. Kreuzungs-Flackern verschwindet durch die
  Y-Staffelung (0.035/0.04/0.045/0.05).
- **Breite aus Daten:** OSM-`width`- bzw. `lanes`-Tags gewinnen, wo gemappt
  (`width` direkt; `lanes × 3.2 m` als Näherung); der Klassen-Default aus S1
  gilt nur als dokumentierter Fallback.
- Fusswege < 2.5 m Breite gehören zum LOD-Ring „mittel" (§2c) — in der fernen
  Stadtansicht ausgeblendet, damit das Bild ruhig bleibt.

### 4. Beleuchtung wie das Original

- **Kamerafolgendes Schatten-Frustum — Hero-Qualität überall:** Zentrum =
  Kamera-Target, Extent = `max(46, min(46 + (radius − 120) × 0.9, 900))` —
  der Extent fällt NIE unter die heutigen 46 m, d. h. beim Reinzoomen in
  irgendeine Gasse gilt exakt die Hero-Texeldichte (46 m / 4096 PCSS).
  Hero-Guard: liegt das Target auf der Hero-Platte (|x|,|z| innerhalb
  `kswPlan.plate`) und `radius ≤ 120`, gelten wert-identisch die heutigen
  Parameter (Extent 46, Zentrum Origin, far 220) → Hero pixel-treu.
- **Wandernde GI-Probe — keine Abkürzung:** die Stadt bleibt in der
  Probe-Szene. Der Probe-Anker folgt dem Kamera-Target (gesnappt auf ein
  30-m-Raster, nur bei `radius ≤ 300`; sonst und auf der Hero-Platte: heutiger
  Anker `(0, giProbeY, 0)`). Anker-Wechsel = `markDirty()` → 6-Face-Re-Walk
  mit der bestehenden 1-Face-pro-Frame-Amortisierung. Damit hat JEDER
  Zoom-Punkt der Stadt dieselbe One-Bounce-GI-Qualität wie das Hero. Shadow-Map-Refresh-Politik aus #111 respektieren
  (Refresh bei Radius-/Target-Änderung, nicht pro Frame).
- **Zweischichtige Wolken (Original-Stimmung auf jeder Distanz):** die
  Hero-Wolkenkuppel (r=400, Tokens unverändert) faded zwischen radius 300→600
  aus; gleichzeitig faded eine **Stadt-Wolkenschicht** ein — gleiches
  Material-Rezept (cloudCfg-Werte wiederverwendet), Dome mit r=`kswCity.domeRadius`,
  gröbere Noise-Skalierung (Wolken wirken auf Stadtdistanz gleich gross wie die
  Hero-Wolken auf Hero-Distanz). Im Hero-Framing exakt heutiges Bild; in der
  Stadtansicht ziehen Wolken über die ganze Platte. Kein grauer Kuppelrand mehr.
- **Stadt-Mist-Ring:** derselbe Dunstring-Aufbau wie am Hero-Plattenrand
  (identisches Rezept aus main.ts) zusätzlich um den Rand der Stadtplatte —
  das „Modell auf dem Tisch"-Gefühl endet nicht an der Hero-Kante. Der
  Hero-Ring bleibt unverändert bestehen.

### 5. Performance (Ruckel-Fix) — messen, dann fixen

- Task 1 des Plans: Profil via `__KSW_INFO()` (cpu.frame/agents/render,
  drawCalls, tris) in `overview` und `city`, Vergleich gegen `origin/main`.
- Erwartete Fixes (nach Messung): Baum-Canopy als Shadow-Caster nur im
  Nahbereich (LOD); Shadow-Cache-Invalidierung durch die neuen Objekte
  beheben; Probe-Kadenz messen. **Die Stadt bleibt in der GI-Probe-Szene**
  (Qualitätsgebot — kein Ausschluss als Perf-Abkürzung); wird die Probe-Last
  messbar zum Problem, ist die Antwort Kadenz/Auflösung, nicht Weglassen.
  **Gate:** cpu.frame im `city`-Blick ≤ 1.5× des Hero-`overview`-Werts von
  origin/main; keine sichtbaren Ruckler beim Orbit.

### 6. Screenshot-Loop (Arbeitsweise)

Jeder Task endet mit: Captures `overview`, `er`, `city`, `bahnhof` (morning;
Licht-Tasks zusätzlich dusk/night) → Selbstbewertung gegen das
Original-Diorama → bei Mangel iterieren, erst dann nächster Task. Hero-Presets
müssen durchgehend pixel-treu zur `before-*`-Referenz bleiben.

## Testing

- Bake: Gate-Tests für Hüllen-Validierung + Dach-Fallback (Fixture mit
  kaputtem Trace); Fassaden-Raster-Ableitung (Stockwerke, Fensterzahl aus
  Segmentlänge) pur + getestet.
- Runtime: Miter-Join-Geometrie (keine degenerierten Quads, Winkel-Kappung),
  Fenster-Instanz-Zahlen deterministisch, Sockel-/Trim-Mesh-Zahlen.
- `smoke-ksw.mjs` grün; `__KSW_INFO`-Budget im Smoke geprüft.

## Nicht in dieser Slice

S3 (echtes KSW-Gebäude + Innenraum), Terrain (swissALTI3D), Bahnhof-Indoor,
Landmarken-Labels/Attribution (S4), Brücken über das Gleisfeld als eigene
Ebene (Strassen bleiben flach auf der Platte), **Stadt-Fahrzeuge und
-Fussgänger** (eigene Slice; Aussenweg-Walker KSW↔Bahnhof↔ZAG ist als S4
vorgemerkt), erfundene Fassaden-Details ohne Datenbasis (Balkone, Erker).
