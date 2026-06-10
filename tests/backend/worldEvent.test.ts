import { describe, expect, it } from 'vitest';
import { create } from '@bufbuild/protobuf';
import { TileKindSetEventSchema, TileKind } from '../../src/backend/proto/abutown_pb';
import {
  tileKindSetEventFromProto,
  tileKindToTerrainString,
  type TileKindSetEventDto,
} from '../../src/backend/mobilityProtocol';

describe('tileKindSetEventFromProto', () => {
  it('converts chunk coord + local index to absolute tile coords and kind string', () => {
    // chunkSize=32, localIndex=33 → local_y=1, local_x=1 (row-major: 1*32+1=33)
    const event = create(TileKindSetEventSchema, {
      protocolVersion: 1,
      eventId: 'e1',
      commandId: 'c1',
      worldId: 'abutopia',
      tick: 42n,
      version: 7n,
      coord: { x: 2, y: 1 },
      localIndex: 33, // chunkSize 32, row-major: local_y=1, local_x=1
      kind: TileKind.WATER,
    });
    const dto = tileKindSetEventFromProto(event, 32)!;
    // absolute x = chunk_x * chunk_size + local_x = 2*32 + 1 = 65
    // absolute y = chunk_y * chunk_size + local_y = 1*32 + 1 = 33
    expect(dto).toEqual({ x: 65, y: 33, kind: 'water', tick: 42 });
  });

  it('handles localIndex=0 (top-left of chunk)', () => {
    const event = create(TileKindSetEventSchema, {
      protocolVersion: 1,
      eventId: 'e2',
      commandId: 'c2',
      worldId: 'abutopia',
      tick: 10n,
      version: 1n,
      coord: { x: 0, y: 0 },
      localIndex: 0,
      kind: TileKind.GRASS,
    });
    const dto = tileKindSetEventFromProto(event, 32);
    expect(dto).toEqual({ x: 0, y: 0, kind: 'grass', tick: 10 });
  });

  it('handles last tile in chunk (bottom-right)', () => {
    // chunkSize=32, localIndex=32*32-1=1023 → local_y=31, local_x=31
    const event = create(TileKindSetEventSchema, {
      protocolVersion: 1,
      eventId: 'e3',
      commandId: 'c3',
      worldId: 'abutopia',
      tick: 5n,
      version: 2n,
      coord: { x: 3, y: 2 },
      localIndex: 1023,
      kind: TileKind.GRASS,
    });
    const dto = tileKindSetEventFromProto(event, 32);
    // absolute x = 3*32 + 31 = 127, y = 2*32 + 31 = 95
    expect(dto).toEqual({ x: 127, y: 95, kind: 'grass', tick: 5 });
  });
});

describe('tileKindToTerrainString', () => {
  it('maps GRASS → "grass"', () => {
    expect(tileKindToTerrainString(TileKind.GRASS)).toBe('grass');
  });

  it('maps WATER → "water"', () => {
    expect(tileKindToTerrainString(TileKind.WATER)).toBe('water');
  });

  it('maps ROAD → null (roads render from the transport.roads map, not the terrain layer)', () => {
    expect(tileKindToTerrainString(TileKind.ROAD)).toBeNull();
  });

  it('maps BUILDING_FOOTPRINT → null (building footprints render from the buildings layer)', () => {
    expect(tileKindToTerrainString(TileKind.BUILDING_FOOTPRINT)).toBeNull();
  });

  it('maps UNSPECIFIED → null (backend rejects UNSPECIFIED commands; null is the honest representation)', () => {
    expect(tileKindToTerrainString(TileKind.UNSPECIFIED)).toBeNull();
  });
});

describe('tileKindSetEventFromProto — unmappable kinds', () => {
  it('returns null for ROAD (unrenderable in terrain layer)', () => {
    const event = create(TileKindSetEventSchema, {
      protocolVersion: 1,
      eventId: 'e-road',
      commandId: 'c-road',
      worldId: 'abutopia',
      tick: 1n,
      version: 1n,
      coord: { x: 0, y: 0 },
      localIndex: 0,
      kind: TileKind.ROAD,
    });
    expect(tileKindSetEventFromProto(event, 32)).toBeNull();
  });

  it('returns null for BUILDING_FOOTPRINT (unrenderable in terrain layer)', () => {
    const event = create(TileKindSetEventSchema, {
      protocolVersion: 1,
      eventId: 'e-bf',
      commandId: 'c-bf',
      worldId: 'abutopia',
      tick: 1n,
      version: 1n,
      coord: { x: 0, y: 0 },
      localIndex: 0,
      kind: TileKind.BUILDING_FOOTPRINT,
    });
    expect(tileKindSetEventFromProto(event, 32)).toBeNull();
  });

  it('returns null for UNSPECIFIED', () => {
    const event = create(TileKindSetEventSchema, {
      protocolVersion: 1,
      eventId: 'e-unspec',
      commandId: 'c-unspec',
      worldId: 'abutopia',
      tick: 1n,
      version: 1n,
      coord: { x: 0, y: 0 },
      localIndex: 0,
      kind: TileKind.UNSPECIFIED,
    });
    expect(tileKindSetEventFromProto(event, 32)).toBeNull();
  });
});
