# Economy LOD v0 Design — chunk-LOD-gated market compute

Date: 2026-05-30

## Status

Economy roadmap **slice 4 (Economy LOD)**, foundational sub-slice. The full
deferred slice reads: *"active chunks materialize traders/inventories; warm and
asleep chunks use aggregate flows and pools."* That is genuinely 2–3 independent
subsystems (compute gating, an aggregate warm-flow economic model, and visible
materialization of traders as mobility agents). Per the writing-plans scope rule
we decompose and ship the **load-bearing foundation first**: gate the *cost* of
the economy by chunk LOD so per-tick economy work is proportional to the number
of **observed** markets, not the total market count — exactly the million-agent
roadmap Phase 6 thesis ("LOD makes per-tick work proportional to active-chunk
count, not entity count").

The aggregate warm-flow tier and visible mobility materialization are explicit
follow-on sub-slices (see "Deferred").

## Goal

A market is anchored to the chunk that contains its market node. When that chunk
is **Active** or **Hot** (someone is observing it) the market runs at full
fidelity: pool order generation, trader actions, and auction clearing all run as
today. When the chunk is **Warm** or **Asleep** (unobserved) the market goes
**dormant**: its demand/supply pools generate no orders and its traders take no
actions. Dormant markets move no money and no goods, so conservation is trivially
preserved, and the simulation state on wake is explainable from prior state plus
elapsed time (the v2 chunk invariant).

This is **backend-only**. The economy is not yet replicated to clients (the
debugging/API surface is deferred slice 6), so nothing here crosses the
frontend↔backend boundary and no browser smoke is required.

## Architecture

### Spatial anchoring (decoupled, no per-tick Graph dependency)

The economy core stays free of a routing `Graph` dependency (as the trader slice
did with precomputed `distance_tiles`). Anchoring is a plain lookup resource that
whoever owns the spatial world (a seeder, who *does* have the `Graph`) populates:

```rust
/// MarketId -> the chunk that contains its market node. Populated by the spatial
/// seeder via `chunk_of(graph.node(site.node_id).position, chunk_size)`. Markets
/// absent from this map are treated as un-anchored and ALWAYS simulated (this is
/// what keeps pure-economy unit tests, which never set up a spatial world, at
/// full fidelity).
#[derive(Resource, Default)]
pub struct MarketChunks(pub BTreeMap<MarketId, ChunkCoord>);
```

`ChunkCoord` already derives `Ord`, so it is a valid `BTreeSet`/`BTreeMap` key.

### Derived dormancy (single source of truth, recomputed each tick)

```rust
/// The set of markets that are currently DORMANT — i.e. anchored (present in
/// `MarketChunks`) to a chunk that is NOT Active/Hot. Recomputed every tick by
/// the bridge system. Anything NOT in this set (un-anchored markets, or markets
/// anchored to an Active/Hot chunk) runs at full fidelity.
#[derive(Resource, Default)]
pub struct DormantMarkets(pub BTreeSet<MarketId>);
```

We express the gate as a **dormant** set (not an *active* set) deliberately: a
market that is unknown to the spatial layer (absent from `MarketChunks`, or not
even in `Markets`) is never dormant, so the default everywhere is "simulate".
This makes the change strictly additive and backwards-compatible.

### Bridge system (`refresh_dormant_markets_system`)

A normal `Res`/`ResMut` + `Query` system. It reads the LOD marker components that
the world plugin already maintains on chunk entities and the anchoring map, then
rewrites `DormantMarkets`:

```rust
pub fn refresh_dormant_markets_system(
    anchors: Res<MarketChunks>,
    active_chunks: Query<
        &ChunkCoordComp,
        Or<(With<ActiveChunk>, With<HotChunk>)>,
    >,
    mut dormant: ResMut<DormantMarkets>,
) {
    let active: BTreeSet<ChunkCoord> = active_chunks.iter().map(|c| c.0).collect();
    dormant.0 = anchors
        .0
        .iter()
        .filter(|(_, coord)| !active.contains(coord))
        .map(|(market, _)| *market)
        .collect();
}
```

A one-tick lag relative to LOD reclassification is harmless: LOD transitions have
30-tick hysteresis, and a dormant/awake market merely starts/stops emitting
orders one tick later.

### Gating the expensive work (single code path — `continue`, no second engine)

Two pure helpers gain a `dormant: &BTreeSet<MarketId>` parameter and skip the
work for dormant markets. There is **no parallel aggregate settlement engine** —
dormancy is simply "don't run", which is why conservation is automatic.

- `generate_pool_orders_at_tick`: for each demand/supply pool, `if
  dormant.contains(&pool.market) { continue; }` *before* the interval check, and
  **without** touching `last_generated_tick`. Generation is one-order-per-call by
  construction, so on wake the pool emits exactly one order and resumes its normal
  cadence — there is no backlog burst and no catch-up accounting to get wrong.
- `run_traders_at_tick`: `if dormant.contains(&trader.source) { continue; }`. A
  trader is anchored to its `source` ("home") market; when home is unobserved the
  whole route hibernates, its `TraderState` (including any in-flight travel
  countdown and carried goods/cash) frozen on the `Trader` struct and resumed
  verbatim on wake.

Auction **clearing** needs no explicit gate: a dormant market generates no new
orders, so it is never added to `DirtyMarketGoods`, so it is never cleared.

Order **expiry** and EWMA **telemetry** stay ungated: both are cheap, and letting
a dormant market's pre-existing orders expire (releasing their locked
cash/goods back to `available`) is correct hygiene and stays conserved.

