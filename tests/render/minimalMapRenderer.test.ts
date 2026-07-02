import { describe, expect, it } from 'vitest';
import { createCameraState } from '../../src/cameraController';
import { applyMobilitySnapshot, createMobilityOverlayState } from '../../src/backend/mobilityState';
import {
  flowsDrawnLastFrame,
  marketGuideEdgesDrawnLastFrame,
  renderMinimalMap,
  type MinimalMapRendererState,
} from '../../src/render/minimalMapRenderer';
import { MINIMAL_MAP_TILE_SIZE } from '../../src/render/minimalMapProjection';
import { AGENT_INK, GROUND, TRADER_RED } from '../../src/render/designTokens';

const RETIRED_LANDMARK_FILL = '#d8c277';
const RETIRED_CITY_PLACE_FILLS = ['#dfe6cf', '#e7dbc6', '#d4e4e7', '#e2d9ea'] as const;
const RETIRED_CITY_PLACE_FILL_SET = new Set<string>(RETIRED_CITY_PLACE_FILLS);

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
  | { type: 'closePath' }
  | { type: 'fill'; fillStyle: string; globalAlpha: number }
  | { type: 'fillText'; text: string; x: number; y: number }
  | { type: 'lineTo'; x: number; y: number }
  | { type: 'moveTo'; x: number; y: number }
  | { type: 'quadraticCurveTo'; cpx: number; cpy: number; x: number; y: number }
  | { type: 'rect'; x: number; y: number; width: number; height: number }
  | { type: 'restore' }
  | { type: 'save' }
  | { type: 'scale'; x: number; y: number }
  | { type: 'setTransform'; a: number; b: number; c: number; d: number; e: number; f: number }
  | { type: 'stroke'; strokeStyle: string; lineWidth: number; globalAlpha: number }
  | { type: 'translate'; x: number; y: number };

