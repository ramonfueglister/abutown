// src/diorama/ksw/glowUniform.ts
// ONE shared module-level uniform driving the whole city's night glow. Its own
// tiny module so every builder (staticBatch, cityMassing, lamps) and main.ts's
// boot can import it without creating a cycle. 0 = day look (no glow), 1 = full
// night glow. In Task 3 the presets still set it once at boot
// (`lampGlowU.value = preset.lampOn ? 1 : 0`); Task 4 fades it per-frame with
// the real sunset.
import { uniform } from 'three/tsl';

export const lampGlowU = uniform(0);
