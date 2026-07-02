# KSW Kamera-Zoom-Tuning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dächer blenden beim Reinzoomen deutlich früher aus (navigierbar), und man kann deutlich weiter rauszoomen — ohne White-out durch Fog/Sky-Clipping.

**Architecture:** Reine Token-/Szenen-Anpassung, kein neues Modul. Die Fade-Zone (`roofFadeNear/Far`) wandert zu grösseren Radien; `radiusMax` steigt auf 320. Damit die Weitsicht funktioniert: Sky-Sphere und Wolken-Dome wachsen über den neuen Max-Radius hinaus, `camera.far` steigt, und der Fog skaliert **zoom-adaptiv** pro Frame mit (bei Overview identischer Look, bei Weitzoom kein Zunebeln).

**Tech Stack:** three.js r185 WebGPU, bestehende Tokens (`kswCamera`, `kswScene`), Vitest, Playwright-Smoke.

## Global Constraints

- Kein Hexwert/Zahlwert-Streuung: alle neuen Werte in `src/diorama/designTokens.ts`.
- `look.ts`/`capture-look.mjs`/`post`-Tokens des Look-Agenten unangetastet.
- Bestehende Golden-Framings dürfen sich nicht verschieben: Overview (Radius ~111) behält Dächer AN, `er`/`ops` (Radius 13/14) Dächer AUS.
- Gates: `npm run typecheck && npx vitest run` + `node scripts/smoke-ksw.mjs` (13 Checks) + Captures.

---

### Task 1: Token-Werte — frühere Fade-Zone + weiterer Zoom

**Files:**
- Modify: `src/diorama/designTokens.ts` (kswCamera: `radiusMax: 150→320`, `roofFadeNear: 16→34`, `roofFadeFar: 30→62`; kswScene: `domeRadius: 170→400`, `skyScale: 360→900`)
- Test: `tests/diorama/cameraRig.test.ts` (neuer Vertrags-Test gegen die echten Tokens)

**Interfaces:** Produces: unveränderte Signaturen; nur Werte.

- [ ] **Step 1: Vertrags-Test schreiben (RED)** — in `cameraRig.test.ts`, neuer describe-Block:

```ts
import { kswCamera } from '../../src/diorama/designTokens';

describe('kswCamera contract (navigierbarer Zoom)', () => {
  it('roofs are gone well before close-up: fade completes at radius ≥ 30', () => {
    expect(kswCamera.roofFadeNear).toBeGreaterThanOrEqual(30);
  });
  it('roofs are fully on at the overview framing (radius ~111)', () => {
    expect(roofFade(111, kswCamera)).toBe(1);
  });
  it('interior presets stay roofless', () => {
    expect(roofFade(14, kswCamera)).toBe(0);
  });
  it('allows zooming far out', () => {
    expect(kswCamera.radiusMax).toBeGreaterThanOrEqual(300);
  });
});
```

- [ ] **Step 2:** `npx vitest run tests/diorama/cameraRig.test.ts` → RED (roofFadeNear 16 < 30, radiusMax 150 < 300).
- [ ] **Step 3: Tokens setzen:** `radiusMax: 320`, `roofFadeNear: 34`, `roofFadeFar: 62`, `domeRadius: 400`, `skyScale: 900`.
- [ ] **Step 4:** Test-Lauf → GREEN.

### Task 2: Szene weitsichtfähig — camera.far + zoom-adaptiver Fog

**Files:**
- Modify: `src/diorama/ksw/main.ts`

**Interfaces:** Consumes `kswScene.domeRadius/skyScale`, `preset.fogNear/fogFar`, `kswScene.fogScale`, `rig.radius`.

- [ ] **Step 1: camera.far anheben** — `new THREE.PerspectiveCamera(kswCamera.fov, aspect, 0.1, 400)` → far `1400` (Sky-Sphere 900 + Max-Radius 320 < 1400).
- [ ] **Step 2: Fog zoom-adaptiv machen.** Basiswerte merken, pro Frame skalieren (Overview-Look identisch, Weitzoom nebelt nicht zu):

```ts
// bei Szenen-Setup:
const fogBaseNear = preset.fogNear * kswScene.fogScale;
const fogBaseFar = preset.fogFar * kswScene.fogScale;
scene.fog = new THREE.Fog(preset.fogColor, fogBaseNear, fogBaseFar);
// im animate(), nach dem Rig-Update:
const fogZoom = Math.max(1, rig.radius / 110);
(scene.fog as THREE.Fog).near = fogBaseNear * fogZoom;
(scene.fog as THREE.Fog).far = fogBaseFar * fogZoom;
```

- [ ] **Step 3:** `npm run typecheck` → sauber.
- [ ] **Step 4: Visuelle Verifikation** — Captures: `overview-morning` (Dächer an, Look unverändert), neuer Shot `far-morning` mit einem Zoom-Radius nahe Max (Harness-Param oder temporär via `?cam=overview` + Wheel im Smoke prüfen); Preview-Check im Browser: weit rauszoomen → Diorama bleibt klar sichtbar, Sky nicht geclippt, Wolkendome umschliesst die Kamera.
- [ ] **Step 5: Smoke** — `node scripts/smoke-ksw.mjs` → 13/13 PASS (Zoom-out-Check landet jetzt bei 320).
- [ ] **Step 6: Commit + Push**

```bash
git add -A && git commit -m "feat(ksw): earlier roof fade + far zoom-out with adaptive fog" && git push
```

## Self-Review
- Beide User-Anforderungen abgedeckt (früher ausblenden ✓ Task 1; weiter rauszoomen ✓ Task 1+2).
- Folgekosten des Weitzooms adressiert: camera.far, skyScale > radiusMax, domeRadius > radiusMax, Fog adaptiv ✓.
- Bestehende Framings geschützt via Vertrags-Test (roofFade(111)=1, roofFade(14)=0) ✓.
- Keine Platzhalter; exakte Werte benannt ✓.