function createContext(): CanvasRenderingContext2D & { operations: RenderOperation[] } {
  const operations: RenderOperation[] = [];
  let fillStyle = '';
  let strokeStyle = '';
  let lineWidth = 1;
  let globalAlpha = 1;
  const alphaStack: number[] = [];
  const context = {
    operations,
    imageSmoothingEnabled: true,
    font: '',
    textBaseline: 'alphabetic' as CanvasTextBaseline,
    arc: () => undefined,
    beginPath: () => operations.push({ type: 'beginPath' }),
    clearRect: (x: number, y: number, width: number, height: number) =>
      operations.push({ type: 'clearRect', x, y, width, height }),
    closePath: () => operations.push({ type: 'closePath' }),
    fill: () => operations.push({ type: 'fill', fillStyle, globalAlpha }),
    fillRect: (x: number, y: number, width: number, height: number) =>
      operations.push({ type: 'fillRect', fillStyle, globalAlpha, x, y, width, height }),
    fillText: (text: string, x: number, y: number) => operations.push({ type: 'fillText', text, x, y }),
    lineTo: (x: number, y: number) => operations.push({ type: 'lineTo', x, y }),
    measureText: (text: string) => ({ width: text.length * 6 }),
    moveTo: (x: number, y: number) => operations.push({ type: 'moveTo', x, y }),
    quadraticCurveTo: (cpx: number, cpy: number, x: number, y: number) =>
      operations.push({ type: 'quadraticCurveTo', cpx, cpy, x, y }),
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
    stroke: () => operations.push({ type: 'stroke', strokeStyle, lineWidth, globalAlpha }),
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
    set lineCap(_value: CanvasLineCap) {},
    set lineJoin(_value: CanvasLineJoin) {},
    set lineWidth(value: number) {
      lineWidth = value;
    },
    get lineWidth() {
      return lineWidth;
    },
    set strokeStyle(value: string | CanvasGradient | CanvasPattern) {
      strokeStyle = String(value);
    },
    get strokeStyle() {
      return strokeStyle;
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

function fillRectOperations(ctx: { operations: RenderOperation[] }, fillStyle: string): FillRectOperation[] {
  return ctx.operations.filter(
    (operation): operation is FillRectOperation =>
      operation.type === 'fillRect' && operation.fillStyle === fillStyle,
  );
}

function translateOperations(ctx: { operations: RenderOperation[] }): Array<{ type: 'translate'; x: number; y: number }> {
  return ctx.operations.filter(
    (operation): operation is { type: 'translate'; x: number; y: number } =>
      operation.type === 'translate',
  );
}

function mobilityStateWithAgents(agentCount: number) {
  return mobilityStateWithAgentSpriteKeys(Array.from({ length: agentCount }, () => '0'));
}

function mobilityStateWithAgentSpriteKeys(spriteKeys: readonly string[]) {
  return applyMobilitySnapshot(createMobilityOverlayState(), {
    protocol_version: 1,
    world_id: 'test',
    tick: 1,
    agents: spriteKeys.map((spriteKey, index) => ({
      id: `agent:${index}`,
      state: { type: 'walking' as const, link_id: 'link:1', progress: 0.5 },
      plan_cursor: 0,
      world_coord: { x: index + 1, y: 1 },
      direction: 'e' as const,
      sprite_key: spriteKey,
      age_seconds: 0,
    })),
    vehicles: [],
    stops: [],
  }, 0);
}

describe('minimal map renderer', () => {
  it('does not add a scene-origin offset after the camera transform', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx));

    expect(translateOperations(ctx)).toEqual([{ type: 'translate', x: 0, y: 0 }]);
  });

  it('draws a single paper world layer', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx));

    expect(fillRectOperations(ctx, GROUND)).toEqual([
      {
        type: 'fillRect',
        fillStyle: GROUND,
        globalAlpha: 1,
        x: -0.6,
        y: -0.6,
        width: 4 * MINIMAL_MAP_TILE_SIZE.width + 1.2,
        height: 3 * MINIMAL_MAP_TILE_SIZE.height + 1.2,
      },
    ]);
  });

  it('does not draw legacy terrain or road material fills', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx, {
      terrain: new Map([
        ['1:0', 'water'],
        ['2:0', 'riverbank'],
      ]),
      terrainKinds: new Map([
        ['0:0', { kind: 'forest' }],
        ['1:0', { kind: 'water' }],
        ['2:0', { kind: 'riverbank' }],
      ]),
      roads: new Map([
        ['0:1', { coord: { x: 0, y: 1 }, kind: 'street', mask: 10 }],
      ]),
    }));

    expect(ctx.operations.filter((operation) => operation.type === 'fill')).toHaveLength(0);
    expect(ctx.operations.filter((operation) => operation.type === 'stroke')).toHaveLength(0);
    expect(fillRectOperations(ctx, GROUND)).toHaveLength(1);
  });

  it('keeps backend economy flows unrendered while keeping guide diagnostics hidden', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx, {
      camera: createCameraState({ x: 0, y: 0, scale: 0.22 }),
      markets: [
        { marketId: 9001, name: 'A', tileX: 1, tileY: 1, wagePaidLastTick: 0 },
        { marketId: 9002, name: 'B', tileX: 2, tileY: 1, wagePaidLastTick: 0 },
      ],
      goods: [
        {
          marketId: 9001,
          goodId: 1,
          tradedQtyLastTick: 8,
          unmetDemandLastTick: 2,
          unsoldSupplyLastTick: 0,
          lastSettlementPrice: 2,
          ewmaReferencePrice: 1,
        },
      ],
      flows: [
        { srcMarketId: 9001, dstMarketId: 9002, goodId: 1, rate: 12 },
      ],
    }));

    expect(flowsDrawnLastFrame()).toBe(0);
    expect(marketGuideEdgesDrawnLastFrame()).toBe(0);
  });

  it('does not draw station occupancy specks or building landmark boxes in the default map', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx, {
      markets: [
        { marketId: 9001, name: 'Harbor Depot', tileX: 1, tileY: 1, wagePaidLastTick: 0 },
      ],
      buildings: [
        { coord: { x: 1, y: 1 }, sheet: 'houses', frame: 0, district: 'harbor' },
      ],
      mobilityState: mobilityStateWithAgents(120),
      pedestrianSprites: [{ sheet: 'minimal-pedestrian', variantIndex: 0, kind: 'pedestrian', scale: 1 }],
    }));

    expect(ctx.operations.filter((operation) => operation.type === 'fill' && operation.fillStyle === RETIRED_LANDMARK_FILL))
      .toHaveLength(0);
  });

  it('draws real backend mobility agents in the default map', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx, {
      mobilityState: mobilityStateWithAgents(3),
      pedestrianSprites: [{ sheet: 'minimal-pedestrian', variantIndex: 0, kind: 'pedestrian', scale: 1 }],
    }));

    expect(ctx.operations.filter((operation) => operation.type === 'fill' && operation.fillStyle === AGENT_INK))
      .toHaveLength(3);
  });

  it('hides backend flow-shipment traders from the default agent layer', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx, {
      mobilityState: mobilityStateWithAgentSpriteKeys(['0', 'trader:2']),
      pedestrianSprites: [{ sheet: 'minimal-pedestrian', variantIndex: 0, kind: 'pedestrian', scale: 1 }],
    }));

    expect(ctx.operations.filter((operation) => operation.type === 'fill' && operation.fillStyle === AGENT_INK))
      .toHaveLength(1);
    expect(ctx.operations.filter((operation) => operation.type === 'fill' && operation.fillStyle === TRADER_RED))
      .toHaveLength(0);
  });

  it('does not draw colored place fields around stations in the default map', () => {
    const ctx = createContext();

    renderMinimalMap(createState(ctx, {
      markets: [
        { marketId: 9001, name: 'Harbor Depot', tileX: 1, tileY: 1, wagePaidLastTick: 0 },
        { marketId: 9002, name: 'Central Works', tileX: 2, tileY: 1, wagePaidLastTick: 0 },
      ],
      buildings: [
        { coord: { x: 1, y: 1 }, sheet: 'houses', frame: 0, district: 'Harbor Depot' },
        { coord: { x: 2, y: 1 }, sheet: 'office', frame: 0, district: 'Central Works' },
      ],
    }));

    expect(ctx.operations.filter(
      (operation) => operation.type === 'fill' && RETIRED_CITY_PLACE_FILL_SET.has(operation.fillStyle),
    )).toHaveLength(0);
  });
});
