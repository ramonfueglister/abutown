import { describe, expect, it } from 'vitest';
import { createCameraState } from '../../src/cameraController';
import { createMobilityOverlayState } from '../../src/backend/mobilityState';
import {
  renderMinimalMap,
  type MinimalMapRendererState,
} from '../../src/render/minimalMapRenderer';
import { MINIMAL_MAP_TILE_SIZE } from '../../src/render/minimalMapProjection';

type FillRectOperation = {
  type: 'fillRect';
  fillStyle: string;
  globalAlpha: number;
  x: number;
  y: number;
  width: number;
  height: number;
};

type RenderOperation =
  | FillRectOperation
  | { type: 'beginPath' }
  | { type: 'clearRect'; x: number; y: number; width: number; height: number }
  | { type: 'fill'; fillStyle: string; globalAlpha: number }
  | { type: 'setTransform'; a: number; b: number; c: number; d: number; e: number; f: number }
  | { type: 'rect'; x: number; y: number; width: number; height: number }
  | { type: 'translate'; x: number; y: number }
  | { type: 'scale'; x: number; y: number }
  | { type: 'save' }
  | { type: 'restore' }
  | { type: 'fillText'; text: string; x: number; y: number };

function createContext(): CanvasRenderingContext2D & { operations: RenderOperation[] } {
  const operations: RenderOperation[] = [];
  let fillStyle = '';
  let globalAlpha = 1;
  const alphaStack: number[] = [];
  const context = {
    operations,
    imageSmoothingEnabled: true,
    font: '',
    textBaseline: 'alphabetic' as CanvasTextBaseline,
    beginPath: () => operations.push({ type: 'beginPath' }),
    clearRect: (x: number, y: number, width: number, height: number) =>
      operations.push({ type: 'clearRect', x, y, width, height }),
    fill: () => operations.push({ type: 'fill', fillStyle, globalAlpha }),
    fillRect: (x: number, y: number, width: number, height: number) =>
      operations.push({ type: 'fillRect', fillStyle, globalAlpha, x, y, width, height }),
    fillText: (text: string, x: number, y: number) => operations.push({ type: 'fillText', text, x, y }),
    rect: (x: number, y: number, width: number, height: number) =>
      operations.push({ type: 'rect', x, y, width, height }),
    restore: () => {
      globalAlpha = alphaStack.pop() ?? 1;
      operations.push({ type: 'restore' });
    },
    save: () => {
      alphaStack.push(globalAlpha);
      operations.push({ type: 'save' });
    },
    scale: (x: number, y: number) => operations.push({ type: 'scale', x, y }),
    setTransform: (a: number, b: number, c: number, d: number, e: number, f: number) =>
      operations.push({ type: 'setTransform', a, b, c, d, e, f }),
    translate: (x: number, y: number) => operations.push({ type: 'translate', x, y }),
    set fillStyle(value: string | CanvasGradient | CanvasPattern) {
      fillStyle = String(value);
    },
    get fillStyle() {
      return fillStyle;
    },
    set globalAlpha(value: number) {
      globalAlpha = value;
    },
    get globalAlpha() {
      return globalAlpha;
    },
  };
  return context as unknown as CanvasRenderingContext2D & { operations: RenderOperation[] };
}

function createState(
  ctx: CanvasRenderingContext2D,
  overrides: Partial<MinimalMapRendererState> = {},
): MinimalMapRendererState {
  return {
    ctx,
    viewport: { width: 180, height: 120, devicePixelRatio: 2 },
    camera: createCameraState({ x: 0, y: 0, scale: 0.18 }),
    world: { width: 4, height: 3 },
    tileSize: MINIMAL_MAP_TILE_SIZE,
    terrain: new Map(),
    terrainKinds: new Map(),
    roads: new Map(),
    rails: new Map(),
    railPaths: [],
    railStations: [],
    buildings: [],
    trees: [],
    details: [],
    mobilityState: createMobilityOverlayState(),
    mobilityTickPeriodMs: 100,
    vehicleSprites: [],
    pedestrianSprites: [],
    selectedAgentId: null,
    selectedVehicleId: null,
    now: () => 0,
    simTime: 0,
    ...overrides,
  };
}

