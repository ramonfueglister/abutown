# Per-tick cost reduction + economy-event retention — design

Date: 2026-06-10

## Status

Follow-up to the 2026-06-09 abutopia live outage. PR #91 (`60043ec`) fixed the
*symptom* (a saturated tick loop starved `/health`, tripping Fly's health check)
by yielding once per tick and switching the ticker to `MissedTickBehavior::Delay`.
Two underlying problems remained and are the likely cause of the OOM kill
(exit 137, `oom_killed=true`) on the 1 gb `shared-cpu-1x` machine:

1. **Per-tick work is ~80 % waste.** The simulation step is cheap; the published
   read-view rebuild dominates and is recomputed in full every 100 ms even though
   almost all of it is read only on-demand.
2. **`economy_events` grows without bound** (449 MB / 1.9 M rows locally after a
   few days), with no retention. The durable table is a Supabase-side cost; the
   *in-memory* ledger only grows unbounded under sustained flush failure, which
   Problem 1 makes far more likely by starving the persistence task.

This change is **backend-only** and makes **no wire/proto/frontend change** and
**no economy-mechanism change**: event *emission*, the in-memory `TradeLedger`,
conservation, and determinism are all untouched. Only (a) how the read-view is
*materialized* and (b) which already-emitted events are *durably retained* change.

## Evidence (measured, not assumed)

`profile_tick_phases` (release, abutopia fixture, 300 agents / 28 chunks, median
ms over 40 iters — see `app/tests.rs`):

```
tick_world_mobility (the actual sim) ....   0.557
build_read_view_from_runtime (FULL) .....   4.661   ← 8.4× the sim
  └ chunk loop (tile + mobility) ........   4.237   ← 81 % of the whole tick
  └ mobility_full_dto ...................   0.252
  └ economy_snapshot ....................   ~0.000  ← the assumed suspect: negligible
  └ world_summary / subscriber_counts ...   ~0.000
```

Two independent costs inside the chunk loop, both removable without any staleness:

- **Tile snapshots** — `chunk_snapshot(coord)` re-reads ~1 024 tiles and re-encodes
  a proto for *every* loaded chunk every tick. Tiles change only on a `SetTileKind`
  command (rare) or chunk (un)load, so this is almost pure waste.
- **Mobility snapshots** — `build_mobility_chunk_snapshot` collects *all* agents and
  filters, once per chunk: **O(chunks × agents)** ≈ 28 × 518 ≈ 14.5 k agent-scans
  per tick in production, and it gets worse as the (now density-bounded, #89)
  population grows.

`economy_events` row mix (1.9 M rows): `regenerated`+`consumed`+`produced` = 45 %,
`order_created`+`order_expired` = 23 %, `cash_locked`+`cash_released` = 20 %. The
financially meaningful events are a rounding error (`wage_paid` 3.4 k,
`profit_distributed` 21 k, `transport_rebate` 299).

## Part A — incremental read-view rebuild

`build_read_view_from_runtime` gains a `prev: Option<&RuntimeReadView>` argument.

### A1. Tile snapshots: version-gated reuse

`RuntimeReadView.chunk_snapshots` becomes `HashMap<ChunkCoord, Arc<w::ChunkSnapshot>>`.
For each loaded chunk, compare the runtime's current `ChunkVersion` (new cheap
`SimulationRuntime::chunk_version(coord)`) with the version carried by the previous
view's cached proto. **Unchanged ⇒ reuse the `Arc` (no tile read, no encode, no
clone). Changed/new ⇒ rebuild once and wrap in a fresh `Arc`.** Correctness is exact:
a changed chunk is always rebuilt that tick (its `ChunkVersion` bumped).

### A2. Mobility snapshots: single bucketing pass

New sim-core API `build_mobility_chunk_snapshots(world, &[ChunkCoord]) ->
HashMap<ChunkCoord, MobilityChunkSnapshot>` iterates agents/vehicles **once**,
bucketing each by `chunk_of(pos)`, then materializes an entry (possibly empty) for
every requested chunk — same output as N calls to `build_mobility_chunk_snapshot`,
at **O(agents + chunks)** instead of O(chunks × agents). No staleness: rebuilt every
tick, just without the quadratic. (The per-chunk function stays for callers that want
a single chunk.)

`mobility_full_dto` (0.25 ms, already O(agents)) is left as-is.

Net: the chunk loop drops from ~4.24 ms to ~0.3 ms; total tick ~5.2 ms → ~1.1 ms
(~80 %), and — just as important for the OOM — the per-tick multi-MB allocation
churn collapses, since unchanged tile snapshots are now shared `Arc`s rather than
freshly built-and-dropped every 100 ms.

