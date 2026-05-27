import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

describe('no legacy terrain runtime truth', () => {
  it('does not use Zurich frontend builders as runtime render authority', () => {
    const main = readFileSync('src/main.ts', 'utf8');

    expect(main).not.toContain('buildZurichWorld({');
    expect(main).not.toContain('buildZurichTransport(');
    expect(main).not.toContain('buildZurichPlacement(');
  });
});
