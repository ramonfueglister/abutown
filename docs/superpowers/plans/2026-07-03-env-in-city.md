# Echtzeit-Environment in der Winterthur-Stadt-App — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Die Winterthur-Stadt-App (`/`, `src/diorama/ksw/`) läuft strikt in Echtzeit mit echter Sonne/Mond/Sternen und Live-Open-Meteo-Wetter (volle Parität zum Prototyp inkl. Regen/Schnee), `?preset=`/`?cycle=` → `?at=`/`?wx=`.

**Architecture:** Phase 1 holt das `environment/`-Modul per echtem Git-Merge von `klinik/look-prototype` (Konflikte semantisch trivial, aber die designTokens-Auflösung ist eine ECHTE UNION — der Wetter-Branch hat Tokens gelöscht, die die Stadt noch braucht). Phase 2 macht das Modul wiederverwendbar (Sternenfeld/Mond/Partikel parametrisiert), verlegt das Nacht-Glühen der Stadt von Bake-Zeit auf eine Laufzeit-Uniform und verdrahtet `ksw/main.ts` auf `computeEnvironment` mit einem stadt-eigenen `applyCityEnvironment`.

**Tech Stack:** TypeScript, three.js r185 WebGPU/TSL, suncalc, Open-Meteo, vitest, playwright.

**Spec:** `docs/superpowers/specs/2026-07-03-env-in-city-design.md`

**Branch/Worktree:** `klinik/env-in-city` in `/Users/ramonfuglister/Coding/abutown/.worktrees/winterthur-main` (ab origin/main `318cfd0`). Zeilennummern für `src/diorama/ksw/main.ts` beziehen sich auf den Stand `318cfd0` (1079 Zeilen) — nach dem Merge in Task 1 verschieben sie sich NICHT (der Merge fasst ksw/ nicht an), nach eigenen Edits schon: immer gegen die Datei verifizieren.

## Global Constraints

- Szenen-Konvention (Stadt = Geo-Daten = solar.ts): **+x = Ost, +z = Süd, +y = oben** (`scripts/geo/lib/project.mjs`). Keine Rotations-Shims.
- Design-Token-Regel: keine Farb-/Material-/Radius-Werte ausserhalb `src/diorama/designTokens.ts`.
- Kein toter Code am Ende: alles Preset-Keyed (`lightPresets`, `skyPhys`, `cloudCfg.coverage`, `kswPost.skyUnfogged`, `kswPost.godraysMix[preset]`, `kswPost.envScale[preset]`, `gi.hemiCut`, `sunArcCfg`-Bogen-Felder, `moonLight.position`) fliegt raus — aber ERST wenn der letzte Nutzer umgestellt ist (Task 5), nicht im Merge.
- Per-Frame-Budget: nur Uniform-/Mutable-Writes, keine Allokationen, keine Instanz-Loops.
- Der pure Kern (`environment/solar.ts`, `weather.ts`, `environment.ts`) wird NICHT verändert (nur konsumiert); `precipitation.ts`/Nachthimmel werden parametrisiert, Prototyp-Werte bleiben default → Prototyp-Smoke muss unverändert 10/10 bleiben.
- Tests `npm test` (vitest), Typecheck `npm run typecheck`, Build `npm run build`, Smoke `node scripts/smoke-environment.mjs`. Browser-Smoke ist Pflicht-Gate (CLAUDE.md) — auch für die Stadt-Seite.
- Hero-Guard der Stadt respektieren: Shadow-Rig/`currentSunDir`-Mechanik (main.ts:761-795) bleibt strukturell erhalten — nur die QUELLE des Sonnenvektors wechselt.
- Dev-Server auf 127.0.0.1:5175, Port-Hygiene (vorher prüfen, nachher killen).

## File Structure (Ziel)

```
[Merge] src/diorama/environment/*            unverändert vom Wetter-Branch (5 Dateien)
[Merge] src/diorama/look.ts                  Wetter-Version (Prototyp-Zimmer)
[Merge+Union] src/diorama/designTokens.ts    Wetter-Tokens + ksw-Tokens + Übergangs-Tokens
Create  src/diorama/environment/nightSky.ts  createStarField/createMoonDisc (aus look.ts extrahiert, parametrisiert)
Modify  src/diorama/environment/precipitation.ts  createPrecipitation(opts) statt Konstanten
Create  src/diorama/ksw/applyCityEnvironment.ts   CityEnvironmentTargets + per-Frame-Apply
Modify  src/diorama/ksw/main.ts               Treiber: ?at/?wx, computeEnvironment, Uniforms
Modify  src/diorama/ksw/staticBatch.ts        glowNight kontinuierlich (lampGlowU)
Modify  src/diorama/ksw/geo/cityMassing.ts    facadeMaterial-Emissive × lampGlowU
Modify  src/diorama/ksw/geo/lamps.ts          Lampenkopf kontinuierlich
Modify  scripts/smoke-environment.mjs         beide Seiten (look.html + /)
Modify  scripts/capture-env.mjs               Seiten-Parameter + Stadt-Matrix
Test    tests/diorama/nightSky.test.ts        Vollkugel-Verteilung (pur)
```

