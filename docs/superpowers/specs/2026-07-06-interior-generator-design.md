# Interior-Generator: datengetriebene Innenräume für alle Gebäude (SOTA 2026)

**Datum:** 2026-07-06 · **Status:** Design approved (Brainstorming-Session)
**Basis:** `origin/main` @ eb4a3b9

## 1. Kontext & Problem

Das KSW-Diorama hat heute genau ein Gebäude mit Innenräumen (das Hero-Hauptgebäude,
`src/diorama/ksw/interior/`). Zwei Bugs, ein gemeinsamer Kern:

**Bug 1 — Räume passen nicht ins Gebäude:**
- Das Interior ist ein einziger flacher EG-Slab (alle Räume bei y≈0.14m), die Shell
  ist mehrgeschossig (Traufe ~10m+). `generatePlan.ts` liest `height`/`eaveH` nie.
- `zones.ts` rasterisiert den Footprint mit Corner-Inside-Sampling auf 2m-Raster und
  ~61% Coverage-Ziel → jede Zone schrumpft nach innen, Räume stehen von der Fassade
  abgesetzt.
- Das "grösste Gebäude" wird **zweimal** unabhängig bestimmt (`main.ts` und
  `kswCampus.ts`) — latentes Divergenz-Risiko der Footprints.

**Bug 2 — Zoom nicht nahtlos:**
- Interior-Sichtbarkeit ist ein Boolean-Toggle bei `upperFade < 0.5`, während
  Dach/Wände noch 50% opak sind → harter Pop-in.
- Der Cutaway ist ein globaler, pixelscharfer Y-Slice (`cutH = 3.2m`,
  `discard()` in `cityMassing.ts`), der Wände und Möbel mitten durchschneidet und
  nicht mit dem Dach-Fade koordiniert ist.

Beide Fixes konvergieren auf dieselbe Architektur: ein **geschoss-bewusstes
Dollhouse** mit koordiniertem Etagen-Dissolve — und genau das ist auch das Fundament
für den generischen Generator über alle ~846 Gebäude der Gemeinde.

## 2. Ziele

1. **KSW-Fix:** Räume füllen das Hauptgebäude korrekt (voller Footprint, alle
   Geschosse), Zoom-Reveal ist nahtlos (kein Pop, kein Schnitt durch Wände).
2. **Generalisierung:** Jedes Gebäude der Gemeinde bekommt eine deterministische,
   datengetriebene Innenraum-Spezifikation — Raumgrössen aus dem realen
   GWR-Wohnungsregister, Branchen-Fitout aus realen OSM-POIs.
3. **Wohnungen ≠ Gewerbe:** Wohnbauten werden als Wohnungen (echte Zimmerzahl +
   Fläche) eingerichtet, Gewerbe branchenspezifisch (Bäckerei ≠ Coiffeur ≠ Büro).
4. **Diorama-Schönheit:** stilisierte, konsistente Möblierung im bestehenden
   Clay-Look; Anordnung per Constraint-Solver, nicht zufällig gestreut.
5. **120fps** über die ganze Gemeinde bleibt hart.

### Nicht-Ziele

- Kein Fotorealismus, keine Fototexturen — Diorama-Stil (Cities-Skylines /
  Monument-Valley-Miniatur).
- Keine Untergeschosse (im Cutaway nie sichtbar).
- Kein möbliertes Dachvolumen: oberstes Geschoss endet an der Traufe; der
  Dachraum über der Traufe bleibt leer (bewusste Vereinfachung).
- Keine echten Grundriss-Blueprints *unserer* Gebäude: für die konkreten
  Winterthur-Adressen sind Wandpositionen nicht open source. SOTA heisst hier:
  *prozedurale Layouts, konditioniert auf reale Per-Gebäude-Metadaten*
  (Footprint, Geschosse, Wohnungsflächen, Zimmerzahlen, Branchen) **und auf
  statistische Priors aus 242k echten Schweizer Räumen** (Swiss Dwellings,
  §6a).
- Sim-seitige Raum-Logik (Bürger wohnen in Wohnung X) ist ein Hook (stabile
  Raum-IDs), nicht Scope dieser Arbeit.

