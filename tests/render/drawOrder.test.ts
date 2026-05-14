import { describe, expect, it } from 'vitest';
import { drawPassForType } from '../../src/render/drawOrder';

describe('scene draw order', () => {
  it('always draws road infrastructure before moving vehicles', () => {
    expect(drawPassForType('rail')).toBeLessThan(drawPassForType('car'));
    expect(drawPassForType('road')).toBeLessThan(drawPassForType('car'));
  });

  it('keeps tall objects after moving vehicles so they can occlude naturally', () => {
    expect(drawPassForType('car')).toBeLessThan(drawPassForType('building'));
    expect(drawPassForType('car')).toBeLessThan(drawPassForType('tree'));
    expect(drawPassForType('car')).toBeLessThan(drawPassForType('railStation'));
  });
});