---

### Task 1: Integrations-Merge (Phase 1)

**Files:**
- Merge: `klinik/look-prototype` → `klinik/env-in-city` (Konflikte: `src/diorama/look.ts`, `src/diorama/designTokens.ts`, `package.json`, `package-lock.json`)

**Interfaces:**
- Produces: das komplette `src/diorama/environment/`-Modul, aktualisiertes `look.ts`, `tests/diorama/*` (51 Tests), `scripts/smoke-environment.mjs` (10 Checks), `scripts/capture-env.mjs`, suncalc-Dependency — alles auf dem Stadt-Branch. designTokens enthält BEIDE Welten (Union, siehe Step 2).

- [ ] **Step 1: Merge starten**

```bash
cd /Users/ramonfuglister/Coding/abutown/.worktrees/winterthur-main
git merge klinik/look-prototype
# erwartet: CONFLICT in look.ts, designTokens.ts, package.json, package-lock.json
git status --short
```

- [ ] **Step 2: Konflikte auflösen**

1. **`src/diorama/look.ts`** → komplette Wetter-Version: `git checkout --theirs src/diorama/look.ts`. (main hält byte-identisch den alten Prototyp-Stand `3e24f87` — verifizierbar: `git diff 3e24f87 HEAD -- src/diorama/look.ts` war vor dem Merge leer.)
2. **`package.json`** → Union: dependencies bekommen `"suncalc"`, devDependencies `"@types/suncalc"` (Versionen aus der theirs-Seite); alles andere identisch. `package-lock.json` → `git checkout --theirs package-lock.json` und danach `npm install` (regeneriert konsistent).
3. **`src/diorama/designTokens.ts`** → **ECHTE UNION**, von Hand:
   - Von der **Wetter-Seite** (theirs) übernehmen: `EnvKeyframe`, `envKeyframes`, `envAnchors`, `weatherLook`, `cloudLook`, `precipLook`, `moonDisc`, `nightGlow.boost`, `post` OHNE `godraysMix`/`godraysMixDusk` (dort gelöscht), `sunArcCfg` in der REDUZIERTEN Form **NICHT** übernehmen — siehe nächster Punkt.
   - Von der **ksw-Seite** (ours) behalten — die Stadt referenziert sie noch (Task 4 stellt um, Task 5 löscht): `lightPresets` + `LightPreset`, `skyPhys`, **volles `sunArcCfg`** (inkl. `azRise/azSet/elevMax/elevBase/cycleSeconds` UND `colorLow/colorHigh`), `moonLight` MIT `position` UND `intensity`, `gi.hemiCut`, `cloudVol` (Prototyp-Raymarcher braucht seine Felder — Achtung: die Wetter-Seite hat `cloudVol.coverage` und `cloudVol.drift` GELÖSCHT; ksw nutzt `cloudCfg`, nicht `cloudVol` → für `cloudVol` gilt die Wetter-Version), sowie alle ksw-Tokens (`kswScene`, `kswPost`, `kswCity`, `kswCityStyle`, `kswCamera`, `kswAgents`, `cloudCfg`, …).
   - Kollisionscheck `palette`/`radii`/`clay`/`grade`/`post`/`gi`: beide Seiten können Felder ergänzt haben — Feld-für-Feld vereinigen, bei GLEICHEM Feld mit verschiedenen Werten gilt die Wetter-Seite für Prototyp-Felder (`post.dof` etc. unverändert auf beiden) — tatsächliche Differenzen beim Auflösen dokumentieren (Merge-Commit-Message).
   - Ergebnis-Regel: `npx tsc` findet KEINEN unaufgelösten Bezug — jede von `look.ts`, `environment/*` und `ksw/*` importierte Konstante existiert.
4. `git add` der aufgelösten Dateien, `git merge --continue` (Standard-Merge-Message + Zusatz: Auflösungsstrategie in 3 Zeilen).

- [ ] **Step 3: Gate**

```bash
npm install
npm test                       # erwartet: 51/51 (die gemergten Tests)
npm run typecheck              # 0 errors
npm run build                  # grün
node scripts/smoke-environment.mjs   # 10/10 (Prototyp-Seite look.html)
```
Zusätzlich Stadt-Regression: `npm run dev` (Hintergrund), `/` lädt ohne Page-Errors (playwright-Probe: `__LOOK_READY` wird true), Server killen. Die Stadt läuft noch auf Presets — das ist hier korrekt.

- [ ] **Step 4: Commit ist der Merge-Commit** (aus Step 2). Nichts weiter.

---

### Task 2: Modul-Parametrisierung — nightSky.ts + createPrecipitation(opts)

**Files:**
- Create: `src/diorama/environment/nightSky.ts`
- Modify: `src/diorama/environment/precipitation.ts` (Signatur), `src/diorama/look.ts` (konsumiert beide neu), `src/diorama/designTokens.ts` (Wertesätze Raum/Stadt)
- Test: `tests/diorama/nightSky.test.ts`