## Part B — economy-event retention ("window + trim writes")

Per the [audit-store design](2026-05-31-economy-audit-store-design.md), the table is
an *observability* log (NOT recovery; snapshots remain the source of truth) and
retention/query were explicitly deferred. This slice adds retention.

### B1. Trim writes — durable-event class

A sim-core classifier `EconomyEvent::is_audit_durable(&self) -> bool` partitions
already-emitted events into durable vs. transient. **Durable** (kept): the
conservation heartbeat `TickAudit`, and money-settlement outcomes — `WagePaid`,
`ProfitDistributed`, `TransportRebate`, `FinalConsumed`, `OrderRejected`.
**Transient** (not written): high-frequency intra-tick mechanics — `Regenerated`,
`Consumed`, `Produced`, `OrderCreated`, `OrderExpired`, `CashLocked`, `CashReleased`,
`GoodsLocked`, `GoodsReleased`, `MacroFlow`. This removes ~88 % of rows while keeping
a meaningful financial audit trail.

The persist loop (`persist_snapshots_once`, Phase 2d) filters `pending` to the durable
subset before `append`, but **commits the full `pending` count** so the
`LedgerAuditCursor` advances past every consumed event and the in-memory ledger still
trims (the latent-leak guard from the audit-store slice is preserved). When the durable
subset is empty, the cursor still advances (nothing durable to lose).

### B2. Rolling retention — row cap

`EconomyEventStore` gains `prune(world_id, keep_last)`:
- Postgres: a single index-friendly `DELETE` keyed on the existing `(world_id, id)`
  index — find the id of the `(keep_last+1)`-th newest row via `ORDER BY id DESC
  OFFSET keep_last LIMIT 1` and delete everything at/below it. **No schema migration,
  no new index.**
- In-memory: truncate the front of the per-world `Vec` to the last `keep_last`.

A dedicated low-frequency `spawn_retention_loop` (default every 5 min) calls `prune`
for the world with a configurable cap (`ABUTOWN_ECONOMY_EVENTS_RETENTION_CAP`,
default 200 000 rows). Combined with B1, the table is bounded.

### Not changed / not needed

- No `economy_snapshots` schema change — `ledger_tail` (last 1024) is untouched, so
  **no `DELETE FROM economy_snapshots`** before deploy.
- No `economy_events` schema migration.
- **Operational note for the user:** the retention prune executes at runtime against
  the configured DB, including prod Supabase. The first prune against the existing
  ~1.4 M-row backlog is a single large `DELETE`. Recommended: run a one-time, bounded
  manual cleanup on prod (e.g. delete in `id`-range batches) rather than letting the
  first automatic prune delete the whole backlog at once. Do **not** run destructive
  SQL against prod from this session.

## Why not the alternatives

- *Throttle the full rebuild every N ticks* — still does O(world) work, just less
  often, and introduces read staleness. A1/A2 remove the cost with zero staleness.
- *Lazy/on-demand view build* — architecturally cleaner but a larger refactor of the
  read-view boundary; deferred.
- *Stop emitting the noisy events* — that **is** an economy-mechanism change (the
  events feed the conservation audit and the snapshot tail). We only change durable
  retention, leaving the economy bit-identical.

## Literature / grounding

This is infrastructure (observability retention + a rebuild-avoidance cache), not an
economic mechanism, so no academic-economics citation is warranted; the relevant
precedents are internal: the [audit-store design](2026-05-31-economy-audit-store-design.md)
(retention deferred; observability-not-recovery; in-memory-ledger bound) and the
read-view materialization introduced in
[app-runtime-refactor](2026-05-27-app-runtime-refactor-design.md). Row-cap retention
and high-cardinality-event trimming are standard structured-logging/observability
practice (bounded retention + sampling of low-value high-frequency records).

## Test plan (TDD)

- sim-core: `build_mobility_chunk_snapshots(world, coords)` equals N per-chunk calls
  (same agents/vehicles per chunk, empty entry for agent-less loaded chunks).
- sim-core: `is_audit_durable` classification table (one assert per variant).
- sim-core: `InMemoryEconomyEventStore::prune` keeps exactly the last N.
- read-view: tile snapshot `Arc` is **reused** across ticks when `ChunkVersion` is
  unchanged (pointer-eq) and **replaced** after a `SetTileKind` mutation.
- persist: durable filter writes only the durable subset yet advances the cursor past
  all pending (ledger trims; transient events never re-pend).
- Postgres prune: opt-in integration test (gated on `ABUTOWN_TEST_DATABASE_URL`).
- Re-run `profile_tick_phases` before/after to confirm the chunk-loop collapse.
```