## 3. Architektur (4 Schichten)

```
BAKE (offline, deterministisch, Seed = fnv1a(uuid), kein Date/Math.random)
  geo:fetch-gwr       → GWR-Register (Geschosse, Wohnungen, m², Zimmer)
  geo:fetch-osm       → OSM-POIs (Branche + Name pro Footprint)
  geo:bake-interiors  → public/winterthur-world/interiors.bin (Protobuf)
                        eine InteriorSpec pro Gebäude

CLIENT
  buildInteriorFromSpec(spec) → Geschoss-Slabs (Böden, Innenwände mit Türen,
                                Möbel via BatchedMesh, Beschilderung)
  LOD-Manager                 → fern Massing/Impostor · mittel Shell+Fensterlicht ·
                                Fokus volles Interior
  Dollhouse-Cutaway v2        → koordinierter Etage-für-Etage-Dissolve
```

Die **InteriorSpec ist die einzige Schnittstelle** zwischen Bake und Client. Der
Client kennt weder GWR noch OSM; die Sim-Seite kann dieselbe Spec später lesen
(persistentes Welt-Modell, M1).

## 4. Datenpipeline

### 4.1 `geo:fetch-gwr` (neu)

- Quelle: BFS-Vollexport des Gebäude- und Wohnungsregisters (madd.bfs.admin.ch,
  CSV, ~100MB ZIP). Open data.
- Filter: BFS-Gemeinde-Nr. 230 (Winterthur) → `data/cache/gwr-winterthur.json`
  (gitignored, wie andere fetch-Caches).
- Gebäude-Ebene: `GASTW` (Geschosszahl), `GANZWHG` (Anzahl Wohnungen), `GKLAS`
  (Gebäudeklasse), `GBAUJ` (Baujahr).
- Wohnungs-Ebene: `EWID`, `WAREA` (m²), `WAZIM` (Zimmerzahl), `WSTWK`
  (Stockwerk), `WKCHE` (Küche).
- Die GKLAS-Code-Tabelle wird bei der Implementierung gegen den offiziellen
  BFS-Merkmalskatalog verifiziert — nicht aus dem Gedächtnis kodiert.

### 4.2 `geo:fetch-osm` (neu)

- Quelle: Overpass-API, Gemeinde-BBox. Tags: `shop`, `office`, `amenity`,
  `craft`, `healthcare`, `tourism` + `name` + `level`.
- Punkt-in-Footprint-Join über `scripts/geo/lib/join.mjs` →
  `data/cache/osm-pois-winterthur.json`.
- Lizenz ODbL: Attribution wird ins Artefakt und in die UI-Credits geschrieben.

### 4.3 `geo:bake-interiors` (neu)

Input: `buildings.json` + `building-attributes.json` (846 Records: EGID,
gwrCategory, GKLAS, Bauzone) + GWR-Cache + OSM-Cache + Gebäude-Bodenkote aus dem
World-Bake. Output: `interiors.bin` (Protobuf, deterministisch, gitignored,
Muster wie `bake-world`).

**Join-Kaskade pro Gebäude** (deckt jede Datenlage ab):

1. `raw.egids` (oft mehrere EGIDs pro swissBUILDINGS3D-Footprint) → **alle**
   GWR-Records mergen: Geschosse = max, Wohnungen = Summe, pro EGID eine
   Hauseingangs-Einheit.
2. Kein EGID / kein GWR-Match → Schätzer: Geschosse =
   `clamp(round(eaveH / 3.0), 1, 8)`; Wohnungszahl aus Bauzonen-Dichte
   (z.B. W3 ≈ 2 WE/Etage); Flächen aus `area_m2 × Geschosse × 0.8`.
3. `GASTW` fehlt, aber Wohnungszeilen mit `WSTWK` vorhanden →
   Geschosse = max(WSTWK).
4. `GASTW` ↔ Höhe widersprüchlich → GWR gewinnt für die *Anzahl*;
   Geschosshöhe = `eaveH / GASTW`, geclamped 2.4–4.5m (Industriehallen bis 8m).

