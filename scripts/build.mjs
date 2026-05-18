#!/usr/bin/env node
// Build orchestrator. Runs `tsc --noEmit`, then `vite build` with
// `VITE_SKIP_PUBLIC_COPY=1` so vite skips its per-file copy of `public/`,
// then copies `public/` into `dist/` with POSIX `cp -R` (one fork). The
// per-file Node copy in vite has been observed to flake on `ETIMEDOUT`
// over the 8.4 MB `public/simutrans-assets/` tree; `cp -R` is one
// syscall per inode and has not reproduced the failure.

import { spawnSync } from 'node:child_process';
import { existsSync, rmSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const publicDir = resolve(repoRoot, 'public');
const distDir = resolve(repoRoot, 'dist');

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
  // `cp -R public/. dist/` — the trailing `.` copies directory contents,
  // not the directory itself, so files land at dist/<…> as vite would.
  run('cp', ['-R', `${publicDir}/.`, distDir]);
}

console.log('\n✓ build complete');
