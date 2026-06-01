# Chunk snapshot u16 write-side guard — design spec

**Date:** 2026-05-31
**Status:** implemented

## Problem

`build_chunk_snapshot_from_parts` (`backend/crates/sim-core/src/persistence.rs`)
converted tile counts and indices to `u16` with raw `as` casts:

```rust
let tile_count = tiles.len() as u16;      // silently truncates > 65535
local_index: index as u16,                // silently truncates > 65535
```

On a tile slice longer than `u16::MAX` this **silently truncates**, producing a
corrupt `ChunkSnapshotDto` instead of failing. It is a `pub` function whose
contract (the caller passes a u16-sized chunk) was enforced only by convention.

### Reachability

Not reachable today: `Chunk::try_new` (`chunk.rs:72`) rejects any `chunk_size`
whose `chunk_size² > u16::MAX`, so production chunks (size 32 → 1024 tiles) are
far below the limit, and all three callers feed already-validated chunks. This
is a **latent footgun on a public API**, not a live bug.

### Asymmetry

The read side already validates: `protocol/src/lib.rs` uses
`u16::try_from(...).map_err(...)`. Only the write side truncated silently. This
change makes the two sides symmetric.

## Decision

Fail loudly, do not silently truncate. Tiles here come from server-built,
already-validated chunks, so a violation is an internal programmer error, not
external input — the idiomatic response is a panic with a clear message,
mirroring `Chunk::new`'s existing `.expect("chunk size must fit u16 tile
indices")` (`chunk.rs:69`) for the same invariant.

Not chosen: returning `Result` (would churn three callers + the
`SnapshotProvider::collect` trait for an unreachable case — rejected per the
project's "no defensive plumbing for unreachable states" rule); `debug_assert`
(still truncates in release).

## Change

`backend/crates/sim-core/src/persistence.rs`, both casts:

```rust
let tile_count = u16::try_from(tiles.len())
    .expect("chunk tile count exceeds u16; chunk_size must be <= 255 (see Chunk::try_new)");
...
local_index: u16::try_from(index)
    .expect("tile index exceeds u16; chunk_size must be <= 255 (see Chunk::try_new)"),
```

No signature change, no caller churn.

## Testing

In the existing `#[cfg(test)] mod tests` in `persistence.rs`:

1. `build_chunk_snapshot_accepts_u16_max_tiles` — 65535 tiles do not panic,
   `tile_count == u16::MAX` (off-by-one boundary guard).
2. `build_chunk_snapshot_panics_when_tile_count_exceeds_u16` — 65536 tiles
   `#[should_panic(expected = "chunk tile count exceeds u16")]` (the true
   red→green: fails against the old `as` cast, passes after).

Backend-only; no frontend boundary touched → no browser smoke. Gate: fmt,
clippy, `-p sim-core` tests.
