import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

describe('no legacy terrain runtime truth', () => {
  it('does not use Zurich frontend builders as runtime render authority', () => {
    const main = readFileSync('src/main.ts', 'utf8');

    expect(main).not.toContain('buildZurichWorld({');
    expect(main).not.toContain('buildZurichTransport(');
    expect(main).not.toContain('buildZurichPlacement(');
  });

  it('does not keep retired procedural frontend world builders in the runtime bundle', () => {
    const main = readFileSync('src/main.ts', 'utf8');

    expect(main).not.toContain('const districtSeeds');
    expect(main).not.toContain('function buildTerrain(');
    expect(main).not.toContain('function buildRoadNetwork(');
    expect(main).not.toContain('function buildBuildings(');
    expect(main).not.toContain('function buildTrees(');
  });
});