**Bake-Report:** Jede Schätzung und Anomalie wird gezählt und als Summary
ausgegeben (Repo-Regel: kein silent fallback, Konsequenzen sichtbar).

## 5. Programm-Klassifikation & Archetypen

Priorität: **OSM-POI (mit `level`) → GKLAS → gwrCategory → usage**.

**Mischnutzung ist der Schweizer Normalfall** und wird explizit modelliert:
POIs mit `level=0` besetzen das EG, GWR-Wohnungen mit `WSTWK` die Obergeschosse.
Ein Gebäude = Stapel von **Units** unterschiedlichen Programms.

### Archetyp-Katalog

| Gruppe | Archetypen |
|---|---|
| Wohnen | `apartment` (echte WAZIM/WAREA: Entrée, Wohnzimmer, Küche/WKCHE, N Schlafzimmer, Bad; Möblierungs-Varianten Familie/Single/Senior per Seed), `efh` (Treppenkern), `heim` (Zimmerzeilen + Gemeinschaftsraum) |
| Retail | `baeckerei`, `coiffeur`, `apotheke`, `supermarkt`, `kiosk`, `fashion`, `bank` |
| Gastro | `restaurant` (Gastraum, Bar, Profi-Küche, WC), `cafe`, `bar`, `takeaway` |
| Arbeit | `buero_zellen`, `buero_open`, `praxis`, `zahnarzt`, `physio`, `werkstatt`, `garage`, `industriehalle` (offene Halle, Maschinenreihen, Büro-Mezzanin), `lager` |
| Öffentlich | `schule`, `kita`, `turnhalle`, `kirche`, `museum`, `hotel` (EG Lobby, OGs Zimmerraster mit Bad-Kernen), `parkhaus` |
| Klinik | `clinic` — bestehende authored KSW-Leiter, mehrstöckig realistisch vertikal zoniert: EG Empfang/Notfall/Radiologie, mittlere Geschosse OP/IPS/Labor, obere Geschosse Bettenstationen, oberstes Technikgeschoss |

Fallbacks ohne Branchen-Info: `buero_zellen` (Commercial), `apartment`
(Residential), `industriehalle` (Industrial). **Kein Gebäude bleibt hohl.**
OSM-`name` wird als Mieter-Beschilderung an Fassade/Eingang gerendert.

## 6. Layout-Generator (literatur-fundiert, deterministisch)

**SOTA-Einordnung (verifiziert 2026-07-06):** Die Forschung 2024–2026 kennt
zwei Zweige: (a) LLM-/Diffusions-Generierung (DiffuScene; DirectLayout,
NeurIPS 2025; SceneSmith, CVPR 2026) — stark für Einzelszenen aus Text, aber
nicht-deterministisch und nicht auf einen reproduzierbaren 846-Gebäude-Bake
skalierbar; (b) constraint-basierte prozedurale Systeme mit hierarchischem
Solver — Referenz: Infinigen Indoors (Raistrick et al., 2024), gleiches Muster
in ProcTHOR/Embodied-AI-Sims. Für deterministische, sim-ready Welten ist (b)
der Standard; dieses Design folgt (b) und grundiert es zusätzlich mit realen
Schweizer Verteilungen (§6a) — was keiner der beiden Zweige von Haus aus tut.

Pro Geschoss-Unit, im **vollen Footprint** (Ablösung der 61%-Coverage-Zonen):

1. **Kern:** Treppenhaus/Lift-Kern (Pflicht bei >1 Geschoss; zwei Kerne bei
   >400m²/Etage) an der strassenabgewandten tiefsten Footprint-Stelle; alle
   Geschosse teilen denselben Kern → vertikale Konsistenz.
2. **Zirkulation nach Typologie:** kleiner Wohnbau = Treppenhaus direkt in
   Wohnungen; grosser Wohnbau/Hotel/Büro/Klinik = Mittelkorridor entlang der
   Footprint-Hauptachse (PCA-Achse, nicht naiv achsparallel); Halle/Turnhalle/
   Kirche = keine Korridore.
