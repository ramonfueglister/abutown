# Project conventions for Claude Code sessions

Abutown is currently a **card-hand + Supabase-login** app. The former economy
simulation was removed (see
`docs/superpowers/specs/2026-07-01-strip-to-cardhand-design.md`); recover any of
it from git history. A new simulation is planned — scaffolding kept for it:
the proto/buf toolchain (`buf.*`, `scripts/generate-proto-ts.mjs`, `protocol`
crate placeholder), the `/ws` route stub in `sim-server`, `scripts/cargo-serial.sh`,
and the browser-smoke template `scripts/smoke-7a.mjs`.

## Browser-smoke for frontend↔backend changes

Any change crossing `src/` ↔ `backend/` (auth flow, card fetch, new routes)
should be verified with a real browser smoke, not just unit tests. Adapt
`scripts/smoke-7a.mjs` per feature.

## Route cargo through `scripts/cargo-serial.sh`

Never run two cargo commands at once against `backend/target/` — the second
stalls on the build lock. Use `scripts/cargo-serial.sh <cmd>` and run slow
builds in the background.

## Notes

- `tsconfig.json` has `"include": ["src"]` — tests are not type-checked by
  `tsc --noEmit`. Generated `src/proto/` must stay type-clean.
- The card hand renders "Login unavailable" if `VITE_SUPABASE_*` are unset.
