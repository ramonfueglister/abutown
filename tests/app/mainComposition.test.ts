import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

describe('main composition root', () => {
  it('delegates runtime startup, diagnostics, interaction, context, and rendering to app modules', () => {
    const source = readFileSync(new URL('../../src/main.ts', import.meta.url), 'utf8');

    expect(source).toContain("from './app/appRuntime'");
    expect(source).toContain("from './app/backendRequiredView'");
    expect(source).toContain("from './app/entitySelection'");
    expect(source).toContain("from './app/interaction'");
    expect(source).toContain("from './app/runtimeDiagnostics'");
    expect(source).toContain("from './render/worldRuntimeTypes'");
    expect(source).toContain("from './render/minimalMapRenderer'");
    expect(source).not.toContain('function drawRoad(');
    expect(source).not.toContain('function drawPedestrian(');
    expect(source).not.toContain('function cityDiagnostics(');
  });
});
