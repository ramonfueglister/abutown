import { backendErrorMessage } from '../backend/backendGate';

export type BackendRequiredViewport = {
  width: number;
  height: number;
  devicePixelRatio: number;
};

export type RenderBackendRequiredOptions = {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  baseUrl: string;
  background: string;
  error: unknown;
  viewport?: BackendRequiredViewport;
  logError?: (message: string) => void;
};

export function renderBackendRequired(options: RenderBackendRequiredOptions): void {
  const viewport = options.viewport ?? {
    width: window.innerWidth,
    height: window.innerHeight,
    devicePixelRatio: window.devicePixelRatio || 1,
  };
  const message = backendErrorMessage(options.error);

  options.canvas.dataset.ready = 'false';
  options.canvas.dataset.backendRequired = 'true';
  options.ctx.save();
  options.ctx.setTransform(viewport.devicePixelRatio, 0, 0, viewport.devicePixelRatio, 0, 0);
  options.ctx.fillStyle = options.background;
  options.ctx.fillRect(0, 0, viewport.width, viewport.height);
  options.ctx.restore();

  document.querySelector<HTMLElement>('[data-backend-required]')?.remove();
  const panel = document.createElement('section');
  panel.className = 'backend-required-panel';
  panel.dataset.backendRequired = 'true';
  panel.innerHTML = `
    <h1>Backend required</h1>
    <p>Start Abutown backend at ${escapeHtml(options.baseUrl)} and reload.</p>
    <pre>cargo run --manifest-path backend/Cargo.toml -p sim-server</pre>
    <small>${escapeHtml(message)}</small>
  `;
  document.body.appendChild(panel);
  (options.logError ?? console.error)(`Abutown backend required: ${message}`);
}

export function escapeHtml(value: unknown): string {
  return String(value ?? '').replace(/[&<>"']/g, (char) => ({
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;',
  })[char] ?? char);
}
