# OpenTTD Fan Assets

Downloaded from the OpenTTD BaNaNaS content service on 2026-05-14T18:36:07.706Z. See manifest.json for per-package source, author, version, license, MD5, and paths.

These archives are kept separate from OpenGFX2 because licenses are mixed, including GPL v2, GPL v3, and CC-BY-NC-SA v3.0. Do not merge these into a combined sprite atlas until the selected package licenses are reviewed for the target distribution.

Note: BaNaNaS content MD5s identify the content payload used by OpenTTD; archiveMd5 is the MD5 of the downloaded .tar.gz file stored here.

Additional source checkouts:

- `sources/chuffing_stations`: https://github.com/andybiotic/chuffing_stations at `a6f0054cc12017f0216780f8c95f9e6928515ba9`.
- `sources/velo`: https://github.com/EratoNysiad/velo at `d59a11887eb8fdaf8491369b4408753a1466fefd`.

The `.git` directories were intentionally removed from the source checkouts before storing them under `public`.

GRF decoding:

- GRFCodec was built from https://github.com/OpenTTD/grfcodec at `78edf59e9145cb1252432d50385862414914bae2` and installed locally under `tools/grfcodec`.
- Decoded output is under `decoded`.
- Regenerate with `node scripts/decode-openttd-fan-grfs.mjs` from the repository root.
