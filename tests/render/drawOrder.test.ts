import { describe, expect, it } from 'vitest';
import { compareDrawableOrder } from '../../src/render/drawOrder';

describe('draw order', () => {
  it('always renders roads before pedestrians even when the road is lower on screen', () => {
    expect(compareDrawableOrder(
      { type: 'pedestrian', isoY: 10, x: 0 },
      { type: 'road', isoY: 100, x: 0 },
    )).toBeGreaterThan(0);
  });

  it('always renders rails before trains so the train sits on top of the track', () => {
    expect(compareDrawableOrder(
      { type: 'train', isoY: 10, x: 0 },
      { type: 'rail', isoY: 100, x: 0 },
    )).toBeGreaterThan(0);
  });

  it('renders rail above road on the same crossing tile', () => {
    expect(compareDrawableOrder(
      { type: 'rail', isoY: 100, x: 20 },
      { type: 'road', isoY: 100, x: 20 },
    )).toBeGreaterThan(0);
  });

  it('renders a lower street tile above a building behind it', () => {
    expect(compareDrawableOrder(
      { type: 'road', isoY: 120, x: 20 },
      { type: 'building', isoY: 100, x: 20 },
    )).toBeGreaterThan(0);
  });

  it('keeps foreground buildings above street tiles behind them', () => {
    expect(compareDrawableOrder(
      { type: 'building', isoY: 120, x: 20 },
      { type: 'road', isoY: 100, x: 20 },
    )).toBeGreaterThan(0);
  });

  it('lets foreground buildings hide pedestrians on roads behind them', () => {
    expect(compareDrawableOrder(
      { type: 'pedestrian', isoY: 10, x: 0 },
      { type: 'building', isoY: 100, x: 0 },
    )).toBeLessThan(0);
  });

  it('renders pedestrians in front of buildings when the pedestrian is lower on screen', () => {
    expect(compareDrawableOrder(
      { type: 'pedestrian', isoY: 120, x: 0 },
      { type: 'building', isoY: 100, x: 0 },
    )).toBeGreaterThan(0);
  });
});
