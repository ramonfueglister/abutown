# KSW-Diorama + dynamische Kamera — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dynamische Diorama-Kamera (Mausrad-Zoom, Linksklick-Drag-Rotation) und ein komplettes Kantonsspital Winterthur (KSW) auf einer Ebene im bestehenden Clay-Diorama-Stil, mit licht-wirksamen Dächern, die beim Reinzoomen ausfaden.

**Architecture:** Basis ist `klinik/look-prototype@29e0db1` (Design-Tokens, Clay-Materialrezept, Sky/GI/Post-Stack, ER-Raum-Prototyp). Dessen `src/diorama/look.ts` bleibt UNANGETASTET (der andere Agent iteriert dort weiter). Das KSW lebt in `src/diorama/ksw/` mit eigenem Entry `ksw.html`: ein deklarativer Grundriss (`floorPlan.ts`) treibt einen Gebäude-Builder (`building.ts`), eine erweiterte Prop-Bibliothek (`props.ts`) liefert alle Geräte, ein pure-math Kamera-Rig (`cameraRig.ts`) liefert Orbit/Zoom + Roof-Fade-Signal, `main.ts` bindet alles an den (adaptierten) Licht/Post-Stack des Prototyps.

**Tech Stack:** three.js r185 WebGPURenderer + TSL, Vite 8, Vitest, Playwright-Capture-Harness (`scripts/capture-ksw.mjs`, Muster `capture-look.mjs`).

## Global Constraints

- **Kein Hexwert außerhalb `src/diorama/designTokens.ts`** — neue Farben/Werte werden dort als Tokens ergänzt (append-only, um Konflikte mit dem look-Agenten zu minimieren).
- **Alles prozedural** — keine Assets, keine gemodelten Meshes (Spec 2026-07-01-klinik-diorama-design.md).
- Chunky-Clay-Formsprache: nur `radii`-Skala, keine dünnen Stäbe/scharfen Kanten; Materialien nur via `clayMat`-Rezept.
- USK-12-diskret: keine Blut-/Leidensdarstellung; Geräte realistisch benannt, Darstellung mild.
- `tsconfig.json` hat `"include": ["src"]`; Typecheck-Gate ist `npm run typecheck` (src+tests+scripts).
- Cargo nie parallel (hier irrelevant — reine Frontend-Arbeit).
- `look.ts`, `look.html`, `capture-look.mjs` nicht modifizieren (Parallel-Agent).
- Kamera-Verhalten: Mausrad = Zoom rein/raus; Linksklick+Halten+Ziehen = Rotation (Yaw frei, Pitch sanft geklemmt); Dach-Fade ist eine reine Funktion des Zoom-Radius.
- Dächer: echte Meshes mit `castShadow`/`receiveShadow` (Licht berücksichtigt sie); Fade über Material-Opacity, Schattenwurf endet unterhalb Fade-Schwelle, `visible=false` unter 0.02.

---

### Task 1: Basis übernehmen (Look-Prototyp in den Arbeits-Branch)

**Files:** keine neuen — Git-Operation + Toolchain.

- [x] **Step 1:** `git merge klinik/look-prototype` (Fast-Forward von 7d1ff07 auf 29e0db1; eigener Branch hat keine eigenen Commits).
- [x] **Step 2:** `npm ci` im Worktree; `npx playwright install chromium` falls nötig.
- [x] **Step 3:** Gates grün: `npm run typecheck && npm test` und ein Referenz-Capture `node scripts/capture-look.mjs base morning` → `CAPTURE OK`.

### Task 2: Kamera-Rig als pure Math (TDD)

**Files:**
- Create: `src/diorama/ksw/cameraRig.ts`
- Test: `tests/diorama/cameraRig.test.ts`

