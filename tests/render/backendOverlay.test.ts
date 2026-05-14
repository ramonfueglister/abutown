import { describe, expect, it } from 'vitest';
import { activeBackendPulses, localIndexToWorldCoord } from '../../src/render/backendOverlay';

describe('backend overlay helpers', () => {
  it('maps chunk-local tile indices to world coordinates', () => {
    expect(localIndexToWorldCoord({ x: 0, y: 0 }, 32, 0)).toEqual({ x: 0, y: 0 });
    expect(localIndexToWorldCoord({ x: 0, y: 0 }, 32, 33)).toEqual({ x: 1, y: 1 });
    expect(localIndexToWorldCoord({ x: 2, y: 1 }, 32, 65)).toEqual({ x: 65, y: 34 });
  });

  it('keeps only visible pulse effects inside their lifetime', () => {
    const pulses = [
      { coord: { x: 0, y: 0 }, localIndex: 1, tick: 1, version: 1, receivedAtMs: 100 },
      { coord: { x: 0, y: 0 }, localIndex: 2, tick: 2, version: 2, receivedAtMs: 1300 },
    ];

    expect(activeBackendPulses(pulses, 1500).map((pulse) => pulse.localIndex)).toEqual([2]);
  });
});
