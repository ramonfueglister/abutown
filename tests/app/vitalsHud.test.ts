// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { setVitalsHud } from '../../src/app/vitalsHud';

describe('setVitalsHud', () => {
  afterEach(() => { document.querySelector('[data-vitals-hud]')?.remove(); });
  it('renders population, routed, and money; idempotent', () => {
    const vitals = { population: 348, routedCitizens: 13, totalMoney: 3_000_000, routesAssigned: 5, routesFailed: 1 };
    setVitalsHud(document, vitals);
    setVitalsHud(document, vitals);
    const els = document.querySelectorAll('[data-vitals-hud]');
    expect(els).toHaveLength(1);
    expect(els[0].textContent).toContain('pop 348');
    expect(els[0].textContent).toContain('routed 13');
    expect(els[0].textContent).toContain('money 3000.00');
  });
  it('removes the HUD when vitals are absent', () => {
    setVitalsHud(document, { population: 1, routedCitizens: 0, totalMoney: 0, routesAssigned: 0, routesFailed: 0 });
    setVitalsHud(document, undefined);
    expect(document.querySelector('[data-vitals-hud]')).toBeNull();
  });
});
