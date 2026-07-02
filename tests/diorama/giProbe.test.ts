// Slice E/F: the GI probe scheduler policy — at most one cube face per frame.
// Boot warm-up (full 6-face update) happens outside the scheduler; these
// tests pin the steady-state policy: static presets render one face every
// staticFaceInterval frames in the background, a dirty mark walks exactly 6
// consecutive faces immediately (all of them, in order, PMREM rebuild once
// at the end) and then returns to the background cadence, cycle mode walks
// forever.

import { describe, expect, it } from 'vitest';
import { GiProbeScheduler } from '../../src/diorama/ksw/giProbe';
import { kswGi } from '../../src/diorama/designTokens';

function drain(s: GiProbeScheduler, frames: number) {
  const out: Array<{ face: number; cubeComplete: boolean } | null> = [];
  for (let i = 0; i < frames; i++) out.push(s.next());
  return out;
}

describe('GiProbeScheduler', () => {
  it('static mode idles between background faces: one face every interval frames', () => {
    const s = new GiProbeScheduler('static', 4);
    const out = drain(s, 12);
    // frames 1-3 idle, frame 4 renders face 0; frames 5-7 idle, frame 8 face 1; ...
    expect(out.map((r) => r?.face ?? null)).toEqual([null, null, null, 0, null, null, null, 1, null, null, null, 2]);
  });

  it('defaults the background interval to kswGi.staticFaceInterval', () => {
    const s = new GiProbeScheduler('static');
    const out = drain(s, kswGi.staticFaceInterval);
    expect(out.slice(0, -1)).toEqual(Array(kswGi.staticFaceInterval - 1).fill(null));
    expect(out[out.length - 1]?.face).toBe(0);
  });

  it('background cadence completes the cube every 6th rendered face', () => {
    const s = new GiProbeScheduler('static', 2);
    const rendered = drain(s, 26).filter((r) => r !== null);
    expect(rendered.map((r) => r!.face)).toEqual([0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, 0]);
    expect(rendered.map((r) => r!.cubeComplete)).toEqual([
      false, false, false, false, false, true,
      false, false, false, false, false, true,
      false,
    ]);
  });

  it('a dirty mark walks exactly 6 faces (one per frame), completes the cube on the last, then returns to the background cadence', () => {
    const s = new GiProbeScheduler('static', 10);
    s.markDirty();
    const walk = drain(s, 6);
    expect(walk.map((r) => r?.face)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(walk.map((r) => r?.cubeComplete)).toEqual([false, false, false, false, false, true]);
    // background cadence resumes: 9 idle frames, then the next face
    const after = drain(s, 10);
    expect(after.slice(0, 9)).toEqual(Array(9).fill(null));
    expect(after[9]?.face).toBe(0);
    expect(after[9]?.cubeComplete).toBe(false); // 1 of 6 background faces
  });

  it('a second dirty mark re-walks all 6 faces continuing from the current face', () => {
    const s = new GiProbeScheduler('static', 100);
    s.markDirty();
    drain(s, 6);
    s.markDirty();
    const walk = drain(s, 6);
    expect(walk.map((r) => r?.face)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(walk[5]?.cubeComplete).toBe(true);
    expect(s.next()).toBeNull();
  });

  it('a dirty mark mid-walk restarts the 6-face countdown (still covers all faces)', () => {
    const s = new GiProbeScheduler('static', 100);
    s.markDirty();
    drain(s, 2); // faces 0, 1 rendered
    s.markDirty(); // e.g. the fade crossed the second threshold during the walk
    const walk = drain(s, 6);
    // 6 consecutive faces mod 6 = every face exactly once
    expect([...walk.map((r) => r?.face)].sort()).toEqual([0, 1, 2, 3, 4, 5]);
    expect(walk.map((r) => r?.cubeComplete)).toEqual([false, false, false, false, false, true]);
    expect(s.next()).toBeNull();
  });

  it('a dirty mark interrupting the background cadence renders immediately and resets the cube boundary', () => {
    const s = new GiProbeScheduler('static', 3);
    drain(s, 6); // background faces 0, 1 rendered (frames 3 and 6)
    s.markDirty();
    const walk = drain(s, 6); // immediate, no idle gap
    expect(walk.map((r) => r?.face)).toEqual([2, 3, 4, 5, 0, 1]);
    expect(walk.map((r) => r?.cubeComplete)).toEqual([false, false, false, false, false, true]);
  });

  it('cycle mode walks one face per frame forever, completing the cube every 6th frame', () => {
    const s = new GiProbeScheduler('cycle');
    const walk = drain(s, 13);
    expect(walk.map((r) => r?.face)).toEqual([0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, 0]);
    expect(walk.filter((r) => r?.cubeComplete).length).toBe(2);
    expect(walk[5]?.cubeComplete).toBe(true);
    expect(walk[11]?.cubeComplete).toBe(true);
  });

  it('cycle mode ignores dirty marks (it re-renders everything continuously anyway)', () => {
    const s = new GiProbeScheduler('cycle');
    drain(s, 2);
    s.markDirty();
    expect(s.next()?.face).toBe(2); // sequence uninterrupted
  });
});