3. **Unit-Split:** Wohnungen einer Etage per squarified-treemap-Split (Bruls
   et al., 2000) auf die echten WAREA-Anteile; Raum-Split innerhalb der Wohnung
   per rekursivem KD-Split constrained auf WAZIM, Mindestraum 7m², Nasszellen
   innenliegend, Wohnräume an der Fassade (Tageslicht-Regel; Merrell et al.,
   2010; Wu et al., 2019 als SOTA-Referenz für datengetriebene Wohnungslayouts).
4. **Fenster-Snapping:** Trennwände snappen auf das Fassaden-Fensterraster
   (Parameter aus `geo/windows.ts`) — nie eine Wand mitten durchs Fenster.
   Sichtbarster Realismus-Gewinn im Cutaway.
5. **Möblierung:** pro Raumtyp ein Constraint-Set (Wandabstand, Ausrichtung,
   Türfreihaltung, Paarbeziehungen wie Bett+Nachttisch, Gruppen-Anker wie
   Teppich unter Sofa+TV, Symmetrie- und Mengen-Constraints) — gelöst per
   seeded, **hierarchischem** Simulated Annealing im Bake (Grundriss →
   Grossmöbel → Kleinobjekte/Dressing), dem Solver-Muster von Infinigen
   Indoors (Raistrick et al., 2024) folgend, aufbauend auf Yu et al. (2011).
   In die Spec geschrieben wird **Archetyp + Seed**, nicht die Item-Liste —
   der Client re-instanziiert deterministisch. Ziel: `interiors.bin` < 2MB
   gzip für ~40k Räume.

### 6a. Swiss-Dwellings-Priors (Realdaten-Grundierung der Layouts)

Das Swiss Dwellings Dataset (Archilyse; Standfest et al., 2022; CC BY 4.0)
enthält 42'207 echte Schweizer Wohnungen mit 242'257 semantisch annotierten
Räumen (Geometrien, Raumtypen, Fenster/Türen). Ein einmaliger Offline-Schritt
(`geo:derive-dwelling-priors`, läuft nicht in CI) leitet daraus kompakte
Verteilungen ab und checkt sie als kleines JSON ins Repo ein
(`data/priors/swiss-dwelling-priors.json`):

- Raumflächen-Anteile nach Raumtyp, konditioniert auf (WAZIM, WAREA-Klasse)
  — z.B. Küchenanteil in einer 3-Zimmer-80m²-Wohnung.
- Raum-Adjazenz-Häufigkeiten (Küche↔Wohnzimmer, Bad↔Entrée …) → steuert die
  KD-Split-Reihenfolge.
- Seitenverhältnis-Verteilungen pro Raumtyp (verhindert Schlauchzimmer).
- Fassaden- vs. innenliegend-Quoten pro Raumtyp (validiert die
  Tageslicht-Regel empirisch statt axiomatisch).

Der Generator sampelt seine Split-Parameter aus diesen Priors (seeded) statt
aus Daumenregeln. Determinismus bleibt: die Priors sind ein statisches,
eingechecktes Artefakt mit dokumentierter Ableitung. Attribution CC BY 4.0 im
Artefakt + UI-Credits.
6. **Stabile IDs:** `uuid/storey/roomIndex`, plus `EWID` wo vorhanden —
   MMORPG-Hook: Bürger können später echten Wohnungen zugewiesen werden.

## 7. Prop-Katalog: ~220 parametrische Familien, >1500 Varianten

Props sind **parametrische Builder-Familien**, keine Einzelmodelle. Achsen:
Grösse (Bett 90/140/160/180), Proportionen (Regal 2–6 Fächer, Tisch 2–8
Plätze), Stil-Seed (Armlehnen, Kissen), Material-Slot (Palette). ~220 Familien
→ 1500–2500 Geometrie-Varianten, plus Instanz-Farbvariation.

