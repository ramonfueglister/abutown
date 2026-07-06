// Storey-peel dollhouse state (Phase A). Pure, deterministic mapping from the
// camera orbit radius to the per-storey peel of the main building: L peel
// units over the [startR, endR] radius window. Unit 0 fades the roof + eave
// out while the TOP storey's interior fades in; unit j (1 ≤ j ≤ L−1)
// dissolves the shell band of storey L−j (screen-door dissolve between
// bandLo and discardAbove, progress bandFade) while that storey's interior
// fades out and the storey below fades in — the SAME ramp drives both sides,
// so there is never a boolean pop. The EG (level 0) never fades out.
import { kswPeel } from '../../designTokens';

export type PeelCfg = {
  storeyCount: number;
  storeyH: number;
  baseY: number;
  startR: number;
  endR: number;
};

export type PeelState = {
  p: number;
  roofFade: number;
  discardAbove: number;
  bandLo: number;
  bandFade: number;
  storeyFades: number[];
};

const OFF = 1e6;
const clamp01 = (v: number): number => Math.min(1, Math.max(0, v));

export function peelState(radius: number, cfg: PeelCfg): PeelState {
  const L = cfg.storeyCount;
  const t = clamp01((cfg.startR - radius) / (cfg.startR - cfg.endR));
  const p = t * L;

  const roofFade = 1 - clamp01(p);

  // Shell dissolve band: only during units j ≥ 1. q = p clamped a hair below
  // L so floor() lands on the last unit at p = L exactly.
  let discardAbove = OFF;
  let bandLo = OFF;
  let bandFade = 0;
  if (p > 1 && L > 1) {
    const q = Math.min(p, L);
    const j = q >= L ? L - 1 : Math.floor(q);
    bandFade = q >= L ? 1 : q - j;
    bandLo = cfg.baseY + (L - j) * cfg.storeyH;
    discardAbove = cfg.baseY + (L - j + 1) * cfg.storeyH;
  }

  const storeyFades: number[] = new Array(L);
  for (let k = 0; k < L; k++) {
    const fadeIn = clamp01(p - (L - 1 - k));
    const fadeOut = k > 0 ? clamp01(p - (L - k)) : 0;
    storeyFades[k] = fadeIn - fadeOut;
  }

  return { p, roofFade, discardAbove, bandLo, bandFade, storeyFades };
}

export function closedPeel(cfg: PeelCfg): PeelState {
  return peelState(cfg.startR, cfg);
}

// Storey count + slab pitch from the baked eave height: round to the nominal
// 3.4 m pitch, clamp the count to [1, maxStoreys]; the resulting pitch is
// eaveH / count (a 1-storey shed keeps its real low eave as its pitch).
export function storeyLayout(eaveH: number): { storeyCount: number; storeyH: number } {
  let count = Math.min(kswPeel.maxStoreys, Math.max(1, Math.round(eaveH / kswPeel.nominalStoreyH)));
  // Enforce the pitch bounds by adjusting the count — best effort: when both
  // bounds are unsatisfiable (very low or very tall eaves), the [1, maxStoreys]
  // count bound wins and the pitch may fall outside [minStoreyH, maxStoreyH].
  while (count < kswPeel.maxStoreys && eaveH / count > kswPeel.maxStoreyH) count++;
  while (count > 1 && eaveH / count < kswPeel.minStoreyH) count--;
  return { storeyCount: count, storeyH: eaveH / count };
}
