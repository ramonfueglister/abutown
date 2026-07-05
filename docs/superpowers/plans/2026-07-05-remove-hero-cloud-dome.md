# Hero-Wolken-Dome entfernen — Implementation Plan

> **For agentic workers:** execute task-by-task; this is a focused single-file wiring change verified by browser smoke + live look-check.

**Goal:** Die Hero-Wolken-Dome aus der City-Diorama-App entfernen; nur die City-Dome (1800) bleibt, immer aktiv. Behebt die „dunkle Halbkugel über dem KSW" an der Wurzel.

**Architecture:** Reines Wiring in `src/diorama/ksw/main.ts`. Hero-Dome-Mesh/Material/Uniform raus, City-Cloud-Opacity konstant 1, Crossfade-Zeile raus. `cloudMix`/`cloudSwap` bleiben (City-Mist nutzt sie).

## Global Constraints

- Nur `src/diorama/ksw/main.ts` (Code) — kein anderer Konsument der Hero-Dome existiert.
- `cloudMix`/`kswCityStyle.cloudSwap` NICHT entfernen (City-Mist-Fade hängt dran).
- Browser-Smoke ist Pflicht (Frontend-Wiring). Zusätzlich Live-Look-Check bei Tag (`?at=12:00`).
- Kein Retuning ohne dass der Nah-Look es erzwingt (YAGNI).

---

### Task 1: Hero-Wolken-Dome entfernen, City-Dome immer-an

**Files:** Modify `src/diorama/ksw/main.ts`

- [ ] **Step 1: Setup-Block umbauen** — im Cloud-Setup (~Zeile 226–251):
  - Kommentar „Two-layer clouds (spec §4): the hero dome fades out … Both opacities are driven by cloudMix …" → „Single city cloud layer (the hero dome was removed): the 1800 dome is the only cloud layer, always on."
  - `const heroCloudOpacity = uniform(1);` **entfernen**.
  - `const cityCloudOpacity = uniform(0);` → `const cityCloudOpacity = uniform(1); // always on (single cloud layer)`.
  - Den ganzen `cloudMatDome`-Block **entfernen**: Material-Erzeugung (`cloudMatDome = new THREE.MeshBasicNodeMaterial(); …fog=false;`), den `{ … }`-Node-Block (dir/p/n/dens/horizonFade/opacityNode/colorNode), und `const cloudDome = new THREE.Mesh(new THREE.SphereGeometry(kswScene.domeRadius, …), cloudMatDome); scene.add(cloudDome);`.
  - `cloudMatCity` + `cityCloudDome` **bleiben** unverändert.

- [ ] **Step 2: animate-Crossfade bereinigen** — im `animate()` (~Zeile 1218–1222):
  - `heroCloudOpacity.value = 1 - cloudMix;` **entfernen**.
  - `cityCloudOpacity.value = cloudMix;` **entfernen** (City-Wolken konstant 1).
  - `const swap = kswCityStyle.cloudSwap; const cloudMix = …;` und `cityMistMat.opacity = mistBaseOpacity.value * 0.8 * cloudMix;` **behalten** (Mist).

- [ ] **Step 3: unused-Symbole prüfen** — sicherstellen, dass `kswScene` weiter genutzt wird (Fog etc.) und keine jetzt-ungenutzten Imports/Variablen übrig bleiben. `grep -nE "heroCloudOpacity|cloudMatDome|cloudDome\b" src/diorama/ksw/main.ts` muss leer sein.

- [ ] **Step 4: Typecheck** — `npx tsc --noEmit`; 0 NEUE Fehler (Baseline separat prüfen).

- [ ] **Step 5: Browser-Smoke** — `node scripts/smoke-ksw.mjs` → `SMOKE OK` (Boot + Env-Apply ohne Regression).

- [ ] **Step 6: Live-Look-Check bei Tag** — Preview starten, `?at=12:00`, an 3 Framings screenshoten (Nah ~250 über KSW, Gefahren-Radius ~500, Übersicht ~520/820): kein dunkler Dome, City-Wolken sehen ok aus. Bei sichtbarem Nah-Look-Problem: STOP und Retuning erwägen (nicht blind mergen).

- [ ] **Step 7: Commit**

```
fix(sky): remove the hero cloud dome — single city cloud layer
```

---

## Self-Review

- Spec-Coverage: Hero-Dome-Mesh/Material/Uniform raus (Step 1), City immer-an (Step 1), Crossfade raus (Step 2), Mist bleibt (Step 2). ✅
- Kein Placeholder; jeder Step nennt konkrete Symbole/Zeilen. ✅
- Risiko: Nah-Look mit City-Wolken → explizit im Live-Check (Step 6) abgesichert.
