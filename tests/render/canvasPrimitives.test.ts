import { describe, expect, it } from 'vitest';
import { drawCapsule } from '../../src/render/canvasPrimitives';

type Operation =
  | ['beginPath']
  | ['lineCap', CanvasLineCap]
  | ['lineTo', number, number]
  | ['lineWidth', number]
  | ['moveTo', number, number]
  | ['restore']
  | ['rotate', number]
  | ['save']
  | ['stroke']
  | ['strokeStyle', string]
  | ['translate', number, number];

function createContext(): CanvasRenderingContext2D & { operations: Operation[] } {
  const operations: Operation[] = [];
  const context = {
    operations,
    beginPath: () => operations.push(['beginPath']),
    lineTo: (x: number, y: number) => operations.push(['lineTo', x, y]),
    moveTo: (x: number, y: number) => operations.push(['moveTo', x, y]),
    restore: () => operations.push(['restore']),
    rotate: (angle: number) => operations.push(['rotate', angle]),
    save: () => operations.push(['save']),
    stroke: () => operations.push(['stroke']),
    translate: (x: number, y: number) => operations.push(['translate', x, y]),
    set lineCap(value: CanvasLineCap) {
      operations.push(['lineCap', value]);
    },
    set lineWidth(value: number) {
      operations.push(['lineWidth', value]);
    },
    set strokeStyle(value: string | CanvasGradient | CanvasPattern) {
      operations.push(['strokeStyle', String(value)]);
    },
  };
  return context as unknown as CanvasRenderingContext2D & { operations: Operation[] };
}

describe('canvasPrimitives', () => {
  it('draws a cased capsule around a centered stroke', () => {
    const context = createContext();

    drawCapsule(context, { x: 10, y: 20 }, Math.PI / 4, 30, 6, '#123456', '#abcdef');

    expect(context.operations).toEqual([
      ['save'],
      ['translate', 10, 20],
      ['rotate', Math.PI / 4],
      ['lineCap', 'round'],
      ['strokeStyle', '#abcdef'],
      ['lineWidth', 8.6],
      ['beginPath'],
      ['moveTo', -15, 0],
      ['lineTo', 15, 0],
      ['stroke'],
      ['strokeStyle', '#123456'],
      ['lineWidth', 6],
      ['beginPath'],
      ['moveTo', -15, 0],
      ['lineTo', 15, 0],
      ['stroke'],
      ['restore'],
    ]);
  });

  it('draws a single centered stroke when casing is omitted', () => {
    const context = createContext();

    drawCapsule(context, { x: -2, y: 3 }, 0, 12, 4, '#222');

    expect(context.operations).toEqual([
      ['save'],
      ['translate', -2, 3],
      ['rotate', 0],
      ['lineCap', 'round'],
      ['strokeStyle', '#222'],
      ['lineWidth', 4],
      ['beginPath'],
      ['moveTo', -6, 0],
      ['lineTo', 6, 0],
      ['stroke'],
      ['restore'],
    ]);
  });
});
