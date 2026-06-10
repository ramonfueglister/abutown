import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// Each `path` is scanned for forbidden demo/stale-seed patterns in PRODUCTION
// source only. A path may be a single file or a directory: directories (the
// sim-server `runtime/` and `app/` module dirs after the god-file split) are
// scanned recursively across their non-test `.rs` files. `tests.rs` files are
// excluded — they are the crates' test modules and legitimately reference seed
// helpers.
const checks = [
  {
    path: 'backend/crates/sim-server/src/runtime',
    patterns: [
      'SEEDED_CHUNKS',
      `CityNetwork::${['empty', 'for', 'world'].join('_')}`,
      `${['tiny', 'world()'].join('_')}`,
      ['legacy', 'seeded'].join('_'),
      'TileKind::BuildingFootprint',
    ],
  },
  {
    path: 'backend/crates/sim-server/src/app',
    patterns: [
      'Err(_) => SimulationRuntime::new()',
      ['empty', 'for', 'world'].join('_'),
    ],
  },
  {
    path: 'backend/crates/sim-core/src/mobility',
    patterns: ['legacy_seed_polyline', 'fallback'],
  },
  {
    path: 'backend/crates/sim-core/src/mobility_geometry.rs',
    patterns: ['fallback'],
  },
  {
    path: 'src/render/backendMobilityDrawables.ts',
    patterns: ['syntheticPath'],
  },
  {
    path: 'src/main.ts',
    patterns: [
      'buildNorthboundTrainPath(',
      'trainWrappedOffset(',
      'for (const train of trains)',
    ],
  },
];

describe('production stale-seed removal', () => {
  it('does not keep demo world recovery paths in production entrypoints', () => {
    const hits = checks.flatMap(({ path, patterns }) => {
      const source = productionSource(path);
      return patterns
        .filter((pattern) => source.includes(pattern))
        .map((pattern) => `${path}: ${pattern}`);
    });

    expect(hits).toEqual([]);
  });
});

/** Strip the inline test module (everything from the first `#[cfg(test)]`). */
function productionPortion(source: string): string {
  const testModuleStart = source.indexOf('\n#[cfg(test)]');
  return testModuleStart === -1 ? source : source.slice(0, testModuleStart);
}

/** Recursively collect production source for a file or a module directory. */
function productionSource(relPath: string): string {
  const abs = join(process.cwd(), relPath);
  if (!statSync(abs).isDirectory()) {
    return productionPortion(readFileSync(abs, 'utf8'));
  }
  const parts: string[] = [];
  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const child = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(child);
      } else if (entry.name.endsWith('.rs') && entry.name !== 'tests.rs') {
        parts.push(productionPortion(readFileSync(child, 'utf8')));
      }
    }
  };
  walk(abs);
  return parts.join('\n');
}
