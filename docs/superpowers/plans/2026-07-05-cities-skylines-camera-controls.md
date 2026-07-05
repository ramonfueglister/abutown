# Cities-Skylines-Kamerasteuerung Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rotieren auf die rechte Maustaste legen und Verschieben über WASD/Pfeiltasten (plus Q/E-Tastatur-Rotation) ergänzen, damit die Diorama-Kamera dem Cities-Skylines-Standard folgt.

**Architecture:** Die bestehende Zwei-Schichten-Trennung bleibt: reine, three.js-freie Mathe in `cameraRig.ts` (unit-getestet) und Input-Wiring in `main.ts`. Neu ist eine reine `keyboardPanVelocity`-Funktion (gespiegelt zu `edgePanVelocity`) plus Konfig-Feld `keyRotateSpeed`; das Wiring stellt den Rotate-Trigger auf die rechte Maustaste um und pflegt einen Tastatur-`held`-Zustand, den der `animate()`-Loop in `applyPan`/`applyDrag` einspeist.

**Tech Stack:** TypeScript, three.js (WebGPU), Vitest (Unit), Playwright (Browser-Smoke), Vite Dev-Server.

## Global Constraints

- Reine Mathe in `src/diorama/ksw/cameraRig.ts` bleibt three.js-frei und vollständig unit-testbar (bestehende Datei-Konvention, Kommentar Zeile 1–4).
- Keine Legacy-/Fallback-Shims, keine optionalen Konfig-Felder als Krücke: `keyRotateSpeed` wird ein **required** Feld von `RigConfig` (Projekt-Regel „no legacy/fallback cruft").
- Tastenerkennung layout-unabhängig über `KeyboardEvent.code` (nie `.key`).
- Browser-Smoke ist Pflicht für diese Änderung (berührt die Frontend-Input-Grenze `src/render`↔`main.ts`, CLAUDE.md „Browser-smoke is mandatory").
- Nur ein `cargo`/Dev-Server-Prozess relevant hier — kein cargo betroffen (reines Frontend).
- Bestehende Werte (verbatim aus `designTokens.ts` `kswCamera`): `dragSpeed: 0.005`, `panSpeed: 30`, `panBoundsX: 34`, `panBoundsZ: 26`, `radiusMax: 320`.

---

### Task 1: Reine Mathe — `keyboardPanVelocity` + `keyRotateSpeed`

**Files:**
- Modify: `src/diorama/ksw/cameraRig.ts` (RigConfig-Typ + neue Funktion)
- Modify: `src/diorama/designTokens.ts:253-273` (`kswCamera`-Literal)
- Test: `tests/diorama/cameraRig.test.ts` (cfg-Literal + neuer describe-Block)

**Interfaces:**
- Consumes: bestehendes `RigConfig`, `panSpeed`-Feld.
- Produces:
  - `export type PanKeys = { up: boolean; down: boolean; left: boolean; right: boolean }`
  - `export function keyboardPanVelocity(held: PanKeys, yaw: number, cfg: RigConfig): [number, number]`
  - neues required Feld `RigConfig.keyRotateSpeed: number` (rad/s)
  - `kswCamera.keyRotateSpeed = 1.2`

- [ ] **Step 1: Failing test schreiben** — in `tests/diorama/cameraRig.test.ts`

Import-Zeile (aktuell Zeile 3–13) um `keyboardPanVelocity` und `type PanKeys` ergänzen:

```ts
import {
  applyDrag,
  applyPan,
  applyZoom,
  edgePanVelocity,
  keyboardPanVelocity,
  rigFromLookAt,
  rigPosition,
  roofFade,
  type CameraRigState,
  type PanKeys,
  type RigConfig,
} from '../../src/diorama/ksw/cameraRig';
```

Im `cfg`-Literal (aktuell Zeile 15–28) das neue Feld nach `panBoundsZ: 26,` ergänzen:

```ts
  panBoundsZ: 26,
  keyRotateSpeed: 1.2,
```

Neuen describe-Block am Ende der Datei (vor der letzten `});` der Datei, also nach dem `roofFade`-Block) einfügen:

```ts
describe('keyboardPanVelocity (WASD/arrow keyboard pan)', () => {
  const none: PanKeys = { up: false, down: false, left: false, right: false };

  it('is zero when nothing is held', () => {
    expect(keyboardPanVelocity(none, 0, cfg)).toEqual([0, 0]);
  });

  it('is zero when opposing keys cancel (up+down)', () => {
    expect(keyboardPanVelocity({ ...none, up: true, down: true }, 0, cfg)).toEqual([0, 0]);
  });

  it('W (up) pans screen-up = world -z at yaw 0', () => {
    const [dx, dz] = keyboardPanVelocity({ ...none, up: true }, 0, cfg);
    expect(dz).toBeLessThan(0);
    expect(Math.abs(dx)).toBeLessThan(1e-9);
  });

  it('D (right) pans world +x at yaw 0', () => {
    const [dx, dz] = keyboardPanVelocity({ ...none, right: true }, 0, cfg);
    expect(dx).toBeGreaterThan(0);
    expect(Math.abs(dz)).toBeLessThan(1e-9);
  });

  it('pan direction rotates with the camera yaw (90 deg: screen-right -> world -z)', () => {
    const [dx, dz] = keyboardPanVelocity({ ...none, right: true }, Math.PI / 2, cfg);
    expect(Math.abs(dx)).toBeLessThan(1e-6);
    expect(dz).toBeLessThan(0);
  });

  it('diagonal (up+right) has both components non-zero', () => {
    const [dx, dz] = keyboardPanVelocity({ ...none, up: true, right: true }, 0, cfg);
    expect(dx).toBeGreaterThan(0);
    expect(dz).toBeLessThan(0);
  });
});
```

- [ ] **Step 2: Test laufen lassen — muss fehlschlagen**

Run: `npx vitest run tests/diorama/cameraRig.test.ts`
Expected: FAIL — `keyboardPanVelocity` ist nicht exportiert / `PanKeys` unbekannt (Import-Fehler).

- [ ] **Step 3: `RigConfig` erweitern** — in `src/diorama/ksw/cameraRig.ts`, im `RigConfig`-Typ (Zeile 13–27) nach `panBoundsZ: number;`:

```ts
  panBoundsZ: number; // |target.z| clamp
  keyRotateSpeed: number; // rad/s for Q/E keyboard rotation
};
```

- [ ] **Step 4: `PanKeys` + `keyboardPanVelocity` implementieren** — in `src/diorama/ksw/cameraRig.ts`, direkt nach `edgePanVelocity` (nach dessen schließender `}` bei Zeile 98) einfügen:

```ts
export type PanKeys = { up: boolean; down: boolean; left: boolean; right: boolean };

// WASD/arrow keyboard pan: mirrors edgePanVelocity's screen->world basis so
// keyboard and edge scrolling agree exactly. Held direction flags collapse to
// screen axes (sx = right-left, sy = up-down), then project through the yaw.
export function keyboardPanVelocity(held: PanKeys, yaw: number, cfg: RigConfig): [number, number] {
  const sx = (held.right ? 1 : 0) - (held.left ? 1 : 0);
  const sy = (held.up ? 1 : 0) - (held.down ? 1 : 0);
  if (sx === 0 && sy === 0) return [0, 0];
  const rightX = Math.cos(yaw);
  const rightZ = -Math.sin(yaw);
  const fwdX = -Math.sin(yaw);
  const fwdZ = -Math.cos(yaw);
  return [(sx * rightX + sy * fwdX) * cfg.panSpeed, (sx * rightZ + sy * fwdZ) * cfg.panSpeed];
}
```

- [ ] **Step 5: `kswCamera`-Token erweitern** — in `src/diorama/designTokens.ts`, im `kswCamera`-Literal nach `panBoundsZ: 26,` (Zeile 272):

```ts
  panBoundsZ: 26,
  keyRotateSpeed: 1.2, // rad/s for Q/E keyboard rotation (~69 deg/s)
```

- [ ] **Step 6: Tests laufen lassen — müssen grün sein**

Run: `npx vitest run tests/diorama/cameraRig.test.ts`
Expected: PASS — alle bisherigen Tests plus die 6 neuen `keyboardPanVelocity`-Fälle grün.

- [ ] **Step 7: Typecheck**

Run: `npx tsc --noEmit`
Expected: keine Fehler (kswCamera erfüllt jetzt das erweiterte RigConfig).

- [ ] **Step 8: Commit**

```bash
git add src/diorama/ksw/cameraRig.ts src/diorama/designTokens.ts tests/diorama/cameraRig.test.ts
git commit -m "feat(camera): keyboardPanVelocity + keyRotateSpeed config

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Input-Wiring + Browser-Smoke

**Files:**
- Modify: `src/diorama/ksw/main.ts:39` (Import), `:347-371` (Pointer-Wiring), `~:1086-1093` (animate-Loop)
- Modify: `scripts/smoke-ksw.mjs:2-6` (Header-Kommentar), `:111-126` (Drag-Contract), Neu: Keyboard-Pan-Check

**Interfaces:**
- Consumes: `keyboardPanVelocity`, `applyPan`, `applyDrag` aus `cameraRig.ts`; `kswCamera.keyRotateSpeed`, `kswCamera.dragSpeed` aus `designTokens.ts`; `window.__KSW` (bereits vorhanden: `{ radius, yaw, pitch, roofFade, target }`).
- Produces: keine neuen Exporte — reines Event-Wiring.

- [ ] **Step 1: Smoke auf den neuen Contract umschreiben (das ist der RED-Test)** — in `scripts/smoke-ksw.mjs`.

Header-Kommentar Zeile 2–6 ersetzen:

Alt:
```js
// dynamic camera contract end-to-end:
//   1. wheel up   -> radius shrinks (zoom in) -> roofs fade out
//   2. wheel down -> radius grows (zoom out)  -> roofs fade back in
//   3. left-drag  -> yaw changes (orbit), radius unchanged
// Exits non-zero on any violation. Usage: node scripts/smoke-ksw.mjs
```
Neu:
```js
// dynamic camera contract end-to-end (Cities-Skylines controls):
//   1. wheel up    -> radius shrinks (zoom in) -> roofs fade out
//   2. wheel down  -> radius grows (zoom out)  -> roofs fade back in
//   3. right-drag  -> yaw changes (orbit), radius unchanged
//   4. left-drag   -> nothing (freed for future selection)
//   5. hold W      -> camera target pans (WASD)
// Exits non-zero on any violation. Usage: node scripts/smoke-ksw.mjs
```

Den Drag-Block Zeile 111–126 ersetzen:

Alt:
```js
  // left-drag: orbit
  await page.mouse.move(640, 400);
  await page.mouse.down({ button: 'left' });
  await page.mouse.move(880, 430, { steps: 12 });
  await page.mouse.up({ button: 'left' });
  await page.waitForTimeout(250);
  const s3 = await state();
  check('left-drag rotates the camera (yaw changes)', Math.abs(s3.yaw - s2.yaw) > 0.2, `${s2.yaw.toFixed(2)} -> ${s3.yaw.toFixed(2)}`);
  // eased zoom may still be settling by a hair — drag itself must not dolly
  check('drag does not change the zoom radius', Math.abs(s3.radius - s2.radius) < 0.5, `${s2.radius.toFixed(2)} vs ${s3.radius.toFixed(2)}`);

  // moving without the button held must not rotate
  await page.mouse.move(400, 300, { steps: 6 });
  await page.waitForTimeout(150);
  const s4 = await state();
  check('hover without button held does not rotate', Math.abs(s4.yaw - s3.yaw) < 1e-6, `${s3.yaw.toFixed(3)} vs ${s4.yaw.toFixed(3)}`);
```
Neu:
```js
  // right-drag: orbit (Cities-Skylines standard)
  await page.mouse.move(640, 400);
  await page.mouse.down({ button: 'right' });
  await page.mouse.move(880, 430, { steps: 12 });
  await page.mouse.up({ button: 'right' });
  await page.waitForTimeout(250);
  const s3 = await state();
  check('right-drag rotates the camera (yaw changes)', Math.abs(s3.yaw - s2.yaw) > 0.2, `${s2.yaw.toFixed(2)} -> ${s3.yaw.toFixed(2)}`);
  // eased zoom may still be settling by a hair — drag itself must not dolly
  check('drag does not change the zoom radius', Math.abs(s3.radius - s2.radius) < 0.5, `${s2.radius.toFixed(2)} vs ${s3.radius.toFixed(2)}`);

  // left-drag must NOT rotate any more (freed for future selection)
  await page.mouse.move(640, 400);
  await page.mouse.down({ button: 'left' });
  await page.mouse.move(880, 430, { steps: 12 });
  await page.mouse.up({ button: 'left' });
  await page.waitForTimeout(250);
  const s4 = await state();
  check('left-drag no longer rotates the camera', Math.abs(s4.yaw - s3.yaw) < 1e-6, `${s3.yaw.toFixed(3)} vs ${s4.yaw.toFixed(3)}`);

  // keyboard pan: holding W moves the camera target (Cities-Skylines WASD).
  // click first so the page (window keydown listener) has focus; left-click is
  // a no-op now, so it can't perturb the camera.
  await page.mouse.click(640, 400);
  const kb0 = await state();
  await page.keyboard.down('w');
  await page.waitForTimeout(700);
  await page.keyboard.up('w');
  await page.waitForTimeout(150);
  const kb1 = await state();
  const kbMoved = Math.hypot(kb1.target[0] - kb0.target[0], kb1.target[2] - kb0.target[2]);
  check('holding W pans the camera target (WASD)', kbMoved > 3, `moved ${kbMoved.toFixed(1)} units`);
```

- [ ] **Step 2: Smoke gegen den UNVERÄNDERTEN main.ts laufen lassen — muss fehlschlagen (RED)**

Run: `node scripts/smoke-ksw.mjs`
Expected: SMOKE FAIL — mind. „right-drag rotates the camera" (rechte Taste rotiert noch nicht), „left-drag no longer rotates" (linke Taste rotiert noch) und „holding W pans" (keine Tastatur) schlagen fehl.

- [ ] **Step 3: Import in `main.ts` ergänzen** — Zeile 39:

Alt:
```ts
import { applyDrag, applyPan, applyZoom, edgePanVelocity, rigPosition, roofFade, type CameraRigState } from './cameraRig';
```
Neu:
```ts
import { applyDrag, applyPan, applyZoom, edgePanVelocity, keyboardPanVelocity, rigPosition, roofFade, type CameraRigState } from './cameraRig';
```

- [ ] **Step 4: Rotate auf die rechte Maustaste + Kontextmenü unterdrücken** — in `src/diorama/ksw/main.ts`, im `pointerdown`-Handler (Zeile 351–355):

Alt:
```ts
  renderer.domElement.addEventListener('pointerdown', (e: PointerEvent) => {
    if (e.button !== 0) return;
    dragging = true;
    renderer.domElement.setPointerCapture(e.pointerId);
  });
```
Neu:
```ts
  renderer.domElement.addEventListener('pointerdown', (e: PointerEvent) => {
    if (e.button !== 2) return; // right button rotates (Cities-Skylines standard)
    dragging = true;
    renderer.domElement.setPointerCapture(e.pointerId);
  });
  // right-drag rotates → suppress the browser context menu over the canvas
  renderer.domElement.addEventListener('contextmenu', (e: Event) => e.preventDefault());
```

- [ ] **Step 5: Tastatur-`held`-Zustand einführen** — in `src/diorama/ksw/main.ts`, direkt nach dem `pointercancel`-Listener (nach Zeile 371, vor dem `── light rig ──`-Block):

```ts
  // ── keyboard: WASD/arrows pan, Q/E rotate (Cities-Skylines standard) ──────
  const held = { up: false, down: false, left: false, right: false, rotL: false, rotR: false };
  const keyFlag = (code: string): keyof typeof held | null => {
    switch (code) {
      case 'KeyW':
      case 'ArrowUp':
        return 'up';
      case 'KeyS':
      case 'ArrowDown':
        return 'down';
      case 'KeyA':
      case 'ArrowLeft':
        return 'left';
      case 'KeyD':
      case 'ArrowRight':
        return 'right';
      case 'KeyQ':
        return 'rotL';
      case 'KeyE':
        return 'rotR';
      default:
        return null;
    }
  };
  const typingTarget = (): boolean => {
    const el = document.activeElement;
    return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA');
  };
  window.addEventListener('keydown', (e: KeyboardEvent) => {
    if (typingTarget()) return;
    const f = keyFlag(e.code);
    if (!f) return;
    held[f] = true;
    if (e.code.startsWith('Arrow')) e.preventDefault(); // no page scroll
  });
  window.addEventListener('keyup', (e: KeyboardEvent) => {
    const f = keyFlag(e.code);
    if (f) held[f] = false;
  });
```

- [ ] **Step 6: Keyboard-Pan + Q/E-Rotate im animate-Loop** — in `src/diorama/ksw/main.ts`, direkt nach dem bestehenden Edge-Pan-Block (nach der schließenden `}` bei Zeile 1093, unmittelbar vor `applyRig();`):

```ts
    // keyboard pan (WASD/arrows) — same map-relative zoom scaling as edge-pan
    const [kvx, kvz] = keyboardPanVelocity(held, rig.yaw, kswCamera);
    if (kvx !== 0 || kvz !== 0) {
      const zoomScale = Math.min(Math.max(rig.radius / 110, 0.15), 1);
      rig = applyPan(rig, kvx * zoomScale, kvz * zoomScale, dt, kswCamera);
    }
    // keyboard rotate (Q/E) via the shared drag path: convert a rad/s rate into
    // the equivalent horizontal drag-pixel delta (applyDrag: yaw -= dxPx*dragSpeed)
    if (held.rotL !== held.rotR) {
      const dir = held.rotL ? 1 : -1;
      const dxPx = (dir * kswCamera.keyRotateSpeed * dt) / kswCamera.dragSpeed;
      rig = applyDrag(rig, dxPx, 0, kswCamera);
    }
```

- [ ] **Step 7: Typecheck**

Run: `npx tsc --noEmit`
Expected: keine Fehler. (`held` hat Extra-Felder `rotL/rotR`; als Variable an `keyboardPanVelocity(held, …)` übergeben ist das strukturell zulässig — kein Excess-Property-Check auf Nicht-Literalen.)

- [ ] **Step 8: Smoke erneut laufen lassen — muss grün sein (GREEN)**

Run: `node scripts/smoke-ksw.mjs`
Expected: `SMOKE OK — dynamic camera verified in a real browser`. Insbesondere: „right-drag rotates", „left-drag no longer rotates", „holding W pans the camera target" grün.

- [ ] **Step 9: Unit-Tests + Typecheck als Regression**

Run: `npx vitest run tests/diorama/cameraRig.test.ts && npx tsc --noEmit`
Expected: alle grün, keine Typfehler.

- [ ] **Step 10: Commit**

```bash
git add src/diorama/ksw/main.ts scripts/smoke-ksw.mjs
git commit -m "feat(camera): Cities-Skylines controls — right-drag rotate, WASD/QE keyboard

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Rechte Maustaste = Rotieren → Task 2 Step 4. ✅
- WASD + Pfeiltasten = Pan → Task 1 (`keyboardPanVelocity`) + Task 2 Step 5/6. ✅
- Q/E = Rotieren → Task 1 (`keyRotateSpeed`) + Task 2 Step 6. ✅
- Kontextmenü unterdrücken → Task 2 Step 4. ✅
- Linke Maustaste frei → Task 2 Step 4 (button-Guard auf 2). ✅
- Edge-Scroll unverändert → nicht angefasst. ✅
- Unit-Tests → Task 1 Step 1. ✅
- Browser-Smoke (Pflicht) → Task 2 Step 1/2/8. ✅

**Placeholder scan:** kein TBD/TODO; jeder Code-Step zeigt vollständigen Code. ✅

**Type consistency:** `keyboardPanVelocity(held: PanKeys, yaw, cfg)` identisch in Task 1 (Definition), Task 1-Test (Import) und Task 2 (Aufruf). `keyRotateSpeed` als required RigConfig-Feld in Task 1 Step 3 definiert, in Task 1 Step 5 (kswCamera) und Task 1 Step 1 (test-cfg) gesetzt, in Task 2 Step 6 gelesen. `held` mit `up/down/left/right` (+`rotL/rotR`) konsistent zwischen Step 5 und Step 6. ✅
