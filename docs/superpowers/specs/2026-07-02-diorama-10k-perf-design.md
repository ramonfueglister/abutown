# KSW-Diorama: GPU-driven Rendering für 10'000 Agenten/Objekte (Design)

**Datum:** 2026-07-02
**Status:** Design, autonome Umsetzung beauftragt (User: „mach die sauberste Variante, Zwischenschritte committen, Codex reviewt am Ende")
**Basis:** Branch `klinik/perf-10k` = Merge aus `klinik/look-prototype` (Wolken, SkyMesh-fog-Fix) + `claude/adoring-mayer-2ce3ca` (KSW-Szene, PR #110)
**Ziel:** 60 fps bei 10'000 Agenten + der vollen KSW-Szene, **WebGPU-only** (kein WebGL2-Fallback). Der Look (Clay, PCSS, GI, TRAA/GTAO/DOF/Bloom/Godrays, Wolken) bleibt unverändert.
**Tabu:** `src/diorama/ksw/cameraRig.ts` und der Kamera-Wiring-Block in `ksw/main.ts` (Zeilen ~208–258) — ein Kollege arbeitet parallel an der Kamera. Der Rig-State wird nur gelesen (radius → Fog/RoofFade/DOF-Fokus).

## Ist-Befund (Ursachen des Ruckelns)

Gemessen/gelesen auf dem Merge-Stand; Baseline aus Commit 5b08cc1: **49 fps Overview / 114 fps Interior** (headless, 72 Agenten).

1. **Kein Instancing, keine Geometrie-Wiederverwendung.** Jeder `box()`-Aufruf (`props.ts:47`) erzeugt eine **eigene** `RoundedBoxGeometry` und ein eigenes `THREE.Mesh`. Wände werden pro Raum in Bänder/Segmente zerlegt (`building.ts`), jedes Segment, jeder Fensterrahmen, jede Mullion, jede Scheibe, jedes Prop-Teil, jedes Personen-Teil = eigenes Mesh. Bei 24 Räumen, ~500 Props aus 56 Buildern (5–15 Teile je Prop) und 72 Personen (6–10 Teile) liegt die Szene bei grob **4'000–8'000 Meshes → ebenso viele Draw Calls**, nochmal fast verdoppelt durch die Shadow-Pass. Das ist der dominante, CPU-gebundene Kostenblock — und er skaliert linear mit jedem weiteren Objekt.
2. **Periodischer Hitch: GI-Cube-Probe.** `cubeCam.update(...)` (`ksw/main.ts:565–567`) rendert **alle 240 Frames die ganze Szene 6×** in einem einzigen Frame — der klassische „alle ~4 Sekunden ruckelt es kurz"-Spike.
3. **Shadow-Map jeden Frame neu**, obwohl die Szene fast vollständig statisch ist (Sonne steht außer im `cycle`-Modus still). Alle Tausende Meshes laufen jeden Frame zusätzlich durch die PCSS-Shadow-Pass.
4. **Per-Agent-Szenengraph:** jeder Agent ist eine `THREE.Group` mit Kind-Meshes; Position/Rotation/Squash werden pro Frame am Objektbaum gedreht (`ksw/main.ts:547–561`). Bei 72 ok, bei 10'000 unmöglich.
5. **Post-Stack** (MRT + TRAA + GTAO + Godrays + DOF + Bloom, pixelRatio bis 2) ist teuer, aber **konstant** — er ruckelt nicht, er senkt nur den Deckel. Er bleibt in v1 unangetastet (der Look ist der Punkt des Projekts).

## Architektur (Soll)

Drei Säulen, alle drei sind Standard-SOTA für 2026-WebGPU-Szenen dieser Art:

### 1 · Statische Szene → geteilte Geometrien + `BatchedMesh`
- **Geometrie-Cache:** `RoundedBoxGeometry` (und Cylinder/Sphere/Torus) werden nach Parametern gekeyt und wiederverwendet (`geometryCache` in `props.ts`).
- **Batching:** Alles Statische (Wände, Böden, Slabs, Props, Signage, Kanopien) wandert in wenige `THREE.BatchedMesh`-Buckets, gekeyt nach Material-Klasse:
  - `clay-opaque` — **ein** `MeshPhysicalNodeMaterial`; die heutige Pro-Farbe-Materialkarte wird durch **per-Instance-Color** ersetzt (`batchedMesh.setColorAt`), `sheenColorNode = mix(instanceColor, white, 0.5)` reproduziert das heutige Sheen-Rezept exakt.
  - `glass`, `glow` — je ein kleiner Bucket.
  - `roof-fade` — Dächer + Dachaufbauten in einem eigenen Bucket mit dem heutigen Fade-Material; `setFade` steuert wie bisher Opacity/Sichtbarkeit, nur eben an einem Batch statt an ~100 Meshes.
- Fenster-Nachtglühen (`windowPane`-Materialtausch) wird zu per-Instance-Color-Umschaltung im Glas/Glow-Bucket.
- **Budget: statische Szene ≤ ~10 Draw Calls** (heute Tausende). `blinkers`/`rotors` (Ambulanz, Heli) bleiben als die ~3 individuellen Meshes, die sie sind.

### 2 · Agenten → Storage-Buffer-Instancing + Shader-Animation + LOD
- **Datenhaltung statt Szenengraph:** kein `THREE.Group` pro Agent mehr. Pro Agent ein Slot in Storage-Buffern (`instancedArray`): `posXZ`, `yaw`, `walkPhase`/`squash`-Parameter, `roleId`, `lodFlags`.
- **Verhalten bleibt CPU, aber gedeckelt:** `updateAgent` (dwell/route/walk, `agents.ts`) ist O(1) pro Frame und Agent — bei 10k unkritisch. Teuer ist nur `routePath`; deshalb **Re-Plan-Budget**: max. K Routenplanungen pro Frame (K ≈ 64), Dwell-Abläufe werden gestaffelt statt synchron. CPU schreibt pro Frame die kompakten Buffer (10k × ~24 B ≈ 240 KB, trivial).
- **Animation im Vertex-Shader (TSL):** Watschel-Roll, Squash-and-Stretch, Yaw-Drehung und Gebäude-Bodenlift werden aus `(time, walkPhase, instanzdaten)` im Shader berechnet — exakt die heutigen Formeln aus `ksw/main.ts:553–560`, nur pro Instanz statt pro Objekt.
- **Geometrie-LOD pro Rolle:**
  - **LOD0 (nah):** die heutige Bohnen-Person inkl. Rollen-Accessoire, zu **einer** Geometrie pro Rolle gemerged (Vertex-Colors statt Multi-Material).
  - **LOD1 (mittel/fern):** eine Kapsel + Kopf in Rollenfarbe (~2 % der Vertices).
  - Ein `InstancedMesh` (bzw. BatchedMesh-Range) pro Rolle×LOD; LOD-Zuordnung pro Frame per **TSL-Compute-Pass** aus Kameradistanz.
- **Culling GPU-seitig, pragmatisch:** der Compute-Pass klassifiziert zusätzlich Frustum-/Distanz-Sichtbarkeit; unsichtbare Instanzen werden auf **Scale 0** kollabiert (Null-Flächen-Dreiecke sind auf der GPU quasi gratis). Indirect-Draw-Kompaktierung ist als Stretch notiert, nicht Bedingung — der Scale-0-Pfad ist robust, three-r185-nativ und reviewbar.
- **Agenten-Schatten:** 10k Figuren fallen aus der PCSS-Kaskade raus; sie bekommen **instanzierte Blob-Schatten** (weiche dunkle Scheibe am Boden — passt zur Claymation-Sprache). Die ~100 nächsten könnten später echte Caster werden (Stretch).
- **Spawn-Skalierung:** `?agents=N` (Default: die heutigen Plan-Personen). Zusätzliche Agenten werden deterministisch (seeded) auf Korridore/Outdoor-Slabs/Räume verteilt, Rollenmix aus dem Plan extrapoliert.

### 3 · Hitch-Killer
- **GI-Probe amortisieren:** die 6 Cube-Faces werden auf 6 aufeinanderfolgende Frames verteilt (manuelles per-Face-Rendering) **und** nur bei Bedarf aktualisiert (Sonnenstand geändert / RoofFade-Schwelle gekreuzt / `cycle`-Modus); statische Presets rendern die Probe nach dem Laden einmal fertig und nie wieder.
- **Shadow-Map cachen:** `sun.shadow.autoUpdate = false`; `needsUpdate` nur bei Sonnenbewegung (`cycle`) oder RoofFade-Schwellen (castShadow-Umschaltung). Im Normalbetrieb rendert die Shadow-Pass **null** statt Tausende Draw Calls pro Frame.
- Der `__KSW`-Debug-Snapshot (`ksw/main.ts:533`) wird auf ein Update alle ~15 Frames gedrosselt (Allokationen + `filter` über alle Agenten pro Frame).

## Fehlerbehandlung
- WebGPU nicht verfügbar → klare Vollbild-Meldung („Dieses Diorama braucht WebGPU"), kein stiller schwarzer Screen. Der bisherige `__LOOK_BACKEND`-Report bleibt.
- `?agents=N` wird auf [1, 20'000] geklemmt.
- Compute-/Storage-Pfade haben keine Fallbacks (No-legacy-Cruft-Regel): WebGPU-only ist die Plattformentscheidung.

## Verifikation (Abnahme)
1. **`scripts/fps-check.mjs` erweitert:** misst pro Preset zusätzlich `renderer.info.render.drawCalls` (via Debug-Hook) und läuft die Matrix `{overview, er} × {agents: default, 10000}`. **Gates:** Overview@10k ≥ 55 fps headless, Draw Calls Overview < 60, kein Frame > 40 ms während der Messung (Hitch-Gate für die GI-Probe).
2. **Look-Erhalt:** `scripts/capture-look.mjs`-Shots (morning/dusk/night × overview/er/ops) vor und nach dem Umbau, Seite-an-Seite-Vergleich; die Golden Shots sind die Messlatte.
3. **Browser-Smoke** (CLAUDE.md-Pflicht): `__KSW.agents.total === N`, Agenten-Samples bewegen sich, Roof-Fade funktioniert weiter.
4. Bestehende Vitest-Suites (`tests/diorama/*`) bleiben grün; neue Unit-Tests für Geometrie-Cache-Keying, Re-Plan-Budget-Scheduler und Spawn-Verteilung.

## Umsetzungs-Slices (je ein Commit)
- **A** Geometrie-Cache + Material-Konsolidierung (mechanisch, delegierbar).
- **B** BatchedMesh-Umbau der statischen Szene (Wände/Böden/Props/Dächer inkl. Fade + Nachtglühen).
- **C** Agenten auf Storage-Buffer-Instancing + Shader-Animation (72 Agenten, identisches Verhalten, Groups gelöscht).
- **D** 10k: Spawn-Skalierung, Re-Plan-Budget, LOD-Compute, Scale-0-Culling, Blob-Schatten.
- **E** Hitch-Killer: GI-Probe-Amortisierung, Shadow-Caching, `__KSW`-Drosselung.
- **F** Verifikation: erweiterte fps-Probe, Golden-Shot-Vergleich, Smoke, Doku (`progress.md`).

## Bewusst nicht in v1
- Indirect-Draw-Kompaktierung, echte Schatten für nahe Agenten, Impostor-LOD2 (erst wenn Kapsel-LOD nicht reicht), Post-Stack-Tuning (Half-Res-DOF etc.), WebGL2-Pfad (gestrichen).
