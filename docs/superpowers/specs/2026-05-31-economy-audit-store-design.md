# Economy Audit Store Design — durable append-only economy event log

Date: 2026-05-31

## Status

Economy-infrastructure follow-on. Today the economy keeps an **unbounded in-memory
`TradeLedger`** (`Vec<EconomyEvent>`) and persists only a **bounded tail**
(`PERSISTED_LEDGER_TAIL = 1024`) inside the per-world `EconomyPersistSnapshot`
JSON. There is **no durable, unbounded record** of economy events. This slice adds
one: a durable, append-only **audit log** of every `EconomyEvent`, for
observability / debugging / analytics.

**Purpose: observability/audit — NOT event-sourcing.** The log is additive. The
`EconomyPersistSnapshot` remains the recovery source of truth; this store does
**not** touch hydration/recovery and the economy is **never** reconstructed from
it. (User-chosen scope.) It mirrors the established
`2026-05-15-persistent-world-event-store-design.md` precedent: a store contract in
`sim-core`, an in-memory default, a Postgres adapter in `sim-server`, durable
append, and **query/read APIs deferred to a later slice** (the table is indexed so
SQL queries are possible; an app-level query endpoint is out of scope here).

**Backend-only. No wire/frontend change. No per-tick DB writes** (batched on the
existing snapshot-persistence cadence). Conservation/determinism of the economy
are untouched (the audit append is a read-only side effect of already-emitted
events).

## Architecture

### Bundled fix: bound the in-memory ledger

The live `TradeLedger` currently grows **without bound** (only the snapshot trims
to 1024; the live resource never does → a latent memory leak). The audit store is
the enabler to fix it: once events are durably appended, the live ledger is
trimmed to the last `PERSISTED_LEDGER_TAIL` (1024) after each flush. Since the
snapshot tail and `GET /economy` already use only the last 1024, **this changes no
visible behavior** — it only bounds memory, while the full history lives durably
in the audit store.

### `EconomyEventStore` contract (`sim-core/src/persistence.rs`)

Alongside the existing `EconomySnapshotStore`:

```rust
#[async_trait]
pub trait EconomyEventStore: std::fmt::Debug + Send + Sync {
    /// Durably append a batch of economy events for a world, in order. The store
    /// assigns each row a monotonic id (insertion order). Best-effort: the caller
    /// treats failures as non-fatal (see "Flush").
    async fn append(
        &mut self,
        world_id: &str,
        tick: u64,
        events: &[EconomyEvent],
    ) -> Result<(), EconomyEventStoreError>;
}
```

- `InMemoryEconomyEventStore` (default test/local): `Vec<(u64 /*tick*/, EconomyEvent)>`
  per world; preserves append order; exposes a `len()`/`events()` test helper (not
  on the trait — a SQL store can't return a borrowed slice cheaply, per the
  world-event-store precedent).
- The trait is the only thing `sim-core`/the runtime depends on; the Postgres
  adapter lives in `sim-server`.

### Flush — drain/commit on the snapshot-persistence cadence

A `LedgerAuditCursor(usize)` resource records how many of the live `TradeLedger`
events have been durably appended. The flush is driven by the **sim-server
persistence loop** (the same async cycle that writes the `EconomySnapshot`),
because the append is async (DB) — it is NOT an ECS system:

1. Under the runtime lock, read the unflushed slice + current tick via a sim-core
   helper `pending_ledger_audit(world) -> (tick, Vec<EconomyEvent>)` (returns
   `ledger.0[cursor..].to_vec()`); skip if empty.
2. `event_store.append(world_id, tick, &pending).await`.
3. **On success**, under the lock, `commit_ledger_audit(world)`: advance the cursor
   and trim the live ledger to the last `PERSISTED_LEDGER_TAIL`, resetting
   `cursor = ledger.len()` (post-trim, every retained event is flushed).
4. **On failure** (best-effort): log a warning, do **not** advance the cursor or
   trim — the events stay in the ledger and are retried next cycle. A failed audit
   append never halts the sim (this is the key difference from the *authoritative*
   world-event-store, whose append failure rejected the command).

Invariant after each successful flush: `cursor == ledger.len()` and every retained
ledger event has been appended.

### Restart (no double-append, best-effort)