**Interfaces:**
- Produces:
  ```ts
  // nightSky.ts
  export type StarFieldOpts = { radius: number; quadSize: number; count: number; seed?: number };
  export function starDirections(count: number, seed: number): Array<[number, number, number]>; // pur, Vollkugel-gleichverteilt
  export function createStarField(opts: StarFieldOpts): { object3d: THREE.Object3D; material: { opacity: number } /* Points-frei: InstancedMesh + NodeMaterial-Opacity-Uniform */ };
  export function createMoonDisc(opts: { radius: number; distance: number }): { mesh: THREE.Mesh; phaseDir: { value: THREE.Vector3 } };
  // precipitation.ts
  export type PrecipOpts = { boxX: number; boxY: number; boxZ: number; count: number };
  export function createPrecipitation(opts?: Partial<PrecipOpts>): PrecipitationSystem; // Defaults = bisherige Konstanten
  ```
- Tokens: `export const nightSkyLook = { room: { starRadius: 17, starQuad: <bisheriger Wert aus look.ts>, starCount: <bisher>, moonRadius: 0.46, moonDistance: 17 }, city: { starRadius: <kswScene.domeRadius*0.85>, starQuad: <skaliert, kuratierbar>, starCount: 400, moonRadius: 3.4, moonDistance: <kswScene.domeRadius*0.82> } } as const;` — die room-Werte sind EXAKT die heutigen look.ts-Konstanten (beim Extrahieren ablesen, nicht raten); `precipLook` bekommt `room: { boxX: 24, boxY: 14, boxZ: 20, count: 3000 }` (heutige Konstanten) und `city: { boxX: 90, boxY: 40, boxZ: 90, count: 4500 }` (Startwerte, Task-6-Capture-Review kuratiert nach).

- [ ] **Step 1: Failing Test**

`tests/diorama/nightSky.test.ts`:
```ts
import { describe, expect, it } from 'vitest';
import { starDirections } from '../../src/diorama/environment/nightSky';

describe('starDirections', () => {
  it('is deterministic for a seed and unit-length', () => {
    const a = starDirections(200, 42);
    const b = starDirections(200, 42);
    expect(a).toEqual(b);
    for (const [x, y, z] of a) expect(Math.hypot(x, y, z)).toBeCloseTo(1, 6);
  });
  it('covers the full sphere (mean ~0 per axis, both hemispheres populated)', () => {
    const dirs = starDirections(2000, 7);
    const mean = dirs.reduce((m, d) => [m[0] + d[0], m[1] + d[1], m[2] + d[2]], [0, 0, 0]).map((v) => v / dirs.length);
    for (const v of mean) expect(Math.abs(v)).toBeLessThan(0.05);
    expect(dirs.filter((d) => d[1] < 0).length).toBeGreaterThan(600); // untere Halbkugel real besiedelt
  });
});
```

- [ ] **Step 2: Run — FAIL** (`nightSky` fehlt): `npx vitest run tests/diorama/nightSky.test.ts`

- [ ] **Step 3: Implementieren**

`nightSky.ts`: den Sternenfeld-Block (InstancedMesh-Billboards, Seed-PRNG, Vollkugel via `sinEl = rand*2-1`) und den Mond-Block (SphereGeometry + `MeshBasicNodeMaterial` mit `normalLocal`-Terminator + `moonDiscTokens`) AUS `look.ts` HERAUSLÖSEN — Code verschieben, nicht neu erfinden; Konstanten (`STAR_R=17`, Quad-Grösse, Count, Mond 0.46/17) werden zu `opts` mit den look.ts-Werten als `nightSkyLook.room`. `starDirections` ist die pure Richtungs-Funktion (Seed-PRNG identisch zum bisherigen Inline-Code). `precipitation.ts`: `COUNT`/`BOX`-Konstanten → `opts` mit Defaults `precipLook.room`. `look.ts`: konsumiert `createStarField(nightSkyLook.room)`, `createMoonDisc(nightSkyLook.room)`, `createPrecipitation()` (Defaults) — Verhalten byte-gleich.

- [ ] **Step 4: Gate**

```bash
npx vitest run tests/diorama/nightSky.test.ts   # PASS
npm test && npm run typecheck && npm run build  # 53/53, clean, grün
node scripts/smoke-environment.mjs              # 10/10 — Prototyp UNVERÄNDERT
```

- [ ] **Step 5: Commit**

```bash
git add src/diorama/environment/nightSky.ts src/diorama/environment/precipitation.ts src/diorama/look.ts src/diorama/designTokens.ts tests/diorama/nightSky.test.ts
git commit -m "refactor(environment): parametrize night sky + precipitation for reuse (room values unchanged)"
```

---

### Task 3: Nacht-Stadt kontinuierlich — lampGlow: boolean → Laufzeit-Uniform

