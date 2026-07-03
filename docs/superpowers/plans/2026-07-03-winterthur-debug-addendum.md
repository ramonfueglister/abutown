# Debug-Addendum: echte Hüllen, Shader-Fassaden, Baum-Silhouetten (T12–T14)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Fortsetzung von `2026-07-02-winterthur-diorama-style.md` nach Systematic-Debugging-Befund (2026-07-03).

**Bewiesene Root Causes:**
- A: 659/846 Gebäude haben >30% Dachpunkte außerhalb des Footprints — eine swisstopo-UUID enthält oft MEHRERE getrennte Gebäudeteile; Ein-Ring-Prisma + Bbox-Gate tragen sie nicht → schwebende Dächer. Prisma-Modell ist für Multi-Part falsch.
- B: 176 Gebäude haben Fensterreihen über der Wandoberkante (bis 17.4 m) — facadeLayout nutzt Firsthöhe, Wände enden an der Traufe.
- C: Fern-Baum-Impostor = einheitliche Einzelkugel ohne Silhouette/Tint-Struktur.

**Global Constraints:** wie Hauptplan (Hero pixel-treu, Geodäsie, additive Tokens, Screenshot-Gate je Task inkl. Vorher/Nachher, `npx vitest run && npm run typecheck && node scripts/smoke-ksw.mjs` grün, Bake via `export PATH="/opt/anaconda3/bin:$PATH" && npm run geo:bake`, data ≤ 8 MB).

---

### Task 12: Echte LoD2-Wände statt Prisma (Schwebe-Root-Fix)

**Files:** `scripts/geo/lib/transform.mjs`, `scripts/geo/bake-winterthur.mjs` (Gate), Tests `tests/geo/transform.test.ts`, Re-bake `data/winterthur/`.

- `transformBuildings`: `wall`-Mesh = **echte Wand-Facetten** `meshFromRings(b.walls, groundY)` (Welding existiert) statt `extrudeWalls(footprint,…)`. Giebel-Skirts (Task 3) BEHALTEN und weiter dem Wall-Mesh anhängen (sie schließen echte Lücken zwischen Traufe und Dach). `extrudeWalls` nur noch als Fallback, wenn ein Gebäude 0 Wand-Facetten hat (zählen + loggen).
- Footprint bleibt für Plinth/Türen/Fassaden-Metadaten (unverändert traced/hull).
- **Neues Bake-Gate (Daten-Beweis):** pro Gebäude Anteil der Dach-Eckpunkte, deren XZ innerhalb 2 m eines Wand-Basispunkts liegt; Gesamtquote < 90 % → throw; Report druckt die Quote. Ziel nach Fix: > 95 % (vorher-Metrik: 659/846 schlecht).
- Test: synthetisches 2-Teil-Gebäude (eine UUID, zwei getrennte Wand-/Dachpaare) → BEIDE Teile haben Wände unterm Dach (die alte Prisma-Version deckt nur Teil 1 — Test muss vorher rot sein).
- Plinth-/Traufband (`cityMassing`) laufen weiter über den Footprint — akzeptiert: Bänder nur am Hauptteil (Folge-Politur), aber KEIN Teil mehr ohne Wände.
- Screenshot-Gate: `t12-bahnhof`, `t12-city` + ein Nah-Zoom-Vergleich; Read + Urteil: keine schwebenden Dächer mehr in dichten Zeilen.
- Größen-Gate: welded Walls ≈ Task-3-Messung (13.1 MB naiv → ~6.5 gewelded); wenn > 8 MB: Dach-Underside droppen (0.22-Slab bleibt via Skirts sichtbar) UND im Commit dokumentieren.

### Task 13: Prozedurale TSL-Shader-Fassaden (SOTA-Fenster) — Modell: opus

**Files:** `scripts/geo/lib/transform.mjs` (Fassaden-Attribute), `src/diorama/ksw/geo/cityMassing.ts` (Facade-Material), `src/diorama/ksw/geo/windows.ts` (nur noch Türen), `src/diorama/ksw/main.ts` (Wiring), Tests.

