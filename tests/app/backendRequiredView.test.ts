import { afterEach, describe, expect, it, vi } from 'vitest';
import { escapeHtml, renderBackendRequired } from '../../src/app/backendRequiredView';

type FakeElement = {
  className: string;
  dataset: Record<string, string>;
  innerHTML: string;
  remove: () => void;
};

function installFakeDom(existingPanel = false) {
  let panel: FakeElement | null = existingPanel
    ? { className: '', dataset: { backendRequired: 'true' }, innerHTML: 'old', remove: () => { panel = null; } }
    : null;
  const document = {
    body: {
      appendChild: vi.fn((element: FakeElement) => {
        panel = element;
      }),
    },
    createElement: vi.fn(() => {
      const element: FakeElement = {
        className: '',
        dataset: {},
        innerHTML: '',
        remove: () => {
          if (panel === element) panel = null;
        },
      };
      return element;
    }),
    querySelector: vi.fn(() => panel),
    querySelectorAll: vi.fn(() => (panel ? [panel] : [])),
  };
  vi.stubGlobal('document', document);
  vi.stubGlobal('window', { innerWidth: 800, innerHeight: 600, devicePixelRatio: 1 });
  return { currentPanel: () => panel };
}

function createCanvas(): HTMLCanvasElement {
  const context = {
    save: vi.fn(),
    restore: vi.fn(),
    setTransform: vi.fn(),
    fillRect: vi.fn(),
    fillStyle: '',
  } as unknown as CanvasRenderingContext2D;
  return {
    dataset: {},
    getContext: vi.fn(() => context),
  } as unknown as HTMLCanvasElement;
}

describe('backendRequiredView', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('escapes text inserted into the backend-required view', () => {
    expect(escapeHtml('<backend "down" & unsafe>')).toBe('&lt;backend &quot;down&quot; &amp; unsafe&gt;');
  });

  it('renders a fail-closed backend-required panel and marks the canvas not ready', () => {
    const dom = installFakeDom();
    const canvas = createCanvas();
    const ctx = canvas.getContext('2d') as CanvasRenderingContext2D;
    const logError = vi.fn();

    renderBackendRequired({
      canvas,
      ctx,
      baseUrl: 'http://127.0.0.1:8080',
      background: '#f6f0e3',
      error: new Error('network <down>'),
      viewport: { width: 800, height: 600, devicePixelRatio: 2 },
      logError,
    });

    const panel = dom.currentPanel();
    expect(canvas.dataset.ready).toBe('false');
    expect(canvas.dataset.backendRequired).toBe('true');
    expect(ctx.save).toHaveBeenCalled();
    expect(ctx.setTransform).toHaveBeenCalledWith(2, 0, 0, 2, 0, 0);
    expect(ctx.fillStyle).toBe('#f6f0e3');
    expect(ctx.fillRect).toHaveBeenCalledWith(0, 0, 800, 600);
    expect(ctx.restore).toHaveBeenCalled();
    expect(panel).not.toBeNull();
    expect(panel?.innerHTML).toContain('Backend required');
    expect(panel?.innerHTML).toContain('network &lt;down&gt;');
    expect(panel?.innerHTML).toContain('http://127.0.0.1:8080');
    expect(panel?.innerHTML).toContain('cargo run --manifest-path backend/Cargo.toml -p sim-server');
    expect(logError).toHaveBeenCalledWith('Abutown backend required: network <down>');
  });

  it('replaces any existing backend-required panel instead of stacking panels', () => {
    const dom = installFakeDom(true);
    const canvas = createCanvas();
    const ctx = canvas.getContext('2d') as CanvasRenderingContext2D;

    renderBackendRequired({
      canvas,
      ctx,
      baseUrl: 'http://127.0.0.1:8080',
      background: '#f6f0e3',
      error: 'Backend required',
      viewport: { width: 800, height: 600, devicePixelRatio: 1 },
      logError: vi.fn(),
    });

    expect(dom.currentPanel()?.innerHTML).not.toContain('old');
    expect(dom.currentPanel()?.innerHTML).toContain('Backend required');
  });
});
