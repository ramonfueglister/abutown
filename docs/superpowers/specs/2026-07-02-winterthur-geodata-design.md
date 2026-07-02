# KSW-Diorama auf echter Winterthur-Karte (swisstopo + OSM)

**Datum:** 2026-07-02 · **Branch:** `geo/winterthur-map` (Basis `origin/main` @ df8d9d4, #111)

## Ziel

Das KSW-Diorama wird geodätisch echt: das Spitalgebäude selbst aus den realen
KSW-Umrissen gebaut (Variante A — abteilungs-real, Räume prozedural), umgeben
von der echten Stadt im Korridor **KSW ↔ Bahnhof Winterthur ↔ ZAG** (beide
Campus: Turbinenstrasse 5 und Konradstrasse 14). Alle Gebäude im Umkreis kommen
als echte 3D-Modelle aus swisstopo swissBUILDINGS3D 3.0 (LoD2, echte
Dachformen), Strassen und Namen aus OSM.

**Nicht-Ziel / hart garantiert:** Der Look bleibt **pixel-treu** — Clay-Materialien,
designTokens-Werte, Sky/Sun/Clouds/GI/Post-Stack und die 10k-Perf-Pipeline
(BatchedMesh, GI/Shadow-Caching) aus #110/#111 werden nicht verändert, nur
wiederverwendet. Neue Geometrie geht durch dieselben Builder (`clayMat`, `box`,
`staticBatch`).

## Verifizierte Datenlage (2026-07-02, end-to-end geprüft)

| Quelle | Inhalt | Beleg |
|---|---|---|
| swisstopo swissBUILDINGS3D 3.0, Kachel `1072-14` (GDB, 38 MB, STAC `data.geo.admin.ch`) | LoD2-Layer `Floor`/`Wall`/`Roof` + `Building_solid`; echte Z-Koordinaten (Terrain ~450 m ü. M.) | 844 Gebäude / 543 Dachflächen in der Ziel-Bbox extrahiert (ogr2ogr, lokal vorhanden) |
| ⚠ Attribut-Falle | `GESAMTHOEHE`/`GELAENDEPUNKT` sind in dieser Kachel **leer** — Höhen ausschliesslich aus der Geometrie (Dach-Z − min-Z) ableiten | verifiziert |
| OSM (Overpass) | Gebäudenamen/-nutzung (KSW-Abteilungen: „Radio-Onkologie", „Rheumatologie", …), Strassen, Gleise | 179 Gebäude nahe KSW, 147 mit Levels, 12+ benannte Abteilungen |
| OSM Indoor | **KSW: 0 Indoor-Elemente** (Raumpläne nicht öffentlich) → Räume bleiben prozedural. Bahnhof ist indoor-gemappt → Ausbaustufe, nicht v1 | verifiziert |

**Anker (WGS84):** KSW 47.5069/8.7285 · Bahnhof 47.5003/8.7240 ·
ZAG Tu5 47.4973/8.7182 · ZAG Ko14 47.5022/8.7219.
**Bbox:** 8.7150–8.7300 E, 47.4955–47.5085 N (≈ 1.13 × 1.45 km).

## Architektur

### 1. Offline-Bake: `scripts/geo/bake-winterthur.mjs`

Einmaliger, reproduzierbarer Schritt (nutzt lokales `ogr2ogr`; GDB-Kachel wird
nach `scratch` geladen, **nicht** committet):

1. STAC-Download Kachel `1072-14` → GDB entpacken.
2. `Roof`/`Wall`/`Floor` → GeoJSON, geclippt auf die Bbox.
3. Projektion: WGS84/LV95 → lokale ENU-Meter, **Ursprung = KSW-Anker** (die
   bestehende Kamera-/Agenten-Logik behält ihren Ursprung). Terrain wird
   geflattet: pro Gebäude min-Z → y=0 (Winterthur ist im Korridor flach genug;
   swissALTI3D-Heightfield ist Ausbaustufe).
4. OSM-Overlay (Overpass): `building`-Footprints mit `name`/`healthcare`/
   `amenity`, `highway`-Strassen (motorway…footway klassifiziert), `railway`.
   Join swisstopo↔OSM per Footprint-Zentroid-Containment (EGID gibt es in OSM
   nicht flächig).
5. Output (committet, Ziel < ~4 MB): `data/winterthur/buildings.json`
   (pro Gebäude: Footprint, LoD2-Dachflächen trianguliert, Höhe, Name/Nutzung,
   Flag `kswCampus`/`zag`/`bahnhof`), `roads.json`, `rails.json`, `meta.json`
   (Bbox, Anker, Quellen-Attribution: © swisstopo, © OpenStreetMap ODbL).

### 2. Runtime: `src/diorama/ksw/geo/`

- `cityMassing.ts` — alle Nicht-Hero-Gebäude als Clay-Volumen: Footprint
  extrudiert bis Traufhöhe + echte LoD2-Dachflächen obendrauf; Material/Radius
  aus bestehenden Tokens; alles in die bestehenden `staticBatch`-Buckets.
- `roads.ts` — Strassen als flache Clay-Bänder (Breite nach Klasse), Gleise
  als schmale Bänder; auf der Platte, unterhalb Gebäude-Sockel.
- Landmarken-Beschriftung (Bahnhof, ZAG ×2, KSW-Abteilungen) über die
  vorhandene Signage aus `building.ts`.
- Platte wächst auf die Bbox (`kswPlan.plate` wird aus `meta.json` gespeist);
  Mist-Ring am Plattenrand bleibt (läuft bereits über `kswPlan.plate`).

### 3. KSW-Hero aus echtem Umriss

- Der erfundene 60×38-Rechteck-Shell in `floorPlan.ts` wird ersetzt: die
  **realen, benannten KSW-Gebäude-Footprints** werden die Hüllen.
- Der grösste zusammenhängende Bau bekommt den begehbaren Innenausbau: das
  bewährte Leiter-Schema (Korridore + Raumzeilen) wird **prozedural in den
  realen Footprint eingepasst** (rektilineare Zerlegung des Footprints →
  Korridor-Spine → Raumzeilen mit den bestehenden Raum-Typen/Props/Rollen).
- Agenten/Nav unverändert im Mechanismus: `nav.ts`/`agentSpawn.ts` konsumieren
  den generierten Plan wie bisher den handgeschriebenen; Walker zusätzlich auf
  Aussenwegen KSW ↔ Bahnhof ↔ ZAG (echte Fusswege aus OSM).
- Übrige Campus-Bauten: Massing mit Abteilungs-Signage, Dächer opak (kein
  Roof-Fade nötig ausser beim Hero — Roof-Fade-Policy unverändert).

### 4. Kamera

Presets erweitert: `overview` (ganzer Korridor), `ksw` (heutiger Rahmen),
`bahnhof`, `zag`. Rig/Zoom/Orbit-Verhalten unverändert (`cameraRig.ts` bleibt).

## Fehlerbehandlung

- Bake bricht hart ab (kein Silent-Skip) bei: leerer Bbox-Extraktion,
  Projektionsabweichung > 1 m im Anker-Roundtrip, nicht-triangulierbaren
  Dachflächen (Report mit UUID).
- Runtime lädt `data/winterthur/*.json` statisch (Vite-Import) — kein
  Netz-Fetch, keine Fallback-Pfade (No-legacy-Regel).

## Testing

- **Bake-Unit-Tests:** LV95→ENU-Roundtrip an den 4 Ankern (< 1 m Fehler),
  Clip-Invarianten, Namens-Join (KSW-Abteilungen matchen), Höhen > 0 für alle
  Gebäude, Triangulations-Validität (geschlossene, nicht-degenerierte Meshes).
- **FloorPlan-Fit-Tests:** eingepasster Plan erfüllt die bestehenden
  Invarianten aus `tests/diorama/floorPlan.test.ts` (Korridor-Anbindung, Türen
  auf Wänden, Props in Räumen) — gegen den realen Footprint.
- **Look-Gate:** `scripts/capture-ksw.mjs` Screenshots vorher/nachher aus den
  bestehenden Presets — Clay-Look, Himmel, Beleuchtung unverändert (visueller
  Abgleich); `smoke-ksw.mjs` + Draw-Call-/Tri-Budget via `__KSW_INFO()`
  (Ziel: Stadt-Massing komplett gebatcht, Draw-Calls bleiben zweistellig).
- **Browser-Smoke ist Pflicht** (CLAUDE.md) vor jeder „fertig"-Meldung.

## Slices (je eigener PR-fähiger Schritt)

1. **S1 — Bake-Pipeline + Daten-Artefakte** (`scripts/geo/`, `data/winterthur/`, Tests).
2. **S2 — Stadt-Massing + Strassen/Gleise + grosse Platte + Kamera-Presets.**
3. **S3 — KSW-Hero aus realen Footprints** (Grundriss-Einpassung, Agenten, Signage).
4. **S4 — Politur:** Aussenweg-Walker Bahnhof↔ZAG, Landmarken-Labels, Attribution-Einblendung.

## Risiken

- **Skalensprung Platte 72×56 m → ~1130×1450 m:** Sonnen-Schattenkamera,
  GI-Probe-Reichweite und Fog/Mist sind auf die kleine Platte getunt. Der
  Look selbst (Materialien, Grading, Himmel) bleibt unangetastet; aber
  Reichweiten-Parameter (Shadow-Frustum, Fog-Distanzen) müssen auf die grosse
  Platte **erweitert** werden — als neue `kswScene`-Werte, per Look-Gate
  gegen die Vorher-Screenshots im Hero-Framing abgesichert. Falls die
  Schattenauflösung im Hero-Bereich sichtbar leidet: Schattenkamera folgt dem
  Kamera-Target (fitted frustum) statt der ganzen Platte.
- **LoD2-Tri-Budget:** 543+ Dachflächen sind harmlos, aber der Bake report
  Tri-Zahlen; Budget-Gate via `__KSW_INFO()` im Smoke.

## Bewusst verschoben

Bahnhof-Indoor (OSM-Daten vorhanden), swissALTI3D-Terrain, Vegetation/
Wasser (Eulach), Verkehrs-Agenten auf Strassen.
