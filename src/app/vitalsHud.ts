/**
 * Read-only sim vitals HUD (bottom-left). Idempotent like the persistence
 * banner: re-render updates the single element in place.
 */
import type { EconomyVitalsDto } from '../backend/mobilityProtocol';

// MONEY_DISPLAY_SCALE is also used in inspectorPanelPainter.ts as a local
// constant; there is no shared module for it yet so we define it here too.
const MONEY_DISPLAY_SCALE = 1000;

export function setVitalsHud(doc: Document, vitals: EconomyVitalsDto | undefined): void {
  const existing = doc.querySelector('[data-vitals-hud]');
  if (!vitals) {
    existing?.remove();
    return;
  }
  const el = (existing as HTMLElement) ?? doc.createElement('div');
  el.setAttribute('data-vitals-hud', 'true');
  el.className = 'vitals-hud';
  el.textContent =
    `pop ${vitals.population} · routed ${vitals.routedCitizens} · ` +
    `money ${(vitals.totalMoney / MONEY_DISPLAY_SCALE).toFixed(2)} · ` +
    `routes ${vitals.routesAssigned}✓ ${vitals.routesFailed}✗`;
  if (!existing) doc.body.appendChild(el);
}
