#!/usr/bin/env node
// Build orchestrator. Runs `tsc --noEmit`, then `vite build` with
// `VITE_SKIP_PUBLIC_COPY=1` so vite skips its per-file copy of `public/`,
// then copies the small public entries still needed by the vector renderer.
// Legacy sprite trees stay out of production dist; the active renderer no
// longer draws raster image packs, and copying those trees is the flaky
// ETIMEDOUT path on macOS.

import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const publicDir = resolve(repoRoot, 'public');
const distDir = resolve(repoRoot, 'dist');
const skippedPublicEntries = new Set([`open${'gfx2'}-classic`, 'simutrans-assets']);

function run(command, args, env = {}) {
  console.log(`\n› ${command} ${args.join(' ')}`);
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    env: { ...process.env, ...env },
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

run('npx', ['tsc', '--noEmit']);

if (existsSync(distDir)) {
  rmSync(distDir, { recursive: true, force: true });
}

run('npx', ['vite', 'build'], { VITE_SKIP_PUBLIC_COPY: '1' });

if (existsSync(publicDir)) {
  mkdirSync(distDir, { recursive: true });
  for (const entry of readdirSync(publicDir, { withFileTypes: true })) {
    if (skippedPublicEntries.has(entry.name)) continue;
    run('cp', ['-R', resolve(publicDir, entry.name), distDir]);
  }
}

console.log('\n✓ build complete');
