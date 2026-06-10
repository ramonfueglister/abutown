import { afterEach, describe, expect, it, vi } from 'vitest';
import { setVitalsHud } from '../../src/app/vitalsHud';

type FakeHudElement = {
  tagName: string;
  className: string;
  textContent: string;
  setAttribute: (name: string, value: string) => void;
  getAttribute: (name: string) => string | null;
  remove: () => void;
  dataset: Record<string, string>;
};

function installFakeDom(existingHud = false) {
  let hud: FakeHudElement | null = null;

  if (existingHud) {
    hud = makeFakeHudElement(() => {
      hud = null;
    });
  }

  function makeFakeHudElement(onRemove: () => void): FakeHudElement {
    const attrs: Record<string, string> = {};
    const el: FakeHudElement = {
      tagName: 'DIV',
      className: '',
      textContent: '',
      dataset: {},
      setAttribute: vi.fn((name: string, value: string) => { attrs[name] = value; }),
      getAttribute: vi.fn((name: string) => attrs[name] ?? null),
      remove: vi.fn(onRemove),
    };
    return el;
  }

  const doc = {
    body: {
      appendChild: vi.fn((element: FakeHudElement) => {
        hud = element;
      }),
    },
    createElement: vi.fn(() => makeFakeHudElement(() => { hud = null; })),
    querySelector: vi.fn((_selector: string) => hud),
  } as unknown as Document;

  return { doc, getHud: () => hud };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('setVitalsHud', () => {
  it('renders population, routed, and money; idempotent single element', () => {
    const { doc, getHud } = installFakeDom(false);
    const vitals = { population: 348, routedCitizens: 13, totalMoney: 3_000_000, routesAssigned: 5, routesFailed: 1 };
    setVitalsHud(doc, vitals);
    setVitalsHud(doc, vitals);
    const hud = getHud();
    expect(hud).not.toBeNull();
    expect((doc.body.appendChild as ReturnType<typeof vi.fn>).mock.calls).toHaveLength(1);
    expect(hud?.textContent).toContain('pop 348');
    expect(hud?.textContent).toContain('routed 13');
    expect(hud?.textContent).toContain('money 3000.00');
  });

  it('removes the HUD when vitals are undefined', () => {
    const { doc, getHud } = installFakeDom(true);
    expect(getHud()).not.toBeNull();
    setVitalsHud(doc, undefined);
    expect(getHud()).toBeNull();
  });
});
