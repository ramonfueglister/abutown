import { describe, expect, it, vi } from 'vitest';
import { create } from '@bufbuild/protobuf';
import { WorldTileSchema } from '../../src/proto/world_pb.js';
import { tileTreeSpecs } from '../../src/diorama/ksw/geo/worldData';

describe('tileTreeSpecs', () => {
  it('empty t_family → family undefined, kind from t_kind', () => {
    const tile = create(WorldTileSchema, {
      tX: [1, 2], tZ: [3, 4], tH: [10, 12], tR: [3, 4], tKind: [0, 1], tFamily: [],
    });
    const specs = tileTreeSpecs(tile);
    expect(specs).toHaveLength(2);
    expect(specs[0]).toEqual({ x: 1, z: 3, h: 10, r: 3, kind: 'broad', family: undefined });
    expect(specs[1]).toEqual({ x: 2, z: 4, h: 12, r: 4, kind: 'conifer', family: undefined });
  });

  it('filled t_family → correctly mapped, consistent with kind', () => {
    const tile = create(WorldTileSchema, {
      tX: [0, 1, 2, 3, 4], tZ: [0, 0, 0, 0, 0], tH: [1, 1, 1, 1, 1], tR: [1, 1, 1, 1, 1],
      tKind: [0, 0, 0, 1, 1],
      tFamily: [0, 1, 2, 3, 4], // spreading, oval, tall, conic, slender
    });
    const specs = tileTreeSpecs(tile);
    expect(specs.map((s) => s.family)).toEqual(['spreading', 'oval', 'tall', 'conic', 'slender']);
    expect(specs.map((s) => s.kind)).toEqual(['broad', 'broad', 'broad', 'conifer', 'conifer']);
  });

  it('#143 M5: unbekannter t_family-Code → family undefined, kind bleibt t_kind, gewarnt + gezählt', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const tile = create(WorldTileSchema, {
      tX: [0, 1], tZ: [0, 0], tH: [1, 1], tR: [1, 1],
      tKind: [1, 0], // conifer, broad
      tFamily: [99, 42], // beides ausserhalb FAMILY_CODES (0..4)
    });
    const specs = tileTreeSpecs(tile);
    expect(specs[0].family).toBeUndefined();
    expect(specs[1].family).toBeUndefined();
    // kind bleibt bei der t_kind-Angabe — kein stilles Umkippen auf broad durch
    // die Mismatch-Logik (unbekannte family darf kind NICHT überschreiben).
    expect(specs[0].kind).toBe('conifer');
    expect(specs[1].kind).toBe('broad');
    // Ein zusammengefasster Warn-Aufruf für die unbekannten Codes (analog kind-Mismatch).
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(warnSpy.mock.calls[0][0]).toMatch(/unknown t_family/i);
    warnSpy.mockRestore();
  });

  it('contradiction: family says conifer but t_kind says broad → kind derived from family, warned + counted', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const tile = create(WorldTileSchema, {
      tX: [0], tZ: [0], tH: [1], tR: [1],
      tKind: [0], // broad
      tFamily: [3], // conic → implies conifer
    });
    const specs = tileTreeSpecs(tile);
    expect(specs[0].family).toBe('conic');
    expect(specs[0].kind).toBe('conifer'); // derived from family, overriding t_kind
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(warnSpy.mock.calls[0][0]).toMatch(/mismatch/);
    warnSpy.mockRestore();
  });
});