**Files:**
- Modify: `src/diorama/ksw/staticBatch.ts:41-53,130-134`, `src/diorama/ksw/geo/cityMassing.ts:237,332-340`, `src/diorama/ksw/geo/lamps.ts:39,63`, `src/diorama/ksw/building.ts:192`, `src/diorama/ksw/main.ts:450,524,542,700-716`

**Interfaces:**
- Produces: `export const lampGlowU = uniform(0)` — EIN gemeinsames Modul-Level-Uniform (neue kleine Datei `src/diorama/ksw/glowUniform.ts`, um Import-Zyklen zu vermeiden: `import { uniform } from 'three/tsl'; export const lampGlowU = uniform(0);`). Alle Builder verlieren den `lampGlow: boolean`-Parameter; Glüh-Geometrie wird IMMER gebaut, Intensität hängt an `lampGlowU` (0 = Tag-Look, 1 = volles Nachtglühen).
- Consumes: nichts aus Task 2.

- [ ] **Step 1: Builder umstellen**

1. `staticBatch.ts`: `classifyMesh`/`batchHospital` verlieren `opts.lampGlow` — `glowNight`-Klassifikation gilt immer (Z.45: `if (mesh.userData.lampBulb)`, Z.51-53 Fenster-Hash unverändert). Bucket-Material Z.134: `MeshBasicMaterial({opacity: 0.9})` → `MeshBasicNodeMaterial` mit `opacityNode = float(0.9).mul(lampGlowU)` (transparent, depthWrite false unverändert; Farbe weiterhin `nightGlow.bulb`).
2. `cityMassing.ts` `facadeMaterial` (Z.237, 332-340): den `if (opts.lampGlow)`-Zweig entfernen — Emissive-Node IMMER bauen, Z.340: `m.emissiveNode = vec3(warm.r, warm.g, warm.b).mul(glow.mul(float(0.9)).mul(lampGlowU))`. `opts` verliert `lampGlow`.
3. `lamps.ts` (Z.39,63): Lampenkopf immer `MeshBasicNodeMaterial` mit `colorNode = mix(vec3(<palette.white als rgb01>), vec3(<0xffe3b0 als rgb01>), lampGlowU)` — die Warm-Farbe `0xffe3b0` wandert dabei als `nightGlow.lampHead` in designTokens (Token-Regel!). Der Tag-Look (weisser Kopf) weicht minimal vom bisherigen clayMat ab — bewusst, wird im Task-6-Capture-Review abgenommen.
4. `building.ts` `buildHospital` (Z.192) + alle Aufrufer in `main.ts` (Z.450, 524, 542): `{ lampGlow: preset.lampOn }`-Argumente entfernen (Signaturen nachziehen).
5. `main.ts` Z.700-716: `if (preset.lampOn)`-Gate entfernen — PointLight-Pool + `emLamp` IMMER erzeugen, Intensitäten `14 * preset.lampBoost` / `20 * preset.lampBoost` → Basiswerte `14`/`20` als `nightGlow.cityPool`/`nightGlow.emergency` in designTokens, Laufzeit `light.intensity = nightGlow.cityPool * nightGlow.boost * glow01` — in DIESEM Task initial einmalig `const glow01 = preset.lampOn ? 1 : 0;` gesetzt (Presets treiben noch), Task 4 macht es pro Frame kontinuierlich. Die Lichter in einem Array `lampLights: THREE.PointLight[]` sammeln und aus `boot` heraus reichbar machen (Task 4 braucht das Array in den Targets).
6. `lampGlowU.value = preset.lampOn ? 1 : 0;` einmalig in boot setzen.

- [ ] **Step 2: Gate (Verhalten preset-identisch)**

```bash
npm test && npm run typecheck && npm run build
```
Visuelle Probe (dev-Server, playwright): `/?preset=morning` und `/?preset=night` laden, Screenshots nach `artifacts/glow-check-{morning,night}.png`, selbst ansehen: morning ohne Glühen (Fenster/Lampen aus), night mit warmem Glühen — wie vorher. Server killen.

- [ ] **Step 3: Commit**

```bash
git add src/diorama/ksw/ src/diorama/designTokens.ts
git commit -m "refactor(ksw): night glow from bake-time boolean to runtime lampGlowU uniform (preset-driven for now)"
```

---

### Task 4: Stadt-Treiber + applyCityEnvironment.ts (das Herzstück)

**Files:**
- Create: `src/diorama/ksw/applyCityEnvironment.ts`
- Modify: `src/diorama/ksw/main.ts` (Boot Z.102-116, Sonne/Sky Z.159-190, Wolken Z.191-249, Discs/Sterne Z.250-309, Licht-Rig Z.365-380, hemi Z.437, GI Z.750, Post Z.827-844, animate-Schluss)

