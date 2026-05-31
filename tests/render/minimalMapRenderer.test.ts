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
  x: number;
  y: number;
  width: number;
  height: number;
};

type RenderOperation =
  | FillRectOperation
  | { type: 'clearRect'; x: number; y: number; width: number; height: number }
  | { type: 'setTransform'; a: number; b: number; c: number; d: number; e: number; f: number }
  | { type: 'translate'; x: number; y: number }
  | { type: 'scale'; x: number; y: number }
  | { type: 'save' }
  | { type: 'restore' }
  | { type: 'fillText'; text: string; x: number; y: number };

function createContext(): CanvasRenderingContext2D & { operations: RenderOperation[] } {
  const operations: RenderOperation[] = [];
  let fillStyle = '';
  const context = {
    operations,
    imageSmoothingEnabled: true,
    font: '',
    textBaseline: 'alphabetic' as CanvasTextBaseline,
    clearRect: (x: number, y: number, width: number, height: number) =>
      operations.push({ type: 'clearRect', x, y, width, height }),
    fillRect: (x: number, y: number, width: number, height: number) =>
      operations.push({ type: 'fillRect', fillStyle, x, y, width, height }),
    fillText: (text: string, x: number, y: number) => operations.push({ type: 'fillText', text, x, y }),
    restore: () => operations.push({ type: 'restore' }),
    save: () => operations.push({ type: 'save' }),
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
  };
  return context as unknown as CanvasRenderingContext2D & { operations: RenderOperation[] };
}

function createState(ctx: CanvasRenderingContext2D): MinimalMapRendererState {
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
  };
}

describe('minimal map renderer', () => {
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
        x: -0.6,
        y: -0.6,
        width: 4 * MINIMAL_MAP_TILE_SIZE.width + 1.2,
        height: 3 * MINIMAL_MAP_TILE_SIZE.height + 1.2,
      },
    ]);
  });
});
