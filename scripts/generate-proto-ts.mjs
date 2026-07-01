#!/usr/bin/env node
// Runs `buf generate` to produce TS types from
// backend/crates/protocol/proto/abutown.proto into src/backend/proto/.

import { spawnSync } from 'node:child_process';
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const outDir = resolve(root, 'src/proto');
mkdirSync(outDir, { recursive: true });

const result = spawnSync('npx', ['buf', 'generate'], {
  cwd: root,
  stdio: 'inherit',
});

if (result.status !== 0) {
  console.error('buf generate failed with exit code', result.status);
  process.exit(result.status ?? 1);
}

for (const file of readdirSync(outDir)) {
  if (!file.endsWith('_pb.ts')) continue;
  const path = resolve(outDir, file);
  const source = readFileSync(path, 'utf8');
  writeFileSync(path, `${source.trimEnd()}\n`);
}

console.log('proto codegen complete →', outDir);
