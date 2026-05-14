# Decoded OpenTTD Fan GRFs

Generated with `tools/grfcodec/bin/grfcodec` from the `.grf` files in `public/openttd-fan-assets/extracted`.

Regenerate from the repository root:

```sh
node scripts/decode-openttd-fan-grfs.mjs
```

GRFCodec output:

- `.nfo`: decoded NewGRF metadata/instructions.
- `.png`: decoded 8bpp sprite sheets.
- `.32.png`: decoded 32bpp sprite sheets when present.

Use the package-specific `license.txt` files in `public/openttd-fan-assets/extracted` and `manifest.json` before moving any decoded sprite into the game runtime.
