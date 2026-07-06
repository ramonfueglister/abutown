// src/diorama/live/attribution.ts
//
// Fixed bottom-right data-attribution footer (MMORPG M1 Task 15). The baked
// world manifest carries the authoritative attribution strings
// (world.proto WorldManifest.attribution, written by scripts/geo/
// bake-world.mjs); when the world isn't loaded (or the list is empty) the
// static swisstopo/OSM line applies. Deliberately understated —
// designTokens colors, small type, no pointer events.

import { palette } from '../designTokens';

function hex(c: number): string {
  return `#${c.toString(16).padStart(6, '0')}`;
}

export const STATIC_ATTRIBUTION = '© swisstopo · © OpenStreetMap contributors';

export function createAttributionFooter(lines?: string[]): HTMLElement {
  const el = document.createElement('div');
  el.id = 'map-attribution';
  el.style.cssText = [
    'position:fixed',
    'right:8px',
    'bottom:6px',
    'z-index:20',
    `color:${hex(palette.eye)}`,
    'opacity:0.55',
    'font:10px/1.4 system-ui,sans-serif',
    'text-align:right',
    'pointer-events:none',
    'text-shadow:0 0 4px rgba(251,244,232,0.8)',
  ].join(';');
  el.textContent = lines && lines.length > 0 ? lines.join(' · ') : STATIC_ATTRIBUTION;
  return el;
}
