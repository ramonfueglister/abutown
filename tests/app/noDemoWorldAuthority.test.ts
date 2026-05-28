import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const repoRoot = process.cwd();

function read(path: string): string {
  return readFileSync(join(repoRoot, path), 'utf8');
}

describe('base world cutover guards', () => {
  it('does not use procedural Zurich builders as runtime map authority', () => {
    const runtimeEntrypoints = [
      'src/main.ts',
      'src/app/appRuntime.ts',
    ];

    const forbidden = [
      'createZurichRuntimeContext(',
      'buildZurichWorld(',
      'buildZurichTransport(',
      'buildZurichPlacement(',
    ];

    const hits = runtimeEntrypoints.flatMap((file) => {
      const source = read(file);
      return forbidden
        .filter((pattern) => source.includes(pattern))
        .map((pattern) => `${file}: ${pattern}`);
    });

    expect(hits).toEqual([]);
  });

  it('does not reference retired pak or simutrans assets from runtime code', () => {
    const runtimeFiles = [
      'src/main.ts',
      'src/render/minimalMapRenderer.ts',
      'src/app/appRuntime.ts',
    ];

    const forbidden = [/pak128/iu, /simutrans/iu, /opengfx/iu];

    const hits = runtimeFiles.flatMap((file) => {
      const source = read(file);
      return forbidden
        .filter((pattern) => pattern.test(source))
        .map((pattern) => `${file}: ${pattern}`);
    });

    expect(hits).toEqual([]);
  });
});