**Interfaces (Produces):**
```ts
export type CameraRigState = { yaw: number; pitch: number; radius: number; target: [number, number, number] };
export function rigFromLookAt(position: [number,number,number], target: [number,number,number]): CameraRigState;
export function applyZoom(s: CameraRigState, wheelDeltaY: number, cfg: RigConfig): CameraRigState;
export function applyDrag(s: CameraRigState, dxPx: number, dyPx: number, cfg: RigConfig): CameraRigState;
export function rigPosition(s: CameraRigState): [number, number, number];
export function roofFade(radius: number, cfg: RigConfig): number; // 1 = Dach voll da (weit weg), 0 = weg (nah)
export type RigConfig = { radiusMin: number; radiusMax: number; zoomSpeed: number; dragSpeed: number; pitchMin: number; pitchMax: number; roofFadeNear: number; roofFadeFar: number };
```
Zoom exponentiell (`radius *= exp(deltaY * zoomSpeed)`, geklemmt), Drag: `yaw -= dx*dragSpeed`, `pitch += dy*dragSpeed` geklemmt. `roofFade` = smoothstep(roofFadeNear, roofFadeFar, radius). Konfig-Defaults kommen als Token `kswCamera` in `designTokens.ts`.

- [x] Failing Tests (Roundtrip lookAt↔position, Zoom-Klemmen, Monotonie, Yaw-Wrap-frei, Pitch-Klemmen, roofFade 0/1/monoton) → RED
- [x] Implementierung → GREEN (`npx vitest run tests/diorama/cameraRig.test.ts`)
- [x] Commit `feat(ksw): pure orbit camera rig with roof-fade signal`

### Task 3: KSW-Grundriss deklarativ + Invarianten-Tests

**Files:**
- Create: `src/diorama/ksw/floorPlan.ts`
- Test: `tests/diorama/floorPlan.test.ts`

**Schema (Produces):**
```ts
export type PropPlacement = { kind: string; x: number; z: number; rotY?: number; scale?: number };
export type PersonPlacement = { role: 'nurse'|'doctor'|'patient'|'child'|'visitor'|'surgeon'|'labtech'|'paramedic'; x: number; z: number; yaw: number };
export type Room = {
  id: string; label: string; accent: number;          // accent = Token-Farbe für Signage/Boden-Teppich
  rect: { x: number; z: number; w: number; d: number }; // Außenkante in Weltkoordinaten (eine Ebene, y=0)
  doors: Array<{ wall: 'n'|'s'|'e'|'w'; center: number; width: number }>;
  windows: Array<{ wall: 'n'|'s'|'e'|'w'; center: number; width: number }>;
  props: PropPlacement[];
  people: PersonPlacement[];
};
export const kswPlan: { plate: { w: number; d: number }; corridors: Array<{x:number;z:number;w:number;d:number}>; rooms: Room[] };
```

**Abteilungen (alle auf einer Ebene, KSW-Roster):** Eingangshalle/Empfang, Interdisziplinäres Notfallzentrum (Triage + 3 Kojen + Schockraum), Radiologie (Röntgen, CT, MRI getrennt), Zentral-OP (2 Säle + Einleitung), Intensivstation (IPS, 3 Plätze), Bettenstation Chirurgie (4 Betten), Bettenstation Medizin (4 Betten), Geburtsabteilung + Neonatologie (Gebärsaal, 2 Inkubatoren), Kinderklinik, Onkologie-Tagesklinik (Infusionssessel), Dialyse (3 Plätze), Labor, Spitalapotheke, Physiotherapie, Kardiologie/Herzkatheter, Endoskopie, Cafeteria/Restaurant, Verwaltung/Büro. Korridor-Kreuz als Erschliessung, Rettungszufahrt mit Ambulanz vor dem Notfall, Eingangsvorplatz mit Pflanzen/Bänken.

**Invarianten-Tests:** Räume überlappen nicht (paarweise Rect-Test, Korridore ausgenommen), alle Räume innerhalb der Plate, jede Tür/Fenster liegt innerhalb ihrer Wandlänge, jeder Raum hat ≥1 Tür, alle Prop-Positionen innerhalb des Raum-Rects, alle `kind`-Strings existieren in der Prop-Registry (Import aus Task 4 — Test wird nach Task 4 grün), Personen innerhalb Raum/Korridor.

- [x] Tests RED → Plan-Daten schreiben → GREEN → Commit `feat(ksw): declarative KSW floor plan with invariant tests`

### Task 4: Prop-Bibliothek (alle Geräte)

