# Economy Persistence v0 Design — snapshot serialization + SnapshotProvider (slice 6a)

Date: 2026-05-30

## Status

Economy roadmap **slice 6 (Persistence/API)**, foundational sub-slice **6a**. The
full deferred slice reads: *"snapshot economy books and expose debugging views
only after v0 invariants are stable."* That is two independent pieces: (a) make
the economy state **serializable + round-trippable** and expose it through the
existing `SnapshotProvider` hook (sim-core only, backend, no I/O), and (b) the
**durable Postgres store + persist-loop/hydration wiring + a debug HTTP view**
(sim-server, migrations, runtime). This spec is **6a**; 6b is an explicit
follow-on.

6a is entirely within `sim-core/src/economy/` plus one trivial derive on
`routing::NodeId`. No migration, no `sim-server`, no wire protocol — fully
round-trip-testable in isolation.

## Goal

A deterministic, serde-serializable `EconomyPersistSnapshot` that captures the
authoritative mutable economy state, with `extract_from_world` (live `World` →
snapshot) and `apply_into_world` (snapshot → freshly-installed `World`) that
round-trip exactly, plus an `EconomySnapshotProvider` implementing the existing
`crate::world::persistence::SnapshotProvider` trait (`kind = "economy"`,
`schema_version = 1`) so the economy plugs into the same persistence machinery as
mobility/chunks.

## What is persisted (authoritative resumable state)

- `AccountBook` (`BTreeMap<EconomicActorId, MoneyAccount>`)
- `InventoryBook` (`BTreeMap<(EconomicActorId, GoodId), InventoryBalance>`)
- `OrderBook` (`bids: BTreeMap<OrderId, Bid>`, `asks: BTreeMap<OrderId, Ask>`) + `NextOrderId`
- `Markets` (`BTreeMap<MarketId, MarketSite>`), `MarketGoods` (`BTreeMap<MarketGoodKey, MarketGoodState>`)
- `DemandPools`, `SupplyPools`, `ProductionPools` (`BTreeMap<EconomicActorId, …>`) — incl. `last_generated_tick`
- `Traders` (`BTreeMap<EconomicActorId, Trader>`) — incl. each `TraderState`
- `MarketChunks` (`BTreeMap<MarketId, ChunkCoord>`) — LOD anchoring

## What is NOT persisted (and why)

- `TradeLedger` — append-only audit/telemetry event log; transient and unbounded.
  Not needed to resume deterministically (mobility likewise does not snapshot its
  event log). The durable event stream is a separate concern.
- `DormantMarkets` — derived every tick by `refresh_dormant_markets_system` from
  chunk LOD; recomputed on resume.
- `DirtyMarketGoods` — within-tick scratch; cleared each `ClearMarkets`.
- `EconomyConfig` — configuration, re-injected by `EconomyPlugin` at boot.

Each exclusion is safe under the v2 "state explainable on wake" invariant: the
excluded resources are either re-derived or re-injected, never authoritative
accumulated state.

## Architecture

### Serde derives on economy value types

The economy types are plain value structs (not entity-backed components), so they
serialize directly — no parallel `Persisted*` record types are warranted (the
mobility `AgentRecord`/`VehicleRecord` split exists only because mobility state
lives in ECS *entities*). Add `Serialize, Deserialize` to: `Money`, `Quantity`;
`GoodId`, `MarketId`, `OrderId`, `EconomicActorId`; `MoneyAccount`;
`InventoryBalance`; `Bid`, `Ask`; `MarketSite`, `MarketGoodKey`,
`MarketGoodState`; `DemandPool`, `SupplyPool`; `Recipe`, `ProductionPool`;
`TraderState`, `Trader`. `ChunkCoord` already derives serde.

`MarketSite` carries a `routing::NodeId`, which does **not** derive serde today.
Add `Serialize, Deserialize` to `NodeId` (a `u32` newtype) — the single
cross-module touch, justified because a market is anchored to a routing node and
that id must round-trip. `MarketSite` and `MarketGoodState` currently carry no
derives at all; give them `Debug, Clone, PartialEq, Serialize, Deserialize`, and
add `Debug, Clone, PartialEq` to the `Markets` / `MarketGoods` / `MarketChunks`
resource wrappers so round-trip equality is assertable.

### `EconomyPersistSnapshot` (byte-stable, map-key-safe)

