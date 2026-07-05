# Hero-Wolken-Dome entfernen (nur noch City-Dome)

**Datum:** 2026-07-05
**Status:** Design genehmigt (User), bereit für Plan

## Ziel

Die **Hero-Wolken-Dome** aus der City-Diorama-App (`/`) ganz entfernen. Nur die
**City-Wolken-Dome** (Radius `kswCity.domeRadius` = 1800) bleibt als einzige
Wolkenschicht, immer aktiv.

## Warum

Die Hero-Dome ist eine **origin-zentrierte `BackSide`-Kugel** mit Radius
`kswScene.domeRadius` (400), zentriert **über dem KSW**. Beim Rauszoomen (Radius
> 400) verlässt die Kamera die Kugel, während sie noch teil-opak ist → man sieht
ihre ferne Innen-Schale als dunkle Halbkugel über dem KSW. Das ist die Quelle
einer ganzen Bug-Klasse (dieselbe Hero→Gemeinde-Migrationslücke wie #124/#125).

Ihr einziger Nutzen (feineres Wolken-Rauschen im Nah-Framing) ist bei City-Scale
marginal: im Nah-Framing schaut man aufs KSW-Gebäude, vom Himmel ist kaum etwas
im Bild, und die City-Dome (1800) rendert von innen sauber. Entfernen = Root
Cause weg + weniger Code (passt zu „saubere Simulation, kein Legacy-Cruft").

Die City-Dome bleibt unproblematisch: `radiusMax` = 1500 < 1800, die Kamera
verlässt sie im normalen Bereich nie.

## Umfang (alles in `src/diorama/ksw/main.ts`, plus 1 Kommentar)

**Entfernen (Hero-Dome):**
- `heroCloudOpacity`-Uniform (Zeile ~229).
- `cloudMatDome`-Material + sein `{ … }`-Node-Block (Zeilen ~231–249).
- `cloudDome`-Mesh + `scene.add(cloudDome)` (Zeilen ~250–251).
- Im `animate()`-Loop: `heroCloudOpacity.value = 1 - cloudMix;` (Zeile ~1220).

**Ändern (City-Dome wird immer-an):**
- `cityCloudOpacity = uniform(0)` → `uniform(1)` (Zeile ~230).
- Im `animate()`-Loop: `cityCloudOpacity.value = cloudMix;` **entfernen** (die
  City-Wolken sind jetzt konstant an, kein Zoom-Crossfade mehr).
- Setup-Kommentar „Two-layer clouds … hero dome fades out …" (Zeilen ~226–228)
  auf „single city cloud layer" aktualisieren.

**Bleibt unverändert:**
- `cityCloudDome` (1800-Mesh), `cloudMatCity`, geteilte Uniforms
  (`sunDirUniform`, `cloudLit`, `cloudShadow`, `driftUV`, `coverageU`).
- `cloudMix` / `kswCityStyle.cloudSwap` — der **City-Mist** nutzt sie weiter
  (`cityMistMat.opacity = mistBaseOpacity.value * 0.8 * cloudMix;`, Zeile ~1222):
  Mist blendet beim Rauszoomen ein, unabhängig von den Wolken-Domes.
- `applyCityEnvironment` — treibt nur die geteilten Cloud-Uniforms, nicht die
  entfernten Opacity-Uniforms. Nicht betroffen.

## Verifikation

- **Unit:** bestehende Suite grün (keine neue reine Funktion; die Entfernung ist
  Wiring). Der frühere `cloudSwap.end ≤ domeRadius`-Invariantentest (aus dem
  verworfenen #126) wird NICHT übernommen — er referenziert die nun nicht mehr
  existente Hero-Dome.
- **Browser-Smoke** (`scripts/smoke-ksw.mjs`): Boot + Env-Apply grün (Pflicht,
  Frontend-Wiring).
- **Live-Look-Check (entscheidend):** bei mehreren Framings prüfen, dass der
  Himmel überall sauber ist —
  1. Nah-Framing über dem KSW (Radius ~200–300): City-Wolken sehen ok aus
     (nicht zu grob/leer), **keine** dunkle Kuppel.
  2. Alter Gefahren-Radius ~450–550: **keine** dunkle Halbkugel mehr.
  3. Übersicht (Radius ~520/820): Himmel unverändert gut.
  Bei Tag (`?at=12:00`), da nachts eine dunkle Kuppel gegen schwarzen Himmel eh
  unsichtbar wäre.

## Bewusst weggelassen (YAGNI)

- Keine kamera-folgende Skybox-Dome (wäre die vollständig immune Architektur,
  aber Overkill; die 1800-Dome deckt den Zoom-Bereich ab).
- Kein Retuning des City-Wolken-Rauschens — nur falls der Nah-Look es zeigt.
- `cloudSwap` NICHT löschen (Mist braucht es); kein Rename in diesem PR.