**Interfaces:**
- Consumes: `computeEnvironment`, `EnvironmentState`, `moonPhaseLightDir` (environment.ts); `startWeatherLoop`, `sampleWeather`, `CLEAR_SKY`, `WeatherSeries`, `WeatherState` (weather.ts); `createStarField`, `createMoonDisc`, `nightSkyLook.city` (Task 2); `createPrecipitation(precipLook.city)` (Task 2); `lampGlowU`, `lampLights` (Task 3). Die `WX_OVERRIDES`-Tabelle und das `now()`-/`?at=`-Muster stehen fertig in `src/diorama/look.ts` (nach Task 1 im Repo) — 1:1 übernehmen.
- Produces:
  ```ts
  export type CityEnvironmentTargets = {
    renderer: THREE.WebGPURenderer;
    fog: THREE.Fog;              // Achtung: near/far werden von animate() zoom-skaliert — apply schreibt BASIS-Werte in fogBase (s.u.)
    fogBase: { near: number; far: number; };   // von applyCityEnvironment beschrieben, von animate() × Zoom-Faktor konsumiert
    sun: THREE.DirectionalLight;
    currentSunDir: THREE.Vector3;  // IN-PLACE aktualisiert — das Shadow-Rig (updateShadowFrustum) liest ihn
    hemi: THREE.HemisphereLight;
    skyMesh: { turbidity/rayleigh/mieCoefficient/mieDirectionalG/sunPosition wie im Prototyp };
    cloud: { sunDirUniform: {value: THREE.Vector3}; lit: {value: THREE.Color}; shadow: {value: THREE.Color}; coverageU: {value: number}; driftUV: {value: THREE.Vector2} };
    post: { saturationU: {value: number}; contrastU: {value: number}; godraysMixU: {value: number} };
    mist: { mat: THREE.MeshBasicMaterial; cityMat: THREE.MeshBasicMaterial; baseOpacity: { value: number } }; // apply schreibt baseOpacity + Farbe; animate() verrechnet Fade/cloudMix wie bisher, nur mit baseOpacity.value statt preset.mistOpacity
    sunDisc: THREE.Mesh;
    moon: { mesh: THREE.Mesh; phaseDir: {value: THREE.Vector3} };
    stars: { object3d: THREE.Object3D; material: { opacity: number } };
    lampLights: THREE.PointLight[];
    lampBaseIntensities: number[];   // parallel zu lampLights (14er-Pool / 20er-emLamp aus Task 3)
    precipitation: PrecipitationSystem;
    giBase: { value: number };       // scene.environmentIntensity-Basis; animate() schreibt scene.environmentIntensity = giBase.value
    exposure: (v: number) => void;   // renderer.toneMappingExposure
    scratch: { v3: THREE.Vector3; c1: THREE.Color; c2: THREE.Color };
  };
  export function applyCityEnvironment(t: CityEnvironmentTargets, env: EnvironmentState, dtSeconds: number): void;
  ```

- [ ] **Step 1: `applyCityEnvironment.ts` schreiben**

Gleiches Muster wie `environment/applyEnvironment.ts` (lesen und als Vorlage nehmen!), mit den Stadt-Abweichungen:

