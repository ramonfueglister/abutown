// Dollhouse-cutaway state (T18, S3c). Pure, deterministic mapping from the
// camera orbit radius to the two uniforms that drive the main-building cutaway
// material: `cutH` (the world-y slice height — fragments above it are discarded)
// and `upperFade` (opacity of the upper storeys + roof, 1 = closed, 0 = open).
//
// Ablauf (plan §Task 18): zooming IN past fadeStartR fades the upper storeys +
// roof out first (upperFade 1→0 across [fadeStartR, fadeEndR]); only once that
// fade is nearly complete (< 0.15) does the hard height-slice engage
// (cutH 1e6 → cutHeight), so the building opens like a dollhouse rather than
// popping. Zooming out reverses it. No slice while the upper mass is still
// visible avoids a hard edge cutting through a still-opaque wall.
import { kswS3 } from '../../designTokens';

export type CutawayState = { cutH: number; upperFade: number };

// upperFade < this → the height slice is engaged (else cutH stays "off" = 1e6).
const SLICE_ON_BELOW = 0.15;
const OFF = 1e6;

export function cutawayState(radius: number): CutawayState {
  const { fadeStartR, fadeEndR, cutHeight } = kswS3;
  // linear ramp: fade = 1 at/above fadeStartR, 0 at/below fadeEndR
  const t = (radius - fadeEndR) / (fadeStartR - fadeEndR);
  const upperFade = Math.min(1, Math.max(0, t));
  const cutH = upperFade < SLICE_ON_BELOW ? cutHeight : OFF;
  return { cutH, upperFade };
}
