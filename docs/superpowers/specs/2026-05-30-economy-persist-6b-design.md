# Economy Persistence 6b Design — durable store + wiring + debug view

Date: 2026-05-30

## Status

Economy roadmap **slice 6 (Persistence/API)**, final sub-slice **6b**. Slice 6a
made the economy serde-round-trippable (`EconomyPersistSnapshot`,
`extract_from_world`/`apply_into_world`, `EconomySnapshotProvider`). 6b makes the
economy **durably persisted and restored across restart**, mirroring the existing
mobility-snapshot path end-to-end, and exposes a backend-only **`GET /economy`**
debugging view. This closes the economy roadmap.

`EconomyPlugin` and `PersistencePlugin` are already installed in the server
runtime (both the seed and hydrate paths), so the economy resources exist and run
each tick; 6b only adds the durable save/restore + view.

## Boundary / browser-smoke

Backend-only. The store, persist loop, and hydration are pure backend. The
`/economy` endpoint returns **JSON** (`serde_json` of `EconomyPersistSnapshot`,
which already derives `Serialize`) — a debugging view in the same family as the
existing backend-only `/mobility` and `/chunks` endpoints. No protocol/protobuf
change, no generated-TS change, no frontend consumes it. The CLAUDE.md
frontend↔backend browser-smoke trigger does **not** fire.

## sqlx / CI

The repo uses **runtime** `sqlx::query(...)` / `sqlx::query_as(...)` (no
compile-checked macros, no `.sqlx/` offline data, no `SQLX_OFFLINE`). A new
economy query needs no offline regeneration. Migrations are executed **inline**
in the Postgres adapter's `connect()` (via `include_str!` + split on `;`), not a
`sqlx::migrate!` runner. CI has no Postgres, so the Postgres path is
compile-only; all behavior tests use the in-memory store.

## Architecture (mirror mobility exactly)

### Store trait + adapters

In `sim-core/src/persistence.rs`, mirroring `MobilitySnapshotStore`:

```rust
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct EconomySnapshotStoreError { message: String }
impl EconomySnapshotStoreError { pub fn unavailable(message: impl Into<String>) -> Self { … } }

#[async_trait]
pub trait EconomySnapshotStore: std::fmt::Debug + Send + Sync {
    async fn write(&mut self, world_id: &str, tick: u64, snapshot: &EconomyPersistSnapshot,
                   compatibility: &SnapshotCompatibility) -> Result<(), EconomySnapshotStoreError>;
    async fn read(&self, world_id: &str, compatibility: &SnapshotCompatibility)
        -> Result<Option<(u64, EconomyPersistSnapshot)>, EconomySnapshotStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryEconomySnapshotStore { /* HashMap<(String, SnapshotCompatibility), (u64, EconomyPersistSnapshot)> */ }
```

`PostgresEconomySnapshotStore` (new `sim-server/src/postgres_economy.rs`) mirrors
`PostgresMobilitySnapshotStore`: one row per `world_id` (UPSERT on conflict),
`payload JSONB`, `base_world_id` / `base_world_schema_version` columns, runtime
`sqlx::query`. `connect()` runs the inline migration.

Migration `sim-server/migrations/202605300001_economy_snapshots.sql`:

```sql
CREATE TABLE IF NOT EXISTS economy_snapshots (
    world_id TEXT PRIMARY KEY,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    base_world_id TEXT,
    base_world_schema_version INTEGER,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS economy_snapshots_base_world_idx
  ON economy_snapshots (world_id, base_world_id, base_world_schema_version);
```

### Provider registration

`PersistencePlugin::install` pushes `EconomySnapshotProvider { world_id }` into
`SnapshotProviders` (one line), so `collect_provider_items()` emits an
`"economy"` item alongside `"chunk"`/`"mobility"`.

### Write path (persist loop)

`PersistPayload` gains `economy_tick: u64` and `economy_world:
EconomyPersistSnapshot`. The `CollectPersistData` handler's provider dispatch
gains an `"economy"` arm (deserialize the item into `EconomyPersistSnapshot`).
`persist_snapshots_once` gains an economy write block mirroring the mobility one
(lock the economy store, `write(world_id, economy_tick, &economy_world,
&compatibility)`; failure is logged, not fatal — same posture as a bad mobility
snapshot). No agent-count guard (economy-specific validity is out of scope).

