# Cities-Skylines-Kamerasteuerung für das Winterthur-Diorama

**Datum:** 2026-07-05
**Status:** Design genehmigt, bereit für Implementierungsplan

## Ziel

Die Diorama-Kamerasteuerung auf den *Cities: Skylines*-Standard umstellen:
Rotieren wandert auf die **rechte** Maustaste, Verschieben wird über
**WASD + Pfeiltasten** greifbar (zusätzlich zum bestehenden Edge-Scrolling),
und die linke Maustaste wird für spätere Selektion frei.

## Ist-Zustand

Steuerung liegt in zwei Schichten:

- **Reine Mathe** — `src/diorama/ksw/cameraRig.ts` (three.js-frei, voll unit-getestet
  in `tests/diorama/cameraRig.test.ts`). Enthält bereits `applyZoom`, `applyDrag`
  (Rotation), `edgePanVelocity` + `applyPan` (Edge-Scrolling).
- **Input-Wiring** — `src/diorama/ksw/main.ts` ab Zeile ~314. Aktuell:
  - Mausrad → `applyZoom` (Dolly, über `zoomTarget`).
  - `pointerdown` mit `e.button !== 0` (linke Taste) → Rotation-Drag via `applyDrag`.
  - `animate()`-Loop → `edgePanVelocity`/`applyPan`, pausiert während des Drags.

Kein Keyboard-Handling vorhanden.

## Ziel-Belegung

| Eingabe | Aktion | Änderung |
|---|---|---|
| Rechte Maustaste + ziehen | Rotieren/Kippen (Yaw/Pitch) | von links → rechts verschoben |
| WASD + Pfeiltasten | Verschieben (Keyboard-Pan) | **neu** |
| Maus an Bildschirmrand | Verschieben (Edge-Scroll) | unverändert |
| Q / E | Rotieren per Tastatur | **neu** |
| Mausrad | Zoom | unverändert |
| Linke Maustaste | frei (reserviert für Selektion) | entkoppelt |

## Design

### 1. Reine Mathe (`cameraRig.ts`)

**Neue Funktion `keyboardPanVelocity`**, exakt gespiegelt zu `edgePanVelocity`:

```
keyboardPanVelocity(
  held: { up: boolean; down: boolean; left: boolean; right: boolean },
  yaw: number,
  cfg: RigConfig,
): [number, number]
```

- Bildet gehaltene Richtungen auf Screen-Achsen ab: `sx = right - left`,
  `sy = up - down` (jeweils 0/±1).
- Projiziert über denselben Yaw-Basiswechsel wie `edgePanVelocity`
  (screen-right = `(cos yaw, -sin yaw)`, screen-forward = `(-sin yaw, -cos yaw)`),
  skaliert mit `cfg.panSpeed`.
- Gibt `[0, 0]` zurück, wenn nichts (oder nur Gegentasten) gehalten wird.
- Die Ergebnis-Geschwindigkeit wird wie beim Edge-Pan über `applyPan(s, vx, vz, dt, cfg)`
  auf das Ziel angewendet (inkl. `panBounds`-Clamp).

**Keyboard-Rotation (Q/E):** neues Feld `keyRotateSpeed` (rad/s) in `RigConfig`
und in `designTokens` (`kswCamera`). Anwendung im Loop über das vorhandene `applyDrag`,
indem ein synthetisches horizontales `dxPx = ±(keyRotateSpeed / dragSpeed) * dt`
durchgereicht wird — so bleibt die Rotationslogik an einer Stelle.

### 2. Input-Wiring (`main.ts`)

- **Rotate-Trigger** von `if (e.button !== 0) return;` auf `if (e.button !== 2) return;`
  (rechte Taste) umstellen. Pointer-Capture und `endDrag` (button-agnostisch auf
  `pointerup`/`pointercancel`) bleiben unverändert.
- **`contextmenu`-Listener** auf `renderer.domElement` mit `preventDefault`, damit
  Rechts-Drag nicht das Browser-Kontextmenü öffnet.
- **`keydown`/`keyup` auf `window`**, die ein `held`-Set pflegen. Tastenerkennung
  über `e.code` (layout-unabhängig): `KeyW/KeyA/KeyS/KeyD`, `ArrowUp/ArrowDown/
  ArrowLeft/ArrowRight` (→ Pan), `KeyQ/KeyE` (→ Rotation). Pfeiltasten `preventDefault`
  (verhindert Seiten-Scroll). Tasten ignorieren, wenn `document.activeElement` ein
  `INPUT`/`TEXTAREA` ist (defensiv).
- **`animate()`**: nach dem bestehenden Edge-Pan-Block das Keyboard-Pan mit
  demselben `zoomScale` und `dt` anwenden (`keyboardPanVelocity` → `applyPan`),
  danach das Q/E-Yaw-Delta via `applyDrag`. Kein `dragging`-Guard nötig, da
  Keyboard und rechte Maustaste unabhängig sind.

## Tests

### Unit (`tests/diorama/cameraRig.test.ts`)

Neuer `describe('keyboardPanVelocity')`-Block:

- null, wenn nichts gehalten wird und wenn nur Gegentasten (up+down) gehalten werden;
- `W` (up) pant screen-hoch = Welt −z bei yaw 0 (`dz < 0`, `dx ≈ 0`);
- `D` (right) pant Welt +x bei yaw 0;
- Richtung dreht mit dem Yaw (yaw 90°: screen-right → Welt −z);
- Diagonale (up+right) hat beide Komponenten ≠ 0.

### Browser-Smoke (`scripts/smoke-ksw.mjs`) — Pflicht laut CLAUDE.md

Weil die Änderung die Frontend-Input-Grenze berührt, ist ein realer Browser-Smoke
verpflichtend (nicht durch Unit-Tests ersetzbar):

- `W`-keydown (dispatch) verschiebt das Kamera-Ziel messbar (`__ENV_STATE`- bzw. eine
  Debug-Sonde auf `rig.target` lesen);
- Rechts-Drag (`pointerdown` button 2 → `pointermove`) ändert Yaw/Pitch;
- **Links**-Drag ändert die Kamera **nicht** mehr (Regression gegen das versehentliche
  Beibehalten der alten Belegung).

## Bewusst weggelassen (YAGNI)

- Kein Greif-Drag-Pan auf einer Maustaste (CS-treue Wahl des Nutzers).
- Keine Rebind-/Options-UI.
- Kein Pan-Momentum / Trägheit.
- Keine Änderung an Zoom, Roof-Fade oder den Kamera-Presets.