function fillOperations(ctx: { operations: RenderOperation[] }, fillStyle: string): Array<{ type: 'fill'; fillStyle: string; globalAlpha: number }> {
  return ctx.operations.filter(
    (operation): operation is { type: 'fill'; fillStyle: string; globalAlpha: number } =>
      operation.type === 'fill' && operation.fillStyle === fillStyle,
  );
}

function rectOperations(ctx: { operations: RenderOperation[] }): Array<{ type: 'rect'; x: number; y: number; width: number; height: number }> {
  return ctx.operations.filter(
    (operation): operation is { type: 'rect'; x: number; y: number; width: number; height: number } =>
      operation.type === 'rect',
  );
}

function translateOperations(ctx: { operations: RenderOperation[] }): Array<{ type: 'translate'; x: number; y: number }> {
  return ctx.operations.filter(
    (operation): operation is { type: 'translate'; x: number; y: number } =>
      operation.type === 'translate',
  );
}

describe('minimal map renderer', () => {
  it('does not add a scene-origin offset after the camera transform', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx));

    expect(translateOperations(ctx)).toEqual([
      { type: 'translate', x: 0, y: 0 },
    ]);
  });

  it('draws grass as one continuous base layer to avoid zoomed-out tile seams', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx));

    const grassFills = ctx.operations.filter(
      (operation): operation is FillRectOperation =>
        operation.type === 'fillRect' && operation.fillStyle === '#91c86f',
    );
    expect(grassFills).toEqual([
      {
        type: 'fillRect',
        fillStyle: '#91c86f',
        globalAlpha: 1,
        x: -0.6,
        y: -0.6,
        width: 4 * MINIMAL_MAP_TILE_SIZE.width + 1.2,
        height: 3 * MINIMAL_MAP_TILE_SIZE.height + 1.2,
      },
    ]);
  });

  it('batches forest terrain overlays into one material path', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx, {
      terrainKinds: new Map([
        ['0:0', { kind: 'forest' }],
        ['1:0', { kind: 'forest' }],
        ['0:1', { kind: 'forest' }],
      ]),
    }));

    const parkFills = fillOperations(ctx, '#cfe5bf');
    expect(parkFills).toHaveLength(1);
    expect(parkFills[0].globalAlpha).toBeCloseTo(0.82);
    expect(rectOperations(ctx)).toEqual([
      { type: 'rect', x: -0.6, y: -0.6, width: 19.2, height: 19.2 },
      { type: 'rect', x: 17.4, y: -0.6, width: 19.2, height: 19.2 },
      { type: 'rect', x: -0.6, y: 17.4, width: 19.2, height: 19.2 },
    ]);
  });

  it('batches water overlays by visual material', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx, {
      terrain: new Map([
        ['2:0', 'water'],
        ['3:0', 'water'],
        ['2:1', 'riverbank'],
      ]),
      terrainKinds: new Map([
        ['2:0', { kind: 'water' }],
        ['3:0', { kind: 'water' }],
        ['2:1', { kind: 'riverbank' }],
      ]),
    }));

    const waterFills = fillOperations(ctx, '#92d8e9');
    const riverbankFills = fillOperations(ctx, '#bde8df');
    expect(waterFills).toHaveLength(1);
    expect(waterFills[0].globalAlpha).toBeCloseTo(0.96);
    expect(riverbankFills).toHaveLength(1);
    expect(riverbankFills[0].globalAlpha).toBeCloseTo(0.96);
    expect(rectOperations(ctx)).toEqual([
      { type: 'rect', x: 35.4, y: -0.6, width: 19.2, height: 19.2 },
      { type: 'rect', x: 53.4, y: -0.6, width: 19.2, height: 19.2 },
      { type: 'rect', x: 35.4, y: 17.4, width: 19.2, height: 19.2 },
    ]);
  });
});
