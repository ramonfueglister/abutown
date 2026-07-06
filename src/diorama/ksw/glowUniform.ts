// src/diorama/ksw/glowUniform.ts
// ONE shared module-level uniform driving the whole city's night glow. Its own
// tiny module so every builder (staticBatch, cityMassing, lamps) and main.ts's
// boot can import it without creating a cycle. 0 = day look (no glow), 1 = full
// night glow. In Task 3 the presets still set it once at boot
// (`lampGlowU.value = preset.lampOn ? 1 : 0`); Task 4 fades it per-frame with
// the real sunset.
import { uniform } from 'three/tsl';

export const lampGlowU = uniform(0);

// Snow cover 0..1 (SOTA weather pass 2026-07-07): applyCityEnvironment drives
// it from the live/pinned precip state; terrain, plate greens and roofs mix
// toward a snow tone so a snowing city actually reads WINTER instead of a
// green summer scene under white particles.
export const snowU = uniform(0);