### Schedule

A new set `EconomySet::RefreshLod` runs first in the economy chain so the
dormant set is fresh before any gated work:

```
RefreshLod -> ExpireOrders -> Production -> Traders -> GeneratePoolOrders -> ClearMarkets -> Telemetry
```

`refresh_dormant_markets_system` runs in `RefreshLod`. `generate_pool_orders_system`
and `run_traders_system` additionally take `Res<DormantMarkets>` and forward
`&dormant.0`. `EconomyPlugin` inserts `MarketChunks::default()` and
`DormantMarkets::default()`.

## Conservation / determinism

- **Money & goods conserved across LOD transitions:** a dormant market runs no
  pool generation and no trader actions, so it neither creates nor destroys
  orders, cash, or goods. Sum-of-all-accounts and sum-of-all-goods are invariant
  across going-dormant and waking.
- **Deterministic:** dormancy is a pure function of chunk LOD marker components
  and the `BTreeMap` anchoring; the dormant set is a `BTreeSet`; gating is a set
  membership test. No rng, wall-clock, or float.
- **Wake is explainable (v2 invariant):** a woken market resumes from frozen
  state; pools emit one order and continue cadence; traders continue their stored
  state machine. No state is invented on wake.

## Testing

1. `refresh_dormant_markets_marks_only_anchored_inactive`: spawn chunk entities
   with `ChunkCoordComp` + each marker (Asleep/Warm/Active/Hot); anchor four
   markets; run the bridge; assert exactly the Asleep/Warm-anchored markets are
   dormant and the Active/Hot ones are not.
2. `unanchored_market_is_never_dormant`: a market with no `MarketChunks` entry is
   never added to `DormantMarkets`, even with no active chunks in the world.
3. `dormant_market_generates_no_orders`: a supply+demand pool whose market is in
   `DormantMarkets` produces zero orders from `generate_pool_orders_at_tick`;
   total money and goods unchanged.
4. `awake_market_still_generates_orders`: the same pools with an empty dormant set
   generate orders exactly as before (gating is opt-in).
5. `market_resumes_with_single_order_no_burst`: a pool dormant for N ticks then
   awake emits exactly one order on the wake tick (not N), and resumes cadence.
6. `dormant_trader_is_frozen_and_conserves`: a trader whose `source` is dormant
   takes no action across several ticks; its state and balances are unchanged;
   money + goods conserved. With the source awake it proceeds normally.
7. `dormant_gating_is_deterministic`: two worlds driven through identical LOD
   transitions produce identical `TradeLedger`s.
8. End-to-end via the full schedule (Core + Mobility + Economy plugins): a market
   anchored to an asleep chunk stays frozen while an un-anchored (or active)
   market trades; total money conserved across the run; `EconomyPlugin` installs
   both new resources.

Full gate: existing economy/production/transport/trader suites unaffected
(they never populate `MarketChunks`, so nothing is dormant); fmt + clippy
`-D warnings` + `test --workspace --all-targets` green.

## What this is NOT (deferred sub-slices)

- **No aggregate warm-flow economic model.** Warm/asleep markets freeze; they do
  not evolve via a gravity/flow model. A later sub-slice can give the warm tier a
  cheap aggregate update (mirroring mobility's `warm_chunk_flow_system`).
- **No visible materialization.** Traders remain abstract; they are not spawned as
  walking mobility agents in active chunks (that crosses the render boundary and
  is its own slice).
- **No new replication.** `MarketChunks`/`DormantMarkets` are internal backend
  resources; nothing new is sent over the wire.
- No per-good dormancy (dormancy is per-market), no partial-LOD blending, no
  change to player-visible behavior inside Active/Hot chunks.

## Open questions (resolved)

1. Anchoring without a per-tick Graph dependency → precomputed `MarketChunks`
   lookup, populated by the spatial seeder. Resolved.
2. Backwards compatibility with pure-economy tests (no spatial world) →
   express the gate as a *dormant* set; absence ⇒ simulate. Resolved.
3. Backlog burst on wake → none; generation is one-order-per-call. Resolved.
4. Bridge query surface → `Query<&ChunkCoordComp, Or<(With<ActiveChunk>,
   With<HotChunk>)>>` against the existing world marker components. Resolved.