**Files:**
- Create: `src/diorama/ksw/props.ts` (Registry `propBuilders: Record<string, () => THREE.Group>`)
- Modify: `src/diorama/designTokens.ts` (append: `kswPalette`-Ergänzungen, `kswCamera`, `kswScene`)

**Bestand portieren (aus look.ts kopiert, look.ts bleibt unberührt):** `hospitalBed`, `careCart`, `ivStand`, `vitalsMonitor`, `plant`, `sideTable`, `beanPerson` (+ Rollen-Accessoires), `wallWithWindows`-Hilfen wandern nach `building.ts`.

**Neue Geräte (jeder Builder chunky-clay, nur Token-Farben):**
Empfang: `receptionDesk`, `waitingBench`, `infoBoard`, `wheelchair`. Notfall: `triageDesk`, `stretcher`, `defibrillator`, `shockroomLight`, `ambulance` (aussen, mit mildem Blinklicht-Mesh). Radiologie: `xrayMachine` (Säule+Detektor+Tisch), `ctScanner` (Gantry-Torus + Liege), `mriScanner` (dicker Ring + lange Liege), `leadShieldWindow`, `radiologyConsole`. OP: `opTable`, `opLightDouble` (Deckenarm + 2 Leuchtscheiben), `anesthesiaMachine`, `instrumentTable`, `scrubSink`. IPS: `icuBed` (Bett + Monitorbrücke), `ventilator`. Geburt/Neo: `birthingBed`, `incubator` (Glashaube!), `babyCrib`. Onko/Dialyse: `infusionChair`, `dialysisMachine`. Labor: `labBench`, `microscope`, `centrifuge`, `sampleRack`. Apotheke: `pharmacyShelf`, `counterDesk`. Physio: `physioTable`, `exerciseBike`, `parallelBars`, `gymBall`. Kardio: `cathLabArm` (C-Bogen), Endoskopie: `endoscopyTower`. Cafeteria: `cafeTable`, `cafeChair`, `counterBar`, `espressoMachine`. Verwaltung: `officeDesk`, `officeChair`, `filingCabinet`, `deskPlant`. Überall: `ceilingSign` (Abteilungs-Schild in Akzentfarbe), `linenCart`, `handSanitizer`.

Masse: Menschenmass Kapsel ~0.9 hoch → Betten ~2.0×0.95, CT-Gantry Ø~2.2, MRI-Ring Ø~2.6 länge ~3.4, OP-Tisch 2.1×0.8. Alles `box`/`cylinder`/`torus`/Kapsel-Vokabular mit `radii`.

- [x] Test: Registry-Vollständigkeit gegen `kswPlan` (Task-3-Test wird grün); Smoke-Test: jeder Builder liefert Group mit >0 Kindern und castShadow-Meshes.
- [x] Commit `feat(ksw): full procedural equipment library for all departments`

### Task 5: Gebäude-Builder (Wände, Türen, Böden, Dächer)

**Files:**
- Create: `src/diorama/ksw/building.ts`
- Test: `tests/diorama/building.test.ts`

**Interfaces (Produces):**
```ts
export function buildHospital(plan: typeof kswPlan): { group: THREE.Group; roofs: RoofControl };
export type RoofControl = { setFade(fade01: number): void }; // 1=Dach voll, 0=weg
```
- Wände: `wallSegments`-Funktion (verallgemeinertes `wallWithWindows` mit Tür-Öffnungen bis Boden). Aussenwände creamBase, Innenwände creamLight, Höhe 2.9, Dicke 0.42/0.28 (aussen/innen). Gemeinsame Wände zwischen Raum und Korridor nur EINMAL bauen (Raum besitzt seine 4 Wände; Korridor ist wandlos).
- Boden je Raum `floorWarm`, Akzent-Teppich/Streifen in Raum-Akzentfarbe; Korridorboden `white`-getönt (Token `kswPalette.corridorFloor`).
- **Dächer:** pro Raum ein RoundedBox-Deckel (Überstand 0.18, Dicke 0.24, Token `kswPalette.roofClay` Terracotta) + flacher Aufsatz; Material: EIN geteiltes `MeshPhysicalMaterial` (Clay-Rezept, `transparent:true`); `RoofControl.setFade(f)`: `opacity=f`, alle Roof-Meshes `castShadow = f > 0.5`, `visible = f > 0.02`.
- Tests: Segment-Zerlegung (Tür in Wandmitte → 2 Segmente + Sturz), Dach deckt Raum-Rect + Überstand, setFade toggelt castShadow/visible an den Schwellen.