On hydrate the live `TradeLedger` is restored from the snapshot's `ledger_tail`
(≤1024 events that were already appended before shutdown). The runtime initializes
`LedgerAuditCursor = TradeLedger.len()` right after economy hydration, so the
restored tail is **not** re-appended. Trade-off (acceptable for an audit log): if
the server crashed between the last flush and shutdown, the unflushed window (at
most the events since the last snapshot) is lost — best-effort, no duplicates.

### Postgres adapter (`sim-server`) + migration

`migrations/<ts>_economy_events.sql`:

```sql
CREATE TABLE IF NOT EXISTS economy_events (
    id          BIGSERIAL PRIMARY KEY,
    world_id    TEXT     NOT NULL,
    tick        BIGINT   NOT NULL CHECK (tick >= 0),
    event_type  TEXT     NOT NULL,
    payload     JSONB    NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS economy_events_world_id_idx ON economy_events (world_id, id);
CREATE INDEX IF NOT EXISTS economy_events_world_tick_idx ON economy_events (world_id, tick);
```

- `id BIGSERIAL` gives a durable monotonic ordering per insertion (per-world order
  via the `(world_id, id)` index); no app-managed per-world seq needed.
- `event_type` is the `EconomyEvent` variant name (a small `fn event_type(&EconomyEvent) -> &'static str`
  in `ledger.rs`), stored as an indexed column so a future query slice can filter
  by kind without parsing JSON; `payload` is the full `serde_json` of the event.
- `PostgresEconomyEventStore::append` does ONE batched multi-row `INSERT` (e.g.
  `UNNEST`-based or a built `VALUES` list) for the whole flush batch.

### Configuration (mirror the snapshot store)

- Default local/runtime: `InMemoryEconomyEventStore`.
- If `DATABASE_URL` is configured: `sim-server` startup creates a
  `PostgresEconomyEventStore` (the migration runs inline in `connect()`, like the
  other Postgres stores). Fail-fast if it can't initialize.
- Wired into the runtime next to the `EconomySnapshotStore` (constructor +
  `hydrate_from_stores` signature gain an `EconomyEventStore`).

## Conservation / determinism

The audit append reads already-emitted `TradeLedger` events and writes them to an
external store; it never mutates accounts/inventory/orders/`Traders`, so money/
goods conservation and economy determinism are untouched. The ledger trim only
drops events already durably stored and already outside the snapshot tail window.

## Testing

**sim-core (headless, in-memory store):**
1. `economy_event_store_appends_in_order` — append two batches → events in order;
   `len()` correct.
2. `ledger_audit_flush_appends_new_and_trims` — seed a ledger with N > 1024 events,
   run the pending/commit helpers → store has all N (in order), live ledger trimmed
   to 1024, `cursor == 1024`; a second flush with no new events appends nothing.
3. `ledger_audit_does_not_re_append_after_restart` — set cursor = ledger.len()
   (simulating post-hydrate init) → flush appends nothing.
4. `ledger_audit_failure_is_best_effort` — a failing test store → flush returns the
   error to the caller, cursor NOT advanced, ledger NOT trimmed, sim resources
   otherwise unchanged (no panic).
5. `event_type` maps every `EconomyEvent` variant to a stable non-empty string
   (exhaustive match — compile-forces coverage of new variants).

**sim-server:**
6. Runtime wires an `EconomyEventStore`; a persistence cycle with new ledger events
   appends exactly those events to the store and advances the cursor; a cycle with
   no new events appends none.
7. `EconomyPersistSnapshot` round-trip + economy conservation suites unaffected
   (additive).
8. **Postgres adapter integration test OPT-IN behind an env var** (e.g.
   `ABUTOWN_PG_TESTS=1`), so normal `cargo test --workspace` stays local/in-memory
   and needs no live database.

Full gate: `cargo fmt --check` + clippy `-D warnings` + `cargo test --workspace
--all-targets` + frontend `typecheck`/`vitest` (frontend unchanged) green on the
CI-stable toolchain.

## What this is NOT (deferred)

- **No query/read API** (`GET /economy/events`, analytics) — a later slice. The
  table is indexed so it's query-ready; this slice only writes.
- **No event-sourcing / replay** — the snapshot remains the recovery source; the
  audit log is never read back to reconstruct state.
- **No per-tick DB writes** — appends are batched on the snapshot cadence.
- **No retention/rotation policy** for the durable table (unbounded by design — it
  IS the audit log; rotation/partitioning is a later concern).
- Per-chunk economy snapshot partitioning (the other deferred infra item) is out
  of scope.