```ts
// Applies EnvironmentState to the city scene. Uniform/mutable writes only.
// City-specific: fog goes through fogBase (animate() zoom-scales it), the sun
// vector is shared with the shadow-follow rig via t.currentSunDir (in-place),
// clouds are the two fbm domes (coverage/drift uniforms), night glow drives
// lampGlowU + the pooled point lights.
import * as THREE from 'three/webgpu';
import { cloudLook, kswScene, moonLight, nightGlow, weatherLook } from '../designTokens';
import { moonPhaseLightDir, type EnvironmentState } from '../environment/environment';
import { lampGlowU } from './glowUniform';
// Kernstücke:
export function applyCityEnvironment(t: CityEnvironmentTargets, env: EnvironmentState, dt: number): void {
  const isDay = env.sunIntensity > 0.02;
  const sunDir = t.scratch.v3.set(env.sunDir[0], env.sunDir[1], env.sunDir[2]);
  // sky
  t.skyMesh.turbidity.value = env.turbidity; /* rayleigh, mie, mieG analog */
  t.skyMesh.sunPosition.value.copy(sunDir);
  // fog: BASIS schreiben, animate() skaliert mit Zoom wie bisher
  t.fog.color.set(env.fogColor);
  t.fogBase.near = env.fogNear * kswScene.fogScale;
  t.fogBase.far = env.fogFar * kswScene.fogScale;
  t.exposure(env.exposure);
  // key light: Sonne bei Tag, Mond bei Nacht — Richtung IN-PLACE in currentSunDir,
  // Position setzt weiterhin das Shadow-Rig (updateShadowFrustum) aus currentSunDir.
  if (isDay) {
    t.currentSunDir.copy(sunDir);
    t.sun.color.set(env.sunColor);
    t.sun.intensity = Math.max(env.sunIntensity, 0.05);
    t.cloud.sunDirUniform.value.copy(sunDir);
    t.cloud.lit.value.set(env.sunColor).lerp(t.scratch.c1.set(cloudLook.litWhite), cloudLook.litWhiteMix);
    t.cloud.shadow.value.set(cloudLook.shadowBase).lerp(t.scratch.c2.set(env.sunColor), 0.15);
  } else {
    const moonDir = t.scratch.v3.set(env.moonDir[0], Math.max(env.moonDir[1], 0.15), env.moonDir[2]).normalize();
    t.currentSunDir.copy(moonDir);
    t.sun.color.set(moonLight.color);
    t.sun.intensity = Math.max(env.moonIntensity, 0.12);
    t.cloud.sunDirUniform.value.copy(moonDir);
    t.cloud.lit.value.set(cloudLook.nightLit);
    t.cloud.shadow.value.set(cloudLook.nightShadow);
  }
  // hemi (hemiCut ist in envKeyframes bereits gefaltet), clouds, post, mist, GI-Basis
  t.hemi.color.set(env.hemiSky); t.hemi.groundColor.set(env.hemiGround); t.hemi.intensity = env.hemiIntensity;
  t.cloud.coverageU.value = env.cloudCoverage;
  t.cloud.driftUV.value.x += env.cloudDriftDir[0] * env.cloudDriftSpeed * dt;
  t.cloud.driftUV.value.y += env.cloudDriftDir[1] * env.cloudDriftSpeed * dt;
  t.post.saturationU.value = env.saturation; t.post.contrastU.value = env.contrast;
  t.post.godraysMixU.value = env.godraysMix;
  t.mist.mat.color.set(env.mistColor); t.mist.cityMat.color.set(env.mistColor);
  t.mist.baseOpacity.value = env.mistOpacity;
  t.giBase.value = env.giScale; // animate(): scene.environmentIntensity = gi.environmentIntensity * t.giBase.value * kswPost.envScaleScalar
  // discs / night sky
  t.sunDisc.position.set(env.sunDir[0], env.sunDir[1], env.sunDir[2]).multiplyScalar(kswScene.domeRadius * 0.82);
  t.sunDisc.visible = env.sunDir[1] > 0.015;
  t.moon.mesh.position.set(env.moonDir[0], env.moonDir[1], env.moonDir[2]).multiplyScalar(/* nightSkyLook.city.moonDistance */);
  t.moon.mesh.visible = env.moonDir[1] > 0.02 && env.starVisibility > 0.02;
  t.moon.phaseDir.value.set(...moonPhaseLightDir(env.moonPhase));
  t.stars.material.opacity = 0.85 * env.starVisibility;
  t.stars.object3d.visible = env.starVisibility > 0.01;
  t.stars.object3d.rotation.set(0, 0, 0);
  t.stars.object3d.rotateOnWorldAxis(POLE_AXIS, env.siderealAngleRad); // POLE_AXIS wie in applyEnvironment.ts (Modul-Konstante dort exportieren statt duplizieren!)
  // night glow
  lampGlowU.value = env.lampOn01;
  for (let i = 0; i < t.lampLights.length; i++) t.lampLights[i].intensity = t.lampBaseIntensities[i] * nightGlow.boost * env.lampOn01;
  // precipitation
  t.precipitation.update(env.precipType, env.precipIntensity, env.windSpeedMs, env.windDirRad, dt);
}
```
Dafür in `environment/applyEnvironment.ts` die Konstante `POLE_AXIS` exportieren (`export const POLE_AXIS = …`) und hier importieren — nicht duplizieren. Kein `shaft01` in der Stadt (Ost-Fenster-Shafts sind ein Zimmer-Feature).

- [ ] **Step 2: main.ts umstellen (nummerierte Änderungen)**

