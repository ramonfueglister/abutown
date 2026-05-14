import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { basename, extname, join, resolve } from 'node:path';

const root = process.cwd();
const grfcodec = join(root, 'tools', 'grfcodec', 'bin', 'grfcodec');
const extractedRoot = join(root, 'public', 'openttd-fan-assets', 'extracted');
const decodedRoot = join(root, 'public', 'openttd-fan-assets', 'decoded');

if (!existsSync(grfcodec)) {
  throw new Error(`Missing GRFCodec binary: ${grfcodec}`);
}

rmSync(decodedRoot, { recursive: true, force: true });
mkdirSync(decodedRoot, { recursive: true });
writeFileSync(join(decodedRoot, 'README.md'), `# Decoded OpenTTD Fan GRFs

Generated with \`tools/grfcodec/bin/grfcodec\` from the \`.grf\` files in \`public/openttd-fan-assets/extracted\`.

Regenerate from the repository root:

\`\`\`sh
node scripts/decode-openttd-fan-grfs.mjs
\`\`\`

GRFCodec output:

- \`.nfo\`: decoded NewGRF metadata/instructions.
- \`.png\`: decoded 8bpp sprite sheets.
- \`.32.png\`: decoded 32bpp sprite sheets when present.

Use the package-specific \`license.txt\` files in \`public/openttd-fan-assets/extracted\` and \`manifest.json\` before moving any decoded sprite into the game runtime.
`);

const grfs = [];
walk(extractedRoot, (file) => {
  if (extname(file).toLowerCase() === '.grf') grfs.push(file);
});

for (const grf of grfs.sort()) {
  const output = join(decodedRoot, slugify(basename(grf, extname(grf))));
  mkdirSync(output, { recursive: true });
  console.log(`Decoding ${grf} -> ${output}`);
  execFileSync(grfcodec, ['-d', '-s', '-o', 'png', '-w', '2048', resolve(grf), output], { stdio: 'inherit' });
}

console.log(`Decoded ${grfs.length} GRF files into ${decodedRoot}`);

function walk(dir, visitor) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walk(full, visitor);
    if (entry.isFile()) visitor(full);
  }
}

function slugify(value) {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
}
