# Project conventions for Claude Code sessions

Short, opinionated rules accumulated from things that bit us. Add new ones
sparingly — only when a real bug shipped that a convention would have caught.

## Browser-smoke is mandatory for features that change frontend wiring

**Trigger:** any change that touches the frontend↔backend boundary, including
WebSocket message flow, subscription/connection state, the coordinate systems
used to project camera ↔ tile ↔ chunk, or anything that crosses
`src/backend/` ↔ `src/render/` ↔ `src/main.ts`.

**Why:** Phase 7a (2026-05-18) shipped to `main` with full TDD coverage,
spec-review, two-stage code-quality review, and a holistic final review —
yet the production code sent **zero** subscription messages to the server
because `screenToWorld` returns render-pixel coords but `chunkOf` expects
tile coords. The unit tests all used `createCameraState({x:0, y:0, scale:1})`
where the two coordinate systems coincidentally line up, so every test was
green while production was 100% broken.

**Action:** before claiming a frontend-touching feature complete, run a real
browser smoke. The repo has `scripts/smoke-7a.mjs` as a template — it
launches headless chromium against the dev stack and reports every WS frame
the client sends. Adapt the script (or write a similar one) per feature.

**Don't accept "all tests pass" as a substitute** when the feature crosses
the wire. The lesson cost two extra commits to fix after the original
"complete" claim; a browser smoke would have caught it in one minute.

## Never run two cargo commands at once — route cargo through `scripts/cargo-serial.sh`

**Trigger:** any `cargo` invocation (test / build / clippy / check), whether you
run it yourself, in a background task, or via a dispatched subagent.

**Why:** two cargo processes against the same `backend/target/` block on
cargo's build lock. Nothing corrupts — but the second one silently stalls
waiting for the lock, which looks exactly like the session hanging for
minutes. This bit us hard (escalated to "ALARM") when a background subagent
ran a broad `cargo test --workspace --all-targets` while a scoped
`cargo test -p sim-server` was still going, and an orphaned test process
lingered after the subagent finished.

**Action:**
- Run cargo through `scripts/cargo-serial.sh` (e.g.
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server`).
  It serializes via an atomic mkdir lock: the second caller WAITS with visible
  progress instead of stalling, reclaims the lock if the holder died, and times
  out via `CARGO_SERIAL_MAX_WAIT` (default 1200s).
- Run slow cargo in the **background** (`run_in_background`) and poll, so you
  stay responsive instead of blocking the whole turn.
- Dispatch Rust-touching subagents **one at a time**, and tell them to run
  exactly the scoped cargo command given — never a broad
  `--workspace --all-targets` run during iteration.
- Before launching cargo, `pgrep -f cargo` and clear orphans.

## Other notes for future sessions

- `progress.md` lines 4-18 are chronological-ascending; from line 19 onward
  (Phase 5+) the convention switched to reverse-chronological at the head
  of the list. Insert new entries at the top of that block, not appended
  at the end. The file is internally inconsistent — don't try to fix it
  retroactively, just match the local convention.
- `tsconfig.json` has `"include": ["src"]` — **tests are not type-checked
  by `tsc --noEmit`**. Vitest transpiles them at runtime but with looser
  checks. Don't rely on tsc to catch test type errors. If a test calls a
  production function with a stale signature, vitest may still pass it
  while production breaks.
- `Arc<RwLock<T>>: Send` requires `T: Send + Sync`. When swapping
  `Mutex<X>` for `RwLock<X>` in async code, expect to add `+ Sync` to
  every store-trait bound that `X` carries as a `Box<dyn …>`.
- The vite build wrapper (`scripts/build.mjs`) exists because vite's
  per-file `copyFileSync` over the 8 MB `public/simutrans-assets/` tree
  intermittently `ETIMEDOUT`s on macOS. Don't try to remove the wrapper.
