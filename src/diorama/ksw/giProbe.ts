// GI cube-probe scheduling (Slice E of the 10k-perf design). The one-bounce
// GI probe used to re-render the whole scene 6x inside a single frame every
// 240 frames — the classic "every ~4 seconds it hitches" spike. This module
// amortizes that to at most ONE cube face per frame:
//   - boot: main.ts still runs a full synchronous 6-face warm-up
//     (CubeCamera.update) before the first presented frame, so the
//     environment map is never black.
//   - 'cycle' (sun moves continuously): walk one face per frame, forever.
//   - 'static' presets: a slow background cadence — one face every
//     kswGi.staticFaceInterval frames (full cube per 6x that, the
//     pre-Slice-E refresh rate without the 6-in-one-frame hitch) — so
//     anything the dirty triggers miss (moving agents, blinkers) still
//     converges. markDirty() (the roof fade crossing the castShadow or the
//     visibility threshold, or settling after a fade) additionally schedules
//     an immediate full walk: 6 consecutive faces, one per frame.
// A walk always covers 6 consecutive faces mod 6, i.e. all of them, so the
// PMREM rebuild (scene.environment ingest) fires once per completed walk —
// `cubeComplete` on the last face — and, on the background cadence, once
// per 6 background faces. Any 6 consecutive renders cover the full cube, so
// both boundaries ingest a coherent capture.

import * as THREE from 'three/webgpu';
import { kswGi } from '../designTokens';

export type GiProbeMode = 'cycle' | 'static';

export class GiProbeScheduler {
  private face = 0;
  private pending = 0;
  private idleFrames = 0;
  private bgSinceComplete = 0;

  constructor(
    private readonly mode: GiProbeMode,
    private readonly staticFaceInterval: number = kswGi.staticFaceInterval,
  ) {}

  // Schedule a full 6-face re-walk (static mode; a no-op in cycle mode,
  // which re-renders every face continuously anyway). Re-marking mid-walk
  // restarts the countdown so the trigger state is always fully captured.
  markDirty(): void {
    if (this.mode === 'static') this.pending = 6;
  }

  // The face to render this frame, or null (idle). `cubeComplete` is true
  // when this face finishes a full cube — the caller triggers the PMREM
  // rebuild then.
  next(): { face: number; cubeComplete: boolean } | null {
    if (this.mode === 'cycle') {
      const face = this.face;
      this.face = (face + 1) % 6;
      return { face, cubeComplete: face === 5 };
    }
    if (this.pending > 0) {
      // dirty walk: 6 consecutive faces, one per frame, PMREM on the last
      const face = this.face;
      this.face = (face + 1) % 6;
      this.pending -= 1;
      this.idleFrames = 0;
      const cubeComplete = this.pending === 0;
      if (cubeComplete) this.bgSinceComplete = 0;
      return { face, cubeComplete };
    }
    // background cadence: one face every staticFaceInterval frames
    this.idleFrames += 1;
    if (this.idleFrames < this.staticFaceInterval) return null;
    this.idleFrames = 0;
    const face = this.face;
    this.face = (face + 1) % 6;
    this.bgSinceComplete += 1;
    const cubeComplete = this.bgSinceComplete >= 6;
    if (cubeComplete) this.bgSinceComplete = 0;
    return { face, cubeComplete };
  }
}

// Render a single cube face — the per-face slice of THREE.CubeCamera.update
// (three/src/cameras/CubeCamera.js): same target/face binding, same
// reversed-depth clear, same state restore. CubeCamera holds its 6 face
// cameras as children[0..5]; their orientation is set up by
// updateCoordinateSystem(), which the boot-time full update() has already
// run (guarded here anyway for robustness against renderer swaps).
export function renderProbeFace(
  renderer: THREE.WebGPURenderer,
  cubeCam: THREE.CubeCamera,
  scene: THREE.Scene,
  face: number,
): void {
  if (cubeCam.parent === null) cubeCam.updateMatrixWorld();
  if (cubeCam.coordinateSystem !== renderer.coordinateSystem) {
    cubeCam.coordinateSystem = renderer.coordinateSystem;
    cubeCam.updateCoordinateSystem();
  }
  const faceCam = cubeCam.children[face] as THREE.PerspectiveCamera;
  const prevRT = renderer.getRenderTarget();
  const prevFace = renderer.getActiveCubeFace();
  const prevMip = renderer.getActiveMipmapLevel();
  renderer.setRenderTarget(cubeCam.renderTarget as unknown as THREE.RenderTarget, face, cubeCam.activeMipmapLevel);
  if (renderer.reversedDepthBuffer && renderer.autoClear === false) renderer.clearDepth();
  renderer.render(scene, faceCam);
  renderer.setRenderTarget(prevRT, prevFace, prevMip);
}
