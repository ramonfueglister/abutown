// src/diorama/live/vitalsHud.ts
//
// The world-vitals HUD card (MMORPG M1 Task 15): a small fixed DOM overlay
// showing world clock, population, total money, SFC-audit state, the four
// market prices and active trips — fed 1 Hz by the live client's onVitals.
// Styling derives from designTokens.palette (the diorama's only color
// source); the two pure formatters are unit-tested (tests/live/vitalsHud).

import { palette } from '../designTokens';
import type { LiveVitals } from './liveClient';

/** #rrggbb string for a designTokens palette number. */
function hex(c: number): string {
  return `#${c.toString(16).padStart(6, '0')}`;
}

/** "HH:MM" for a seconds-of-world-day value (wraps at 24h; sub-minute
 * seconds truncate). */
export function formatWorldClock(sOfWorldDay: number): string {
  const s = ((Math.floor(sOfWorldDay) % 86_400) + 86_400) % 86_400;
  const hh = Math.floor(s / 3_600);
  const mm = Math.floor((s % 3_600) / 60);
  return `${String(hh).padStart(2, '0')}:${String(mm).padStart(2, '0')}`;
}

/** Raw ×1000 money -> whole units, rounded to nearest, Swiss thousands
 * grouping ("123'457"). BigInt-exact (no float detour). */
export function formatMoney(totalMoney: bigint): string {
  const neg = totalMoney < 0n;
  const abs = neg ? -totalMoney : totalMoney;
  const units = (abs + 500n) / 1000n; // round half up on the magnitude
  const grouped = units
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, "'");
  return neg ? `-${grouped}` : grouped;
}

export interface VitalsHud {
  /** Append this to document.body (or any host). */
  element: HTMLElement;
  /** Feed the latest vitals (1 Hz from the live client). */
  update(v: LiveVitals): void;
  /** The last vitals applied (debug/smoke surface). */
  readonly lastVitals: LiveVitals | null;
}

export function createVitalsHud(): VitalsHud {
  const card = document.createElement('div');
  card.id = 'vitals-hud';
  card.style.cssText = [
    'position:fixed',
    'top:12px',
    'right:12px',
    'z-index:20',
    'min-width:200px',
    'padding:10px 14px',
    `background:${hex(palette.creamLight)}e6`, // cream, slightly translucent
    `color:${hex(palette.eye)}`,
    'border-radius:12px',
    'box-shadow:0 2px 10px rgba(58,52,46,0.18)',
    'font:12px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace',
    'pointer-events:none',
    'white-space:pre',
  ].join(';');

  const clockEl = document.createElement('div');
  clockEl.style.cssText = 'font-size:20px;font-weight:700;letter-spacing:1px';
  clockEl.textContent = '--:--';

  const statsEl = document.createElement('div');
  statsEl.textContent = 'warte auf Vitals…';

  const pricesEl = document.createElement('div');
  pricesEl.style.cssText = `margin-top:4px;border-top:1px solid ${hex(palette.woodSoft)};padding-top:4px`;

  card.append(clockEl, statsEl, pricesEl);

  let last: LiveVitals | null = null;

  return {
    element: card,
    get lastVitals() {
      return last;
    },
    update(v: LiveVitals): void {
      last = v;
      clockEl.textContent = formatWorldClock(v.sOfWorldDay);
      const audit = v.auditOk ? '✓' : '✗';
      const auditColor = v.auditOk ? hex(palette.plantGreen) : hex(palette.coral);
      statsEl.innerHTML =
        `Bevölkerung  <b>${v.population}</b>\n` +
        `Geld        <b>${formatMoney(v.totalMoney)}</b>\n` +
        `Audit       <b style="color:${auditColor}">${audit}</b>\n` +
        `Trips aktiv <b>${v.tripsActive}</b>`;
      pricesEl.textContent = v.prices
        .slice(0, 4)
        .map((p) => `${p.marketName || `Markt ${p.marketId}`} g${p.goodId}  ${formatMoney(p.ewmaPrice)}`)
        .join('\n');
    },
  };
}
