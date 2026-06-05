import { afterEach, describe, expect, it, vi } from 'vitest';
import { setPersistenceBanner } from '../../src/app/persistenceBanner';

type FakeBannerElement = {
  tagName: string;
  className: string;
  textContent: string;
  setAttribute: (name: string, value: string) => void;
  getAttribute: (name: string) => string | null;
  remove: () => void;
  dataset: Record<string, string>;
};

function installFakeDom(existingBanner = false) {
  let banner: FakeBannerElement | null = null;

  if (existingBanner) {
    banner = makeFakeBannerElement(() => {
      banner = null;
    });
  }

  function makeFakeBannerElement(onRemove: () => void): FakeBannerElement {
    const attrs: Record<string, string> = {};
    const el: FakeBannerElement = {
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
      appendChild: vi.fn((element: FakeBannerElement) => {
        banner = element;
      }),
    },
    createElement: vi.fn(() => makeFakeBannerElement(() => { banner = null; })),
    querySelector: vi.fn((_selector: string) => banner),
  } as unknown as Document;

  return { doc, getBanner: () => banner };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('setPersistenceBanner', () => {
  it('adds a banner element when status is degraded', () => {
    const { doc, getBanner } = installFakeDom(false);
    setPersistenceBanner(doc, 'degraded');
    const banner = getBanner();
    expect(banner).not.toBeNull();
    expect(banner?.className).toBe('persistence-banner');
    expect(banner?.textContent).toContain('vorübergehend verzögert');
  });

  it('removes the banner when status is healthy', () => {
    const { doc, getBanner } = installFakeDom(true);
    expect(getBanner()).not.toBeNull();
    setPersistenceBanner(doc, 'healthy');
    expect(getBanner()).toBeNull();
  });

  it('removes the banner when status is starting', () => {
    const { doc, getBanner } = installFakeDom(true);
    setPersistenceBanner(doc, 'starting');
    expect(getBanner()).toBeNull();
  });

  it('does not duplicate the banner on repeated degraded calls', () => {
    const { doc, getBanner } = installFakeDom(false);
    setPersistenceBanner(doc, 'degraded');
    const firstBanner = getBanner();
    setPersistenceBanner(doc, 'degraded');
    const secondBanner = getBanner();
    // Same element — no new element appended.
    expect(firstBanner).toBe(secondBanner);
    expect((doc.body.appendChild as ReturnType<typeof vi.fn>).mock.calls).toHaveLength(1);
  });

  it('shows offline text for stale status', () => {
    const { doc, getBanner } = installFakeDom(false);
    setPersistenceBanner(doc, 'stale');
    expect(getBanner()?.textContent).toContain('offline');
  });

  it('shows offline text for down status', () => {
    const { doc, getBanner } = installFakeDom(false);
    setPersistenceBanner(doc, 'down');
    expect(getBanner()?.textContent).toContain('offline');
  });

  it('does not add a banner when status is healthy with no existing banner', () => {
    const { doc, getBanner } = installFakeDom(false);
    setPersistenceBanner(doc, 'healthy');
    expect(getBanner()).toBeNull();
    expect((doc.body.appendChild as ReturnType<typeof vi.fn>).mock.calls).toHaveLength(0);
  });
});