| Gruppe | Familien | Beispiele |
|---|---|---|
| Wohnen | ~55 | Betten, Sofas, Sessel, Ess-/Couchtische, Stühle, Schränke, Küchenzeilen-Module, Bad-Set, TV/Sideboard, Kinderzimmer |
| Gastro/Retail | ~45 | Theken-Module, Vitrinen, Gondelregale, Kühlwand, Kaffeemaschine, Barhocker, Kabinen, Kassen |
| Büro/Praxis | ~35 | Desk-Bänke, Bürostühle, Sitzungstische, Whiteboards, Behandlungsliegen, Empfang, Wartezimmer-Set |
| Industrie/Werkstatt | ~30 | Werkbänke, Maschinenkörper, Palettenregale, Hebebühne, Stapler (parkiert), Fässer/Kisten |
| Öffentlich | ~30 | Schulpulte, Kirchenbänke, Turngeräte, Hotelzimmer-Set, Vitrinen, Kita-Möbel |
| Dressing/Clutter | ~25 | Pflanzen, Lampen, Teppiche, Bilder, Bücherstapel, Geschirr, Wäscheständer, Spielzeug |

Dazu die bestehenden **56 KSW-Klinik-Builder** als fertige siebte Gruppe
(werden zum gemeinsamen Katalog generalisiert).