1. **Params (Z.102-116):** `rawPreset`/`presetName`/`cycleMode` raus; `?at=`/`?wx=`/`now()`/`WX_OVERRIDES`/`currentWeather()`-Block 1:1 aus `look.ts` übernehmen (inkl. `startWeatherLoop`-Gate auf `!wxParam`). `?cam`/`?agents` bleiben. `const preset = lightPresets[presetName]` entfällt — Erstinitialisierung: `let lastEnv = computeEnvironment(now(), currentWeather());`.
2. **Sonnen-Helfer löschen (Z.159-174):** `sunDirFor`, `sunLightFor`, `phys`, `initialSunDir` weg. `applySunState` (Z.264-287) komplett weg — ersetzt durch `applyCityEnvironment`.
3. **SkyMesh (Z.176-190):** Initialwerte aus `lastEnv`; Z.188 `skyUnfogged`-Zeile → `(skyMesh.material as …).fog = false;` (immer).
4. **Wolken (Z.191-249):** `driftU = uniform(0)` → `driftUV = uniform(new THREE.Vector2())`; in BEIDEN Dome-Shadern `add(driftU)` → x: `.add(driftUV.x)`, z: `.add(driftUV.y)` (die y-Komponente des Noise bleibt drift-frei); `const coverage = float(cloudCfg.coverage[presetName])` → gemeinsames `coverageU = uniform(0.44)` in beiden. `driftU.value = t * cloudCfg.drift` in animate (~Z.1037) entfällt (Integration in apply).
5. **Discs/Mond/Sterne (Z.250-309):** `moonDisc`-Inline → `createMoonDisc(nightSkyLook.city)`; `if (preset.showStars)`-Points-Block ersatzlos raus → `createStarField(nightSkyLook.city)` immer, `scene.add(...)`. `sunDisc` bleibt (Position/Sichtbarkeit macht apply).
6. **Licht-Rig (Z.365-380):** der `if (presetName !== 'night')`-Boot-Block entfällt; `sun` neutral erzeugen, `currentSunDir` bleibt als `const currentSunDir = new THREE.Vector3(0, 1, 0)` (apply schreibt in-place; ALLE bisherigen `currentSunDir = …`-Neuzuweisungen entfernen — `updateShadowFrustum` liest wie bisher).
7. **Hemi (Z.437):** `new THREE.HemisphereLight(0xffffff, 0xffffff, 1)` neutral; apply setzt.
8. **GI (Z.750):** `kswPost.envScale[presetName]` → neuer Skalar-Token `kswPost.envScaleScalar` (Wert = bisheriger `envScale.morning`); animate: `scene.environmentIntensity = gi.environmentIntensity * giBase.value * kswPost.envScaleScalar;` pro Frame (giBase aus Targets).
9. **Post (Z.827-844):** `kswPost.godraysMix[presetName]` → `godraysMixU = uniform(0)` mal Stadt-Skalar `kswPost.godraysScale` (neuer Token, Wert = bisheriger `godraysMix.morning / envKeyframes.goldenMorning.godraysMix` — einmal ausrechnen, kommentieren): `lit = withAo.add(chain(raysNode).mul(godraysMixU).mul(float(kswPost.godraysScale)))`; `preset.saturation`/`preset.contrast` → `saturationU`/`contrastU`-Uniforms.
10. **Mist (Z.646-698, animate Z.985/992):** `preset.mistColor/mistOpacity` → Targets-`baseOpacity`; animate-Zeilen: `mistMat.opacity = baseOpacity.value * (1 - fade * 0.75)` bzw. `cityMistMat.opacity = baseOpacity.value * 0.8 * cloudMix`.
11. **Fog (Z.155-157 + Zoom-Stelle in animate):** `fogBase`-Objekt statt `const fogBaseNear/Far`; animate skaliert weiter mit dem Zoom-Faktor, liest aber `fogBase.near/far`.
12. **Niederschlag:** `const precip = createPrecipitation(precipLook.city); scene.add(precip.object3d);`.
13. **animate:** `cycleMode`-Zweig (Z.~1039-1047) ersatzlos; stattdessen pro Frame `lastEnv = computeEnvironment(now(), currentWeather()); applyCityEnvironment(targets, lastEnv, dt);` (dt wie in look.ts geclampt auf 0.1) + `window.__ENV_STATE = lastEnv;`. `GiProbeScheduler(cycleMode ? 'cycle' : 'static')` (Z.748) → `'static'`; DAFÜR den GI-Probe-Refresh zeitgesteuert lassen wie er ist (der Scheduler amortisiert ohnehin, und `computeEnvironment` ändert sich real nur langsam).
14. **Global-Typ:** `__ENV_STATE` ist per look.ts-Merge schon in `declare global` — prüfen, ggf. zentral lassen (nicht doppelt deklarieren).

- [ ] **Step 3: Gate**

```bash
npm test && npm run typecheck && npm run build
```
Browser-Probe (dev-Server, playwright, gleiche Mechanik wie Task-5-Probe des Vorgänger-Plans): `/?at=2026-07-03T11:00:00Z&wx=clear` (Mittag: `__ENV_STATE.sunElevDeg > 55`, Schatten sichtbar), `/?at=2026-07-03T19:03:00Z&wx=clear` (golden), `/?at=2026-07-03T23:30:00Z&wx=clear` (Nacht: Sterne + Lampen/Fenster an, `lampOn01 == 1`), `/?at=2026-07-03T11:00:00Z&wx=rain` (`precipType 'rain'`), `/?wx=fog`. Zero page errors. Screenshots ansehen. Server killen.

- [ ] **Step 4: Commit**

```bash
git add src/diorama/ksw/ src/diorama/environment/applyEnvironment.ts src/diorama/designTokens.ts
git commit -m "feat(ksw): city driven by realtime computeEnvironment — real sun over the real map, live weather, ?at/?wx"
```

---

### Task 5: Cleanup — tote Preset-Tokens löschen

**Files:**
- Modify: `src/diorama/designTokens.ts`, ggf. `tests/` (falls ein Test tote Tokens referenziert)

- [ ] **Step 1: Nutzungs-Sweep + Löschen**

