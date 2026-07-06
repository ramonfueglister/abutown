// src/diorama/webgpuGate.ts
//
// Boot-time WebGPU capability gate (MMORPG M1 Task 15): the diorama renders
// exclusively through THREE.WebGPURenderer — on a browser without
// navigator.gpu the old behavior was an opaque black screen. Call
// `ensureWebGpu()` FIRST in every boot path that constructs the renderer;
// when it returns false, a centered message overlay was mounted and the boot
// must stop.

import { palette } from './designTokens';

function hex(c: number): string {
  return `#${c.toString(16).padStart(6, '0')}`;
}

/** True when WebGPU is available; otherwise mounts the message overlay and
 * returns false (caller stops booting). */
export function ensureWebGpu(): boolean {
  if (typeof navigator !== 'undefined' && 'gpu' in navigator && navigator.gpu) return true;

  const overlay = document.createElement('div');
  overlay.id = 'webgpu-gate';
  overlay.style.cssText = [
    'position:fixed',
    'inset:0',
    'z-index:100',
    'display:flex',
    'align-items:center',
    'justify-content:center',
    `background:${hex(palette.creamBase)}`,
    `color:${hex(palette.eye)}`,
    'font:16px/1.6 system-ui,sans-serif',
    'text-align:center',
    'padding:24px',
  ].join(';');
  const card = document.createElement('div');
  card.style.cssText = [
    'max-width:420px',
    'padding:28px 32px',
    `background:${hex(palette.creamLight)}`,
    'border-radius:16px',
    'box-shadow:0 4px 18px rgba(58,52,46,0.2)',
  ].join(';');
  card.innerHTML =
    '<div style="font-size:22px;font-weight:700;margin-bottom:10px">Abutown braucht WebGPU</div>' +
    '<div>Dieser Browser unterstützt kein WebGPU. Bitte Chrome/Edge 121+ oder Safari 26+ verwenden.</div>';
  overlay.appendChild(card);
  document.body.appendChild(overlay);
  return false;
}
