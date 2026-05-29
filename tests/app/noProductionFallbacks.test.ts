import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const checks = [
  {
    file: 'backend/crates/sim-server/src/runtime.rs',
    patterns: [
      'SEEDED_CHUNKS',
      `CityNetwork::${['empty', 'for', 'world'].join('_')}`,
      `${['tiny', 'world()'].join('_')}`,
      ['legacy', 'seeded'].join('_'),
      ['trams', 'total'].join('_'),
      'TileKind::BuildingFootprint',
    ],
  },
  {
    file: 'backend/crates/sim-server/src/app.rs',
    patterns: [
      'Err(_) => SimulationRuntime::new()',
      ['empty', 'for', 'world'].join('_'),
    ],
  },
  {
    file: 'src/main.ts',
    patterns: [
      'buildNorthboundTrainPath(',
      'trainWrappedOffset(',
      'for (const train of trains)',
    ],
  },
];

describe('production stale-seed removal', () => {
  it('does not keep demo world recovery paths in production entrypoints', () => {
    const hits = checks.flatMap(({ file, patterns }) => {
      const source = productionSource(file);
      return patterns
        .filter((pattern) => source.includes(pattern))
        .map((pattern) => `${file}: ${pattern}`);
    });

    expect(hits).toEqual([]);
  });
});

function productionSource(file: string): string {
  const source = readFileSync(join(process.cwd(), file), 'utf8');
  const testModuleStart = source.indexOf('\n#[cfg(test)]');
  return testModuleStart === -1 ? source : source.slice(0, testModuleStart);
}
