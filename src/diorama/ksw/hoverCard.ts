// src/diorama/ksw/hoverCard.ts
// Ultra-minimal building info card: GWR category (what it IS) over the ÖREB
// Bauzone (what's ALLOWED). Two lines + optional name. DOM-injected,
// pointer-events: none, follows the cursor with a small offset.
import type { BuildingHoverInfo } from './geo/buildingAttributes';

export function createHoverCard(): {
  show(info: BuildingHoverInfo, clientX: number, clientY: number): void;
  hide(): void;
} {
  const el = document.createElement('div');
  el.style.cssText = [
    'position:fixed', 'display:none', 'pointer-events:none', 'z-index:30',
    'padding:6px 9px', 'border-radius:3px',
    'background:rgba(24,26,30,0.82)', 'backdrop-filter:blur(2px)',
    'color:#e9ede1', 'font:11px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace',
    'letter-spacing:0.01em', 'white-space:nowrap',
  ].join(';');
  document.body.appendChild(el);
  return {
    show(info, clientX, clientY) {
      const name = info.name ? `<div style="font-weight:600">${escapeHtml(info.name)}</div>` : '';
      const ist = escapeHtml(info.gwrCategory ?? 'Nutzung unbekannt');
      const erlaubt = info.bauzone
        ? `${escapeHtml(info.bauzone)} · erlaubt`
        : 'keine Bauzone';
      el.innerHTML = `${name}<div>${ist}</div><div style="opacity:0.72">${erlaubt}</div>`;
      el.style.left = `${clientX + 14}px`;
      el.style.top = `${clientY + 14}px`;
      el.style.display = 'block';
    },
    hide() {
      el.style.display = 'none';
    },
  };
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}