- **Bake:** pro Wand-Vertex zwei Attribute mitschreiben: `uv = [u, v]` mit u = Meter entlang der Wandfläche horizontal (Facette-lokal: Distanz des Vertex entlang der Facetten-Hauptrichtung), v = Höhe über Gebäudegrund (m); plus pro Vertex `eaveH` (Traufhöhe des Gebäudes, m) als drittes Attribut (oder uv.z). Kompakt quantisiert (dm reicht). buildings.json wächst — Budget prüfen.
- **Runtime (TSL, im tintedClay-Klon der cityWalls):** Fenstermuster im Fragment: Raster aus `kswCityStyle` (storeyH 3, spacing 2.4, Fenster 1.3×1.4, sill-Anteil), NUR wo `v + 0.2 < eaveH` und Stockwerk vollständig unter Traufe → nie über der Wand. Look: eingelassene Fenster — Fensterfläche dunkler Glass-Ton (palette.glass-Familie), umlaufender RAHMEN in Weiß (Original-Sprache, wie segmentWall-Jambs), leichte Normal-Abdunklung am Rand für Tiefe. Nachts: `nightWindowHash(worldX floor(u/spacing)-Zelle, worldZ)`-Anteil `NIGHT_WINDOW_SHARE` glüht warm (0xffd9a0, emissiveNode), tagsüber keine Emission. Preset-Flag wie gehabt (`lampGlow`).
- **Instanz-Fenster ENTFERNEN** (frames/panes/glow aus windows.ts raus; `cityDoors` bleibt instanziert). LOD-Ref `windows` zeigt danach auf nichts Fensterartiges mehr → LOD-Tabelle: Fern-Ring blendet stattdessen das Fenster-MUSTER aus (uniform `facadeDetail` 0/1 im Shader, von applyCityLod gesetzt via Callback-Ref) — Übergang weich (smoothstep über radius optional simpel binär).
- Tests: Bake-Attribut-Test (u monoton entlang Facette, v = Höhe, eaveH konstant pro Gebäude); windows.ts-Türen-Test; LOD-Ref-Anpassung.
- Screenshot-Gate: `t13-bahnhof` (morning + night!) — Fenster sitzen IN der Wand, weiße Rahmen, exakt an Stockwerken, nichts über der Traufe, Nacht-Glow erhalten; `t13-city` ruhig; Hero unverändert. Vorher/Nachher gegen t6/t12-Shots.
- Perf: 154k Instanzen entfallen; `node scratch/perf-measure.mjs city` dokumentieren.

### Task 14: Baum-Impostor mit Silhouette + Variety

**Files:** `src/diorama/ksw/geo/nature.ts`, Tests `tests/geo/nature-render.test.ts`.

- Impostor-Geometrie = Low-Poly-Merge der echten 4-Puff-Krone (IcosahedronGeometry detail 0 pro Puff) statt Einzelkugel; Nadel-Impostor = Einzel-Kegel. Impostoren übernehmen dieselben per-Instanz-Tints wie die Voll-Bäume (bereits setColorAt — sicherstellen, dass BEIDE kinds im Impostor ihre kind-Farbe behalten).
- Variety: Tint-Spreizung der Kronen verdoppeln (deterministisch aus x/z), Kronen-Default broad r 3→2.4 im BAKE? NEIN — Bake-Defaults sind Spec-deklariert; stattdessen rein visuell: Impostor-/Kronen-Y-Squash-Varianz erhöhen (0.85–1.15 aus Hash) und Tint ±0.08 L. Keine Datenänderung.
- Test: Impostor-Geometrie hat >1 zusammenhängende Kugel (Vertex-Count > Einzel-Ico), Conifer-Impostor Kegel; Tints pro kind unterschiedlich.
- Screenshot-Gate: `t14-city` (Fern-Ring!) — Bäume lesen sich als gestufte Silhouetten mit Farbvariation statt uniformer Kreise; `t14-overview` Hero unverändert.

**Reihenfolge:** 12 → 13 → 14. Nach 14: Voll-Gate + Push + PR-Kommentar.