`grep -rn` über `src/ tests/ scripts/` für jeden Kandidaten; löschen, was 0 Treffer ausserhalb designTokens hat: `lightPresets` + `LightPreset`, `skyPhys`, `sunArcCfg.azRise/azSet/elevMax/elevBase/cycleSeconds` (die Bogen-Geometrie; `colorLow/colorHigh` BLEIBEN — environment.ts nutzt sie), `moonLight.position` (Z.: nur noch color/intensity genutzt — verifizieren), `gi.hemiCut`, `cloudCfg.coverage` (Record) + `cloudCfg.drift`, `kswPost.skyUnfogged`, `kswPost.godraysMix` (Record) + `kswPost.envScale` (Record), `preset.lampOn/lampBoost`-Reste. Jeder Kandidat, der doch noch Treffer hat: NICHT löschen, im Report begründen.

- [ ] **Step 2: Gate + Commit**

```bash
npm test && npm run typecheck && npm run build && node scripts/smoke-environment.mjs
git add -A src/ tests/ && git commit -m "chore(tokens): remove dead preset-keyed tokens after city env wiring"
```

---

### Task 6: Harnesse beide Seiten + Stadt-Look-Review + finaler Gate

**Files:**
- Modify: `scripts/smoke-environment.mjs`, `scripts/capture-env.mjs`

- [ ] **Step 1: Smoke auf beide Seiten erweitern**

`smoke-environment.mjs`: `probe(query, assert)` bekommt einen Seiten-Parameter (`page: 'look.html' | ''`). Bestehende 10 look.html-Checks unverändert; NEU für `/`: (a) Open-Meteo-Wiring-Probe (ohne `?wx`, Route-Intercept mit Fixture, assert Request ging raus + `cloudCoverage ≈ 0.29` applied — gleiche Arithmetik wie look-Probe 1), (b) Mittag `sunElevDeg > 55`, (c) Nacht 23:30Z `starVisibility > 0.7` UND `lampOn01 === 1`, (d) `wx=rain` → `precipType 'rain'`, (e) Winter 16:30Z → `sunElevDeg < 0`. Ausgabe `smoke-environment: N/N passed`, exit 1 bei Fail.

- [ ] **Step 2: Capture-Matrix Stadt**

`capture-env.mjs`: `--page=/ --out=env-city`-Parameter (Default bleibt look.html/env). Stadt-Matrix = dieselben 9 Zustände; zusätzlich je einmal `cam=city` (Establishing-Shot) für dawn/noon/night. Rendern nach `artifacts/env-city/`.

- [ ] **Step 3: Look-Review Stadt (Kern dieses Tasks)**

Alle Stadt-Captures SELBST ANSEHEN, Kalibrierung: Dawn = tiefe warme Schatten über der echten Karte; Noon = neutral, klare Gebäudeschatten; Dusk = DREDGE über der Stadt, Lampen gehen an; Nacht = Sterne + Mond + Lampen-/Fensterglühen (Stadt lebt!); Overcast diffus (keine harten Schatten); Regen-Fäden/Schnee-Flocken sichtbar im Stadt-Massstab (precipLook.city nachkuratieren wenn zu dünn/zu klein); Hochnebel = Stadt versinkt grauweiss; Winternacht-17:30 eindeutig Nacht mit beleuchteten Fenstern. Nachkuration NUR in designTokens (`nightSkyLook.city`, `precipLook.city`, `kswPost.godraysScale`, `envScaleScalar`), Matrix erneut rendern bis es sitzt. Prototyp-Matrix (`artifacts/env/`) einmal gegenrendern: KEINE Regression.

- [ ] **Step 4: Finaler Gate + Commit**

```bash
npm test && npm run typecheck && npm run build && node scripts/smoke-environment.mjs
git add scripts/ src/diorama/designTokens.ts
git commit -m "test(environment): dual-page smoke + city capture matrix; city look curated"
```

- [ ] **Step 5: Branch abschliessen** — superpowers:finishing-a-development-branch (PR gegen `main` auf origin, CI grün abwarten — „wait for green, not just not-red").

---

## Self-Review (durchgeführt)

- **Spec-Abdeckung:** Phase-1-Merge inkl. Union-Falle ✓ (T1), Modul-Parametrisierung Sterne/Mond/Partikel ✓ (T2), Nacht-Stadt kontinuierlich (lamps/windows/Pool) ✓ (T3), Treiber + applyCityEnvironment + alle Mappings (Fog×fogScale, Zwei-Ebenen-Wolken, skyUnfogged-Ende, GI/Post kontinuierlich, `__ENV_STATE` auf `/`) ✓ (T4), No-Cruft-Cleanup ✓ (T5), Dual-Smoke + Stadt-Captures + Look-Review + PR ✓ (T6).
- **Placeholder:** keine TBD/TODO; wo Werte beim Extrahieren abzulesen sind („exakt die heutigen look.ts-Konstanten"), ist die Quelle exakt benannt — bewusst Ablesen statt Raten.
- **Typ-Konsistenz:** `CityEnvironmentTargets`-Felder = T4-Apply-Zugriffe; `lampGlowU`/`lampLights` T3→T4 konsistent; `createStarField/createMoonDisc/starDirections/PrecipOpts` T2→T4 konsistent; `POLE_AXIS` wird exportiert statt dupliziert.
