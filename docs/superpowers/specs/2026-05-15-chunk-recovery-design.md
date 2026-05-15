# Chunk Recovery & Command Idempotency Design

**Date:** 2026-05-15
**Status:** Approved for planning
**Predecessor:** `2026-05-15-supabase-backed-chunk-snapshots.md` (durable writes are now in place; this slice closes the loop on durable reads + idempotent commands)
**Successor (out of scope):** Mobility population and persistence — Mobility is an empty stub today, recovery for it is meaningless until population exists.

---

## Goal

After a `sim-server` restart, the world state observed by clients is identical to the state before the restart, and a client that retries the same command never produces a duplicate world mutation. Achieved without changing the client wire protocol.

## Non-Goals

- Mobility recovery (separate plan).
- Player aggregates (do not exist).
- Snapshot compaction, point-in-time recovery, multi-version history.
- Multi-writer conflict resolution (`expected_chunk_version` from the client).
- Cross-region / multi-world replication.
- Lazy chunk loading triggered by client requests (the three seeded chunks remain the only chunks; lazy hydration is a future hook).

## Architecture

State-of-the-art CQRS + Event Sourcing per chunk aggregate, with snapshot checkpoints.

- **Chunk = aggregate root.** Each chunk has its own monotonically increasing `chunk_version`.
- **`chunk_snapshots`** holds the latest consolidated state per `(world_id, chunk_x, chunk_y)` plus its `chunk_version` as a checkpoint. (Schema unchanged from the previous slice.)
- **`world_events`** is the append-only audit + replay log. Each row carries `chunk_x`, `chunk_y`, and `chunk_version` (the chunk's version AFTER the event has been applied).
- **Startup hydration per chunk:** read snapshot → reconstruct chunk at `chunk_version = V` → read events `WHERE chunk_x = X AND chunk_y = Y AND chunk_version > V` in order → apply each → chunk is current.
- **Command path:** dedupe on `(world_id, command_id)` via unique constraint; on conflict, return the cached `CommandAcceptedDto` rebuilt from the existing event row. No state mutation on retries.
- **Snapshot trigger:** per-chunk, only when the chunk has un-persisted events AND (≥1 event since last snapshot OR ≥30s since last snapshot). The current "persist every loop tick regardless of dirty state" wastes writes.

The server remains single-writer per world; this design intentionally does not introduce client-side optimistic concurrency. The `chunk_version` column makes it possible to add it later without schema migrations beyond what is here.

## Schema Changes

One new migration, `backend/crates/sim-server/migrations/202605160001_chunk_recovery.sql`:

```sql
ALTER TABLE world_events
  ADD COLUMN chunk_x INTEGER,
  ADD COLUMN chunk_y INTEGER,
  ADD COLUMN chunk_version BIGINT;

UPDATE world_events
   SET chunk_x = (payload->'coord'->>'x')::int,
       chunk_y = (payload->'coord'->>'y')::int,
       chunk_version = version
 WHERE chunk_x IS NULL;

ALTER TABLE world_events
  ALTER COLUMN chunk_x SET NOT NULL,
  ALTER COLUMN chunk_y SET NOT NULL,
  ALTER COLUMN chunk_version SET NOT NULL;

CREATE UNIQUE INDEX world_events_world_command_uniq
  ON world_events (world_id, command_id);

CREATE INDEX world_events_chunk_version_idx
  ON world_events (world_id, chunk_x, chunk_y, chunk_version);
```

Rationale:

- Backfilling `chunk_x/chunk_y` from `payload` is safe because today the only event type (`TileKindSet`) embeds `coord`. If a future event type is chunk-agnostic, its insert path must set these columns explicitly.
- `chunk_version` is backfilled from the existing global `version` only for legacy rows. New rows set `chunk_version` correctly per chunk going forward.
- Unique index on `(world_id, command_id)` is the idempotency anchor.

## Recovery Flow

A new async constructor on `SimulationRuntime`:

```rust
pub async fn hydrate_from_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
) -> Result<Self, HydrationError>
```

Steps:

1. For each `coord` in `SEEDED_CHUNKS`:
   - `snapshot_store.read_snapshot(coord)` →
     - if `Some(snap)`: `chunk = Chunk::from_snapshot(&snap.payload)`, `chunk_version = snap.chunk_version`.
     - if `None`: `chunk = seed_default_chunk(coord)`, `chunk_version = 0`.
   - `event_store.read_chunk_events_since(world_id, coord, chunk_version)` → events ordered by `chunk_version` ASC.
   - For each event: `chunk.apply_event(event)`; `chunk_version = event.chunk_version`.
   - `registry.insert_hydrated(chunk, chunk_version, ChunkActivity::Warm)`.
2. `global_tick = event_store.max_tick(world_id).unwrap_or(0)`.
3. `global_version = event_store.max_version(world_id).unwrap_or(0)`.
4. Return runtime with the hydrated registry, tick, and version.

`build_app_from_config` calls `hydrate_from_stores` instead of `new_with_stores` when `database_url` is configured. The in-memory path used by tests keeps `new_with_stores` (sync) for compatibility.

**Failure policy:** Any error during hydration (store read failure, snapshot deserialization failure, event apply rejection) is fatal — the server refuses to start. Silent fallback to fresh seed would lose data without an operator signal.

### New methods required

- `Chunk::from_snapshot(payload: &ChunkSnapshotPayload) -> Result<Chunk, SnapshotDecodeError>` — inverse of the existing `build_chunk_snapshot`.
- `Chunk::apply_event(event: &WorldEventDto) -> Result<(), EventApplyError>` — applies the event's mutation to the chunk and bumps the chunk's internal version. Pure; same input always produces same output.
- `WorldEventStore::read_chunk_events_since(world_id, coord, after_chunk_version) -> Vec<WorldEventDto>` — replay query, ordered by `chunk_version`.
- `WorldEventStore::max_tick(world_id) -> Option<u64>` and `max_version(world_id) -> Option<u64>` — for restoring global counters.

## Command Path with Idempotency

Per command, in `SimulationRuntime::handle_command`:

1. **Pre-flight dedup:** `event_store.find_event_by_command(world_id, command_id)`.
   - If `Some(existing_event)`: reconstruct and return the original `CommandAcceptedDto` from `existing_event`. No chunk mutation, no new event. Logged at INFO level for observability.
   - If `None`: continue.
2. **Compute prospective mutation:** build the `WorldEventDto` and the prospective new chunk state without yet writing it into the registry. Concretely: clone the affected chunk, apply the mutation to the clone, derive the resulting `chunk_version`. The live registry is untouched at this point.
3. **Persist event:** `event_store.append_event(event)` using `INSERT ... ON CONFLICT (world_id, command_id) DO NOTHING RETURNING *`.
   - If `RETURNING` returned 1 row: swap the mutated chunk clone into the registry, mark dirty, return `CommandAcceptedDto` built from the inserted event.
   - If 0 rows returned (race with a concurrent retry that won the insert): discard the chunk clone, `find_event_by_command` to fetch the winning event, return its cached response.

The race window is narrow (single-writer server) but covered by the unique constraint as the last line of defense. Computing the mutation on a clone makes rollback a no-op (just drop the clone) rather than requiring an inverse-event mechanism.

`WorldEventStore` gains:

- `find_event_by_command(world_id, command_id) -> Option<WorldEventDto>`.
- `append_event` already exists but its error type must distinguish "duplicate command_id" from other failures.

## Snapshot Trigger

Today: `persist_chunk_snapshots()` walks all seeded chunks every loop tick and writes all of them, regardless of whether anything changed.

Change: `ChunkRegistry` tracks per-chunk `last_persisted_version` (already partly there as `dirty` flag). The snapshot loop now:

1. Compute candidates: chunks where `current_version > last_persisted_version` AND (`current_version - last_persisted_version >= 1` is trivially true, OR `now - last_snapshot_at >= 30s`).
2. Write only candidates.
3. Update `last_persisted_version` and `last_snapshot_at` on successful write.

The 30s upper bound prevents starvation for chunks with low write rates that nevertheless should checkpoint.

## Public API Changes

- `SimulationRuntime::hydrate_from_stores` — new async constructor.
- `SimulationRuntime::new_with_stores` — kept sync, used only by in-memory tests.
- `app::build_app_from_config` — branches on `database_url` to call hydrate.
- `WorldEventStore` trait — three new methods (`read_chunk_events_since`, `max_tick`, `max_version`, `find_event_by_command`).
- `Chunk` — two new methods (`from_snapshot`, `apply_event`).

**No frontend or wire-protocol changes.** Client continues to send `command_id` strings; today they are not deduped, after this change they are.

## Testing Strategy

**Unit tests (sim-core):**

- `Chunk::from_snapshot` ⇄ `build_chunk_snapshot` roundtrip preserves all tile state and version.
- `Chunk::apply_event` applied twice with the same event leaves the chunk unchanged on the second call (idempotent at the chunk level — defense in depth alongside DB-level idempotency).
- `InMemoryWorldEventStore::find_event_by_command` and `read_chunk_events_since` ordering.

**Unit tests (sim-server):**

- `SimulationRuntime::hydrate_from_stores` with seeded `InMemoryChunkSnapshotStore` + `InMemoryWorldEventStore` reproduces a runtime whose `chunk_snapshot()` matches a runtime that was mutated and never restarted.
- Command with duplicate `command_id` returns cached response without producing a second event.
- Hydration with no snapshots (cold start) successfully replays events from version 0.
- Hydration error (snapshot decode failure) returns `HydrationError`, not panics.

**Integration tests (opt-in, `ABUTOWN_TEST_DATABASE_URL`):**

- Mutate world via HTTP `POST /commands`, drop runtime, re-create via `hydrate_from_stores`, assert chunk snapshot identical.
- Send same command twice via HTTP, assert one event row, identical responses.
- Snapshot trigger writes only when chunk is dirty; unchanged chunks generate no writes over multiple loop iterations.

**Regression:** all existing tests pass — they use in-memory stores and the sync `new_with_stores` path.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Backfill UPDATE on `world_events` is slow on large tables | The table is small (< 1k rows in dev/staging today); migration is acceptable. Production note: if rows grow large before this lands, switch to batched UPDATE before the NOT NULL flip. |
| New event types without `chunk_x/y` populated | Document in the `WorldEventStore` trait: `append_event` must receive events whose chunk coordinates are set. Add a debug assertion. |
| Idempotency leaks: `command_id` collisions across clients | `command_id` is opaque text; if two clients independently generate the same string, the second is silently treated as a retry. Documented as a client responsibility; client UUIDv4 prefixed with client_id is the recommended generation pattern but not enforced server-side in this slice. |
| Hydration latency on large event tails | Bounded by the snapshot trigger: snapshots get written every 30s max, so tail replay is at most 30s of events per chunk. |

## File Structure (anticipated)

- New: `backend/crates/sim-server/migrations/202605160001_chunk_recovery.sql`
- New methods in: `backend/crates/sim-core/src/chunk.rs` (`from_snapshot`, `apply_event`)
- New methods in: `backend/crates/sim-core/src/events.rs` (trait additions, in-memory impl)
- New methods in: `backend/crates/sim-server/src/postgres_events.rs` (the postgres impl of new trait methods)
- Modified: `backend/crates/sim-server/src/runtime.rs` (`hydrate_from_stores`, dedup in `handle_command`, smarter snapshot trigger)
- Modified: `backend/crates/sim-server/src/chunk_registry.rs` (`insert_hydrated`, `last_persisted_version` tracking)
- Modified: `backend/crates/sim-server/src/app.rs` (call `hydrate_from_stores`)
- Modified: `backend/crates/sim-server/tests/http.rs` (idempotency + recovery integration tests)

## Open Questions Resolved During Brainstorming

- **Source of truth at startup:** snapshot is the fast path; world_events is the durable truth — events past the snapshot are always replayed. No data loss.
- **Idempotency key:** existing client-supplied `command_id`. No new field needed.
- **Mobility scope:** out — empty stub today, recovery is premature.
- **Multi-writer concurrency:** out — single-writer server today; `chunk_version` column reserved for the future.
- **Snapshot frequency:** dirty-based + 30s ceiling, not every loop tick.