- [x] RED → Implementierung → GREEN → Commit `feat(ksw): building generator — walls, doors, floors, light-aware fading roofs`

### Task 6: Szene + Entry + Kamera-Verdrahtung

**Files:**
- Create: `src/diorama/ksw/main.ts`, `ksw.html`
- Modify: `src/diorama/designTokens.ts` (`kswScene`: Plate-Masse, Fog-Skalierung, Dome-Radius 160, Shadow-Frustum ±40, MapSize 4096, GI-Probe-Höhe, DOF-Kopplung)

- Sky/Sonne/Wolken/Sterne/Post-Stack vom Prototyp adaptiert (GTAO→DOF→Bloom→Grade→Film; Godrays tagsüber), Presets `morning|dusk|night` via `?preset=`.
- Kamera: `PerspectiveCamera(fov 24)`; Rig-State aus `kswCamera`-Token (`overviewPosition`/`target`); `wheel`-Listener (passive:false, preventDefault) → `applyZoom`; `pointerdown(button 0)`/`pointermove`/`pointerup` → `applyDrag`; pro Frame `camera.position.set(...rigPosition(state))` + `lookAt(target)`.
- **DOF folgt dem Zoom:** `focusDistance` als `uniform`, pro Frame = `state.radius` (Prototyp-Wert 16.5 ≈ Kameradistanz — gleiche Kopplung).
- **Roof-Fade pro Frame:** `roofs.setFade(roofFade(state.radius, cfg))`.
- `?cam=overview|er|ops`-Presets für den Capture-Harness (deterministisch, ohne Interaktion); `window.__LOOK_READY/__LOOK_BACKEND` wie Prototyp.
- Personen aus `plan.people` als `beanPerson` mit Rollen-Accessoire, Idle-Squash-Animation; Ambulanz-Blinklicht sanft pulsend.

- [x] Verifikation: Dev-Server + Browser-Smoke (Zoom ändert Radius, Drag ändert Yaw, Dach fade-out beim Reinzoomen sichtbar), keine Console-Errors.
- [x] Commit `feat(ksw): full KSW diorama scene with dynamic camera and roof fade`

### Task 7: Capture-Harness + volle Gates

**Files:**
- Create: `scripts/capture-ksw.mjs` (Kopie-Adaption von capture-look.mjs: lädt `/ksw.html?preset=&cam=&zoom=`, Zoom-Parameter setzt Rig-Radius für Nah-Shots)

- [x] Shots: `overview-morning` (Dächer voll), `inside-er` (reingezoomt, Dächer weg, Geräte sichtbar), `overview-dusk`, `overview-night` — alle `CAPTURE OK`, Bilder gegen Stil-Messlatte (Prototyp-Golden-Shots) reviewen.
- [x] Volle Gates: `npm run typecheck && npm test && npm run build`.
- [x] Commit + PR gegen main (Basis-Commits des look-Branch sind enthalten; PR-Beschreibung weist darauf hin).

## Self-Review
- Spec-Abdeckung: Kamera (Zoom+Rotation) ✓ Task 2/6; komplettes KSW eine Ebene ✓ Task 3; Diorama-Stil ✓ Tokens/clayMat überall; Dächer licht-wirksam + Fade ✓ Task 5/6; detaillierte Geräte ✓ Task 4; Verifikation Screenshot-Harness ✓ Task 7.
- Platzhalter: Geräteliste ist explizit enumeriert; Masse/Farbquellen definiert; Builder-Details folgen dem etablierten Baustein-Vokabular aus look.ts (im Repo einsehbar) — kein TBD.
- Typkonsistenz: `RigConfig`/`RoofControl`/`Room` einmal definiert, überall gleich benannt.