`serde_json` rejects non-string map keys (`InventoryBook`'s `(actor, good)` tuple
key and `MarketGoods`' `MarketGoodKey` struct key both fail as JSON object keys).
Following the mobility precedent (which serializes `HashMap<ChunkCoord, _>` as a
sorted `Vec<(K, V)>`), every map is represented as a `Vec<(K, V)>`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EconomyPersistSnapshot {
    pub accounts: Vec<(EconomicActorId, MoneyAccount)>,
    pub inventory: Vec<((EconomicActorId, GoodId), InventoryBalance)>,
    pub bids: Vec<(OrderId, Bid)>,
    pub asks: Vec<(OrderId, Ask)>,
    pub next_order_id: u64,
    pub markets: Vec<(MarketId, MarketSite)>,
    pub market_goods: Vec<(MarketGoodKey, MarketGoodState)>,
    pub demand_pools: Vec<(EconomicActorId, DemandPool)>,
    pub supply_pools: Vec<(EconomicActorId, SupplyPool)>,
    pub production_pools: Vec<(EconomicActorId, ProductionPool)>,
    pub traders: Vec<(EconomicActorId, Trader)>,
    pub market_chunks: Vec<(MarketId, ChunkCoord)>,
}
```

`extract_from_world(world)` reads each resource and collects its `BTreeMap` (which
iterates in sorted key order — deterministic, byte-stable) into the matching
`Vec`. `apply_into_world(world, &snap)` rebuilds each `BTreeMap` from its `Vec`
via `into_iter().collect()` and `world.insert_resource(…)` (overwriting the
defaults that `EconomyPlugin` installed). `DormantMarkets` is left at its default
(it is recomputed by the bridge on the next tick).

### `EconomySnapshotProvider`

Mirrors `MobilitySnapshotProvider`:

```rust
pub struct EconomySnapshotProvider { pub world_id: String }

impl SnapshotProvider for EconomySnapshotProvider {
    fn name(&self) -> &'static str { "economy" }
    fn schema_version(&self) -> u32 { 1 }
    fn collect(&self, world: &World) -> Vec<SnapshotItem> {
        let snap = extract_from_world(world);
        let payload = serde_json::to_vec(&snap).expect("serde encodes EconomyPersistSnapshot");
        vec![SnapshotItem {
            key: SnapshotKey { world_id: self.world_id.clone(), kind: "economy", identifier: "full".to_string() },
            schema_version: 1,
            payload,
        }]
    }
    fn migrate(&self, raw: SnapshotItem, _from: u32) -> Result<SnapshotItem, MigrationError> { Ok(raw) }
}
```

Registration into `SnapshotProviders` (done by `sim-server`'s `PersistencePlugin`)
and the durable store/loop are **6b**; 6a defines and unit-tests the provider.

## Determinism / conservation

- **Deterministic:** all source maps are `BTreeMap`; `Vec`s are built in sorted
  key order → JSON bytes are byte-stable across runs.
- **Round-trip exact:** `apply(extract(w))` reproduces every persisted resource
  (asserted by `PartialEq`); a no-op snapshot/restore conserves all money and
  goods because it reinstates the books verbatim.
- **No behavior change:** purely additive (derives + a new module); no system,
  schedule, or matching logic is touched.

## Testing (sim-core only)

1. `economy_snapshot_round_trips`: install `EconomyPlugin`; seed accounts,
   inventory, a bid + an ask, a market + market-good, demand/supply/production
   pools, a trader, a market-chunk anchor; `extract` → `serde_json::to_vec` →
   `from_slice` → `apply` into a *fresh* `EconomyPlugin` world; assert each
   persisted resource equals the original.
2. `economy_snapshot_is_byte_stable`: two `extract`s of the same world serialize
   to identical bytes.
3. `empty_economy_round_trips`: a freshly-installed (empty) economy snapshots and
   restores to an equal empty state.
4. `provider_collects_single_economy_item`: `EconomySnapshotProvider::collect`
   returns one `SnapshotItem` with `kind == "economy"`, `identifier == "full"`,
   `schema_version == 1`, non-empty payload that deserializes back to the same
   `EconomyPersistSnapshot`.

Full gate: existing economy/auction/conservation/pools/traders/production/
transport/LOD suites unaffected (additive derives only); fmt + clippy
`-D warnings` + `test --workspace --all-targets` green.

## What this is NOT (deferred to 6b)

- No `EconomySnapshotStore` trait, no Postgres adapter, no migration.
- No persist-loop or hydration wiring in `sim-server`/runtime.
- No `/economy` debug HTTP endpoint (will be backend-only when added — see the
  persistence exploration: a new HTTP route mirroring `/mobility`, no
  frontend↔backend boundary crossing, so no browser smoke even in 6b).
- No event-log persistence.

## Open questions (resolved)

1. Non-string JSON map keys → represent all maps as sorted `Vec<(K,V)>` (mobility
   precedent). Resolved.
2. `NodeId` lacks serde → add the derive (one-line, additive). Resolved.
3. Round-trip equality on `Markets`/`MarketGoods` → add `PartialEq` + serde to
   their value types and `PartialEq` to the wrappers. Resolved.
4. What to persist → authoritative mutable state only; exclude ledger/derived/
   config. Resolved.
