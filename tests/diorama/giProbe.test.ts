// Slice E: the GI probe scheduler policy — at most one cube face per frame.
// Boot warm-up (full 6-face update) happens outside the scheduler; these
// tests pin the steady-state policy: static presets idle at zero probe
// renders, a dirty mark walks exactly 6 consecutive faces (all of them, in
// order, PMREM rebuild once at the end), cycle mode walks forever.

import { describe, expect, it } from 'vitest';
import { GiProbeScheduler } from '../../src/diorama/ksw/giProbe';

function drain(s: GiProbeScheduler, frames: number) {
  const out: Array<{ face: number; cubeComplete: boolean } | null> = [];
  for (let i = 0; i < frames; i++) out.push(s.next());
  return out;
}

describe('GiProbeScheduler', () => {
  it('static mode renders no probe faces while idle', () => {
    const s = new GiProbeScheduler('static');
    expect(drain(s, 10)).toEqual(Array(10).fill(null));
  });

  it('a dirty mark walks exactly 6 faces (one per frame), completes the cube on the last, then idles', () => {
    const s = new GiProbeScheduler('static');
    s.markDirty();
    const walk = drain(s, 6);
    expect(walk.map((r) => r?.face)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(walk.map((r) => r?.cubeComplete)).toEqual([false, false, false, false, false, true]);
    expect(drain(s, 20)).toEqual(Array(20).fill(null));
  });

  it('a second dirty mark re-walks all 6 faces continuing from the current face', () => {
    const s = new GiProbeScheduler('static');
    s.markDirty();
    drain(s, 6);
    s.markDirty();
    const walk = drain(s, 6);
    expect(walk.map((r) => r?.face)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(walk[5]?.cubeComplete).toBe(true);
    expect(s.next()).toBeNull();
  });

  it('a dirty mark mid-walk restarts the 6-face countdown (still covers all faces)', () => {
    const s = new GiProbeScheduler('static');
    s.markDirty();
    drain(s, 2); // faces 0, 1 rendered
    s.markDirty(); // e.g. the fade crossed the second threshold during the walk
    const walk = drain(s, 6);
    // 6 consecutive faces mod 6 = every face exactly once
    expect([...walk.map((r) => r?.face)].sort()).toEqual([0, 1, 2, 3, 4, 5]);
    expect(walk.map((r) => r?.cubeComplete)).toEqual([false, false, false, false, false, true]);
    expect(s.next()).toBeNull();
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