**Prop-Kompositions-Layer:** kleine interne Bibliothek aus Primitiven (gefaste
Box, Loft, Rundstab, Polster-Form) + Palette-Slots; eine Familie entsteht in
~30–60 Zeilen (Muster: CS-Auto-Lofts, #137). Ein Stil über alles:
`designTokens`-Palette, weiche Bevels, Flat-Shading, keine Fototexturen.

**Licht:** warme emissive Lampenschirme, gekoppelt an die Echtzeit-Umgebung
(#116) — abends leuchten aufgeschnittene Wohnungen von innen.

## 8. Client: LOD & Dollhouse v2

**Entscheidung: Fokus-Gebäude-Dollhouse.** Genau **ein** Gebäude ist offen —
das unter dem Kamera-Fokus (BVH-Hover/Kameraziel, seit #135). Nachbarn schälen
nicht mit. Begründung: Cutaway hängt am globalen Orbit-Radius; würden alle
Gebäude im Radius aufschälen, bauen beim Reinzoomen Dutzende Interiors
gleichzeitig → Frame-Hitch + Instanz-Explosion. Ein Fokus-Gebäude hält das
Budget hart begrenzt; die Kamera kann ohnehin nur eines inspizieren. Nachbarn
behalten die emissiven Fenster.

- **LOD-Leiter:** fern = Massing/Impostor (bestehend) → mittel =
  Shell + Fensterlicht (bestehend) → Fokus = volles mehrstöckiges Interior aus
  `interiors.bin`.
- **Hitch-Vermeidung:** Build amortisiert (ein Geschoss pro Frame); Prefetch
  des wahrscheinlichen nächsten Fokus-Gebäudes aus der Kamerabewegung;
  LRU-Cache der letzten ~6 Interiors; gemeinsamer Prop-`geometryCache` +
  BatchedMesh pro Prop-Kategorie.
- **Dollhouse-Peel:** Cut-Höhe snappt auf **Geschossgrenzen** des
  Fokus-Gebäudes und wandert mit dem Zoom etagenweise von oben nach unten.
  Pro Etage eine `smoothstep`-Rampe, die Shell-Fragmente *aus*- und —
  **dieselbe Rampe** — das Etagen-Interior *ein*blendet. Hysterese gegen
  Flackern. Nie ein Boolean-Pop, nie ein Schnitt mitten durch die Wand.
  Möbel + Agenten faden mit ihrer Etage.
- **Fit-Fixes am KSW:** Hauptgebäude-Wahl nur noch **einmal** (eine Quelle für
  `main.ts` und `kswCampus.ts`); Zonen füllen den vollen Footprint bis zur
  Fassade; Interior-Basis = Gebäude-Bodenkote aus dem Bake (Gemeinde ist
  hügelig — y=0 wäre falsch).
- **Budget:** Fokus-Interior ≤ ~50k zusätzliche sichtbare Dreiecke; 120fps
  bleibt Ziel.

## 9. InteriorSpec (Protobuf-Skizze)

```proto
message InteriorWorld {
  string world_id = 1;
  string attribution = 2;            // ODbL / BFS / swisstopo credits
  repeated BuildingInterior buildings = 3;
}
message BuildingInterior {
  string uuid = 1;                   // swissBUILDINGS3D, bake-stabil
  float base_y = 2;                  // Bodenkote (Terrain)
  float storey_h = 3;
  uint32 seed = 4;                   // fnv1a(uuid)
  repeated Storey storeys = 5;
  optional string tenant_name = 6;   // OSM name (Beschilderung)
}
message Storey {
  uint32 level = 1;                  // 0 = EG
  repeated Unit units = 2;           // Wohnung / Gewerbe-Fläche
  repeated Rect corridors = 3;
  Rect core = 4;                     // Treppe/Lift
}
message Unit {
  string archetype = 1;              // 'apartment' | 'baeckerei' | ...
  optional uint64 ewid = 2;
  repeated Room rooms = 3;
}
message Room {
  string id = 1;                     // uuid/storey/index
  Rect rect = 2;                     // Weltmeter, wie Footprints
  string kind = 3;                   // 'bedroom' | 'kitchen' | 'salesfloor' ...
  uint32 furnish_seed = 4;           // Client re-instanziiert deterministisch
  repeated Door doors = 5;
}
message Rect { float x = 1; float z = 2; float w = 3; float d = 4; }  // Zentrum + Ausdehnung, Weltmeter
message Door { float x = 1; float z = 2; float yaw = 3; }
```

## 10. Eventualitäten-Matrix

| Fall | Verhalten |
|---|---|
| Footprint mit Innenhof (Ring mit Loch) | Even-odd-Test in der Rasterisierung; Räume nur im Mauerring |
| Kleinstgebäude <20m² | kein Interior, bleibt solide |
| 20–60m² | Ein-Raum-Interior (`garage`/`kiosk`/`schuppen`) |
| Riesenhalle >1200m²/Etage (Sulzer-Areal) | `industriehalle`: offenes Volumen, keine Korridor-Leiter |
| Mehrere EGIDs pro Footprint | Units pro EGID, Merge nach §4.3 |
| Mehrere POIs im selben Gebäude | mehrere EG-Units, Split entlang Fassade nach POI-Position |
| POI ohne `level` | EG angenommen |
| Widersprüchliche/fehlende GWR-Felder | Schätzer-Kaskade §4.3, im Bake-Report gezählt |
| Dachgeschoss (First ≫ Traufe) | oberstes Geschoss endet an der Traufe, Dachvolumen unmöbliert |
| Untergeschosse | out of scope |
| Altstadt-Reihenbauten (geteilte Wände) | Gebäude unabhängig; Fokus-Dollhouse zeigt eh nur eines |
| Konkave/rotierte Footprints | Zerlegung entlang PCA-Hauptachse |
| WSTWK ausserhalb GASTW-Bereich | clampen + Report |
| Hotel/Heim ohne Zimmerdaten | Zimmerraster aus Fläche geschätzt (Report) |

## 11. Tests & Verifikation

- **TDD auf Generator-Invarianten:** jeder Raum im Footprint; keine
  Raum-Überlappung; Korridor-Konnektivität (jeder Raum erreicht den Kern);
  Türen erreichbar; Unit-Fläche ±10% von WAREA; Slabs ≤ Gebäudehöhe;
  Determinismus (gleicher Input → identisches Artefakt).
- **Golden-Hash** auf `interiors.bin`.
- **Browser-Smoke Pflicht** pro Phase (Repo-Regel; `smoke-ksw.mjs`-Muster):
  Zoom-Fahrt aufs Fokus-Gebäude, Assertion dass Interior gebaut + Peel-Uniforms
  laufen. „Tests grün" ist kein Ersatz.
- **Visueller Abnahme-Pass pro Archetyp/Prop-Pack:** Capture-Screenshots
  (`capture-ksw.mjs`-Muster) pro Musterraum; Ästhetik ist explizites
  Abnahmekriterium, iteriert wird auf die Bilder.
- Volle CI-Gate vor jedem Push; PRs gegen `origin/main`.

## 12. Phasing (je ein PR)

- **A — KSW-Fix:** mehrstöckige Klinik-Zonierung + Dollhouse-Peel v2 +
  Voll-Footprint-Zonen + einmalige Hauptgebäude-Wahl + Terrain-Basis.
  Referenz-Härtung des Peels und der Geschoss-Slabs.
- **B1 — Daten:** `fetch-gwr` + `fetch-osm` + Join-Kaskade + Bake-Report +
  `derive-dwelling-priors` (Swiss Dwellings → Priors-JSON, §6a).
- **B2 — Generator + Client:** Layout-Generator (Wohnen + Kern-Archetypen),
  `interiors.bin`, `buildInteriorFromSpec`, Fokus-LOD + Prefetch + LRU.
- **B3 — Fitout in Prop-Packs:** Wohnen → Gastro/Retail → Büro/Praxis →
  Industrie/Öffentlich; Mieter-Beschilderung; je Pack ein visueller
  Abnahme-Pass.

## 13. Literatur (APA 7)

- Raistrick, A., Mei, L., Kayan, K., Yan, D., Zuo, Y., Han, B., Wen, H.,
  Parakh, M., Alexandropoulos, S., Lipson, L., Ma, Z., & Deng, J. (2024).
  Infinigen Indoors: Photorealistic indoor scenes using procedural generation.
  In *Proceedings of the IEEE/CVF Conference on Computer Vision and Pattern
  Recognition (CVPR)* (pp. 21783–21794). https://arxiv.org/abs/2406.11824
- Standfest, M., Franzen, M., Schröder, Y., Gonzalez Medina, L., Villanueva
  Hernandez, Y., Buck, J. H., Tan, Y.-L., Niedzwiecka, M., & Colmegna, R.
  (2022). *Swiss Dwellings: A large dataset of apartment models including
  aggregated geolocation-based simulation results* [Data set]. Zenodo.
  https://doi.org/10.5281/zenodo.7070952
- van Engelenburg, C., Mostafavi, F., Kuhn, E., Jeon, Y., Franzen, M.,
  Standfest, M., van Gemert, J., & Khademi, S. (2024). MSD: A benchmark
  dataset for floor plan generation of building complexes. *arXiv*.
  https://arxiv.org/abs/2407.10121
- Bruls, M., Huizing, K., & van Wijk, J. J. (2000). Squarified treemaps. In
  W. C. de Leeuw & R. van Liere (Eds.), *Data Visualization 2000* (pp. 33–42).
  Springer. https://doi.org/10.1007/978-3-7091-6783-0_4
- Merrell, P., Schkufza, E., & Koltun, V. (2010). Computer-generated
  residential building layouts. *ACM Transactions on Graphics, 29*(6),
  Article 181. https://doi.org/10.1145/1882261.1866203
- Wu, W., Fu, X.-M., Tang, R., Wang, Y., Qi, Y.-H., & Liu, L. (2019).
  Data-driven interior plan generation for residential buildings.
  *ACM Transactions on Graphics, 38*(6), Article 234.
  https://doi.org/10.1145/3355089.3356556
- Yu, L.-F., Yeung, S.-K., Tang, C.-K., Terzopoulos, D., Chan, T. F., &
  Osher, S. J. (2011). Make it home: Automatic optimization of furniture
  arrangement. *ACM Transactions on Graphics, 30*(4), Article 86.
  https://doi.org/10.1145/2010324.1964981

**Datenquellen:** swisstopo swissBUILDINGS3D (Footprints/Höhen, bereits
gebakt) · BFS GWR (Geschosse, Wohnungen, WAREA/WAZIM; Level-A open data,
Download-Service pro Perimeter, Attribution Pflicht) · ÖREB/GWR
Attribute-Join (bereits in `building-attributes.json`, #135) · OpenStreetMap
(POIs, ODbL — Attribution erforderlich) · Swiss Dwellings / Archilyse
(242k reale Schweizer Räume → Layout-Priors, CC BY 4.0 — Attribution
erforderlich).