### Restore path (hydration)

`hydrate_from_stores` takes a 4th store
(`economy_snapshot_store: Box<dyn EconomySnapshotStore + Send + Sync>`) and
returns it in the tuple. After `apply_into_world(&mut world, mobility_snap)` it
reads the economy store for `world_id` + compatibility; on a hit it calls
`sim_core::economy::apply_into_world(&mut world, &snap)` (fully-qualified — the
mobility `apply_into_world` is already in scope). On miss / read error it leaves
the freshly-installed default economy (a fresh world has an empty economy, which
is correct — the economy seeds from pools at runtime). The compatibility filter
guarantees a stale-base snapshot is ignored, consistent with mobility.

### Debug view (`GET /economy`)

A `Mutation::CollectEconomySnapshot { reply: oneshot::Sender<EconomyPersistSnapshot> }`
is handled in the tick task by `reply.send(runtime.economy_snapshot())`
(`SimulationRuntime::economy_snapshot(&self) -> EconomyPersistSnapshot` wraps
`sim_core::economy::extract_from_world(&self.world)`). The `/economy` handler
sends the mutation, awaits, and returns the snapshot as JSON. On-demand (debug
endpoints are rare) — not materialized into `RuntimeReadView` every tick, which
keeps the per-tick view cheap and avoids touching the read-view struct.

### AppState threading

`AppState` gains `economy_snapshot_store: Arc<Mutex<Box<dyn EconomySnapshotStore
+ Send + Sync>>>` + an accessor, exactly like `mobility_snapshot_store`.
`new_with_stores` gains an economy-store parameter; `new` /
`new_with_card_hands` pass `Box::new(InMemoryEconomySnapshotStore::default())`;
`build_app_from_config` builds and threads the Postgres store through
`hydrate_from_stores` and `new_with_stores`.

This explicit threading (rather than a default-and-swap shortcut) is the same
pattern chunk/mobility stores already use — consistent and compiler-checked.

## Determinism / conservation

- Persist/restore is the round-trip proven in 6a; a save→restore cycle reinstates
  every persisted resource verbatim → money and goods conserved across restart.
- Compatibility-gated reads ignore stale-base snapshots (no cross-world bleed).
- Write/collect order is deterministic (`BTreeMap`-sourced `Vec`s from 6a).

## Testing

In-memory, runs in CI (Postgres path is compile-only):
1. `InMemoryEconomySnapshotStore` write→read round-trips; compatibility-mismatch reads return `None`.
2. **Persist write**: build `AppState` with an in-memory economy store, seed
   economy resources, drive `persist_snapshots_once`, assert the store now holds
   the economy snapshot equal to the seeded state.
3. **Restore**: pre-load an in-memory economy store with a snapshot, call
   `hydrate_from_stores` with it, assert the runtime's economy state equals the
   snapshot (via `runtime.economy_snapshot()`); and a fresh/empty store hydrates
   to the default empty economy.
4. **Round-trip through the loop**: seed → persist → hydrate a new runtime from
   the same store → economy state matches (money + goods conserved).
5. **`GET /economy`**: returns 200 JSON that deserializes to the current
   `EconomyPersistSnapshot`.
6. All existing sim-server tests still pass after the signature threading
   (compiler-guided update of every `hydrate_from_stores` / `new_with_stores` /
   `PersistPayload` call site).

Full gate: fmt + clippy `-D warnings` + `test --workspace --all-targets`.

## What this is NOT

- No economy event-log persistence (the `TradeLedger` is transient telemetry).
- No per-chunk economy snapshot partitioning (single per-world row, like
  mobility v0).
- No protobuf/WS exposure of the economy (JSON debug endpoint only).
- No new economy validity guard on persist (mobility's agent-count guard has no
  economy analogue in v0).

## Open questions (resolved)

1. Ripple vs cleanliness → thread the store explicitly (consistent with
   chunk/mobility); compiler verifies all ~15 call sites. Resolved.
2. Debug view cost → on-demand `Mutation`, not per-tick `RuntimeReadView`.
   Resolved.
3. `apply_into_world` name clash (mobility vs economy) → fully-qualify
   `sim_core::economy::apply_into_world`. Resolved.
4. sqlx/CI → runtime queries, inline migration, in-memory tests; no hazard.
   Resolved.
