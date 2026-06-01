# Economy Slice 1 — Macro Demand-Driven Flow Implementation Plan
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Replace the warm-only intra-market frozen-price flow (#59) with a deterministic, conservation-exact macro layer that runs for **every** dormant market (warm AND asleep), flows goods demand-driven across markets net of transport cost, and writes discovered prices back so dormant markets converge toward spatial equilibrium and never go hollow.

**Architecture:** Active/hot markets keep the full order-book auction untouched; all dormant markets (the `DormantMarkets` set, recomputed each tick from chunk LOD) are governed by a new mean-field `run_macro_flow_at_tick` that classifies surplus/deficit per (market,good) from `DemandPools`/`SupplyPools`, routes surplus→deficit when the price gap strictly exceeds transport, settles conservation-exactly via a conditional clone-validate-apply boundary, and writes back `MarketGoodState.last_settlement_price`. Distances come from a new persisted `MarketDistances` resource (the per-tick core stays graph-free); the flow runs in the renamed `EconomySet::MacroFlow` slot after `ClearMarkets`, before `Telemetry`, with a new `EconomySet::RefreshLod.after(CoreSet::LodReclassify)` ordering so the stateful flow's mutated-market set is a deterministic function of LOD classification.

**Tech Stack:** Rust, `bevy_ecs`, fixed-point i64 money/quantity (`ECONOMY_SCALE = 1000`) with i128 checked arithmetic, serde-JSON snapshots, `criterion` benches (`harness = false`). Tests live in `backend/crates/sim-core/src/economy/tests/` (registered in `tests/mod.rs`). Every cargo run/test/bench step uses the isolated-target template (CLAUDE.md mandates serializing cargo through `scripts/cargo-serial.sh`):
`TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <filter>`
All paths are relative to `/Users/ramonfuglister/Coding/abutown-vtraders`. This is backend-only: **no render/WebSocket/frontend wiring changes**, so the CLAUDE.md browser-smoke mandate does NOT apply. Commit after each task only when the user/orchestrator asks; otherwise keep the tree green and ready.

---

## File Structure

| File | C/M/D | Responsibility |
| --- | --- | --- |
| `backend/crates/sim-core/src/economy/market.rs` | M | Add `MarketDistances(pub BTreeMap<(MarketId, MarketId), i64>)` resource (directed both-ways, Manhattan tiles); later remove `WarmMarkets`. |
| `backend/crates/sim-core/src/economy/mod.rs` | M | Register `MarketDistances` in `EconomyPlugin::install`; `pub mod macro_flow;` + re-export; later drop `pub mod warm_flow;` / `WarmMarkets` registration. |
| `backend/crates/sim-core/src/economy/persist.rs` | M | Add `market_distances: Vec<((MarketId, MarketId), i64)>` (14th field) to `EconomyPersistSnapshot` + extract/apply wiring. |
| `backend/crates/sim-core/src/economy/seed.rs` | M | Bake `MarketDistances` (both directions) from the `Graph`; add the minimal live-seed second good. |
| `backend/crates/sim-core/src/economy/ledger.rs` | M | Add `EconomyEvent::MacroFlow {…}` + `"macro_flow"` arm; later delete `WarmMarketFlow` + its arm. |
| `backend/crates/sim-core/src/economy/systems.rs` | M | Add `EconomySet::RefreshLod.after(CoreSet::LodReclassify)`; later rename `WarmFlow`→`MacroFlow` + `warm_flow_interval_ticks`→`macro_flow_interval_ticks`, wire `run_macro_flow_system`, strip `WarmMarkets` from `refresh_dormant_markets_system`. |
| `backend/crates/sim-core/src/economy/macro_flow.rs` | **C** | NEW: `run_macro_flow_at_tick` — STEP A–I demand-driven cross-market flow. |
| `backend/crates/sim-core/src/economy/warm_flow.rs` | **D** | Deleted in Task 12 (the atomic clean replacement). |
| `backend/crates/sim-core/src/economy/transport.rs` | (reused) | `transport_cost`/`transport_cost_between`/`manhattan_tiles` reused verbatim (first prod consumer of `transport_cost_between`). |
| `backend/crates/sim-core/src/economy/tests/mod.rs` | M | `mod warm_flow;` → `mod macro_flow;`. |
| `backend/crates/sim-core/src/economy/tests/macro_flow.rs` | **C** | NEW: direct `run_macro_flow_at_tick` unit/conservation/determinism/convergence tests + `macro_flow_world()` builder. |
| `backend/crates/sim-core/src/economy/tests/warm_flow.rs` | **D** | Deleted in the atomic replacement. |
| `backend/crates/sim-core/src/economy/tests/lod.rs` | M | Strip `WarmMarkets` (import :6, inserts :27/:44); invert `asleep_anchored_market_stays_frozen_end_to_end`→`asleep_anchored_market_DOES_flow`. |
| `backend/crates/sim-core/src/economy/tests/persist.rs` | M | Insert `MarketDistances` into the inline `seed()`; extend round-trip/byte-stable for `market_distances`. |
| `backend/crates/sim-core/src/economy/tests/seed.rs` | M | Insert `MarketDistances::default()` into the inline test world before `seed_demo_economy`; add distance assertions. |
| `backend/crates/sim-core/src/economy/tests/systems.rs` | M | Schedule-ordering test for `RefreshLod.after(LodReclassify)` (Task 5) + wired macro-flow chain test. |
| `backend/crates/sim-core/benches/economy_flow.rs` | **C** | NEW: isolated `run_macro_flow_at_tick` bench (`macro_flow_2m_2g`, `macro_flow_10k_pools_scale`). |
| `backend/crates/sim-core/benches/economy_tick.rs` | **C** | NEW: schedule-level `economy_tick` bench over N ticks incl. non-flow ticks. |
| `backend/crates/sim-core/Cargo.toml` | M | Two new `[[bench]]` entries (`economy_flow`, `economy_tick`), `harness = false`. |

---

## Section A — Additive scaffolding (Tasks 1–5)

> These tasks are **additive only**: they ADD `MarketDistances`, the persist field, the seed bake, the `MacroFlow` event, and the LOD ordering **without** deleting `WarmMarketFlow`, renaming the config/set, or removing `WarmMarkets`/`warm_flow.rs`. The build compiles and all existing tests pass after every step. The atomic clean replacement happens later (Task 12 (the atomic clean replacement)).

### Task 1: Add the `MarketDistances` resource + register it

**Files:** `backend/crates/sim-core/src/economy/market.rs`, `backend/crates/sim-core/src/economy/mod.rs`, `backend/crates/sim-core/src/economy/tests/lod.rs` (new test) or a new inline test module — use `tests/lod.rs` (it already imports market resources).

- [ ] Write a failing test in `backend/crates/sim-core/src/economy/tests/lod.rs` (append at end of file). It inserts directed pairs into a `MarketDistances` and reads them back both ways:
  ```rust
  #[test]
  fn market_distances_stores_directed_pairs_both_ways() {
      use crate::economy::MarketDistances;
      let mut d = MarketDistances::default();
      d.0.insert((MarketId(1), MarketId(2)), 7);
      d.0.insert((MarketId(2), MarketId(1)), 7);
      assert_eq!(d.0.get(&(MarketId(1), MarketId(2))).copied(), Some(7));
      assert_eq!(d.0.get(&(MarketId(2), MarketId(1))).copied(), Some(7));
      assert_eq!(d.0.get(&(MarketId(1), MarketId(3))).copied(), None);
  }
  ```
- [ ] Run it, expect **FAIL to COMPILE** (`MarketDistances` does not exist / not exported):
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core market_distances_stores_directed_pairs_both_ways`
- [ ] Add the resource to `backend/crates/sim-core/src/economy/market.rs` (after `DormantMarkets`, before `WarmMarkets`). The `(MarketId, MarketId)` tuple is `Ord` because `MarketId` derives `Ord`:
  ```rust
  /// MarketId-pair -> Manhattan distance in whole tiles, stored DIRECTED both
  /// ways ((a,b) and (b,a) both present) for O(1) symmetric lookup. Baked once
  /// in `seed_demo_economy` from the routing `Graph`; persisted (the economy
  /// core is graph-free at hydrate, so it cannot be recomputed on restore).
  #[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
  pub struct MarketDistances(pub std::collections::BTreeMap<(MarketId, MarketId), i64>);
  ```
  (`market.rs` already imports `BTreeMap` via `use std::collections::{BTreeMap, BTreeSet};` at line 1 — prefer `BTreeMap<(MarketId, MarketId), i64>` without the path qualifier; matching the file's existing style.)
- [ ] Register it in `backend/crates/sim-core/src/economy/mod.rs` `EconomyPlugin::install`, immediately after `world.insert_resource(DormantMarkets::default());` (line 67):
  ```rust
  world.insert_resource(MarketDistances::default());
  ```
  (`market::*` is already glob re-exported at line 29, so `MarketDistances` is in scope at `crate::economy::MarketDistances` with no extra `use`.)
- [ ] Run the test, expect **PASS**:
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core market_distances_stores_directed_pairs_both_ways`
- [ ] Run the whole economy test set to confirm no regression:
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::`
- [ ] Commit only if instructed: `feat(economy): add MarketDistances resource`.

### Task 2: Persist `MarketDistances` as the 14th `EconomyPersistSnapshot` field

**Files:** `backend/crates/sim-core/src/economy/persist.rs`, `backend/crates/sim-core/src/economy/tests/persist.rs`.

- [ ] In `backend/crates/sim-core/src/economy/tests/persist.rs`, extend the existing inline `seed()` (after the `MarketChunks` insert at lines 160-163) to add a distance entry both-ways so the round-trip test actually carries non-empty data:
  ```rust
      {
          let mut d = world.resource_mut::<crate::economy::MarketDistances>();
          d.0.insert((m, crate::economy::MarketId(2)), 4);
          d.0.insert((crate::economy::MarketId(2), m), 4);
      }
  ```
- [ ] Run `economy_snapshot_round_trips`, expect **FAIL to COMPILE** (`extract_from_world` returns a struct with no `market_distances`, but more importantly the round-trip will silently drop the new resource — it compiles today but the data is lost). Verify the failure mode is "field dropped → snap != snap2" once the field exists; for now the test still passes because the field doesn't exist. To force a real red, ALSO add the assertion below in the SAME step, then run and expect **FAIL** (`no field market_distances on EconomyPersistSnapshot`):
  ```rust
  // inside economy_snapshot_round_trips, after the existing assert_eq!(snap, snap2, ...):
      assert_eq!(
          snap.market_distances,
          vec![
              ((crate::economy::MarketId(1), crate::economy::MarketId(2)), 4),
              ((crate::economy::MarketId(2), crate::economy::MarketId(1)), 4),
          ],
          "directed distances persist in sorted BTreeMap order"
      );
  ```
  Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::persist`
- [ ] Add the field to `EconomyPersistSnapshot` in `backend/crates/sim-core/src/economy/persist.rs` (after `ledger_tail`, as the 14th field — keep `ledger_tail` and its doc comment, append below it):
  ```rust
      /// Directed market-pair distances (Manhattan tiles), sorted BTreeMap order.
      /// Recompute-on-hydrate is impossible (the economy core holds no `Graph`),
      /// so this is persisted verbatim. No serde-default shim.
      pub market_distances: Vec<((MarketId, MarketId), i64)>,
  ```
- [ ] Import `MarketDistances` in the `use crate::economy::{...}` block at the top of `persist.rs` (add `MarketDistances,` alphabetically near `MarketChunks`).
- [ ] In `extract_from_world`, read the resource (after `let ledger = world.resource::<TradeLedger>();`):
  ```rust
      let market_distances = world.resource::<MarketDistances>();
  ```
  and populate the field in the returned struct literal (after `ledger_tail,`):
  ```rust
          market_distances: market_distances.0.iter().map(|(k, v)| (*k, *v)).collect(),
  ```
- [ ] In `apply_into_world`, insert the resource (after the `TradeLedger` insert at line 111):
  ```rust
      world.insert_resource(MarketDistances(
          snap.market_distances.iter().cloned().collect(),
      ));
  ```
- [ ] Run the persist tests, expect **PASS** (round-trip, byte-stable, empty, provider, ledger-tail all green):
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::persist`
- [ ] Commit only if instructed: `feat(economy): persist MarketDistances (14th snapshot field)`.

### Task 3: Bake `MarketDistances` (both directions) in `seed_demo_economy`

**Files:** `backend/crates/sim-core/src/economy/seed.rs`, `backend/crates/sim-core/src/economy/tests/seed.rs`.

> **CRITICAL FIX:** the existing test `seed_demo_economy_creates_two_markets_and_one_trader` (tests/seed.rs) builds its world by hand and does NOT register `MarketDistances`. After this task `seed_demo_economy` calls `world.resource_mut::<MarketDistances>()`, which PANICS on a missing resource. There is **no `seed_world()` helper** — the test world is inline. Write against that inline world.

- [ ] In `backend/crates/sim-core/src/economy/tests/seed.rs`, add `MarketDistances` to the `use crate::economy::{...}` block (line 4-6), and insert it into the inline world **before** `seed_demo_economy(&mut world);` (i.e. add after line 31 `world.insert_resource(Traders::default());`):
  ```rust
      world.insert_resource(crate::economy::MarketDistances::default());
  ```
- [ ] In the same test, after the existing assertions (line 51-52), add directed-distance assertions. The two seeded nodes are at `(2.0, 3.0)` and `(13.0, 3.0)` → `manhattan_tiles == |2-13| + |3-3| == 11`, and the markets are `MarketId(9_001)`/`MarketId(9_002)`:
  ```rust
      let distances = world.resource::<crate::economy::MarketDistances>();
      assert_eq!(distances.0.len(), 2, "both directed pairs baked");
      assert_eq!(
          distances.0.get(&(MarketId(9_001), MarketId(9_002))).copied(),
          Some(11)
      );
      assert_eq!(
          distances.0.get(&(MarketId(9_002), MarketId(9_001))).copied(),
          Some(11)
      );
  ```
  Add `MarketId` to the `use crate::economy::{...}` import in this test file if not already present (it is currently NOT imported — add it).
- [ ] Run the test, expect **FAIL** — first to panic on the missing-resource `resource_mut` call (before this task's seed edit), and after that the distance assertions fail because the seed does not bake distances:
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_demo_economy_creates_two_markets_and_one_trader`
- [ ] In `backend/crates/sim-core/src/economy/seed.rs`, add `MarketDistances` to the `use crate::economy::{...}` block (lines 11-15, alphabetically near `MarketChunks`). The `dist` value is already computed at line 51 (`manhattan_tiles(graph, node_a, node_b)`). After the `MarketChunks` anchor block (after line 79, the closing `}` of the `anchors` block), bake both directions:
  ```rust
      {
          let mut distances = world.resource_mut::<MarketDistances>();
          distances.0.insert((m_a, m_b), dist);
          distances.0.insert((m_b, m_a), dist);
      }
  ```
- [ ] Run the test, expect **PASS** (no panic; `Markets.len()==2`, `Traders.len()==1`, both directed distances == 11):
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_demo_economy_creates_two_markets_and_one_trader`
- [ ] Run the broader economy suite to confirm no other inline-seed test panics on the new `resource_mut` (e.g. any full-stack seed call):
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::`
- [ ] Commit only if instructed: `feat(economy): bake MarketDistances both-ways in seed_demo_economy`.

### Task 4: Add `EconomyEvent::MacroFlow` + its `event_type()` arm (do NOT delete `WarmMarketFlow`)

**Files:** `backend/crates/sim-core/src/economy/ledger.rs`, `backend/crates/sim-core/src/economy/tests/audit.rs` (or a new inline test in `ledger`-adjacent test file — use `tests/audit.rs`, which already covers `event_type`).

- [ ] Write a failing test in `backend/crates/sim-core/src/economy/tests/audit.rs` (append at end):
  ```rust
  #[test]
  fn macro_flow_event_type_is_macro_flow() {
      use crate::economy::{EconomyEvent, GoodId, MarketId, Money, Quantity};
      let ev = EconomyEvent::MacroFlow {
          from_market: MarketId(1),
          to_market: MarketId(2),
          good: GoodId(4),
          qty: Quantity(10),
          price: Money(1_000),
          transport: Money(50),
      };
      assert_eq!(ev.event_type(), "macro_flow");
  }
  ```
- [ ] Run it, expect **FAIL to COMPILE** (`no variant MacroFlow`):
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core macro_flow_event_type_is_macro_flow`
- [ ] In `backend/crates/sim-core/src/economy/ledger.rs`, ADD the variant to `EconomyEvent` (after `WarmMarketFlow` at line 77, **keeping** `WarmMarketFlow`):
  ```rust
      MacroFlow {
          from_market: MarketId,
          to_market: MarketId,
          good: GoodId,
          qty: Quantity,
          price: Money,
          transport: Money,
      },
  ```
- [ ] Add its `event_type()` arm in the match (after the `WarmMarketFlow` arm at line 98, keeping that arm):
  ```rust
          Self::MacroFlow { .. } => "macro_flow",
  ```
- [ ] Run the test, expect **PASS**:
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core macro_flow_event_type_is_macro_flow`
- [ ] Run the audit + persist tests to confirm the added serde variant still round-trips (jsonb is variant-agnostic) and no `event_type` exhaustive-match elsewhere broke:
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::audit economy::tests::persist`
- [ ] Commit only if instructed: `feat(economy): add MacroFlow audit event (additive)`.

### Task 5: Add the `EconomySet::RefreshLod.after(CoreSet::LodReclassify)` ordering constraint

**Files:** `backend/crates/sim-core/src/economy/systems.rs`, `backend/crates/sim-core/src/economy/tests/systems.rs`.

> The economy chain is today anchored only `.before(tick_increment_system)` (systems.rs:81) with no tie to LOD reclassification. Because the macro flow will become stateful (writes prices), the set of markets it mutates must be a deterministic function of LOD classification — so `RefreshLod` must run **after** `CoreSet::LodReclassify` (`reclassify_chunk_lod_system`, run in `world/plugin.rs:76`). This ordering is **inert** (no panic) when `EconomyPlugin` installs without `CorePlugin` (the `CoreSet` is simply never configured in that schedule), and load-bearing only in the full `SimPlugin` stack.

- [ ] Write a failing test in `backend/crates/sim-core/src/economy/tests/systems.rs` (append at end). Build the full Core + Mobility + Economy stack and assert reclassify runs before the economy refresh by observing ordering through behaviour: anchor a market to a chunk that `reclassify_chunk_lod_system` will mark, then assert `DormantMarkets` reflects the post-reclassify classification after one `schedule.run`. Use the same install pattern as `tests/lod.rs::lod_world` (CorePlugin + MobilityPlugin + EconomyPlugin):
  ```rust
  #[test]
  fn refresh_lod_runs_after_core_lod_reclassify() {
      use bevy_ecs::prelude::*;
      use crate::economy::{DormantMarkets, EconomyPlugin, MarketChunks, MarketId};
      use crate::ids::ChunkCoord;
      use crate::mobility::resources::Tick;
      use crate::world::plugin::CorePlugin;
      use crate::world::schedule::SimPlugin;

      let mut world = World::new();
      let mut schedule = bevy_ecs::schedule::Schedule::default();
      CorePlugin::default().install(&mut world, &mut schedule);
      crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
      EconomyPlugin.install(&mut world, &mut schedule);

      // Anchor a market to a chunk with NO active/hot subscriber -> reclassify
      // leaves it non-Active, so refresh (running AFTER reclassify) marks it dormant.
      let market = MarketId(77);
      let coord = ChunkCoord { x: 9, y: 9 };
      world
          .resource_mut::<MarketChunks>()
          .0
          .insert(market, coord);
      world.insert_resource(Tick(0));

      schedule.run(&mut world);

      assert!(
          world.resource::<DormantMarkets>().0.contains(&market),
          "RefreshLod observed the reclassified (non-active) chunk -> market is dormant"
      );
  }
  ```
  (If `reclassify_chunk_lod_system` requires a subscriber/chunk-entity to mark an `ActiveChunk`, the absence of one means the chunk is non-active by default — the assertion holds regardless of ordering only if reclassify did not race; the load-bearing guarantee is the `.after` edge added below, which removes nondeterminism in the multi-threaded executor.)
- [ ] Run it, expect **PASS or FAIL depending on executor scheduling** — the point is the edge does not yet exist. To make this a deterministic RED for the *ordering*, instead assert the configured ordering directly is the cleaner approach: replace the behavioural body with a structural check is **not** possible via `bevy_ecs` public API, so keep the behavioural test and treat it as a regression guard. Run:
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core refresh_lod_runs_after_core_lod_reclassify`
- [ ] Add the ordering edge in `backend/crates/sim-core/src/economy/systems.rs` `install_systems`, immediately after the `.chain()` `configure_sets` block (after line 69, before `schedule.add_systems(`):
  ```rust
      // The macro flow is stateful (writes dormant prices), so the set of markets
      // it mutates must be a deterministic function of LOD classification. Anchor
      // RefreshLod after CoreSet::LodReclassify. Inert (the CoreSet is simply not
      // configured) when EconomyPlugin installs without CorePlugin; load-bearing
      // only in the full SimPlugin stack, where it removes a classify/mutate race.
      schedule.configure_sets(
          EconomySet::RefreshLod.after(crate::world::schedule::CoreSet::LodReclassify),
      );
  ```
- [ ] Run the test, expect **PASS** (and now deterministic across executor runs):
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core refresh_lod_runs_after_core_lod_reclassify`
- [ ] Run the economy-only suite to confirm the new cross-plugin set edge does not break schedules where `CoreSet` is absent (e.g. `tests/persist.rs::install_economy` installs `EconomyPlugin` alone — the `.after` edge referencing an unconfigured set must not panic at schedule build):
  `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::`
- [ ] Commit only if instructed: `feat(economy): order RefreshLod after CoreSet::LodReclassify`.

### Task 6: STEP A — `synthetic_price` + dormant-bucket builder (standalone `macro_flow.rs`)

**Files:** `backend/crates/sim-core/src/economy/macro_flow.rs` (NEW), `backend/crates/sim-core/src/economy/mod.rs`, `backend/crates/sim-core/src/economy/tests/mod.rs`, `backend/crates/sim-core/src/economy/tests/macro_flow.rs` (NEW)

This task introduces the new standalone file with the two STEP-A primitives only. Nothing is wired into the schedule; everything is exercised by direct function calls. `warm_flow.rs` is untouched and still compiles.

- [ ] Create the new test module file `backend/crates/sim-core/src/economy/tests/macro_flow.rs` (empty for now, will accrue tests) and register it in `tests/mod.rs` by ADDING the line `mod macro_flow;` **without removing** `mod warm_flow;` (both modules coexist until Task 12). Run `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::macro_flow` — expect PASS (no tests yet, module compiles).

- [ ] Write the first failing test in `tests/macro_flow.rs` for the both-sided price branch. Add imports and the test:
  ```rust
  use crate::economy::macro_flow::synthetic_price;
  use crate::economy::{Money, SettlementPolicy};

  #[test]
  fn synthetic_price_both_sided_clamps_prior_into_band() {
      // prior 1000, band [ask_floor=500, bid_ceiling=2000]; Anchored keeps prior.
      let p = synthetic_price(
          /*has_demand=*/ true,
          /*has_supply=*/ true,
          /*bid_ceiling=*/ Money(2_000),
          /*ask_floor=*/ Money(500),
          /*prior=*/ Money(1_000),
          SettlementPolicy::Anchored,
      );
      assert_eq!(p, Money(1_000));
      // prior below band clamps up to ask_floor.
      let p2 = synthetic_price(true, true, Money(2_000), Money(500), Money(100), SettlementPolicy::Anchored);
      assert_eq!(p2, Money(500));
  }
  ```
  Run `... economy::tests::macro_flow::synthetic_price_both_sided_clamps_prior_into_band` — expect FAIL to compile (`synthetic_price` does not exist).

- [ ] Create `backend/crates/sim-core/src/economy/macro_flow.rs` with the module doc comment, imports, and the minimal `synthetic_price` implementing all three band branches plus the both-sided ordering guard:
  ```rust
  //! Macro demand-driven cross-market flow (Economy LOD). Replaces warm-flow: a
  //! mean-field spatial-price-equilibrium step over ALL dormant markets (warm AND
  //! asleep), per coarse interval, per good. Goods flow surplus->deficit when the
  //! price gap strictly exceeds transport; the realized band-clamped price is
  //! written back so prices drift toward equilibrium across intervals.
  //! Conservation-exact (atomic clone-validate-apply) and deterministic.

  use std::collections::{BTreeMap, BTreeSet};

  use crate::economy::pools::affordable_qty;
  use crate::economy::{
      AccountBook, DemandPools, DirtyMarketGoods, EconomicActorId, EconomyConfig, EconomyError,
      EconomyEvent, GoodId, InventoryBook, MarketDistances, MarketGoodKey, MarketGoodState,
      MarketGoods, MarketId, Money, Quantity, SettlementPolicy, SupplyPools, TradeLedger,
      TRANSPORT_OPERATOR, checked_order_value, prorata_distribute, settlement_price_with_policy,
      transport_cost,
  };

  /// Synthetic per-(market,good) price derived from the pool band each interval.
  /// Both-sided markets clamp `prior` into `[ask_floor, bid_ceiling]` via the
  /// auction primitive (price-discovering, drifts as `prior` updates). One-sided
  /// markets are reservation-price-pinned: supply-only -> `ask_floor` (cheap),
  /// demand-only -> `bid_ceiling` (dear), both ignoring `prior` (no local
  /// discovery to move a reservation price). Callers must skip a `<= 0` result.
  pub fn synthetic_price(
      has_demand: bool,
      has_supply: bool,
      bid_ceiling: Money,
      ask_floor: Money,
      prior: Money,
      policy: SettlementPolicy,
  ) -> Money {
      if has_demand && has_supply {
          if bid_ceiling.0 >= ask_floor.0 {
              settlement_price_with_policy(prior, bid_ceiling, ask_floor, policy)
          } else {
              // Crossed band (no clearable overlap on price): pin to ask_floor so
              // a usable positive price exists; the matched quantity is 0 anyway.
              ask_floor
          }
      } else if has_supply {
          ask_floor
      } else {
          // demand-only (has_demand == true here, else caller has no bucket)
          bid_ceiling
      }
  }
  ```
  Add `pub mod macro_flow;` to `mod.rs` (additive; `pub mod warm_flow;` stays) **without** a `pub use macro_flow::*;` glob yet (avoid colliding re-exports while warm_flow is still glob-exported; tests reference `crate::economy::macro_flow::synthetic_price` by path). Run the same test — expect PASS.

- [ ] Add a failing test for the one-sided price-pinned property (supply-only ignores prior; demand-only ignores prior):
  ```rust
  #[test]
  fn synthetic_price_one_sided_is_reservation_pinned() {
      // supply-only: returns ask_floor regardless of a high prior.
      let s = synthetic_price(false, true, Money(0), Money(500), Money(9_999), SettlementPolicy::Anchored);
      assert_eq!(s, Money(500), "supply-only pins to ask_floor, ignores prior");
      // demand-only: returns bid_ceiling regardless of a low prior.
      let d = synthetic_price(true, false, Money(2_000), Money(0), Money(1), SettlementPolicy::Anchored);
      assert_eq!(d, Money(2_000), "demand-only pins to bid_ceiling, ignores prior");
  }
  ```
  Run `... economy::tests::macro_flow::synthetic_price_one_sided_is_reservation_pinned` — expect PASS (the impl already covers it; this test locks the §3 STEP A caveat).

- [ ] Add a failing test for the dormant-bucket builder. The builder groups `DemandPools`/`SupplyPools` filtered to `dormant`, computes effective demand `min(desired, affordable(cash, p_m))` and effective supply `min(offered, on-hand)`, and yields per-`MarketGoodKey` buyer/seller weight lists keyed against the running clones. Because effective demand needs `p_m`, and `p_m` needs the band (which needs the raw buyers/sellers), the builder is two-phase. Write:
  ```rust
  use crate::economy::macro_flow::{build_macro_buckets, MacroBucket};
  use crate::economy::{AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_FOOD,
      InventoryBook, MarketGoods, MarketId, Quantity, SupplyPool, SupplyPools, EconomyConfig};

  #[test]
  fn build_macro_buckets_caps_effective_demand_and_supply() {
      let market = MarketId(1);
      let buyer = EconomicActorId(1);
      let seller = EconomicActorId(2);
      let mut accounts = AccountBook::default();
      let mut inventory = InventoryBook::default();
      accounts.deposit(buyer, Money(30)).unwrap(); // affords 30 at p_m derived below
      inventory.deposit(seller, GOOD_FOOD, Quantity(20)).unwrap(); // only 20 on hand
      let mut demand = DemandPools::default();
      demand.0.insert(buyer, DemandPool {
          actor: buyer, market, good: GOOD_FOOD, desired_qty_per_tick: Quantity(100),
          max_price: Money(1_000), urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
          last_generated_tick: None });
      let mut supply = SupplyPools::default();
      supply.0.insert(seller, SupplyPool {
          actor: seller, market, good: GOOD_FOOD, offered_qty_per_tick: Quantity(100),
          min_price: Money(500), interval_ticks: 1, last_generated_tick: None });
      let dormant: BTreeSet<MarketId> = [market].into_iter().collect();
      let mg = MarketGoods::default(); // never auctioned -> prior = default ref price
      let cfg = EconomyConfig::default();

      let buckets = build_macro_buckets(&accounts, &inventory, &demand, &supply, &mg, &dormant, &cfg)
          .unwrap();
      let key = MarketGoodKey { market, good: GOOD_FOOD };
      let b: &MacroBucket = buckets.get(&key).expect("bucket exists");
      // both-sided -> p_m = settlement_price_with_policy(prior=1000, bid=1000, ask=500)=1000.
      assert_eq!(b.price, Money(1_000));
      // effective demand = min(100 desired, affordable(30 cash / price 1000 -> 30)) = 30.
      assert_eq!(b.total_demand(), 30);
      // effective supply = min(100 offered, 20 on hand) = 20.
      assert_eq!(b.total_supply(), 20);
  }
  ```
  Run `... build_macro_buckets_caps_effective_demand_and_supply` — expect FAIL (no `build_macro_buckets`/`MacroBucket`).

- [ ] Add `MacroBucket` and `build_macro_buckets` to `macro_flow.rs`. `MacroBucket` holds the synthetic price plus buyer/seller `(actor, eff_qty)` lists; the builder is two-phase per key (raw band → `p_m` → affordability-capped buyers). It reads the LIVE books (Task 11 wires the read-only-first-then-clone flow; here the books are passed by ref):
  ```rust
  /// Per-(market,good) aggregate after STEP A: synthetic price + effective
  /// buyer/seller weight lists (actor, qty), capped by affordability / on-hand.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct MacroBucket {
      pub price: Money,
      /// (actor, effective_demand_qty), filtered to qty > 0, in ascending actor order.
      pub buyers: Vec<(EconomicActorId, i64)>,
      /// (actor, effective_supply_qty), filtered to qty > 0, in ascending actor order.
      pub sellers: Vec<(EconomicActorId, i64)>,
  }

  impl MacroBucket {
      pub fn total_demand(&self) -> i64 {
          self.buyers.iter().map(|(_, q)| *q).sum()
      }
      pub fn total_supply(&self) -> i64 {
          self.sellers.iter().map(|(_, q)| *q).sum()
      }
  }

  fn prior_price(market_goods: &MarketGoods, key: MarketGoodKey, config: &EconomyConfig) -> Money {
      match market_goods.0.get(&key) {
          Some(state) if state.last_settlement_price.0 > 0 => state.last_settlement_price,
          _ => config.trader_default_ref_price,
      }
  }

  /// STEP A: build the dormant aggregate buckets. Groups dormant demand/supply by
  /// market-good, derives the synthetic price from the raw band, then caps demand
  /// by affordability (at `price`) and supply by on-hand stock. Buckets whose
  /// `price <= 0` are dropped (the warm-flow zero-band skip, applied before any
  /// `affordable_qty`). Empty buyer AND empty seller buckets are dropped.
  #[allow(clippy::too_many_arguments)]
  pub fn build_macro_buckets(
      accounts: &AccountBook,
      inventory: &InventoryBook,
      demand: &DemandPools,
      supply: &SupplyPools,
      market_goods: &MarketGoods,
      dormant: &BTreeSet<MarketId>,
      config: &EconomyConfig,
  ) -> Result<BTreeMap<MarketGoodKey, MacroBucket>, EconomyError> {
      // Phase 1: raw bands (max_price ceiling for buyers, min_price floor for sellers).
      type Raw = (Vec<(EconomicActorId, i64)>, Option<Money>); // (entries, band-extreme)
      let mut raw_demand: BTreeMap<MarketGoodKey, Raw> = BTreeMap::new();
      let mut raw_supply: BTreeMap<MarketGoodKey, Raw> = BTreeMap::new();
      for pool in demand.0.values() {
          if !dormant.contains(&pool.market) {
              continue;
          }
          let key = MarketGoodKey { market: pool.market, good: pool.good };
          let entry = raw_demand.entry(key).or_insert_with(|| (Vec::new(), None));
          entry.0.push((pool.actor, pool.desired_qty_per_tick.0));
          entry.1 = Some(match entry.1 {
              Some(c) if c.0 >= pool.max_price.0 => c,
              _ => pool.max_price,
          });
      }
      for pool in supply.0.values() {
          if !dormant.contains(&pool.market) {
              continue;
          }
          let key = MarketGoodKey { market: pool.market, good: pool.good };
          let entry = raw_supply.entry(key).or_insert_with(|| (Vec::new(), None));
          entry.0.push((pool.actor, pool.offered_qty_per_tick.0));
          entry.1 = Some(match entry.1 {
              Some(f) if f.0 <= pool.min_price.0 => f,
              _ => pool.min_price,
          });
      }

      // Phase 2: union of keys -> synthetic price -> effective caps.
      let mut keys: BTreeSet<MarketGoodKey> = BTreeSet::new();
      keys.extend(raw_demand.keys().copied());
      keys.extend(raw_supply.keys().copied());

      let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
      for key in keys {
          let d = raw_demand.get(&key);
          let s = raw_supply.get(&key);
          let has_demand = d.is_some();
          let has_supply = s.is_some();
          let bid_ceiling = d.and_then(|(_, c)| *c).unwrap_or(Money::ZERO);
          let ask_floor = s.and_then(|(_, f)| *f).unwrap_or(Money::ZERO);
          let prior = prior_price(market_goods, key, config);
          let price = synthetic_price(
              has_demand, has_supply, bid_ceiling, ask_floor, prior, config.settlement_policy,
          );
          if price.0 <= 0 {
              continue; // zero/negative band: skip, never ZeroPrice-abort.
          }
          let mut buyers: Vec<(EconomicActorId, i64)> = Vec::new();
          if let Some((entries, _)) = d {
              for (actor, want) in entries {
                  let cash = accounts.account(*actor).available;
                  let afford = affordable_qty(cash, price)?.0;
                  let eff = (*want).min(afford);
                  if eff > 0 {
                      buyers.push((*actor, eff));
                  }
              }
          }
          let mut sellers: Vec<(EconomicActorId, i64)> = Vec::new();
          if let Some((entries, _)) = s {
              for (actor, offer) in entries {
                  let have = inventory.balance(*actor, key.good).available.0;
                  let eff = (*offer).min(have);
                  if eff > 0 {
                      sellers.push((*actor, eff));
                  }
              }
          }
          if buyers.is_empty() && sellers.is_empty() {
              continue;
          }
          buckets.insert(key, MacroBucket { price, buyers, sellers });
      }
      Ok(buckets)
  }
  ```
  Note: `MarketDistances`, `MarketGoodState`, `prorata_distribute`, `checked_order_value`, `transport_cost`, `TRANSPORT_OPERATOR`, `DirtyMarketGoods`, `EconomyEvent`, `TradeLedger`, `GoodId`, `Quantity` are imported now though unused until later tasks — add `#[allow(unused_imports)]` is forbidden; instead only import what each task uses and EXTEND the `use` block in the task that first needs each. For THIS task the live `use` block is exactly: `affordable_qty`, `AccountBook`, `DemandPools`, `EconomicActorId`, `EconomyConfig`, `EconomyError`, `MarketGoodKey`, `MarketGoods`, `MarketId`, `Money`, `SettlementPolicy`, `SupplyPools`, `settlement_price_with_policy` (plus `BTreeMap`, `BTreeSet`). Run `... build_macro_buckets_caps_effective_demand_and_supply` — expect PASS.

- [ ] Add a failing test for the `price <= 0` skip (zero-price band market): a supply-only bucket with `min_price = Money(0)` yields `ask_floor = 0` → `synthetic_price` returns `Money(0)` → bucket dropped, no `ZeroPrice` error:
  ```rust
  #[test]
  fn build_macro_buckets_skips_zero_price_band() {
      let market = MarketId(1);
      let seller = EconomicActorId(2);
      let mut inventory = InventoryBook::default();
      inventory.deposit(seller, GOOD_FOOD, Quantity(50)).unwrap();
      let mut supply = SupplyPools::default();
      supply.0.insert(seller, SupplyPool {
          actor: seller, market, good: GOOD_FOOD, offered_qty_per_tick: Quantity(10),
          min_price: Money(0), interval_ticks: 1, last_generated_tick: None });
      let dormant: BTreeSet<MarketId> = [market].into_iter().collect();
      let buckets = build_macro_buckets(&AccountBook::default(), &inventory,
          &DemandPools::default(), &supply, &MarketGoods::default(), &dormant,
          &EconomyConfig::default()).unwrap();
      assert!(buckets.is_empty(), "zero-price band market produces no bucket and no error");
  }
  ```
  Run `... build_macro_buckets_skips_zero_price_band` — expect PASS (the `price.0 <= 0` guard already covers it; this test locks the §3 STEP A guard).

- [ ] Run the full new module: `... economy::tests::macro_flow` — expect PASS. Commit: `feat(economy): macro-flow STEP A synthetic_price + dormant bucket builder`.

### Task 7: STEP C — classify matched / surplus / deficit

**Files:** `backend/crates/sim-core/src/economy/macro_flow.rs`, `backend/crates/sim-core/src/economy/tests/macro_flow.rs`

- [ ] Add a failing test for the STEP-C classification. The classifier turns a `MacroBucket`'s totals into `(matched, surplus, deficit)` with the disjoint partition `matched = min(D,S)`, `surplus = S - matched`, `deficit = D - matched`:
  ```rust
  use crate::economy::macro_flow::classify_bucket;

  #[test]
  fn classify_bucket_partitions_matched_surplus_deficit() {
      // surplus side: S=80 > D=30 -> matched 30, surplus 50, deficit 0.
      let (m, surplus, deficit) = classify_bucket(/*demand=*/ 30, /*supply=*/ 80);
      assert_eq!((m, surplus, deficit), (30, 50, 0));
      // deficit side: D=120 > S=40 -> matched 40, surplus 0, deficit 80.
      let (m2, sur2, def2) = classify_bucket(120, 40);
      assert_eq!((m2, sur2, def2), (40, 0, 80));
      // balanced: D==S -> matched all, no residual.
      assert_eq!(classify_bucket(50, 50), (50, 0, 0));
      // empty side: D=0 -> matched 0, no surplus exported when supply has no buyers locally.
      assert_eq!(classify_bucket(0, 70), (0, 70, 0));
  }
  ```
  Run `... classify_bucket_partitions_matched_surplus_deficit` — expect FAIL (no `classify_bucket`).

- [ ] Add `classify_bucket` to `macro_flow.rs`:
  ```rust
  /// STEP C: partition a market's aggregate demand/supply into the locally-clearable
  /// overlap `matched = min(D, S)`, the exportable `surplus = S - matched`, and the
  /// importable `deficit = D - matched`. At most one of surplus/deficit is non-zero;
  /// `matched` and the residual are disjoint quantities (overlap vs excess).
  pub fn classify_bucket(total_demand: i64, total_supply: i64) -> (i64, i64, i64) {
      let matched = total_demand.min(total_supply).max(0);
      let surplus = (total_supply - matched).max(0);
      let deficit = (total_demand - matched).max(0);
      (matched, surplus, deficit)
  }
  ```
  Run `... classify_bucket_partitions_matched_surplus_deficit` — expect PASS.

- [ ] Run `... economy::tests::macro_flow` — expect PASS. Commit: `feat(economy): macro-flow STEP C classify matched/surplus/deficit`.

### Task 8: STEP D — candidate directed edges + transport gate + self-edges

**Files:** `backend/crates/sim-core/src/economy/macro_flow.rs`, `backend/crates/sim-core/src/economy/tests/macro_flow.rs`

This task builds the read-only candidate-edge enumeration: per good, cross-edges gated on aggregate `net_gain > 0`, gate-overflow edges pruned, and a `from==to` self-edge for every market with `matched > 0` (exempt from the gate). It depends on STEP-C totals computed per key.

- [ ] Add a failing test that a cross-edge is KEPT when the gap exceeds transport. Use a config rate `Money(50)` so transport is genuinely non-zero. Build buckets manually via the public `MacroBucket` plus an explicit `MarketDistances`, then call `build_candidates`:
  ```rust
  use crate::economy::macro_flow::{build_candidates, Candidate};
  use crate::economy::MarketDistances;

  fn bucket(price: Money, buyers: Vec<(u64, i64)>, sellers: Vec<(u64, i64)>) -> MacroBucket {
      MacroBucket {
          price,
          buyers: buyers.into_iter().map(|(a, q)| (EconomicActorId(a), q)).collect(),
          sellers: sellers.into_iter().map(|(a, q)| (EconomicActorId(a), q)).collect(),
      }
  }

  #[test]
  fn build_candidates_keeps_cross_edge_when_gap_exceeds_transport() {
      let a = MarketId(1); // cheap, supply-only
      let b = MarketId(2); // dear, demand-only
      let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
      buckets.insert(MarketGoodKey { market: a, good: GOOD_FOOD },
          bucket(Money(500), vec![], vec![(10, 100)]));   // p_src=500, surplus 100
      buckets.insert(MarketGoodKey { market: b, good: GOOD_FOOD },
          bucket(Money(2_000), vec![(20, 100)], vec![])); // p_dst=2000, deficit 100
      let mut distances = MarketDistances::default();
      distances.0.insert((a, b), 1); // 1 tile
      distances.0.insert((b, a), 1);
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);

      let candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
      // q_cap = min(surplus 100, deficit 100) = 100.
      // src_revenue = 500*100/1000 = 50 ; dst_value = 2000*100/1000 = 200.
      // transport = (50*100/1000)*1 = 5. net_gain = 200 - 50 - 5 = 145 > 0 -> kept.
      let cross: Vec<&Candidate> = candidates.iter()
          .filter(|c| c.src != c.dst).collect();
      assert_eq!(cross.len(), 1);
      assert_eq!((cross[0].src, cross[0].dst), (a, b));
      assert_eq!(cross[0].q_cap, 100);
      assert_eq!(cross[0].net_gain, 145);
      assert_eq!(cross[0].transport_total, Money(5));
  }
  ```
  Run `... build_candidates_keeps_cross_edge_when_gap_exceeds_transport` — expect FAIL (no `build_candidates`/`Candidate`).

- [ ] Add `Candidate` and `build_candidates` to `macro_flow.rs`. Extend the `use` block to add `GoodId`, `MarketDistances`, `Quantity`, `checked_order_value`, `transport_cost`. The function: per good, gather per-market `(matched, surplus, deficit, price)` via `classify_bucket`; emit a self-edge for each `matched > 0`; emit a cross-edge for each ordered `(src,dst)` with `surplus_src > 0 && deficit_dst > 0`, gated by `net_gain > 0` on `q_cap`, GATE-PRUNING (continue, no candidate) on any checked-op overflow:
  ```rust
  /// One accepted-or-candidate directed flow edge for STEP D-F. `src == dst` is a
  /// self-edge (local clearing of `matched`, transport 0, gate-exempt).
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct Candidate {
      pub good: GoodId,
      pub src: MarketId,
      pub dst: MarketId,
      /// Fill cap = matched (self-edge) or min(surplus_src, deficit_dst) (cross-edge).
      pub q_cap: i64,
      pub p_src: Money,
      pub p_dst: Money,
      pub transport_total: Money,
      /// 0 for self-edges; strictly > 0 for kept cross-edges.
      pub net_gain: i64,
  }

  /// STEP D: enumerate candidate directed edges per good. Self-edges (matched > 0)
  /// are always emitted (gate-exempt). Cross-edges (src surplus -> dst deficit) are
  /// kept iff aggregate `net_gain > 0` on `q_cap`; any checked-op overflow in the
  /// gate PRUNES the edge (an uncomputable edge is not an opportunity). Read-only.
  pub fn build_candidates(
      buckets: &BTreeMap<MarketGoodKey, MacroBucket>,
      distances: &MarketDistances,
      config: &EconomyConfig,
  ) -> Result<Vec<Candidate>, EconomyError> {
      // Per good, per market: (matched, surplus, deficit, price).
      let mut by_good: BTreeMap<GoodId, BTreeMap<MarketId, (i64, i64, i64, Money)>> = BTreeMap::new();
      for (key, b) in buckets {
          let (matched, surplus, deficit) = classify_bucket(b.total_demand(), b.total_supply());
          by_good
              .entry(key.good)
              .or_default()
              .insert(key.market, (matched, surplus, deficit, b.price));
      }

      let mut candidates: Vec<Candidate> = Vec::new();
      for (good, markets) in &by_good {
          // Self-edges: one per market with locally-clearable overlap.
          for (market, (matched, _surplus, _deficit, price)) in markets {
              if *matched > 0 {
                  candidates.push(Candidate {
                      good: *good,
                      src: *market,
                      dst: *market,
                      q_cap: *matched,
                      p_src: *price,
                      p_dst: *price,
                      transport_total: Money::ZERO,
                      net_gain: 0,
                  });
              }
          }
          // Cross-edges: ordered (src surplus, dst deficit) pairs.
          for (src, (_m_s, surplus, _d_s, p_src)) in markets {
              if *surplus <= 0 {
                  continue;
              }
              for (dst, (_m_d, _s_d, deficit, p_dst)) in markets {
                  if src == dst || *deficit <= 0 {
                      continue;
                  }
                  let q_cap = (*surplus).min(*deficit);
                  if q_cap <= 0 {
                      continue;
                  }
                  let dist = match distances.0.get(&(*src, *dst)) {
                      Some(d) => *d,
                      None => continue, // no known route: not a candidate
                  };
                  // Aggregate transport gate on the actual q_cap; checked ops,
                  // any overflow PRUNES the edge (no candidate, no event).
                  let dst_value = match checked_order_value(*p_dst, Quantity(q_cap)) {
                      Ok(v) => v.0,
                      Err(_) => continue,
                  };
                  let src_value = match checked_order_value(*p_src, Quantity(q_cap)) {
                      Ok(v) => v.0,
                      Err(_) => continue,
                  };
                  let transport_total = match transport_cost(
                      dist,
                      Quantity(q_cap),
                      config.transport_cost_per_tile_unit,
                  ) {
                      Ok(t) => t,
                      Err(_) => continue,
                  };
                  let net_gain = match dst_value
                      .checked_sub(src_value)
                      .and_then(|g| g.checked_sub(transport_total.0))
                  {
                      Some(g) => g,
                      None => continue,
                  };
                  if net_gain > 0 {
                      candidates.push(Candidate {
                          good: *good,
                          src: *src,
                          dst: *dst,
                          q_cap,
                          p_src: *p_src,
                          p_dst: *p_dst,
                          transport_total,
                          net_gain,
                      });
                  }
              }
          }
      }
      Ok(candidates)
  }
  ```
  Run `... build_candidates_keeps_cross_edge_when_gap_exceeds_transport` — expect PASS.

- [ ] Add a failing test that a cross-edge is PRUNED when the gap is `<= transport`. Set the rate so transport ≥ the gap. With `p_src=1900`, `p_dst=2000`, `q_cap=100`, rate `Money(50)`, dist `1`: gap value = `200-190 = 10`; transport = `5`; net_gain `= 5 > 0` would keep — so to force prune, set dist `=3` → transport `=15`, net_gain `= 10-15 = -5 <= 0` pruned. Also assert the `==` boundary moves nothing by choosing numbers where net_gain is exactly 0:
  ```rust
  #[test]
  fn build_candidates_prunes_cross_edge_when_gap_not_above_transport() {
      let a = MarketId(1);
      let b = MarketId(2);
      let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
      buckets.insert(MarketGoodKey { market: a, good: GOOD_FOOD },
          bucket(Money(1_900), vec![], vec![(10, 100)]));
      buckets.insert(MarketGoodKey { market: b, good: GOOD_FOOD },
          bucket(Money(2_000), vec![(20, 100)], vec![]));
      let mut distances = MarketDistances::default();
      distances.0.insert((a, b), 3);
      distances.0.insert((b, a), 3);
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);
      // gap value = 200 - 190 = 10 ; transport = (50*100/1000)*3 = 15 ; net = -5 <= 0.
      let candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
      assert!(candidates.iter().all(|c| c.src == c.dst),
          "no cross-edge survives when net_gain <= 0; only self-edges (none here)");
      assert!(candidates.is_empty(), "no matched overlap -> no self-edges either");
  }

  #[test]
  fn build_candidates_prunes_cross_edge_at_exact_break_even() {
      let a = MarketId(1);
      let b = MarketId(2);
      let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
      buckets.insert(MarketGoodKey { market: a, good: GOOD_FOOD },
          bucket(Money(1_900), vec![], vec![(10, 100)]));
      buckets.insert(MarketGoodKey { market: b, good: GOOD_FOOD },
          bucket(Money(2_000), vec![(20, 100)], vec![]));
      let mut distances = MarketDistances::default();
      distances.0.insert((a, b), 2);
      distances.0.insert((b, a), 2);
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);
      // gap value = 10 ; transport = (50*100/1000)*2 = 10 ; net = 0 -> NOT kept (strict >).
      let candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
      assert!(candidates.is_empty(), "net_gain == 0 is pruned (strict greater)");
  }
  ```
  Run `... build_candidates_prunes_cross_edge_when_gap_not_above_transport build_candidates_prunes_cross_edge_at_exact_break_even` — expect PASS.

- [ ] Add a failing test that a gate-overflow edge is dropped (not faulted) — STEP D prune-don't-fault. Use a huge distance so `transport_cost` overflows `i64` inside the gate, and assert `build_candidates` returns `Ok` with no cross-edge (no panic, no error):
  ```rust
  #[test]
  fn build_candidates_drops_overflow_edge() {
      let a = MarketId(1);
      let b = MarketId(2);
      let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
      buckets.insert(MarketGoodKey { market: a, good: GOOD_FOOD },
          bucket(Money(500), vec![], vec![(10, i64::MAX)])); // pathological surplus
      buckets.insert(MarketGoodKey { market: b, good: GOOD_FOOD },
          bucket(Money(2_000), vec![(20, i64::MAX)], vec![])); // pathological deficit
      let mut distances = MarketDistances::default();
      distances.0.insert((a, b), i64::MAX); // pathological distance
      distances.0.insert((b, a), i64::MAX);
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);
      let candidates = build_candidates(&buckets, &distances, &cfg)
          .expect("gate overflow is pruned, never an Err");
      assert!(candidates.iter().all(|c| c.src == c.dst),
          "overflow cross-edge dropped, no candidate, no fault");
  }
  ```
  Run `... build_candidates_drops_overflow_edge` — expect PASS (the `Err(_) => continue` arms already prune; `checked_order_value(2000, i64::MAX)` overflows the i128 product → `Overflow` → pruned).

- [ ] Add a failing test that a self-edge IS emitted for a both-sided market with matched overlap (gate-exempt, transport 0):
  ```rust
  #[test]
  fn build_candidates_emits_gate_exempt_self_edge() {
      let m = MarketId(1);
      let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
      // both sides present: D=40, S=60 -> matched 40, surplus 20.
      buckets.insert(MarketGoodKey { market: m, good: GOOD_FOOD },
          bucket(Money(1_000), vec![(20, 40)], vec![(10, 60)]));
      let candidates = build_candidates(&buckets, &MarketDistances::default(),
          &EconomyConfig::default()).unwrap();
      let self_edges: Vec<&Candidate> = candidates.iter().filter(|c| c.src == c.dst).collect();
      assert_eq!(self_edges.len(), 1);
      assert_eq!(self_edges[0].q_cap, 40, "self-edge clears matched overlap");
      assert_eq!(self_edges[0].transport_total, Money::ZERO);
      assert_eq!(self_edges[0].net_gain, 0, "self-edge net_gain is identically 0, gate-exempt");
  }
  ```
  Run `... build_candidates_emits_gate_exempt_self_edge` — expect PASS.

- [ ] Run `... economy::tests::macro_flow` — expect PASS. Commit: `feat(economy): macro-flow STEP D candidate edges + transport gate + self-edges`.

### Task 9: STEP E deterministic sort + STEP F single greedy pass with disjoint budgets

**Files:** `backend/crates/sim-core/src/economy/macro_flow.rs`, `backend/crates/sim-core/src/economy/tests/macro_flow.rs`

This task adds the deterministic sort (`net_gain DESC, good ASC, src ASC, dst ASC`) and the planning pass that walks sorted candidates, consuming disjoint `remaining_matched`/`remaining_surplus`/`remaining_need` budgets, producing the per-edge `q` to settle. Settlement itself is Task 10; here the pass returns a list of `PlannedFlow { good, src, dst, q, p_src, p_dst, transport_for_q }` so it is testable without mutating books.

- [ ] Add a failing test for the deterministic sort: build candidates out of order and assert `sort_candidates` orders them `net_gain DESC, good ASC, src ASC, dst ASC`:
  ```rust
  use crate::economy::macro_flow::sort_candidates;

  fn cand(good: u16, src: u32, dst: u32, net: i64) -> Candidate {
      Candidate {
          good: GoodId(good), src: MarketId(src), dst: MarketId(dst),
          q_cap: 10, p_src: Money(500), p_dst: Money(2_000),
          transport_total: Money(0), net_gain: net,
      }
  }

  #[test]
  fn sort_candidates_total_order() {
      let mut v = vec![
          cand(1, 1, 2, 100),
          cand(1, 1, 3, 100), // same net & good & src; dst 3 after dst 2
          cand(2, 1, 2, 100), // same net; good 2 after good 1
          cand(1, 1, 2, 200), // higher net first
      ];
      sort_candidates(&mut v);
      assert_eq!(v[0].net_gain, 200);
      assert_eq!((v[1].good.0, v[1].src.0, v[1].dst.0), (1, 1, 2));
      assert_eq!((v[2].good.0, v[2].src.0, v[2].dst.0), (1, 1, 3));
      assert_eq!((v[3].good.0, v[3].src.0, v[3].dst.0), (2, 1, 2));
  }
  ```
  Run `... sort_candidates_total_order` — expect FAIL (no `sort_candidates`).

- [ ] Add `sort_candidates` to `macro_flow.rs`:
  ```rust
  /// STEP E: total order over distinct keys — `net_gain DESC, good ASC, src ASC,
  /// dst ASC`. All ids are BTree-keyed, so no surviving tie affects ordering.
  pub fn sort_candidates(candidates: &mut [Candidate]) {
      candidates.sort_by(|a, b| {
          b.net_gain
              .cmp(&a.net_gain)
              .then(a.good.cmp(&b.good))
              .then(a.src.cmp(&b.src))
              .then(a.dst.cmp(&b.dst))
      });
  }
  ```
  Run `... sort_candidates_total_order` — expect PASS.

- [ ] Add a failing test for the STEP-F planning pass with disjoint budgets. Two cross-edges share a surplus source; a self-edge consumes only matched, never the surplus:
  ```rust
  use crate::economy::macro_flow::{plan_flows, PlannedFlow};

  #[test]
  fn plan_flows_consumes_disjoint_budgets_once() {
      let a = MarketId(1); // surplus 30, matched 0
      let b = MarketId(2); // deficit 20
      let c = MarketId(3); // deficit 50
      let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
      buckets.insert(MarketGoodKey { market: a, good: GOOD_FOOD },
          bucket(Money(500), vec![], vec![(10, 30)]));     // surplus 30
      buckets.insert(MarketGoodKey { market: b, good: GOOD_FOOD },
          bucket(Money(2_000), vec![(20, 20)], vec![]));   // deficit 20
      buckets.insert(MarketGoodKey { market: c, good: GOOD_FOOD },
          bucket(Money(3_000), vec![(30, 50)], vec![]));   // deficit 50 (higher net first)
      let mut distances = MarketDistances::default();
      for (x, y) in [(a, b), (a, c), (b, a), (c, a)] { distances.0.insert((x, y), 1); }
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);

      let mut candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
      sort_candidates(&mut candidates);
      let flows = plan_flows(&candidates, &buckets);
      // a->c has higher net_gain (p_dst 3000 > 2000) so it fills first: q=min(surplus30, need50)=30.
      // a->b then sees remaining_surplus[a]=0 -> q=0 -> skipped.
      let total_from_a: i64 = flows.iter().filter(|f| f.src == a).map(|f| f.q).sum();
      assert_eq!(total_from_a, 30, "surplus consumed exactly once across cross-edges");
      assert!(flows.iter().any(|f| f.src == a && f.dst == c && f.q == 30));
      assert!(!flows.iter().any(|f| f.src == a && f.dst == b),
          "second cross-edge gets nothing once surplus is spent");
  }
  ```
  Run `... plan_flows_consumes_disjoint_budgets_once` — expect FAIL (no `plan_flows`/`PlannedFlow`).

- [ ] Add `PlannedFlow` and `plan_flows` to `macro_flow.rs`. The pass initializes per-market budgets from STEP C (recomputed from buckets), then walks sorted candidates: self-edge consumes `remaining_matched[src]`; cross-edge consumes `min(remaining_surplus[src], remaining_need[dst], q_cap)`. `transport_for_q` is recomputed proportionally via `transport_cost(dist, q, rate)` — but since `plan_flows` does not hold distances, it carries the per-edge transport by recomputing from the candidate's stored `transport_total` scaled by `q/q_cap`; to stay exact and integer, instead store the **distance** on the candidate. Revise: add `pub dist: i64` to `Candidate` (set it in Task 8's `build_candidates` where `dist` is known — go back and ADD the field). For THIS task `plan_flows` recomputes transport for the actual `q` via `transport_cost`:
  ```rust
  /// A flow chosen by the STEP-F pass, ready for STEP-G settlement.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct PlannedFlow {
      pub good: GoodId,
      pub src: MarketId,
      pub dst: MarketId,
      pub q: i64,
      pub p_src: Money,
      pub p_dst: Money,
      pub dist: i64,
  }

  /// STEP F: single greedy pass over the sorted candidates with disjoint per-market
  /// budgets (`remaining_matched` / `remaining_surplus` / `remaining_need`). Self-edges
  /// consume matched; cross-edges consume surplus/need. Each budget is consumed
  /// exactly once -> no double-spend; the output is a pure function of the sort.
  pub fn plan_flows(
      candidates: &[Candidate],
      buckets: &BTreeMap<MarketGoodKey, MacroBucket>,
  ) -> Vec<PlannedFlow> {
      let mut remaining_matched: BTreeMap<(GoodId, MarketId), i64> = BTreeMap::new();
      let mut remaining_surplus: BTreeMap<(GoodId, MarketId), i64> = BTreeMap::new();
      let mut remaining_need: BTreeMap<(GoodId, MarketId), i64> = BTreeMap::new();
      for (key, b) in buckets {
          let (matched, surplus, deficit) = classify_bucket(b.total_demand(), b.total_supply());
          remaining_matched.insert((key.good, key.market), matched);
          remaining_surplus.insert((key.good, key.market), surplus);
          remaining_need.insert((key.good, key.market), deficit);
      }

      let mut flows: Vec<PlannedFlow> = Vec::new();
      for c in candidates {
          if c.src == c.dst {
              let avail = remaining_matched.get_mut(&(c.good, c.src)).copied().unwrap_or(0);
              let q = avail.min(c.q_cap);
              if q <= 0 {
                  continue;
              }
              if let Some(slot) = remaining_matched.get_mut(&(c.good, c.src)) {
                  *slot -= q;
              }
              flows.push(PlannedFlow {
                  good: c.good, src: c.src, dst: c.dst, q,
                  p_src: c.p_src, p_dst: c.p_dst, dist: c.dist,
              });
          } else {
              let surplus = remaining_surplus.get(&(c.good, c.src)).copied().unwrap_or(0);
              let need = remaining_need.get(&(c.good, c.dst)).copied().unwrap_or(0);
              let q = surplus.min(need).min(c.q_cap);
              if q <= 0 {
                  continue;
              }
              if let Some(slot) = remaining_surplus.get_mut(&(c.good, c.src)) {
                  *slot -= q;
              }
              if let Some(slot) = remaining_need.get_mut(&(c.good, c.dst)) {
                  *slot -= q;
              }
              flows.push(PlannedFlow {
                  good: c.good, src: c.src, dst: c.dst, q,
                  p_src: c.p_src, p_dst: c.p_dst, dist: c.dist,
              });
          }
      }
      flows
  }
  ```
  Note the `.copied().unwrap_or(0)` borrow split (first read, then `get_mut` to write) — done above by reading `surplus`/`need` first, then mutating. Go back to Task 8 `build_candidates` and ADD `dist` to every `Candidate` constructed: self-edge `dist: 0`, cross-edge `dist`. Run `... plan_flows_consumes_disjoint_budgets_once` — expect PASS.

- [ ] Add a failing test that self-edge and cross-edge budgets are disjoint (a both-sided surplus market clears its matched locally AND still exports its surplus):
  ```rust
  #[test]
  fn plan_flows_self_and_cross_are_disjoint() {
      let a = MarketId(1); // D=20, S=50 -> matched 20, surplus 30
      let b = MarketId(2); // deficit 40
      let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
      buckets.insert(MarketGoodKey { market: a, good: GOOD_FOOD },
          bucket(Money(500), vec![(11, 20)], vec![(10, 50)]));
      buckets.insert(MarketGoodKey { market: b, good: GOOD_FOOD },
          bucket(Money(2_000), vec![(20, 40)], vec![]));
      let mut distances = MarketDistances::default();
      distances.0.insert((a, b), 1);
      distances.0.insert((b, a), 1);
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);
      let mut candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
      sort_candidates(&mut candidates);
      let flows = plan_flows(&candidates, &buckets);
      let self_q: i64 = flows.iter().filter(|f| f.src == a && f.dst == a).map(|f| f.q).sum();
      let cross_q: i64 = flows.iter().filter(|f| f.src == a && f.dst == b).map(|f| f.q).sum();
      assert_eq!(self_q, 20, "matched cleared locally");
      assert_eq!(cross_q, 30, "surplus exported; budgets never contend");
  }
  ```
  Run `... plan_flows_self_and_cross_are_disjoint` — expect PASS.

- [ ] Add a failing test for the ascending-MarketId tiebreak: two equidistant deficit markets of equal `net_gain` competing for one surplus → the lower-MarketId dst fills first, byte-stable across runs:
  ```rust
  #[test]
  fn plan_flows_tiebreak_is_stable_ascending_dst() {
      let build = || {
          let a = MarketId(1); // surplus 30
          let b = MarketId(2); // deficit 30, p_dst 2000
          let c = MarketId(3); // deficit 30, p_dst 2000 (equal net_gain & dist -> tie)
          let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
          buckets.insert(MarketGoodKey { market: a, good: GOOD_FOOD },
              bucket(Money(500), vec![], vec![(10, 30)]));
          buckets.insert(MarketGoodKey { market: b, good: GOOD_FOOD },
              bucket(Money(2_000), vec![(20, 30)], vec![]));
          buckets.insert(MarketGoodKey { market: c, good: GOOD_FOOD },
              bucket(Money(2_000), vec![(30, 30)], vec![]));
          let mut distances = MarketDistances::default();
          for (x, y) in [(a, b), (a, c), (b, a), (c, a)] { distances.0.insert((x, y), 1); }
          let mut cfg = EconomyConfig::default();
          cfg.transport_cost_per_tile_unit = Money(50);
          let mut candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
          sort_candidates(&mut candidates);
          plan_flows(&candidates, &buckets)
      };
      let flows = build();
      // dst b (lower id) wins the whole surplus; c gets nothing.
      assert!(flows.iter().any(|f| f.dst == MarketId(2) && f.q == 30));
      assert!(!flows.iter().any(|f| f.dst == MarketId(3)));
      assert_eq!(flows, build(), "planning is byte-identical across runs");
  }
  ```
  Run `... plan_flows_tiebreak_is_stable_ascending_dst` — expect PASS.

- [ ] Run `... economy::tests::macro_flow` — expect PASS. Commit: `feat(economy): macro-flow STEP E sort + STEP F greedy disjoint-budget pass`.

### Task 10: STEP G — settle one flow (aggregate floor, transport carve-out, prorata, write-back)

**Files:** `backend/crates/sim-core/src/economy/macro_flow.rs`, `backend/crates/sim-core/src/economy/tests/macro_flow.rs`

This task adds `settle_flow`, which mutates `next_accounts`/`next_inventory`/`market_goods` for ONE `PlannedFlow` and returns the emitted `MacroFlow` event. It uses the single pinned cash scheme (ONE aggregate `src_revenue` floor, transport carved out, two-sided prorata) and the `MarketGoodState` write-back against EFFECTIVE demand/supply. It does NOT touch `dirty`.

- [ ] Add a failing conservation test: settle one cross-flow A→B and assert total money + total good invariant and operator delta == transport exactly. Build buckets, plan a single flow, capture `total_money`/`total_good` before, settle into clones, commit, assert:
  ```rust
  use crate::economy::macro_flow::settle_flow;
  use crate::economy::{TradeLedger, TRANSPORT_OPERATOR, MarketGoodState, MarketGoodKey};

  #[test]
  fn settle_flow_conserves_and_credits_operator_exactly() {
      let a = MarketId(1);
      let b = MarketId(2);
      let seller = EconomicActorId(10);
      let buyer = EconomicActorId(20);
      let mut accounts = AccountBook::default();
      let mut inventory = InventoryBook::default();
      accounts.deposit(buyer, Money(1_000_000)).unwrap();
      inventory.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
      let mut market_goods = MarketGoods::default();
      let mut ledger = TradeLedger::default();

      let flow = PlannedFlow {
          good: GOOD_FOOD, src: a, dst: b, q: 100,
          p_src: Money(500), p_dst: Money(2_000), dist: 1,
      };
      // buyers/sellers weight maps for the prorata (single seller / single buyer here).
      let sellers = vec![(seller, 100i64)];
      let buyers = vec![(buyer, 100i64)];

      let m0 = accounts.total_money().unwrap();
      let g0 = inventory.total_good(GOOD_FOOD).unwrap();
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);

      let mut next_accounts = accounts.clone();
      let mut next_inventory = inventory.clone();
      let event = settle_flow(
          &mut next_accounts, &mut next_inventory, &mut market_goods,
          &flow, &sellers, &buyers,
          /*eff_demand_src=*/ 0, /*eff_supply_src=*/ 100,
          /*eff_demand_dst=*/ 100, /*eff_supply_dst=*/ 0,
          &cfg, /*current_tick=*/ 10,
      ).unwrap();
      accounts = next_accounts;
      inventory = next_inventory;
      ledger.0.push(event);

      // src_revenue = 500*100/1000 = 50 ; transport = (50*100/1000)*1 = 5 ; dst_payment = 55.
      assert_eq!(accounts.total_money().unwrap(), m0, "money conserved (transport is a transfer)");
      assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0, "goods conserved");
      assert_eq!(accounts.account(TRANSPORT_OPERATOR).available, Money(5),
          "operator credited exactly the transport total");
      assert_eq!(inventory.balance(buyer, GOOD_FOOD).available, Quantity(100));
      assert_eq!(inventory.balance(seller, GOOD_FOOD).available, Quantity(900));
      assert_eq!(accounts.account(seller).available, Money(50), "seller paid src_revenue");
      // dst write-back: last_settlement_price = p_dst, traded_qty += q, residual demand 0.
      let st_b = market_goods.0.get(&MarketGoodKey { market: b, good: GOOD_FOOD }).unwrap();
      assert_eq!(st_b.last_settlement_price, Money(2_000));
      assert_eq!(st_b.traded_qty_last_tick, Quantity(100));
      assert_eq!(st_b.last_cleared_tick, 10);
      let st_a = market_goods.0.get(&MarketGoodKey { market: a, good: GOOD_FOOD }).unwrap();
      assert_eq!(st_a.last_settlement_price, Money(500));
  }
  ```
  Run `... settle_flow_conserves_and_credits_operator_exactly` — expect FAIL (no `settle_flow`).

- [ ] Add `settle_flow` to `macro_flow.rs`. Extend the `use` block to add `EconomyEvent`, `InventoryBook` (already), `MarketGoodState`, `prorata_distribute`, `checked_order_value`, `transport_cost`, `TRANSPORT_OPERATOR`. It implements STEP G exactly: aggregate `src_revenue`, transport carved out, prorata sellers/buyers, the `lock_cash`+`debit_locked` buyer scheme, the operator deposit, and the dual `MarketGoodState` write-back against passed-in effective totals:
  ```rust
  /// STEP G: settle ONE accepted flow against the (cloned) books and write the
  /// discovered prices back into `market_goods`. Aggregate-floor cash scheme: one
  /// `src_revenue` floor; transport carved out of (never added on top of) the buyer
  /// total; sellers credited from `src_revenue`, buyers charged `dst_payment` via
  /// lock+debit. Returns the `MacroFlow` event. Does NOT touch `dirty`. The caller
  /// passes the bucket-time effective demand/supply per endpoint for the residual
  /// imbalance write-back (mirrors auction.rs:395-402).
  #[allow(clippy::too_many_arguments)]
  pub fn settle_flow(
      accounts: &mut AccountBook,
      inventory: &mut InventoryBook,
      market_goods: &mut MarketGoods,
      flow: &PlannedFlow,
      sellers: &[(EconomicActorId, i64)],
      buyers: &[(EconomicActorId, i64)],
      eff_demand_src: i64,
      eff_supply_src: i64,
      eff_demand_dst: i64,
      eff_supply_dst: i64,
      config: &EconomyConfig,
      current_tick: u64,
  ) -> Result<EconomyEvent, EconomyError> {
      let q = flow.q;
      let src_revenue = checked_order_value(flow.p_src, Quantity(q))?;
      let transport_total =
          transport_cost(flow.dist, Quantity(q), config.transport_cost_per_tile_unit)?;
      let dst_payment = src_revenue.checked_add(transport_total)?;

      // Sellers at src: prorata goods, prorata cash out of src_revenue (Σ == src_revenue).
      let seller_w: Vec<i64> = sellers.iter().map(|(_, w)| *w).collect();
      let seller_goods = prorata_distribute(&seller_w, q);
      let seller_cash = prorata_distribute(&seller_goods, src_revenue.0);
      for (idx, (actor, _)) in sellers.iter().enumerate() {
          let goods = seller_goods[idx];
          if goods > 0 {
              inventory.consume(*actor, flow.good, Quantity(goods))?;
          }
          let receipt = Money(seller_cash[idx]);
          if receipt.0 > 0 {
              accounts.deposit(*actor, receipt)?;
          }
      }

      // Buyers at dst: prorata goods, prorata charge out of dst_payment (Σ == dst_payment).
      let buyer_w: Vec<i64> = buyers.iter().map(|(_, w)| *w).collect();
      let buyer_goods = prorata_distribute(&buyer_w, q);
      let buyer_charge = prorata_distribute(&buyer_goods, dst_payment.0);
      for (idx, (actor, _)) in buyers.iter().enumerate() {
          let goods = buyer_goods[idx];
          let charge = Money(buyer_charge[idx]);
          if charge.0 > 0 {
              accounts.lock_cash(*actor, charge)?;
              accounts.debit_locked(*actor, charge)?;
          }
          if goods > 0 {
              inventory.deposit(*actor, flow.good, Quantity(goods))?;
          }
      }

      // Transport: deposit to the reserved operator (transfer, never destroyed).
      if transport_total.0 > 0 {
          accounts.deposit(TRANSPORT_OPERATOR, transport_total)?;
      }

      // Write-back at src and dst. Residuals are against EFFECTIVE demand/supply:
      // post-flow unmet/unsold = effective_side - traded_q (clamped at 0).
      write_back(
          market_goods,
          MarketGoodKey { market: flow.src, good: flow.good },
          flow.p_src,
          q,
          (eff_demand_src - q).max(0),
          (eff_supply_src - q).max(0),
          current_tick,
      );
      if flow.dst != flow.src {
          write_back(
              market_goods,
              MarketGoodKey { market: flow.dst, good: flow.good },
              flow.p_dst,
              q,
              (eff_demand_dst - q).max(0),
              (eff_supply_dst - q).max(0),
              current_tick,
          );
      }

      Ok(EconomyEvent::MacroFlow {
          from_market: flow.src,
          to_market: flow.dst,
          good: flow.good,
          qty: Quantity(q),
          price: flow.p_dst,
          transport: transport_total,
      })
  }

  /// Apply the STEP-G market-state write-back for one endpoint. Accumulates
  /// `traded_qty_last_tick` (a market may both self-clear and import in one
  /// interval), sets the discovered price, last cleared tick, and post-flow
  /// residual imbalance. Intentionally does NOT touch `dirty`.
  fn write_back(
      market_goods: &mut MarketGoods,
      key: MarketGoodKey,
      price: Money,
      traded: i64,
      unmet_demand: i64,
      unsold_supply: i64,
      current_tick: u64,
  ) {
      let state = market_goods
          .0
          .entry(key)
          .or_insert_with(|| MarketGoodState::new(key));
      state.last_settlement_price = price;
      state.traded_qty_last_tick = Quantity(state.traded_qty_last_tick.0 + traded);
      state.unmet_demand_last_tick = Quantity(unmet_demand);
      state.unsold_supply_last_tick = Quantity(unsold_supply);
      state.last_cleared_tick = current_tick;
  }
  ```
  Run `... settle_flow_conserves_and_credits_operator_exactly` — expect PASS.

- [ ] Add a failing N-buyer per-line-floor conservation test pinning the aggregate-floor scheme (summing N per-line floors would lose units; the single `dst_payment` floor must not). Use a price/qty combination that floors: `p_dst = Money(2_000)`, `q = 3` split across 2 buyers (weights 2/1) → `dst_payment = checked_order_value(2000,3)=6` (+transport), prorata of 6 by goods 2/1 = 4/2 exactly; choose a fractional case `q=3`, `p_src=Money(500)` so `src_revenue = floor(500*3/1000)=1` and assert seller gets exactly 1 and Σ buyer charges == dst_payment:
  ```rust
  #[test]
  fn settle_flow_n_buyers_aggregate_floor_conserves() {
      let a = MarketId(1);
      let b = MarketId(2);
      let seller = EconomicActorId(10);
      let buyer1 = EconomicActorId(20);
      let buyer2 = EconomicActorId(21);
      let mut accounts = AccountBook::default();
      let mut inventory = InventoryBook::default();
      accounts.deposit(buyer1, Money(1_000_000)).unwrap();
      accounts.deposit(buyer2, Money(1_000_000)).unwrap();
      inventory.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
      let mut market_goods = MarketGoods::default();
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);

      // q=3, p_src=500 -> src_revenue floor(1500/1000)=1 ; transport (50*3/1000)floor=0.
      let flow = PlannedFlow { good: GOOD_FOOD, src: a, dst: b, q: 3,
          p_src: Money(500), p_dst: Money(2_000), dist: 1 };
      let sellers = vec![(seller, 3i64)];
      let buyers = vec![(buyer1, 2i64), (buyer2, 1i64)];
      let m0 = accounts.total_money().unwrap();
      let g0 = inventory.total_good(GOOD_FOOD).unwrap();

      let mut na = accounts.clone();
      let mut ni = inventory.clone();
      let ev = settle_flow(&mut na, &mut ni, &mut market_goods, &flow, &sellers, &buyers,
          0, 3, 3, 0, &cfg, 10).unwrap();
      accounts = na; inventory = ni;

      assert_eq!(accounts.total_money().unwrap(), m0, "money conserved with N buyers");
      assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
      // transport floored to 0 -> dst_payment == src_revenue == 1 ; Σ buyer charges == 1.
      let charged = m0.0 - (accounts.account(buyer1).available.0 + accounts.account(buyer2).available.0)
          - accounts.account(TRANSPORT_OPERATOR).available.0;
      assert_eq!(charged, accounts.account(seller).available.0,
          "Σ buyer charges == seller revenue (no per-line floor leak)");
      if let EconomyEvent::MacroFlow { transport, qty, .. } = ev {
          assert_eq!(transport, Money(0));
          assert_eq!(qty, Quantity(3));
      } else { panic!("expected MacroFlow"); }
  }
  ```
  Run `... settle_flow_n_buyers_aggregate_floor_conserves` — expect PASS.

- [ ] Add a self-edge settlement test (intra-market clearing, transport 0, `from==to`, single write-back). A both-sided market clears its matched overlap at its own price:
  ```rust
  #[test]
  fn settle_flow_self_edge_clears_locally_transport_zero() {
      let m = MarketId(1);
      let seller = EconomicActorId(10);
      let buyer = EconomicActorId(20);
      let mut accounts = AccountBook::default();
      let mut inventory = InventoryBook::default();
      accounts.deposit(buyer, Money(1_000_000)).unwrap();
      inventory.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
      let mut market_goods = MarketGoods::default();
      let cfg = EconomyConfig::default();
      let flow = PlannedFlow { good: GOOD_FOOD, src: m, dst: m, q: 40,
          p_src: Money(1_000), p_dst: Money(1_000), dist: 0 };
      let m0 = accounts.total_money().unwrap();
      let g0 = inventory.total_good(GOOD_FOOD).unwrap();
      let mut na = accounts.clone();
      let mut ni = inventory.clone();
      let ev = settle_flow(&mut na, &mut ni, &mut market_goods, &flow,
          &[(seller, 40)], &[(buyer, 40)], 40, 40, 40, 40, &cfg, 0).unwrap();
      accounts = na; inventory = ni;
      assert_eq!(accounts.total_money().unwrap(), m0);
      assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
      assert_eq!(accounts.account(TRANSPORT_OPERATOR).available, Money(0));
      if let EconomyEvent::MacroFlow { from_market, to_market, transport, .. } = ev {
          assert_eq!(from_market, to_market);
          assert_eq!(transport, Money(0));
      } else { panic!("expected MacroFlow"); }
      // single write-back for the self market.
      let st = market_goods.0.get(&MarketGoodKey { market: m, good: GOOD_FOOD }).unwrap();
      assert_eq!(st.traded_qty_last_tick, Quantity(40));
  }
  ```
  Run `... settle_flow_self_edge_clears_locally_transport_zero` — expect PASS.

- [ ] Run `... economy::tests::macro_flow` — expect PASS. Commit: `feat(economy): macro-flow STEP G settle one flow + price write-back`.

### Task 11: STEP H fault isolation + STEP I emit + assemble `run_macro_flow_at_tick` (conditional clone, interval gate)

**Files:** `backend/crates/sim-core/src/economy/macro_flow.rs`, `backend/crates/sim-core/src/economy/tests/macro_flow.rs`

This task assembles the public entry point with the interval gate, the conditional clone (compute candidates read-only first; clone only if non-empty), the STEP-F→G→H→I loop with per-edge settle-fault isolation, and the atomic commit. After this task `run_macro_flow_at_tick` exists with the canonical signature and is tested by direct calls. The schedule is still wired to warm-flow (replaced in Task 12).

- [ ] Add a failing interval-gate test: tick 3 (not a multiple of 10) does nothing; tick 0 and tick 10 flow. Build via direct call with the canonical signature:
  ```rust
  use crate::economy::macro_flow::run_macro_flow_at_tick;
  use crate::economy::{DirtyMarketGoods};

  fn surplus_deficit_world() -> (AccountBook, InventoryBook, TradeLedger, DemandPools,
      SupplyPools, MarketGoods, DirtyMarketGoods, BTreeSet<MarketId>, MarketDistances, EconomyConfig)
  {
      let a = MarketId(1);
      let b = MarketId(2);
      let seller = EconomicActorId(10);
      let buyer = EconomicActorId(20);
      let mut accounts = AccountBook::default();
      let mut inventory = InventoryBook::default();
      accounts.deposit(buyer, Money(1_000_000)).unwrap();
      inventory.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
      let mut demand = DemandPools::default();
      demand.0.insert(buyer, DemandPool { actor: buyer, market: b, good: GOOD_FOOD,
          desired_qty_per_tick: Quantity(100), max_price: Money(2_000), urgency_bps: 0,
          elasticity_bps: 0, interval_ticks: 1, last_generated_tick: None });
      let mut supply = SupplyPools::default();
      supply.0.insert(seller, SupplyPool { actor: seller, market: a, good: GOOD_FOOD,
          offered_qty_per_tick: Quantity(100), min_price: Money(500), interval_ticks: 1,
          last_generated_tick: None });
      let dormant: BTreeSet<MarketId> = [a, b].into_iter().collect();
      let mut distances = MarketDistances::default();
      distances.0.insert((a, b), 1); distances.0.insert((b, a), 1);
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);
      (accounts, inventory, TradeLedger::default(), demand, supply, MarketGoods::default(),
       DirtyMarketGoods::default(), dormant, distances, cfg)
  }

  #[test]
  fn macro_flow_only_fires_on_interval() {
      let (mut acc, mut inv, mut led, dem, sup, mut mg, dirty, dormant, dist, cfg) =
          surplus_deficit_world();
      // tick 3: not a multiple of 10 -> no flow, no events.
      run_macro_flow_at_tick(&mut acc, &mut inv, &mut led, &dem, &sup, &mut mg, &dirty,
          &dormant, &dist, &cfg, 3).unwrap();
      assert!(led.0.is_empty(), "no flow off-interval");
      // tick 10: fires.
      run_macro_flow_at_tick(&mut acc, &mut inv, &mut led, &dem, &sup, &mut mg, &dirty,
          &dormant, &dist, &cfg, 10).unwrap();
      assert!(led.0.iter().any(|e| matches!(e, EconomyEvent::MacroFlow { .. })),
          "flow on interval tick");
  }
  ```
  Run `... macro_flow_only_fires_on_interval` — expect FAIL (no `run_macro_flow_at_tick`).

- [ ] Add `run_macro_flow_at_tick` to `macro_flow.rs` with the EXACT canonical signature. It gates on the interval, builds buckets read-only against the LIVE books, builds+sorts candidates, plans flows; if the plan is empty it returns `Ok(())` WITHOUT cloning; otherwise it clones, settles each flow with per-edge fault isolation (settle Err → emit `MarketClearFailed`, skip the edge, restore the clones to their pre-edge state by re-cloning from the last good snapshot), emits `MacroFlow` per accepted flow, and commits. To keep per-edge atomicity simple and correct, settle into a throwaway clone of the running books and only fold it back in on success:
  ```rust
  /// STEP H/I + assembly: the per-interval macro flow over all dormant markets.
  /// Interval-gated, conditional-clone (no clone on a quiescent interval), atomic
  /// clone-validate-apply with per-edge settle-fault isolation, then commit + emit.
  #[allow(clippy::too_many_arguments)]
  pub fn run_macro_flow_at_tick(
      accounts: &mut AccountBook,
      inventory: &mut InventoryBook,
      ledger: &mut TradeLedger,
      demand: &DemandPools,
      supply: &SupplyPools,
      market_goods: &mut MarketGoods,
      dirty: &DirtyMarketGoods,
      dormant: &BTreeSet<MarketId>,
      distances: &MarketDistances,
      config: &EconomyConfig,
      current_tick: u64,
  ) -> Result<(), EconomyError> {
      if config.macro_flow_interval_ticks == 0
          || !current_tick.is_multiple_of(config.macro_flow_interval_ticks)
      {
          return Ok(());
      }

      // STEP A-D against the LIVE books, read-only. Skip keys still settling under
      // the auction (handoff skip-guard §5): a (market,good) currently dirty is
      // dropped from the dormant bucket set.
      let buckets = build_macro_buckets(accounts, inventory, demand, supply, market_goods, dormant, config)?;
      let buckets: BTreeMap<MarketGoodKey, MacroBucket> = buckets
          .into_iter()
          .filter(|(key, _)| !dirty.0.contains(key))
          .collect();
      let mut candidates = build_candidates(&buckets, distances, config)?;
      sort_candidates(&mut candidates);
      let flows = plan_flows(&candidates, &buckets);
      if flows.is_empty() {
          return Ok(()); // truly-quiescent interval: NO clone.
      }

      // Atomic boundary: mutate clones, commit on success.
      let mut next_accounts = accounts.clone();
      let mut next_inventory = inventory.clone();
      let mut next_goods = market_goods.clone();
      let mut events: Vec<EconomyEvent> = Vec::new();

      // Per-market effective demand/supply for the write-back residuals (bucket-time).
      let effective = |market: MarketId, good: GoodId| -> (i64, i64) {
          match buckets.get(&MarketGoodKey { market, good }) {
              Some(b) => (b.total_demand(), b.total_supply()),
              None => (0, 0),
          }
      };

      for flow in &flows {
          // Recover the seller/buyer weight lists for this flow's endpoints.
          let sellers = buckets
              .get(&MarketGoodKey { market: flow.src, good: flow.good })
              .map(|b| b.sellers.clone())
              .unwrap_or_default();
          let buyers = buckets
              .get(&MarketGoodKey { market: flow.dst, good: flow.good })
              .map(|b| b.buyers.clone())
              .unwrap_or_default();
          let (eff_demand_src, eff_supply_src) = effective(flow.src, flow.good);
          let (eff_demand_dst, eff_supply_dst) = effective(flow.dst, flow.good);

          // STEP H fault isolation: settle into a scratch clone; fold back only on
          // success, else emit MarketClearFailed and skip (books byte-identical).
          let mut scratch_accounts = next_accounts.clone();
          let mut scratch_inventory = next_inventory.clone();
          let mut scratch_goods = next_goods.clone();
          match settle_flow(
              &mut scratch_accounts,
              &mut scratch_inventory,
              &mut scratch_goods,
              flow,
              &sellers,
              &buyers,
              eff_demand_src,
              eff_supply_src,
              eff_demand_dst,
              eff_supply_dst,
              config,
              current_tick,
          ) {
              Ok(event) => {
                  next_accounts = scratch_accounts;
                  next_inventory = scratch_inventory;
                  next_goods = scratch_goods;
                  events.push(event);
              }
              Err(reason) => {
                  events.push(EconomyEvent::MarketClearFailed {
                      market: flow.dst,
                      good: flow.good,
                      reason,
                  });
              }
          }
      }

      *accounts = next_accounts;
      *inventory = next_inventory;
      *market_goods = next_goods;
      ledger.0.extend(events);
      Ok(())
  }
  ```
  Run `... macro_flow_only_fires_on_interval` — expect PASS.

- [ ] Add a failing test that an idle interval does NO clone, asserted by byte-identical books across a no-op flow tick (empty pools → empty plan → early return). The proof is structural (early `return Ok(())` before any `.clone()`), validated behaviorally by asserting the books are unchanged and `total_money`/`total_good` identical:
  ```rust
  #[test]
  fn macro_flow_idle_interval_is_a_noop() {
      let mut acc = AccountBook::default();
      let mut inv = InventoryBook::default();
      let mut led = TradeLedger::default();
      let mut mg = MarketGoods::default();
      let dormant: BTreeSet<MarketId> = [MarketId(1)].into_iter().collect();
      let cfg = EconomyConfig::default();
      let before_acc = acc.clone();
      let before_inv = inv.clone();
      // tick 0 is an interval tick, but there are no pools -> empty plan -> no clone.
      run_macro_flow_at_tick(&mut acc, &mut inv, &mut led, &DemandPools::default(),
          &SupplyPools::default(), &mut mg, &DirtyMarketGoods::default(), &dormant,
          &MarketDistances::default(), &cfg, 0).unwrap();
      assert_eq!(acc, before_acc, "books byte-identical on idle interval");
      assert_eq!(inv, before_inv);
      assert!(led.0.is_empty());
      assert!(mg.0.is_empty(), "no write-back on idle interval");
  }
  ```
  Run `... macro_flow_idle_interval_is_a_noop` — expect PASS.

- [ ] Add a failing settle-fault-isolation test: two surplus sources target one buyer in one interval; the second edge's `lock_cash` exceeds the buyer's bucket-time affordability (budgets computed once against the live books per STEP A — the buyer's effective demand was capped by affordability, but two sources both routing to it can over-charge at settle time), so the second settle errors → `MarketClearFailed` emitted, the healthy flow conserves, and the faulted edge moves nothing. Construct it: buyer at B affords 100 units at price 2000 (cash exactly `checked_order_value(2000,100)+transport` for ONE source), two sources A1 and A2 each with surplus 100 and a high net_gain so both become candidates; after the first edge consumes the buyer's `remaining_need`, the second is budget-zero — so to force a SETTLE fault (not a budget skip), give the buyer `remaining_need` large enough for both but cash for only one:
  ```rust
  #[test]
  fn macro_flow_settle_fault_isolates_and_conserves() {
      let a1 = MarketId(1); // cheap surplus source 1
      let a2 = MarketId(2); // cheap surplus source 2
      let bdst = MarketId(3); // dear deficit sink (one buyer, limited cash)
      let s1 = EconomicActorId(10);
      let s2 = EconomicActorId(11);
      let buyer = EconomicActorId(20);
      let mut accounts = AccountBook::default();
      let mut inventory = InventoryBook::default();
      // Buyer demand 200 at max 2000 -> needs affordability for 200 to be uncapped,
      // but we give cash that affords only ~120 units of effective demand, while the
      // two edges (100 + 100) both clear against the bucket-time need 120 -> second
      // edge's prorata charge exceeds remaining cash at settle time.
      accounts.deposit(buyer, Money(240_000)).unwrap(); // affords floor(240000*1000/2000)=120
      inventory.deposit(s1, GOOD_FOOD, Quantity(100)).unwrap();
      inventory.deposit(s2, GOOD_FOOD, Quantity(100)).unwrap();
      let mut demand = DemandPools::default();
      demand.0.insert(buyer, DemandPool { actor: buyer, market: bdst, good: GOOD_FOOD,
          desired_qty_per_tick: Quantity(200), max_price: Money(2_000), urgency_bps: 0,
          elasticity_bps: 0, interval_ticks: 1, last_generated_tick: None });
      let mut supply = SupplyPools::default();
      supply.0.insert(s1, SupplyPool { actor: s1, market: a1, good: GOOD_FOOD,
          offered_qty_per_tick: Quantity(100), min_price: Money(500), interval_ticks: 1,
          last_generated_tick: None });
      supply.0.insert(s2, SupplyPool { actor: s2, market: a2, good: GOOD_FOOD,
          offered_qty_per_tick: Quantity(100), min_price: Money(500), interval_ticks: 1,
          last_generated_tick: None });
      let dormant: BTreeSet<MarketId> = [a1, a2, bdst].into_iter().collect();
      let mut distances = MarketDistances::default();
      for (x, y) in [(a1, bdst), (a2, bdst), (bdst, a1), (bdst, a2)] {
          distances.0.insert((x, y), 1);
      }
      let mut cfg = EconomyConfig::default();
      cfg.transport_cost_per_tile_unit = Money(50);
      let mut market_goods = MarketGoods::default();
      let mut ledger = TradeLedger::default();
      let m0 = accounts.total_money().unwrap();
      let g0 = inventory.total_good(GOOD_FOOD).unwrap();

      run_macro_flow_at_tick(&mut accounts, &mut inventory, &mut ledger, &demand, &supply,
          &mut market_goods, &DirtyMarketGoods::default(), &dormant, &distances, &cfg, 0).unwrap();

      // Conservation holds across the whole tick (faulted edge left books unchanged for it).
      assert_eq!(accounts.total_money().unwrap(), m0);
      assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
      // At least one healthy MacroFlow AND exactly one MarketClearFailed for the over-charged edge.
      assert!(ledger.0.iter().any(|e| matches!(e, EconomyEvent::MacroFlow { .. })));
      assert_eq!(
          ledger.0.iter().filter(|e| matches!(e, EconomyEvent::MarketClearFailed { .. })).count(),
          1,
          "the over-charging edge faults exactly once, others healthy"
      );
      assert!(accounts.account(buyer).available.0 >= 0, "no overdraw");
  }
  ```
  Run `... macro_flow_settle_fault_isolates_and_conserves` — expect PASS. (Note: effective demand at B is capped to 120 by affordability in STEP A; need=120; first edge A1→B fills `min(surplus100, need120, q_cap100)=100`; second edge A2→B fills `min(surplus100, need20, q_cap)=20` at settle — that DOES fit affordability, so this configuration actually conserves WITHOUT a fault. ADJUST so the second edge over-charges: give the buyer enough EFFECTIVE demand but not enough cash by making the per-source prices differ so STEP-A affordability is computed at the lower bucket price while settle charges at the higher per-source `p_src`. Concretely set both sources' bucket price equal but make the buyer's STEP-A `affordable_qty` at B's synthetic price exceed its true cash for the COMBINED dst_payment of two edges — achieved by leaving B demand-only so `p_m = bid_ceiling = 2000` and affordability = floor(cash/2). With cash 240000 → affordable 120, need 120; two surplus-100 sources → first takes 100, second takes 20 → both fit. To force the fault, lower the buyer cash to `Money(110_000)` → affordable 55, need 55; first edge A1 takes 55, second edge A2 takes 0 (budget) → no settle fault, just a budget skip. The reliable settle-fault construction is: keep `remaining_need` LARGER than affordability by making STEP-A NOT cap demand — i.e. give the buyer huge desired_qty and huge cash so effective demand = desired = 200, but then have the buyer's cash CONSUMED by an unrelated earlier self-edge in the SAME interval. Simpler and spec-faithful: per STEP H the budgets are computed once against the LIVE books; a buyer that participates as BOTH a local self-edge buyer at B (consuming its cash) AND an importer will, on the import edge, find its post-self-edge cash insufficient. Build B as a both-sided market (local seller + the buyer) so the self-edge debits the buyer first, then the cross-edge import over-charges.) REVISE the test fixture accordingly: add a local seller at B with a small supply so B has a self-edge that runs before the import, draining the buyer's cash; size cash so the self-edge succeeds but the subsequent import `lock_cash` fails. Document the exact arithmetic in a comment and recompute by hand.

- [ ] Run `... economy::tests::macro_flow` — expect PASS. Commit: `feat(economy): macro-flow STEP H/I + run_macro_flow_at_tick (conditional clone, fault isolation)`.

### Task 12 (ATOMIC CLEAN REPLACEMENT): rename config+set, delete warm-flow, wire `run_macro_flow_system`

**Files:** `backend/crates/sim-core/src/economy/systems.rs`, `backend/crates/sim-core/src/economy/ledger.rs`, `backend/crates/sim-core/src/economy/market.rs`, `backend/crates/sim-core/src/economy/mod.rs`, `backend/crates/sim-core/src/economy/warm_flow.rs` (DELETE), `backend/crates/sim-core/src/economy/tests/warm_flow.rs` (DELETE), `backend/crates/sim-core/src/economy/tests/mod.rs`, `backend/crates/sim-core/src/economy/tests/lod.rs`

ONE task, ONE commit. Everything warm-related goes at once so the build is green before and after with no throwaway bridging edit. Per the ORDERING rules this runs AFTER all of Section B's standalone work AND after Section A's additive `MacroFlow` event was added (so the event/config/set already exist additively; here we complete the rename and delete the dead warm path). Coordinate: the scaffolding added `EconomyEvent::MacroFlow` (and its `event_type()` arm) and registered `MarketDistances` additively; this task does the destructive half.

- [ ] In `mod.rs`: delete `pub mod warm_flow;` and `pub use warm_flow::*;`. ADD `pub use macro_flow::*;` (promoting the macro_flow module's public items to the `crate::economy::*` namespace now that warm_flow's colliding glob is gone). Run `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core` — expect FAIL to compile (systems.rs still imports `run_warm_market_flow_at_tick`/`WarmMarkets`; tests/warm_flow.rs still exists; tests/mod.rs still has `mod warm_flow;`). This confirms the dead references that the rest of the task removes.

- [ ] Delete the file `backend/crates/sim-core/src/economy/warm_flow.rs` (`run_warm_market_flow_at_tick` + `warm_ref_price` are replaced by `macro_flow.rs`). Delete the file `backend/crates/sim-core/src/economy/tests/warm_flow.rs`. In `tests/mod.rs` change `mod warm_flow;` to `mod macro_flow;` (the new test module from Task 6).

- [ ] In `ledger.rs`: delete the `WarmMarketFlow { market, good, qty, price }` variant (lines 72-77) and its `event_type()` arm `Self::WarmMarketFlow { .. } => "warm_market_flow",` (line 98). (The `MacroFlow` variant + its `"macro_flow"` arm were already added additively by the scaffolding — leave them.)

- [ ] In `market.rs`: delete the `WarmMarkets` resource (lines 75-79) and its doc comment.

- [ ] In `systems.rs`: rename `EconomyConfig.warm_flow_interval_ticks` → `macro_flow_interval_ticks` (the field at line 37 and its `Default` init at line 49; default stays `10`). Rename `EconomySet::WarmFlow` → `EconomySet::MacroFlow` (the enum variant at line 25, the `.configure_sets` entry at line 64, the `.in_set` at line 78). Rename `run_warm_market_flow_system` → `run_macro_flow_system`.

- [ ] In `systems.rs` `refresh_dormant_markets_system`: remove the `mut warm: ResMut<WarmMarkets>` parameter (line 104), the `warm_coords` binding (line 107), and the `warm.0 = …` block (lines 114-119). The system now produces ONLY `DormantMarkets`. Update its doc comment to drop the WarmMarkets sentence. Remove `WarmMarkets` from the `use crate::economy::{…}` import list (line 9).

- [ ] In `systems.rs`: rewrite `run_macro_flow_system` with the new signature — drop `Res<WarmMarkets>`, add `Res<DormantMarkets>` + `Res<DirtyMarketGoods>` + `Res<MarketDistances>`, take `ResMut<MarketGoods>` (was `Res`), and STOP swallowing the `Result` (on `Err` emit a `MarketClearFailed` audit event). Update the `use` import to swap `run_warm_market_flow_at_tick` → `run_macro_flow_at_tick` and add `DirtyMarketGoods`, `MarketDistances` (and drop `WarmMarkets`):
  ```rust
  #[allow(clippy::too_many_arguments)]
  pub fn run_macro_flow_system(
      tick: Res<Tick>,
      config: Res<EconomyConfig>,
      dormant: Res<DormantMarkets>,
      dirty: Res<DirtyMarketGoods>,
      distances: Res<MarketDistances>,
      mut accounts: ResMut<AccountBook>,
      mut inventory: ResMut<InventoryBook>,
      mut ledger: ResMut<TradeLedger>,
      demand: Res<DemandPools>,
      supply: Res<SupplyPools>,
      mut market_goods: ResMut<MarketGoods>,
  ) {
      if let Err(reason) = run_macro_flow_at_tick(
          &mut accounts,
          &mut inventory,
          &mut ledger,
          &demand,
          &supply,
          &mut market_goods,
          &dirty,
          &dormant.0,
          &distances,
          &config,
          tick.0,
      ) {
          // A whole-interval failure (e.g. a bucket-build overflow) is audited; the
          // atomic boundary left the books unchanged. Per-edge settle faults are
          // already isolated inside run_macro_flow_at_tick (their own
          // MarketClearFailed events). market/good = the demo sentinel for a
          // tick-level fault that is not attributable to one (market,good).
          ledger.0.push(EconomyEvent::MarketClearFailed {
              market: MarketId(0),
              good: GoodId(0),
              reason,
          });
      }
  }
  ```
  Add `MarketDistances`, `DirtyMarketGoods`, `MarketId`, `GoodId`, `run_macro_flow_at_tick` to the `use crate::economy::{…}` import block and remove `run_warm_market_flow_at_tick`, `WarmMarkets`. Note: `run_macro_flow_system` and `update_market_telemetry_system` BOTH take `ResMut<MarketGoods>`, but they are totally ordered by the `.chain()` (MacroFlow before Telemetry), so the multi-threaded executor sees no resource-access ambiguity — no extra `.before()` needed.

- [ ] In `systems.rs` `install_systems`: the chained set tuple now reads `…ClearMarkets, MacroFlow, Materialize, Telemetry` and `run_macro_flow_system.in_set(EconomySet::MacroFlow)`. Update the comment at lines 84-85 (`The set chain places it after WarmFlow.` → `after MacroFlow.`).

- [ ] In `mod.rs` `EconomyPlugin::install`: delete `world.insert_resource(WarmMarkets::default());` (line 68). (the scaffolding already added `world.insert_resource(MarketDistances::default());` additively — leave it.)

- [ ] In `tests/lod.rs`: remove the `WarmMarkets` import (line 6) and the two `world.insert_resource(WarmMarkets::default());` calls (lines 27 and 44). Invert `asleep_anchored_market_stays_frozen_end_to_end` (line 330) into `asleep_anchored_market_DOES_flow`: keep the `lod_world(market, coord, asleep=true)` setup but ALSO seed a demand pool at a SECOND asleep-anchored market so a real A→B gap exists, run several intervals, and assert `last_settlement_price`/`traded_qty_last_tick` changed AND goods moved A→B (proves no-hollow-world). Recompute the exact moved quantity by hand from the seeded pools and `macro_flow_interval_ticks=10`; set `transport_cost_per_tile_unit = Money(50)` in the test's `EconomyConfig` override and a finite `MarketDistances` entry for the two markets so transport > 0.

- [ ] Run `grep -n WarmMarkets backend/crates/sim-core/src/economy/tests/lod.rs` — expect EMPTY output (verify no stray `WarmMarkets` reference survives). Then run `grep -rn 'warm_flow\|WarmMarketFlow\|WarmMarkets\|warm_flow_interval_ticks\|EconomySet::WarmFlow\|run_warm_market_flow' backend/crates/sim-core/src` — expect EMPTY (no dead warm references anywhere).

- [ ] Run the FULL sim-core test suite: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core` — expect PASS (all macro_flow tests, the inverted lod test, the existing conservation/determinism/persist/audit suites green). If the `economy_snapshot_round_trips`/`_is_byte_stable` tests fail on the new `market_distances` field, that is the test section/A's persist task — within THIS task only confirm no warm-related breakage; the persist-field extension is owned elsewhere.

- [ ] Commit: `refactor(economy): atomic clean replacement of warm-flow with macro-flow (rename config+set, delete WarmMarkets/WarmMarketFlow, wire run_macro_flow_system)`.

### Task 13: macro_flow_world builder + multi-market dormant scenario helper + conservation tests

**Files:** `backend/crates/sim-core/src/economy/tests/macro_flow.rs` (NEW — created by Task 12 (the atomic clean replacement) as `mod macro_flow;` in tests/mod.rs; this task is the first Writer-C addition to it), `backend/crates/sim-core/src/economy/tests/mod.rs` (only if `mod macro_flow;` is not yet present — the core section already swapped `mod warm_flow;`→`mod macro_flow;`, so verify, do not re-add).

- [ ] Write the reusable world builder + scenario helper at the top of `tests/macro_flow.rs` (these are reused BY NAME by every later Section-C test). Use the COMPLETE Rust below:

```rust
use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, DemandPool, DemandPools, DormantMarkets, EconomicActorId, EconomyConfig,
    EconomyError, EconomyEvent, GOOD_FOOD, GOOD_TOOLS, InventoryBook, MarketChunks, MarketDistances,
    MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money, Quantity, SupplyPool, SupplyPools,
    TradeLedger, TRANSPORT_OPERATOR, run_macro_flow_at_tick,
};
use crate::economy::EconomyPlugin;
use crate::economy::transport::transport_cost;
use crate::mobility::resources::Tick;
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;

/// Full Core+Mobility+Economy world so the wired `EconomySet` chain runs end to
/// end. Inserts an (initially empty) `MarketDistances` table + `Tick(0)`. Markets
/// are NOT anchored here (callers anchor + set distances), so the schedule-level
/// tests drive the real `run_macro_flow_system`. Reused by every Section-C test.
fn macro_flow_world() -> World {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);
    world.insert_resource(MarketDistances(BTreeMap::new()));
    world.insert_resource(Tick(0));
    world
}

/// A direct-call scenario: two dormant markets, `surplus` (cheap, supply-only or
/// net-surplus) and `deficit` (dear, demand-only or net-deficit), wired into bare
/// books + pools so a single `run_macro_flow_at_tick` call moves goods cheap→dear.
/// All actors funded so affordability never floors. `rate` sets transport.
struct DormantScenario {
    accounts: AccountBook,
    inventory: InventoryBook,
    ledger: TradeLedger,
    demand: DemandPools,
    supply: SupplyPools,
    market_goods: MarketGoods,
    dirty: crate::economy::DirtyMarketGoods,
    dormant: BTreeSet<MarketId>,
    distances: MarketDistances,
    config: EconomyConfig,
}

fn dp(actor: u64, market: MarketId, qty: i64, max_price: i64) -> DemandPool {
    DemandPool {
        actor: EconomicActorId(actor),
        market,
        good: GOOD_FOOD,
        desired_qty_per_tick: Quantity(qty),
        max_price: Money(max_price),
        urgency_bps: 0,
        elasticity_bps: 0,
        interval_ticks: 1,
        last_generated_tick: None,
    }
}
fn sp(actor: u64, market: MarketId, qty: i64, min_price: i64) -> SupplyPool {
    SupplyPool {
        actor: EconomicActorId(actor),
        market,
        good: GOOD_FOOD,
        offered_qty_per_tick: Quantity(qty),
        min_price: Money(min_price),
        interval_ticks: 1,
        last_generated_tick: None,
    }
}

/// Build a one-line surplus@A→deficit@B scenario. `n_buyers` consumers at B share
/// the demand. `rate` is the transport rate; `dist` the A↔B distance (both ways).
fn surplus_deficit_scenario(
    n_sellers: u64,
    seller_qty: i64,
    ask_floor: i64,
    n_buyers: u64,
    buyer_qty: i64,
    bid_ceiling: i64,
    dist: i64,
    rate: Money,
) -> DormantScenario {
    let m_a = MarketId(1);
    let m_b = MarketId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();

    for s in 0..n_sellers {
        let actor = 100 + s;
        inventory
            .deposit(EconomicActorId(actor), GOOD_FOOD, Quantity(1_000_000))
            .unwrap();
        supply.0.insert(EconomicActorId(actor), sp(actor, m_a, seller_qty, ask_floor));
    }
    for c in 0..n_buyers {
        let actor = 200 + c;
        accounts.deposit(EconomicActorId(actor), Money(1_000_000_000)).unwrap();
        demand.0.insert(EconomicActorId(actor), dp(actor, m_b, buyer_qty, bid_ceiling));
    }

    let mut distances = MarketDistances(BTreeMap::new());
    distances.0.insert((m_a, m_b), dist);
    distances.0.insert((m_b, m_a), dist);

    let mut config = EconomyConfig::default();
    config.transport_cost_per_tile_unit = rate;

    DormantScenario {
        accounts,
        inventory,
        ledger: TradeLedger::default(),
        demand,
        supply,
        market_goods: MarketGoods::default(),
        dirty: crate::economy::DirtyMarketGoods::default(),
        dormant: [m_a, m_b].into_iter().collect(),
        distances,
        config,
    }
}

fn run_flow(s: &mut DormantScenario, tick: u64) -> Result<(), EconomyError> {
    run_macro_flow_at_tick(
        &mut s.accounts,
        &mut s.inventory,
        &mut s.ledger,
        &s.demand,
        &s.supply,
        &mut s.market_goods,
        &s.dirty,
        &s.dormant,
        &s.distances,
        &s.config,
        tick,
    )
}
```

- [ ] Write the first conservation test below it:

```rust
#[test]
fn macro_flow_conserves_money_and_goods() {
    // surplus@A (10 units @ ask 500) → deficit@B (10 units @ bid 2000), dist 4,
    // rate 50: transport = (50*10/1000)*4 = floor(0.5)*4 ... q_cap*rate=500 < 1000
    // floors to 0 — so use seller_qty 200 so rate*q = 50*200 = 10000 >= 1000.
    let mut s = surplus_deficit_scenario(1, 200, 500, 1, 200, 2000, 4, Money(50));
    let money_before = s.accounts.total_money().unwrap();
    let good_before = s.inventory.total_good(GOOD_FOOD).unwrap();
    let op_before = s.accounts.account(TRANSPORT_OPERATOR).available;

    run_flow(&mut s, 0).unwrap();

    // q = min(surplus 200, need 200) = 200; transport = (50*200/1000)*4 = 10*4 = 40.
    let q = Quantity(200);
    let expected_transport = transport_cost(4, q, Money(50)).unwrap();
    assert_eq!(expected_transport, Money(40));

    assert_eq!(s.accounts.total_money().unwrap(), money_before, "money conserved");
    assert_eq!(s.inventory.total_good(GOOD_FOOD).unwrap(), good_before, "goods conserved");
    let op_after = s.accounts.account(TRANSPORT_OPERATOR).available;
    assert_eq!(
        op_after.0 - op_before.0,
        expected_transport.0,
        "operator gained exactly transport_total"
    );
    for (_, acct) in &s.accounts.accounts {
        assert!(acct.available.0 >= 0, "no negative available cash");
    }
}
```

- [ ] Run, expect FAIL (compile error — `run_macro_flow_at_tick` exists from the core section, but `MarketDistances`/builder reference is new; if any helper symbol is missing the build fails): `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core macro_flow_conserves_money_and_goods`. Confirm it FAILS to compile or asserts (the builder/test compiles only once all referenced symbols from the scaffolding/B exist).
- [ ] Add the N-buyers-per-line-floor conservation test (pins the aggregate-floor cash scheme — forbids per-line charging):

```rust
#[test]
fn macro_flow_conserves_with_N_buyers_per_line_floor() {
    // 3 buyers each wanting 67 → 201 total demand vs 201 supply; ask 333, bid 999,
    // rate 50, dist 1. With aggregate-floor charging, per-buyer prorata of one
    // floored aggregate value conserves to the unit. Per-line charging would lose
    // up to N-1 scale-units and break operator==transport.
    let mut s = surplus_deficit_scenario(1, 201, 333, 3, 67, 999, 1, Money(50));
    let money_before = s.accounts.total_money().unwrap();
    let good_before = s.inventory.total_good(GOOD_FOOD).unwrap();
    let op_before = s.accounts.account(TRANSPORT_OPERATOR).available;

    run_flow(&mut s, 0).unwrap();

    let q = Quantity(201);
    let expected_transport = transport_cost(1, q, Money(50)).unwrap(); // (50*201/1000)*1 = 10
    assert_eq!(expected_transport, Money(10));
    assert_eq!(s.accounts.total_money().unwrap(), money_before);
    assert_eq!(s.inventory.total_good(GOOD_FOOD).unwrap(), good_before);
    assert_eq!(
        s.accounts.account(TRANSPORT_OPERATOR).available.0 - op_before.0,
        expected_transport.0,
        "operator delta == transport_total despite N buyers (aggregate floor, not per-line)"
    );
}
```

- [ ] Run, expect PASS: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core "macro_flow_conserves"`. Confirm both PASS.
- [ ] Commit: `git add -A && git commit -m "test(economy): macro_flow_world builder + conservation tests"`.

### Task 14: Determinism + tiebreak stability

**Files:** `backend/crates/sim-core/src/economy/tests/macro_flow.rs`.

- [ ] Add a determinism test that builds the same scenario twice and asserts byte-identical ledgers:

```rust
#[test]
fn macro_flow_is_deterministic() {
    let build = || {
        let mut s = surplus_deficit_scenario(2, 100, 400, 2, 100, 1800, 3, Money(50));
        // Several intervals so any iteration-order nondeterminism would surface.
        for tick in [0u64, 10, 20] {
            run_flow(&mut s, tick).unwrap();
        }
        s.ledger.clone()
    };
    let a = build();
    let b = build();
    assert_eq!(a, b, "ledger is a pure deterministic function of inputs");
}
```

- [ ] Add the tiebreak-stability test. Two equidistant deficit markets B and C compete for one surplus source A; the ascending-MarketId / largest-remainder split must be byte-identical across runs. COMPLETE Rust:

```rust
#[test]
fn macro_flow_tiebreak_is_stable() {
    // A surplus, B and C deficit, B and C EQUIDISTANT from A with identical bids.
    // The shared surplus is split by the deterministic candidate sort
    // (net_gain DESC, good ASC, src ASC, dst ASC) + largest-remainder prorata.
    let build = || {
        let m_a = MarketId(1);
        let m_b = MarketId(2);
        let m_c = MarketId(3);
        let mut accounts = AccountBook::default();
        let mut inventory = InventoryBook::default();
        let mut demand = DemandPools::default();
        let mut supply = SupplyPools::default();

        inventory.deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000)).unwrap();
        supply.0.insert(EconomicActorId(100), sp(100, m_a, 100, 400));
        accounts.deposit(EconomicActorId(200), Money(1_000_000_000)).unwrap();
        demand.0.insert(EconomicActorId(200), dp(200, m_b, 100, 1800));
        accounts.deposit(EconomicActorId(201), Money(1_000_000_000)).unwrap();
        demand.0.insert(EconomicActorId(201), dp(201, m_c, 100, 1800));

        let mut distances = MarketDistances(BTreeMap::new());
        for (x, y) in [(m_a, m_b), (m_b, m_a), (m_a, m_c), (m_c, m_a)] {
            distances.0.insert((x, y), 3);
        }
        let mut config = EconomyConfig::default();
        config.transport_cost_per_tile_unit = Money(50);

        let mut s = DormantScenario {
            accounts,
            inventory,
            ledger: TradeLedger::default(),
            demand,
            supply,
            market_goods: MarketGoods::default(),
            dirty: crate::economy::DirtyMarketGoods::default(),
            dormant: [m_a, m_b, m_c].into_iter().collect(),
            distances,
            config,
        };
        run_flow(&mut s, 0).unwrap();
        s.ledger.clone()
    };
    let a = build();
    let b = build();
    assert_eq!(a, b, "equidistant deficit split is byte-identical across runs");

    // The split must favor ascending MarketId on the tie: B (id 2) receives no
    // less than C (id 3), and total exported == surplus capacity (100).
    let mut to_b = 0i64;
    let mut to_c = 0i64;
    for ev in &a.0 {
        if let EconomyEvent::MacroFlow { to_market, qty, .. } = ev {
            if *to_market == MarketId(2) {
                to_b += qty.0;
            } else if *to_market == MarketId(3) {
                to_c += qty.0;
            }
        }
    }
    assert_eq!(to_b + to_c, 100, "all surplus exported");
    assert!(to_b >= to_c, "ascending-MarketId tie favors B");
}
```

- [ ] Run, expect PASS: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core "macro_flow_is_deterministic|macro_flow_tiebreak_is_stable"`. Confirm both PASS.
- [ ] Commit: `git add -A && git commit -m "test(economy): macro flow determinism + tiebreak stability"`.

### Task 15: Convergence (both-sided) + one-sided price-pinned companion

**Files:** `backend/crates/sim-core/src/economy/tests/macro_flow.rs`.

- [ ] Add a both-sided convergence helper + test. Each endpoint has a small local demand AND supply so `p_m = settlement_price_with_policy(prior, …)` drifts as the written-back `last_settlement_price` feeds the next interval; the gap must be monotone non-increasing and converge to `≤ unit_transport_cost + Money(1)`. COMPLETE Rust:

```rust
fn last_price(mg: &MarketGoods, market: MarketId) -> Money {
    mg.0
        .get(&MarketGoodKey { market, good: GOOD_FOOD })
        .map(|s| s.last_settlement_price)
        .unwrap_or(Money::ZERO)
}

/// Both-sided pair: A is net-surplus & cheap (big supplier@low ask + small
/// consumer), B is net-deficit & dear (big consumer@high bid + small supplier).
/// Both markets thus have a binding `settlement_price_with_policy` band → prices
/// drift across intervals.
fn both_sided_pair(rate: Money, dist: i64) -> DormantScenario {
    let m_a = MarketId(1);
    let m_b = MarketId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();

    // A: big seller (300 @ ask 600) + small local buyer (20 @ bid 700).
    inventory.deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000)).unwrap();
    supply.0.insert(EconomicActorId(100), sp(100, m_a, 300, 600));
    accounts.deposit(EconomicActorId(110), Money(1_000_000_000)).unwrap();
    demand.0.insert(EconomicActorId(110), dp(110, m_a, 20, 700));

    // B: big buyer (300 @ bid 1800) + small local seller (20 @ ask 1700).
    accounts.deposit(EconomicActorId(200), Money(1_000_000_000)).unwrap();
    demand.0.insert(EconomicActorId(200), dp(200, m_b, 300, 1800));
    inventory.deposit(EconomicActorId(210), GOOD_FOOD, Quantity(1_000_000)).unwrap();
    supply.0.insert(EconomicActorId(210), sp(210, m_b, 20, 1700));

    let mut distances = MarketDistances(BTreeMap::new());
    distances.0.insert((m_a, m_b), dist);
    distances.0.insert((m_b, m_a), dist);
    let mut config = EconomyConfig::default();
    config.transport_cost_per_tile_unit = rate;

    DormantScenario {
        accounts,
        inventory,
        ledger: TradeLedger::default(),
        demand,
        supply,
        market_goods: MarketGoods::default(),
        dirty: crate::economy::DirtyMarketGoods::default(),
        dormant: [m_a, m_b].into_iter().collect(),
        distances,
        config,
    }
}

#[test]
fn prices_converge_to_within_transport_cost() {
    let rate = Money(50);
    let dist = 2;
    let mut s = both_sided_pair(rate, dist);
    // unit transport per unit good = (rate * 1 / SCALE) * dist — but the per-unit
    // floor is 0; convergence target uses the per-q aggregate divided by q. Assert
    // on the realized aggregate band: gap must shrink toward unit_transport+1.
    // unit_transport for the test's q (≈280) = transport_cost(dist, q)/q rounded up.
    let q_ref = Quantity(280);
    let agg_transport = transport_cost(dist, q_ref, rate).unwrap(); // (50*280/1000)*2 = 14*2 = 28
    let unit_transport = Money((agg_transport.0 + q_ref.0 - 1) / q_ref.0); // ceil = 1

    let mut prev_gap = i64::MAX;
    for k in 0..40u64 {
        run_flow(&mut s, k * 10).unwrap();
        let pa = last_price(&s.market_goods, MarketId(1));
        let pb = last_price(&s.market_goods, MarketId(2));
        if pa.0 == 0 || pb.0 == 0 {
            continue; // not both priced yet
        }
        let gap = (pb.0 - pa.0).abs();
        assert!(gap <= prev_gap + 0, "gap monotone non-increasing: {gap} <= {prev_gap}");
        prev_gap = gap;
    }
    assert!(
        prev_gap <= unit_transport.0 + 1,
        "converged within transport: gap {prev_gap} <= unit_transport {} + 1",
        unit_transport.0
    );
}
```

- [ ] Add the one-sided companion test (documents the §3 STEP A reservation-price-pinned caveat — pure supply-only/demand-only pair flows goods every interval yet its price gap stays constant):

```rust
#[test]
fn one_sided_pair_flows_goods_but_price_is_pinned() {
    // Pure source A (supply-only, ask 500) ↔ pure sink B (demand-only, bid 2000).
    // Reservation-pinned: p_a = ask_floor = 500, p_b = bid_ceiling = 2000 every
    // interval. Goods move each interval; the 1500 gap NEVER narrows.
    let mut s = surplus_deficit_scenario(1, 200, 500, 1, 200, 2000, 2, Money(50));
    let buyer_before = s.inventory.balance(EconomicActorId(200), GOOD_FOOD).available;

    run_flow(&mut s, 0).unwrap();
    let pa0 = last_price(&s.market_goods, MarketId(1));
    let pb0 = last_price(&s.market_goods, MarketId(2));
    assert_eq!(pa0, Money(500), "supply-only price pinned to ask floor");
    assert_eq!(pb0, Money(2000), "demand-only price pinned to bid ceiling");
    let buyer_after_1 = s.inventory.balance(EconomicActorId(200), GOOD_FOOD).available;
    assert!(buyer_after_1.0 > buyer_before.0, "goods flowed on interval 0");

    for k in 1..5u64 {
        run_flow(&mut s, k * 10).unwrap();
        assert_eq!(last_price(&s.market_goods, MarketId(1)), Money(500), "still pinned");
        assert_eq!(last_price(&s.market_goods, MarketId(2)), Money(2000), "still pinned");
    }
}
```

- [ ] Run, expect PASS: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core "prices_converge_to_within_transport_cost|one_sided_pair_flows_goods_but_price_is_pinned"`. Confirm both PASS. If `prices_converge` reveals the convergence target math is off (e.g. gap floors above target), re-derive `unit_transport` by hand from the realized `transport_cost(dist, q)` of the actual traded `q` and adjust the asserted bound — do NOT relax monotonicity.
- [ ] Commit: `git add -A && git commit -m "test(economy): both-sided convergence + one-sided price-pinned caveat"`.

### Task 16: asleep-anchored DOES flow (inversion) + cheap→dear direction + reverse + MacroFlow from/to

**Files:** `backend/crates/sim-core/src/economy/tests/lod.rs` (invert the existing `asleep_anchored_market_stays_frozen_end_to_end` at ~:330), `backend/crates/sim-core/src/economy/tests/macro_flow.rs` (direction tests).

- [ ] In `tests/lod.rs`, extend `lod_world` (or add a sibling `lod_two_market_world`) so it can host TWO asleep-anchored markets A and B with a supply-only A (cheap) and demand-only B (dear), a finite `MarketDistances`, and a transport rate large enough that the gap exceeds transport. Add the imports `MarketDistances`, `MarketId`, `DemandPool`, `DemandPools`, `Money`, `EconomicActorId` as needed (most already present). Use the COMPLETE helper:

```rust
use std::collections::BTreeMap as StdBTreeMap;
use crate::economy::{DemandPool, DemandPools, MarketDistances};

// Two markets A,B both anchored to AsleepChunks; A supply-only (cheap), B
// demand-only (dear); gap > transport. No subscribers, no Active chunk.
fn asleep_two_market_world() -> (World, bevy_ecs::schedule::Schedule, MarketId, MarketId) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let m_a = MarketId(9_101);
    let m_b = MarketId(9_102);
    let coord_a = ChunkCoord { x: 5, y: 5 };
    let coord_b = ChunkCoord { x: 9, y: 5 };
    let supplier = EconomicActorId(50);
    let consumer = EconomicActorId(60);
    {
        let mut inv = world.resource_mut::<InventoryBook>();
        inv.deposit(supplier, GOOD_FOOD, Quantity(1_000_000)).unwrap();
    }
    {
        let mut acc = world.resource_mut::<AccountBook>();
        acc.deposit(consumer, Money(1_000_000_000)).unwrap();
    }
    {
        let mut supply = world.resource_mut::<SupplyPools>();
        supply.0.insert(
            supplier,
            SupplyPool {
                actor: supplier,
                market: m_a,
                good: GOOD_FOOD,
                offered_qty_per_tick: Quantity(200),
                min_price: Money(500),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }
    {
        let mut demand = world.resource_mut::<DemandPools>();
        demand.0.insert(
            consumer,
            DemandPool {
                actor: consumer,
                market: m_b,
                good: GOOD_FOOD,
                desired_qty_per_tick: Quantity(200),
                max_price: Money(2_000),
                urgency_bps: 0,
                elasticity_bps: 0,
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        anchors.0.insert(m_a, coord_a);
        anchors.0.insert(m_b, coord_b);
    }
    {
        let mut dist = MarketDistances(StdBTreeMap::new());
        dist.0.insert((m_a, m_b), 4);
        dist.0.insert((m_b, m_a), 4);
        world.insert_resource(dist);
    }
    {
        let mut cfg = world.resource_mut::<EconomyConfig>();
        cfg.transport_cost_per_tile_unit = Money(50);
    }
    world.spawn((ChunkCoordComp(coord_a), AsleepChunk));
    world.spawn((ChunkCoordComp(coord_b), AsleepChunk));
    world.insert_resource(Tick(0));
    (world, schedule, m_a, m_b)
}
```

- [ ] Replace `asleep_anchored_market_stays_frozen_end_to_end` (~lod.rs:330) with its inversion `asleep_anchored_market_DOES_flow`. (Add `use crate::economy::{MarketDistances, MarketGoodKey, GOOD_FOOD};` / `EconomyConfig` imports if not present.) COMPLETE Rust:

```rust
#[test]
fn asleep_anchored_market_DOES_flow() {
    // INVERSION of the old asleep_anchored_market_stays_frozen_end_to_end: under
    // the macro flow an asleep-anchored market is NO LONGER frozen — goods move
    // A→B and discovered prices are written back, with zero subscribers and no
    // Active chunk. Proves the no-hollow-world guarantee.
    let (mut world, mut schedule, m_a, m_b) = asleep_two_market_world();
    let supplier = EconomicActorId(50);
    let consumer = EconomicActorId(60);
    let a_goods_before = world.resource::<InventoryBook>().balance(supplier, GOOD_FOOD).available;
    let b_goods_before = world.resource::<InventoryBook>().balance(consumer, GOOD_FOOD).available;

    // Run several flow intervals (interval default 10): ticks 0,10,20.
    for _ in 0..21 {
        schedule.run(&mut world);
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }

    let key_b = MarketGoodKey { market: m_b, good: GOOD_FOOD };
    let key_a = MarketGoodKey { market: m_a, good: GOOD_FOOD };
    let mg = world.resource::<MarketGoods>();
    assert!(
        mg.0.get(&key_b).map(|s| s.last_settlement_price.0 > 0).unwrap_or(false),
        "dear market discovered a price"
    );
    assert!(
        mg.0.get(&key_b).map(|s| s.traded_qty_last_tick.0 > 0).unwrap_or(false)
            || mg.0.get(&key_a).map(|s| s.traded_qty_last_tick.0 > 0).unwrap_or(false),
        "traded qty written on a flow interval"
    );
    let inv = world.resource::<InventoryBook>();
    assert!(
        inv.balance(supplier, GOOD_FOOD).available.0 < a_goods_before.0,
        "seller@A inventory decreased"
    );
    assert!(
        inv.balance(consumer, GOOD_FOOD).available.0 > b_goods_before.0,
        "buyer@B inventory increased"
    );
}
```

- [ ] In `tests/macro_flow.rs`, add the direction test + symmetric reverse + MacroFlow from/to assertions:

```rust
#[test]
fn goods_flow_from_cheap_surplus_to_dear_deficit() {
    let mut s = surplus_deficit_scenario(1, 200, 500, 1, 200, 2000, 4, Money(50));
    let seller = EconomicActorId(100);
    let buyer = EconomicActorId(200);
    let seller_before = s.inventory.balance(seller, GOOD_FOOD).available;
    let buyer_before = s.inventory.balance(buyer, GOOD_FOOD).available;

    run_flow(&mut s, 0).unwrap();

    let seller_after = s.inventory.balance(seller, GOOD_FOOD).available;
    let buyer_after = s.inventory.balance(buyer, GOOD_FOOD).available;
    let moved = buyer_after.0 - buyer_before.0;
    assert!(moved > 0, "goods moved into deficit market");
    assert_eq!(seller_before.0 - seller_after.0, moved, "same q left surplus");

    let cross: Vec<_> = s
        .ledger
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::MacroFlow { from_market, to_market, qty, .. }
                if from_market != to_market =>
            {
                Some((*from_market, *to_market, qty.0))
            }
            _ => None,
        })
        .collect();
    assert_eq!(cross.len(), 1, "exactly one cross-market flow");
    assert_eq!(cross[0].0, MarketId(1), "from == surplus A");
    assert_eq!(cross[0].1, MarketId(2), "to == deficit B");
    assert_eq!(cross[0].2, moved);
}

#[test]
fn direction_reverses_when_dear_and_cheap_swap() {
    // Swap: now market 1 is the dear/demand side and market 2 the cheap/supply
    // side — flow must reverse to from==2, to==1.
    let m_a = MarketId(1);
    let m_b = MarketId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    inventory.deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000)).unwrap();
    supply.0.insert(EconomicActorId(100), sp(100, m_b, 200, 500)); // cheap supply at B
    accounts.deposit(EconomicActorId(200), Money(1_000_000_000)).unwrap();
    demand.0.insert(EconomicActorId(200), dp(200, m_a, 200, 2000)); // dear demand at A

    let mut distances = MarketDistances(BTreeMap::new());
    distances.0.insert((m_a, m_b), 4);
    distances.0.insert((m_b, m_a), 4);
    let mut config = EconomyConfig::default();
    config.transport_cost_per_tile_unit = Money(50);

    let mut s = DormantScenario {
        accounts,
        inventory,
        ledger: TradeLedger::default(),
        demand,
        supply,
        market_goods: MarketGoods::default(),
        dirty: crate::economy::DirtyMarketGoods::default(),
        dormant: [m_a, m_b].into_iter().collect(),
        distances,
        config,
    };
    run_flow(&mut s, 0).unwrap();

    let cross: Vec<_> = s
        .ledger
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::MacroFlow { from_market, to_market, .. } if from_market != to_market => {
                Some((*from_market, *to_market))
            }
            _ => None,
        })
        .collect();
    assert_eq!(cross, vec![(MarketId(2), MarketId(1))], "direction reversed");
}
```

- [ ] Run, expect PASS: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core "asleep_anchored_market_DOES_flow|goods_flow_from_cheap_surplus_to_dear_deficit|direction_reverses_when_dear_and_cheap_swap"`. Confirm all PASS.
- [ ] Commit: `git add -A && git commit -m "test(economy): invert asleep test to DOES_flow + cheap->dear direction + reverse"`.

### Task 17: Edge cases (each its own #[test])

**Files:** `backend/crates/sim-core/src/economy/tests/macro_flow.rs`.

- [ ] Add the no-demand / no-supply / single-market / zero-distance cases:

```rust
#[test]
fn no_demand_no_flow() {
    // Supply-only across two dormant markets, no demand anywhere → no flow.
    let m_a = MarketId(1);
    let m_b = MarketId(2);
    let mut s = surplus_deficit_scenario(1, 200, 500, 0, 0, 0, 4, Money(50));
    // surplus_deficit_scenario with n_buyers=0 leaves demand empty.
    let before = (s.accounts.clone(), s.inventory.clone());
    run_flow(&mut s, 0).unwrap();
    assert_eq!(s.accounts, before.0, "no demand → books unchanged");
    assert_eq!(s.inventory, before.1);
    assert!(s.ledger.0.is_empty(), "no MacroFlow event");
    let _ = (m_a, m_b);
}

#[test]
fn no_supply_no_flow() {
    let mut s = surplus_deficit_scenario(0, 0, 0, 1, 200, 2000, 4, Money(50));
    let before = (s.accounts.clone(), s.inventory.clone());
    run_flow(&mut s, 0).unwrap();
    assert_eq!(s.accounts, before.0);
    assert_eq!(s.inventory, before.1);
    assert!(s.ledger.0.is_empty());
}

#[test]
fn single_market_no_partner() {
    // One dormant market with both demand & supply: only a self-edge clears
    // locally; no cross-edge (no partner). Conserves; no cross MacroFlow.
    let m = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    inventory.deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000)).unwrap();
    supply.0.insert(EconomicActorId(100), sp(100, m, 100, 500));
    accounts.deposit(EconomicActorId(200), Money(1_000_000_000)).unwrap();
    demand.0.insert(EconomicActorId(200), dp(200, m, 100, 2000));

    let mut s = DormantScenario {
        accounts,
        inventory,
        ledger: TradeLedger::default(),
        demand,
        supply,
        market_goods: MarketGoods::default(),
        dirty: crate::economy::DirtyMarketGoods::default(),
        dormant: [m].into_iter().collect(),
        distances: MarketDistances(BTreeMap::new()),
        config: {
            let mut c = EconomyConfig::default();
            c.transport_cost_per_tile_unit = Money(50);
            c
        },
    };
    let money_before = s.accounts.total_money().unwrap();
    let good_before = s.inventory.total_good(GOOD_FOOD).unwrap();
    run_flow(&mut s, 0).unwrap();
    assert_eq!(s.accounts.total_money().unwrap(), money_before);
    assert_eq!(s.inventory.total_good(GOOD_FOOD).unwrap(), good_before);
    assert!(
        s.ledger.0.iter().all(|e| match e {
            EconomyEvent::MacroFlow { from_market, to_market, .. } => from_market == to_market,
            _ => true,
        }),
        "only self-edges, no cross flow"
    );
}

#[test]
fn zero_distance_markets() {
    // dist 0 → transport 0 → full equalization of residual still conserves.
    let mut s = surplus_deficit_scenario(1, 200, 500, 1, 200, 2000, 0, Money(50));
    let money_before = s.accounts.total_money().unwrap();
    let good_before = s.inventory.total_good(GOOD_FOOD).unwrap();
    let op_before = s.accounts.account(TRANSPORT_OPERATOR).available;
    run_flow(&mut s, 0).unwrap();
    assert_eq!(s.accounts.total_money().unwrap(), money_before);
    assert_eq!(s.inventory.total_good(GOOD_FOOD).unwrap(), good_before);
    assert_eq!(
        s.accounts.account(TRANSPORT_OPERATOR).available, op_before,
        "zero distance → zero transport"
    );
}
```

- [ ] Add the overflow-pruned / tiny-qty-floors / zero-price-band / dormant-producer-bound cases:

```rust
#[test]
fn overflow_edge_is_pruned_not_faulted() {
    // A pathological distance forces the net_gain transport term to overflow i128
    // → the edge is PRUNED in STEP D (no candidate, no event, no panic). Books
    // unchanged. dist = i64::MAX with a large qty overflows transport_cost.
    let mut s = surplus_deficit_scenario(1, 1_000_000, 500, 1, 1_000_000, 2000, i64::MAX, Money(i64::MAX));
    let before = (s.accounts.clone(), s.inventory.clone());
    run_flow(&mut s, 0).expect("gate overflow is pruned, never an Err");
    assert_eq!(s.accounts, before.0, "pruned edge leaves books unchanged");
    assert_eq!(s.inventory, before.1);
    assert!(
        s.ledger.0.iter().all(|e| !matches!(e, EconomyEvent::MacroFlow { .. })),
        "no MacroFlow event for a pruned edge"
    );
    assert!(
        s.ledger.0.iter().all(|e| !matches!(e, EconomyEvent::MarketClearFailed { .. })),
        "pruned (gate-time) edge is NOT a settle-time fault"
    );
}

#[test]
fn tiny_qty_floors_to_zero() {
    // rate*q < SCALE so transport floors to 0; flow still conserves.
    // rate 5 (default), q 100 → 5*100=500 < 1000 → transport floors to 0.
    let mut s = surplus_deficit_scenario(1, 100, 500, 1, 100, 2000, 3, Money(5));
    let money_before = s.accounts.total_money().unwrap();
    let good_before = s.inventory.total_good(GOOD_FOOD).unwrap();
    let op_before = s.accounts.account(TRANSPORT_OPERATOR).available;
    run_flow(&mut s, 0).unwrap();
    assert_eq!(s.accounts.total_money().unwrap(), money_before, "conserves with floored transport");
    assert_eq!(s.inventory.total_good(GOOD_FOOD).unwrap(), good_before);
    assert_eq!(
        s.accounts.account(TRANSPORT_OPERATOR).available, op_before,
        "transport floored to 0"
    );
}

#[test]
fn zero_price_band_market_skipped() {
    // A demand-only dear market whose only buyer has max_price 0 → p_m guard
    // (p_m.0 <= 0) skips it; no error, no flow. Pair it with a healthy surplus so
    // we prove the guard skips ONLY the degenerate market.
    let m_a = MarketId(1);
    let m_b = MarketId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    inventory.deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000)).unwrap();
    supply.0.insert(EconomicActorId(100), sp(100, m_a, 100, 500));
    accounts.deposit(EconomicActorId(200), Money(1_000_000_000)).unwrap();
    demand.0.insert(EconomicActorId(200), dp(200, m_b, 100, 0)); // bid ceiling 0 → p_m<=0

    let mut distances = MarketDistances(BTreeMap::new());
    distances.0.insert((m_a, m_b), 3);
    distances.0.insert((m_b, m_a), 3);
    let mut s = DormantScenario {
        accounts,
        inventory,
        ledger: TradeLedger::default(),
        demand,
        supply,
        market_goods: MarketGoods::default(),
        dirty: crate::economy::DirtyMarketGoods::default(),
        dormant: [m_a, m_b].into_iter().collect(),
        distances,
        config: {
            let mut c = EconomyConfig::default();
            c.transport_cost_per_tile_unit = Money(50);
            c
        },
    };
    let before = (s.accounts.clone(), s.inventory.clone());
    run_flow(&mut s, 0).expect("zero-band market is skipped, not ZeroPrice-aborted");
    assert_eq!(s.accounts, before.0, "no flow into zero-band market");
    assert_eq!(s.inventory, before.1);
}

#[test]
fn dormant_producer_does_not_burst_dump() {
    // Seller holds 1_000_000 on-hand but offered_qty_per_tick is 50; the flow's
    // effective supply is min(offered, on-hand) = 50, NOT total inventory. So per
    // interval at most 50 leaves the surplus market regardless of accumulated stock.
    let mut s = surplus_deficit_scenario(1, 50, 500, 1, 100, 2000, 2, Money(50));
    let seller = EconomicActorId(100);
    let seller_before = s.inventory.balance(seller, GOOD_FOOD).available;
    run_flow(&mut s, 0).unwrap();
    let moved = seller_before.0 - s.inventory.balance(seller, GOOD_FOOD).available.0;
    assert!(moved <= 50, "per-interval export bounded by offered_qty_per_tick (50), got {moved}");
    assert!(moved > 0, "but it does export up to the cap");
}
```

- [ ] If `surplus_deficit_scenario` with `n_buyers=0`/`n_sellers=0` does not already produce an empty side, confirm the loop `for c in 0..0` is a no-op (it is). Run, expect PASS: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core "no_demand_no_flow|no_supply_no_flow|single_market_no_partner|zero_distance_markets|overflow_edge_is_pruned_not_faulted|tiny_qty_floors_to_zero|zero_price_band_market_skipped|dormant_producer_does_not_burst_dump"`. Confirm all 8 PASS.
- [ ] Commit: `git add -A && git commit -m "test(economy): macro flow edge cases"`.

### Task 18: Poisoning isolation + interval-only firing + active→dormant handoff conserves

**Files:** `backend/crates/sim-core/src/economy/tests/macro_flow.rs`, `backend/crates/sim-core/src/economy/tests/lod.rs` (handoff uses the wired schedule + demotion).

- [ ] Add the settle-time poisoning isolation test (one deficit buyer targeted by two surplus sources in one interval; the second edge's `lock_cash` exceeds the buyer's bucket-time affordability → `MarketClearFailed`; the healthy pair still flows + conserves; the faulted edge moves nothing):

```rust
#[test]
fn poisoning_market_does_not_abort_others() {
    // Three markets. A (cheap surplus) and C (cheap surplus) both target B (dear
    // deficit). B's single buyer can afford the FIRST edge but NOT both: budgets
    // are computed once against live books per STEP A, so when the second edge's
    // lock_cash exceeds bucket-time affordability it faults at SETTLE time →
    // MarketClearFailed, skipped. A healthy independent pair D→E still conserves.
    let (m_a, m_b, m_c) = (MarketId(1), MarketId(2), MarketId(3));
    let (m_d, m_e) = (MarketId(4), MarketId(5));
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();

    // Surpluses A and C: 200 each @ ask 500.
    inventory.deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000)).unwrap();
    supply.0.insert(EconomicActorId(100), sp(100, m_a, 200, 500));
    inventory.deposit(EconomicActorId(101), GOOD_FOOD, Quantity(1_000_000)).unwrap();
    supply.0.insert(EconomicActorId(101), sp(101, m_c, 200, 500));
    // B's buyer wants 400 @ bid 2000 but is funded for only ~one edge's charge.
    // Charge per 200-unit edge ≈ value(p_dst≈2000, 200)+transport. Fund tight so
    // edge 1 succeeds, edge 2 lock_cash underflows affordability → fault.
    accounts.deposit(EconomicActorId(200), Money(450_000)).unwrap();
    demand.0.insert(EconomicActorId(200), dp(200, m_b, 400, 2000));

    // Healthy independent pair D(cheap surplus)→E(dear deficit), fully funded.
    inventory.deposit(EconomicActorId(300), GOOD_FOOD, Quantity(1_000_000)).unwrap();
    supply.0.insert(EconomicActorId(300), sp(300, m_d, 100, 500));
    accounts.deposit(EconomicActorId(400), Money(1_000_000_000)).unwrap();
    demand.0.insert(EconomicActorId(400), dp(400, m_e, 100, 2000));

    let mut distances = MarketDistances(BTreeMap::new());
    for (x, y) in [
        (m_a, m_b), (m_b, m_a), (m_c, m_b), (m_b, m_c), (m_d, m_e), (m_e, m_d),
    ] {
        distances.0.insert((x, y), 2);
    }
    let mut s = DormantScenario {
        accounts,
        inventory,
        ledger: TradeLedger::default(),
        demand,
        supply,
        market_goods: MarketGoods::default(),
        dirty: crate::economy::DirtyMarketGoods::default(),
        dormant: [m_a, m_b, m_c, m_d, m_e].into_iter().collect(),
        distances,
        config: {
            let mut c = EconomyConfig::default();
            c.transport_cost_per_tile_unit = Money(50);
            c
        },
    };
    let money_before = s.accounts.total_money().unwrap();
    let good_before = s.inventory.total_good(GOOD_FOOD).unwrap();
    let d_seller = EconomicActorId(300);
    let e_buyer = EconomicActorId(400);
    let d_before = s.inventory.balance(d_seller, GOOD_FOOD).available;
    let e_before = s.inventory.balance(e_buyer, GOOD_FOOD).available;

    run_flow(&mut s, 0).unwrap();

    // Global conservation holds even with a faulted edge (atomic per-edge).
    assert_eq!(s.accounts.total_money().unwrap(), money_before, "money conserved despite fault");
    assert_eq!(s.inventory.total_good(GOOD_FOOD).unwrap(), good_before, "goods conserved");
    // Healthy pair flowed.
    let moved = s.inventory.balance(e_buyer, GOOD_FOOD).available.0 - e_before.0;
    assert!(moved > 0, "healthy D→E pair flowed");
    assert_eq!(d_before.0 - s.inventory.balance(d_seller, GOOD_FOOD).available.0, moved);
    // The poisoning edge emitted a MarketClearFailed.
    assert!(
        s.ledger.0.iter().any(|e| matches!(
            e,
            EconomyEvent::MarketClearFailed { market, .. } if *market == m_b || *market == m_c
        )),
        "faulted edge emitted MarketClearFailed"
    );
}
```

- [ ] Add the interval-only firing test:

```rust
#[test]
fn macro_flow_only_fires_on_interval() {
    // interval default 10. Tick 3 (not a multiple) → no flow. Tick 0 / 10 → flow.
    let mut s = surplus_deficit_scenario(1, 200, 500, 1, 200, 2000, 4, Money(50));
    let buyer = EconomicActorId(200);
    let before = s.inventory.balance(buyer, GOOD_FOOD).available;

    run_flow(&mut s, 3).unwrap();
    assert_eq!(
        s.inventory.balance(buyer, GOOD_FOOD).available, before,
        "tick 3 is not a flow interval"
    );
    assert!(s.ledger.0.is_empty(), "no event off-interval");

    run_flow(&mut s, 10).unwrap();
    assert!(
        s.inventory.balance(buyer, GOOD_FOOD).available.0 > before.0,
        "tick 10 IS a flow interval"
    );
}
```

- [ ] In `tests/lod.rs`, add the full `active_to_dormant_handoff_conserves` using the wired schedule. Place a real partially-locked bid while the market is Active, demote its chunk to Asleep, run flow intervals across `default_order_ttl_ticks` (=10), and assert (a) every tick conserves, (b) the locked portion is NOT flowed (flow reads only `available`), (c) a dirty key is skipped even on a flow tick, (d) released resources become flow-eligible AFTER expiry. COMPLETE Rust (reuse `lod_world`-style assembly; this needs a two-market world with one Active→demoted market plus a partner surplus):

```rust
#[test]
fn active_to_dormant_handoff_conserves() {
    // m_b starts ACTIVE with a live consumer bid (locks cash); m_a is a dormant
    // surplus. We demote m_b's chunk to Asleep with the bid still locked, then run
    // flow intervals through the TTL window (default 10). The locked cash is
    // unavailable to the flow until the order expires; conservation holds every
    // tick; released cash becomes flow-eligible after expiry.
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let m_a = MarketId(9_201); // dormant surplus
    let m_b = MarketId(9_202); // active→demoted deficit
    let coord_a = ChunkCoord { x: 5, y: 5 };
    let coord_b = ChunkCoord { x: 9, y: 5 };
    let supplier = EconomicActorId(50);
    let consumer = EconomicActorId(60);
    {
        let mut inv = world.resource_mut::<InventoryBook>();
        inv.deposit(supplier, GOOD_FOOD, Quantity(1_000_000)).unwrap();
    }
    {
        let mut acc = world.resource_mut::<AccountBook>();
        acc.deposit(consumer, Money(1_000_000_000)).unwrap();
    }
    {
        let mut supply = world.resource_mut::<SupplyPools>();
        supply.0.insert(supplier, SupplyPool {
            actor: supplier, market: m_a, good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(200), min_price: Money(500),
            interval_ticks: 1, last_generated_tick: None,
        });
    }
    {
        let mut demand = world.resource_mut::<DemandPools>();
        demand.0.insert(consumer, DemandPool {
            actor: consumer, market: m_b, good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(50), max_price: Money(2_000),
            urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1, last_generated_tick: None,
        });
    }
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        anchors.0.insert(m_a, coord_a);
        anchors.0.insert(m_b, coord_b);
    }
    {
        let mut dist = MarketDistances(StdBTreeMap::new());
        dist.0.insert((m_a, m_b), 4);
        dist.0.insert((m_b, m_a), 4);
        world.insert_resource(dist);
    }
    {
        let mut cfg = world.resource_mut::<EconomyConfig>();
        cfg.transport_cost_per_tile_unit = Money(50);
    }
    // m_a asleep (dormant) from the start; m_b ACTIVE so its bid is placed+locked.
    let chunk_b = world.spawn((ChunkCoordComp(coord_b), ActiveChunk)).id();
    world.spawn((ChunkCoordComp(coord_a), AsleepChunk));
    world.insert_resource(Tick(0));

    let money_total = world.resource::<AccountBook>().total_money().unwrap();
    let good_total = world.resource::<InventoryBook>().total_good(GOOD_FOOD).unwrap();

    // Tick 0: m_b active → consumer bids, cash locks. Run one tick.
    schedule.run(&mut world);
    {
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }
    let locked_now = world.resource::<AccountBook>().account(consumer).locked;
    assert!(locked_now.0 > 0, "active bid locked the consumer's cash");

    // Demote m_b to Asleep with the bid still live (locked).
    world.entity_mut(chunk_b).remove::<ActiveChunk>().insert(AsleepChunk);

    // Run through the TTL window; conservation must hold every tick.
    for _ in 0..15 {
        schedule.run(&mut world);
        let m = world.resource::<AccountBook>().total_money().unwrap();
        let g = world.resource::<InventoryBook>().total_good(GOOD_FOOD).unwrap();
        assert_eq!(m, money_total, "money conserved across handoff");
        assert_eq!(g, good_total, "goods conserved across handoff");
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }

    // After the TTL window the order expired and released cash; the flow has by
    // now had eligible (available) cash and moved goods into m_b.
    let consumer_goods = world.resource::<InventoryBook>().balance(consumer, GOOD_FOOD).available;
    assert!(
        consumer_goods.0 > 0,
        "after expiry the released cash funded macro flow into the demoted market"
    );
}
```

- [ ] Run, expect PASS: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core "poisoning_market_does_not_abort_others|macro_flow_only_fires_on_interval|active_to_dormant_handoff_conserves"`. Confirm all PASS. If the poisoning test does not actually fault (the buyer can afford both edges), reduce `Money(450_000)` until exactly one edge settles and the second emits `MarketClearFailed` — verify by inspecting the ledger, not by guessing.
- [ ] Commit: `git add -A && git commit -m "test(economy): poisoning isolation + interval gate + active->dormant handoff"`.

### Task 19: Audit emission/round-trip + replay-across-restart + persist snapshot for market_distances

**Files:** `backend/crates/sim-core/src/economy/tests/macro_flow.rs` (audit + replay), `backend/crates/sim-core/src/economy/tests/persist.rs` (extend `seed`/round-trip for `market_distances`).

- [ ] Add the auditable-events test. Drain through `pending_ledger_audit`/`commit_ledger_audit` into an `InMemoryEconomyEventStore`; assert tick + type + both `from_market`/`to_market` round-trip and the cursor advanced. This needs a wired-world variant (the audit pipeline reads `Tick`/`LedgerAuditCursor`/`TradeLedger` from a `World`). COMPLETE Rust:

```rust
#[test]
fn macro_flow_emits_auditable_events() {
    use crate::economy::audit::{commit_ledger_audit, pending_ledger_audit, LedgerAuditCursor};
    use crate::persistence::{EconomyEventStore, InMemoryEconomyEventStore};

    // event_type extension first.
    assert_eq!(
        EconomyEvent::MacroFlow {
            from_market: MarketId(1),
            to_market: MarketId(2),
            good: GOOD_FOOD,
            qty: Quantity(10),
            price: Money(1_000),
            transport: Money(40),
        }
        .event_type(),
        "macro_flow"
    );

    // Drive the flow directly, push events into a World's ledger, then drain.
    let mut s = surplus_deficit_scenario(1, 200, 500, 1, 200, 2000, 4, Money(50));
    run_flow(&mut s, 0).unwrap();
    assert!(
        s.ledger.0.iter().any(|e| matches!(e, EconomyEvent::MacroFlow { .. })),
        "flow produced at least one MacroFlow event"
    );

    let mut world = World::new();
    world.insert_resource(Tick(0));
    world.insert_resource(s.ledger.clone());
    world.insert_resource(LedgerAuditCursor(0));

    let (tick, pending) = pending_ledger_audit(&world);
    assert_eq!(tick, 0);
    assert!(!pending.is_empty());

    let store = futures::executor::block_on(async {
        let mut store = InMemoryEconomyEventStore::default();
        store.append("w", tick, &pending).await.unwrap();
        store
    });
    commit_ledger_audit(&mut world, pending.len());
    assert!(
        pending_ledger_audit(&world).1.is_empty(),
        "cursor advanced past appended events"
    );

    let stored = store.events("w");
    let mf = stored
        .iter()
        .find_map(|(t, e)| match e {
            EconomyEvent::MacroFlow { from_market, to_market, .. } => {
                Some((*t, *from_market, *to_market))
            }
            _ => None,
        })
        .expect("a MacroFlow row survived the store round-trip");
    assert_eq!(mf, (0, MarketId(1), MarketId(2)), "tick + from/to round-trip via serde jsonb");
}
```

  Note: if `futures` is not a dev-dependency, use the already-available `tokio` runtime instead — wrap the append in `#[tokio::test]` and `.await` directly (mirror `in_memory_event_store_appends_in_order` in tests/audit.rs which is `#[tokio::test]`). Prefer the `#[tokio::test]` form to avoid adding a dependency:

```rust
// PREFERRED form (no new dep): make the whole test `#[tokio::test]` and `.await`
// the append, dropping the futures::executor::block_on wrapper.
```

- [ ] Add the replay-across-restart test (run N intervals, `extract_from_world`→serialize→`apply_into_world` to a fresh world, run M more on both, assert identical `MarketGoods` + `AccountBook` + ledger tail). Use a wired world so the schedule advances the flow:

```rust
#[test]
fn macro_flow_replays_across_restart() {
    use crate::economy::{apply_into_world, extract_from_world};

    // Build a wired world with two asleep-anchored markets that flow; reuse the
    // lod-style assembly inline (macro_flow_world has no anchors). We anchor here.
    fn wired_flow_world() -> (World, bevy_ecs::schedule::Schedule) {
        let mut world = macro_flow_world();
        let mut schedule = bevy_ecs::schedule::Schedule::default();
        // macro_flow_world used its own throwaway schedule; rebuild one that the
        // EconomyPlugin populated. Instead, install fresh so schedule is wired:
        // (re-install is idempotent on a fresh World here — simpler to build anew)
        let mut w2 = World::new();
        CorePlugin::default().install(&mut w2, &mut schedule);
        crate::mobility::MobilityPlugin.install(&mut w2, &mut schedule);
        EconomyPlugin.install(&mut w2, &mut schedule);
        let m_a = MarketId(9_301);
        let m_b = MarketId(9_302);
        w2.resource_mut::<InventoryBook>()
            .deposit(EconomicActorId(50), GOOD_FOOD, Quantity(1_000_000)).unwrap();
        w2.resource_mut::<AccountBook>()
            .deposit(EconomicActorId(60), Money(1_000_000_000)).unwrap();
        w2.resource_mut::<SupplyPools>().0.insert(EconomicActorId(50), SupplyPool {
            actor: EconomicActorId(50), market: m_a, good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(200), min_price: Money(500),
            interval_ticks: 1, last_generated_tick: None,
        });
        w2.resource_mut::<DemandPools>().0.insert(EconomicActorId(60), DemandPool {
            actor: EconomicActorId(60), market: m_b, good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(200), max_price: Money(2_000),
            urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1, last_generated_tick: None,
        });
        w2.resource_mut::<MarketChunks>().0.insert(m_a, ChunkCoord { x: 5, y: 5 });
        w2.resource_mut::<MarketChunks>().0.insert(m_b, ChunkCoord { x: 9, y: 5 });
        let mut dist = MarketDistances(BTreeMap::new());
        dist.0.insert((m_a, m_b), 4);
        dist.0.insert((m_b, m_a), 4);
        w2.insert_resource(dist);
        w2.resource_mut::<EconomyConfig>().transport_cost_per_tile_unit = Money(50);
        w2.spawn((ChunkCoordComp(ChunkCoord { x: 5, y: 5 }), AsleepChunk));
        w2.spawn((ChunkCoordComp(ChunkCoord { x: 9, y: 5 }), AsleepChunk));
        w2.insert_resource(Tick(0));
        let _ = world; // macro_flow_world world dropped; we use w2.
        (w2, schedule)
    }

    let (mut world, mut schedule) = wired_flow_world();
    // Run N=25 ticks (covers ticks 0,10,20 flow intervals).
    for _ in 0..25 {
        schedule.run(&mut world);
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }
    let saved_tick = world.resource::<Tick>().0;

    // Restart: extract → serialize → apply into a freshly-installed world.
    let snap = extract_from_world(&world);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: crate::economy::EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();

    let mut restart = World::new();
    let mut restart_sched = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut restart, &mut restart_sched);
    crate::mobility::MobilityPlugin.install(&mut restart, &mut restart_sched);
    EconomyPlugin.install(&mut restart, &mut restart_sched);
    apply_into_world(&mut restart, &decoded);
    // Distances + chunks restore from the snapshot (market_distances persisted);
    // re-spawn the chunk markers (LOD entities are not in the economy snapshot).
    restart.spawn((ChunkCoordComp(ChunkCoord { x: 5, y: 5 }), AsleepChunk));
    restart.spawn((ChunkCoordComp(ChunkCoord { x: 9, y: 5 }), AsleepChunk));
    restart.insert_resource(Tick(saved_tick));

    // Run M=20 more on BOTH continuations.
    for _ in 0..20 {
        schedule.run(&mut world);
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
        restart_sched.run(&mut restart);
        let mut t2 = restart.resource_mut::<Tick>();
        t2.0 += 1;
    }

    assert_eq!(
        world.resource::<MarketGoods>().0,
        restart.resource::<MarketGoods>().0,
        "MarketGoods identical across restart"
    );
    assert_eq!(
        world.resource::<AccountBook>().accounts,
        restart.resource::<AccountBook>().accounts,
        "AccountBook identical across restart"
    );
    let tail = |w: &World| {
        let l = &w.resource::<TradeLedger>().0;
        l[l.len().saturating_sub(16)..].to_vec()
    };
    assert_eq!(tail(&world), tail(&restart), "ledger tail identical across restart");
}
```

- [ ] In `tests/persist.rs`: extend the `seed(world)` helper to insert a `MarketDistances` entry, and assert the round-trip preserves it. Add to the imports: `MarketDistances`. Append to `seed()`:

```rust
    {
        let mut dist = world.resource_mut::<MarketDistances>();
        dist.0.insert((m, crate::economy::MarketId(2)), 7);
        dist.0.insert((crate::economy::MarketId(2), m), 7);
    }
```

  and add a dedicated assertion test:

```rust
#[test]
fn market_distances_round_trips() {
    let mut world = install_economy();
    seed(&mut world);
    let snap = extract_from_world(&world);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    assert_eq!(
        world.resource::<MarketDistances>().0,
        fresh.resource::<MarketDistances>().0,
        "market_distances survive extract->serialize->apply"
    );
    // And the whole snapshot remains an identity round-trip (covers the new field).
    assert_eq!(snap, extract_from_world(&fresh));
}
```

  The existing `economy_snapshot_round_trips` and `economy_snapshot_is_byte_stable` now also exercise `market_distances` via the extended `seed`.

- [ ] Run, expect PASS: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core "macro_flow_emits_auditable_events|macro_flow_replays_across_restart|market_distances_round_trips|economy_snapshot"`. Confirm all PASS.
- [ ] Commit: `git add -A && git commit -m "test(economy): macro flow audit emission + replay-across-restart + market_distances persist"`.

### Task 20: Minimal live-seed second good + benches + superlinear gate + schedule-level bench

**Files:** `backend/crates/sim-core/src/economy/seed.rs` (additive second good), `backend/crates/sim-core/src/economy/tests/seed.rs` (assert Markets/Traders counts unchanged), `backend/crates/sim-core/benches/economy_tick.rs` (NEW), `backend/crates/sim-core/Cargo.toml` (register `[[bench]]`).

- [ ] In `tests/seed.rs`, first add a failing assertion that the seed exposes a second-good cross-market demand/supply without adding markets or traders. Read the existing `tests/seed.rs` to match its world-build idiom; add:

```rust
#[test]
fn seed_adds_second_good_without_new_markets_or_traders() {
    // After seeding, the live economy still has exactly 2 markets and 1 trader,
    // but now a GOOD_FOOD supplier@m_a + consumer@m_b exists so the macro flow
    // produces a non-vacuous cross-market FOOD flow on the live stream.
    use crate::economy::{DemandPools, SupplyPools, GOOD_FOOD, Markets, Traders};
    let mut world = /* the existing seed-test world builder that runs seed_demo_economy */;
    crate::economy::seed::seed_demo_economy(&mut world);
    assert_eq!(world.resource::<Markets>().0.len(), 2, "still exactly 2 markets");
    assert_eq!(world.resource::<Traders>().0.len(), 1, "still exactly 1 trader");
    let has_food_supply = world
        .resource::<SupplyPools>()
        .0
        .values()
        .any(|p| p.good == GOOD_FOOD);
    let has_food_demand = world
        .resource::<DemandPools>()
        .0
        .values()
        .any(|p| p.good == GOOD_FOOD);
    assert!(has_food_supply && has_food_demand, "FOOD supplier@A + consumer@B added");
}
```

  (Match the exact world-build the existing seed tests use — read `tests/seed.rs` and reuse its helper verbatim rather than inventing one.)

- [ ] Run, expect FAIL (no FOOD pools yet): `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_adds_second_good_without_new_markets_or_traders`. Confirm FAIL on the `has_food_supply && has_food_demand` assert.
- [ ] In `seed.rs`, additively add a GOOD_FOOD supplier@`m_a` + consumer@`m_b` reusing the two existing markets (no new `Markets` / `MarketChunks` / `Traders` entries). Add `GOOD_FOOD` to the import list and insert after the existing GOOD_TOOLS pools (use fresh actor ids so they don't collide with `supplier`/`consumer`/`trader_actor` = 8001/8002/8003):

```rust
    // Second good (FOOD): a cheap supplier at m_a and a dear consumer at m_b,
    // reusing the SAME two markets. This adds POOLS, not markets/traders, so the
    // live macro flow shows a non-vacuous cross-market FOOD MacroFlow without
    // enlarging the world (Markets.len()==2, Traders.len()==1 still hold).
    let food_supplier = EconomicActorId(8_011);
    let food_consumer = EconomicActorId(8_012);
    world
        .resource_mut::<InventoryBook>()
        .deposit(food_supplier, GOOD_FOOD, Quantity(1_000_000))
        .expect("seed: food supplier goods");
    world
        .resource_mut::<AccountBook>()
        .deposit(food_consumer, Money(1_000_000))
        .expect("seed: food consumer cash");
    world.resource_mut::<SupplyPools>().0.insert(
        food_supplier,
        SupplyPool {
            actor: food_supplier,
            market: m_a,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<DemandPools>().0.insert(
        food_consumer,
        DemandPool {
            actor: food_consumer,
            market: m_b,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
```

  Update the `use crate::economy::{…}` line in seed.rs to include `GOOD_FOOD` (it currently imports `GOOD_TOOLS`).

- [ ] Run, expect PASS: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_adds_second_good_without_new_markets_or_traders`. Confirm PASS. Also run the existing seed tests (`scripts/cargo-serial.sh test … -p sim-core seed`) to confirm the count-invariant tests (Markets==2 / Traders==1) still pass.
- [ ] Create `backend/crates/sim-core/benches/economy_tick.rs` with the isolated flow benches, the schedule-level bench, and the programmatic superlinear gate. COMPLETE Rust:

```rust
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use bevy_ecs::prelude::*;
use criterion::{Criterion, criterion_group, criterion_main};

use sim_core::economy::{
    AccountBook, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId, EconomyConfig,
    GoodId, InventoryBook, MarketDistances, MarketGoods, MarketId, Money, Quantity, SupplyPool,
    SupplyPools, TradeLedger, run_macro_flow_at_tick,
};

/// Build M dormant markets × G goods with `pools_per_side` supply/demand pools,
/// arranged so every good has a cheap-surplus market and a dear-deficit market
/// (so the flow actually moves goods). Distances are a complete directed table.
struct FlowFixture {
    accounts: AccountBook,
    inventory: InventoryBook,
    ledger: TradeLedger,
    demand: DemandPools,
    supply: SupplyPools,
    market_goods: MarketGoods,
    dirty: DirtyMarketGoods,
    dormant: BTreeSet<MarketId>,
    distances: MarketDistances,
    config: EconomyConfig,
}

fn build_fixture(m: u32, g: u16) -> FlowFixture {
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    let mut dormant = BTreeSet::new();
    let mut actor: u64 = 1;
    for mi in 0..m {
        let market = MarketId(mi);
        dormant.insert(market);
        for gi in 0..g {
            let good = GoodId(gi + 1);
            // Even markets are cheap surplus sources; odd markets are dear sinks.
            if mi % 2 == 0 {
                inventory
                    .deposit(EconomicActorId(actor), good, Quantity(1_000_000))
                    .unwrap();
                supply.0.insert(EconomicActorId(actor), SupplyPool {
                    actor: EconomicActorId(actor), market, good,
                    offered_qty_per_tick: Quantity(200), min_price: Money(500),
                    interval_ticks: 1, last_generated_tick: None,
                });
            } else {
                accounts.deposit(EconomicActorId(actor), Money(1_000_000_000)).unwrap();
                demand.0.insert(EconomicActorId(actor), DemandPool {
                    actor: EconomicActorId(actor), market, good,
                    desired_qty_per_tick: Quantity(200), max_price: Money(2_000),
                    urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1, last_generated_tick: None,
                });
            }
            actor += 1;
        }
    }
    // Complete directed distance table between consecutive even/odd partners +
    // all pairs (bounded; M is small enough for the scale targets).
    let mut distances = MarketDistances(BTreeMap::new());
    for a in 0..m {
        for b in 0..m {
            if a != b {
                distances.0.insert((MarketId(a), MarketId(b)), 4);
            }
        }
    }
    let mut config = EconomyConfig::default();
    config.transport_cost_per_tile_unit = Money(50);
    FlowFixture {
        accounts, inventory, ledger: TradeLedger::default(), demand, supply,
        market_goods: MarketGoods::default(), dirty: DirtyMarketGoods::default(),
        dormant, distances, config,
    }
}

fn run_once(f: &mut FlowFixture) {
    run_macro_flow_at_tick(
        &mut f.accounts, &mut f.inventory, &mut f.ledger, &f.demand, &f.supply,
        &mut f.market_goods, &f.dirty, &f.dormant, &f.distances, &f.config, 0,
    )
    .unwrap();
}

/// Median wall time of one flow over `iters` rebuilt fixtures (fixture rebuilt
/// each iter so the flow always starts from the same state).
fn time_flow(m: u32, g: u16, iters: u32) -> f64 {
    let mut samples = Vec::with_capacity(iters as usize);
    for _ in 0..iters {
        let mut f = build_fixture(m, g);
        let t = Instant::now();
        run_once(&mut f);
        samples.push(t.elapsed().as_secs_f64());
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

fn macro_flow_2m_2g(c: &mut Criterion) {
    c.bench_function("macro_flow_2m_2g", |b| {
        b.iter_batched(
            || build_fixture(2, 2),
            |mut f| run_once(&mut f),
            criterion::BatchSize::SmallInput,
        );
    });
}

fn macro_flow_10k_pools_scale(c: &mut Criterion) {
    // M=200, G=8 → ~1600 pools per side ≈ ... use pools≈10k via M=200,G=50? Keep
    // M=200,G=8 → 200*8 = 1600 pools/side, 3200 total. To hit ≈10k pools use the
    // M/G the spec names (M≈200,G≈8) interpreted as ~1600/side ≈ 3200; scale via G.
    c.bench_function("macro_flow_10k_pools_scale", |b| {
        b.iter_batched(
            || build_fixture(200, 25), // 200*25 = 5000/side → 10_000 pools total
            |mut f| run_once(&mut f),
            criterion::BatchSize::SmallInput,
        );
    });
}

fn macro_flow_20k_pools_scale(c: &mut Criterion) {
    c.bench_function("macro_flow_20k_pools_scale", |b| {
        b.iter_batched(
            || build_fixture(200, 50), // 200*50 = 10_000/side → 20_000 pools total
            |mut f| run_once(&mut f),
            criterion::BatchSize::SmallInput,
        );
    });
}

/// Programmatic superlinear gate: the 20k/10k flow-cost ratio must be within a
/// generous LINEAR tolerance (pools double → cost ≤ ~2.6×, allowing the O(M²)
/// edge term + noise). A clearly superlinear trend (e.g. ratio > 3) FAILS the
/// bench run, blocking merge.
fn superlinear_gate(c: &mut Criterion) {
    c.bench_function("macro_flow_superlinear_gate", |b| {
        b.iter(|| {
            let t10 = time_flow(200, 25, 5);
            let t20 = time_flow(200, 50, 5);
            let ratio = t20 / t10.max(1e-9);
            assert!(
                ratio <= 3.0,
                "20k/10k flow-cost ratio {ratio:.2} is superlinear (> 3.0) — blocks merge"
            );
        });
    });
}

/// Schedule-level full EconomySet over flow + non-flow ticks, parameterized by
/// (M, G, A). A is the number of active (non-dormant) markets that auction-clear.
/// Large M×G / small A isolates the per-tick EWMA term.
fn economy_tick(c: &mut Criterion) {
    use sim_core::economy::EconomyPlugin;
    use sim_core::mobility::resources::Tick;
    use sim_core::world::plugin::CorePlugin;
    use sim_core::world::schedule::SimPlugin;

    let mut group = c.benchmark_group("economy_tick");
    for &(m, g, a) in &[(2u32, 2u16, 1u32), (200u32, 8u16, 4u32)] {
        group.bench_function(format!("m{m}_g{g}_a{a}"), |b| {
            b.iter_batched(
                || {
                    let mut world = World::new();
                    let mut schedule = bevy_ecs::schedule::Schedule::default();
                    CorePlugin::default().install(&mut world, &mut schedule);
                    sim_core::mobility::MobilityPlugin.install(&mut world, &mut schedule);
                    EconomyPlugin.install(&mut world, &mut schedule);
                    // Seed M markets of G goods into MarketGoods so the per-tick
                    // EWMA scan is exercised; the (M,G,A) wiring lives here.
                    let f = build_fixture(m, g);
                    world.insert_resource(f.accounts);
                    world.insert_resource(f.inventory);
                    world.insert_resource(f.demand);
                    world.insert_resource(f.supply);
                    world.insert_resource(f.distances);
                    world.insert_resource(f.config);
                    let _ = a; // active-market wiring placeholder; dormant set drives flow
                    world.insert_resource(Tick(0));
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    // Run a flow tick (0) and a non-flow tick (1).
                    schedule.run(&mut world);
                    world.resource_mut::<Tick>().0 = 1;
                    schedule.run(&mut world);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    macro_flow_2m_2g,
    macro_flow_10k_pools_scale,
    macro_flow_20k_pools_scale,
    superlinear_gate,
    economy_tick
);
criterion_main!(benches);
```

  Note on counts: the spec names "M≈200, G≈8, pools≈10000". `build_fixture(M,G)` makes `M*G` pools per side (`2*M*G` total). To hit ≈10k total use `(200,25)` and ≈20k use `(200,50)` (as above). If the planner prefers exact `(200,8)` per the spec, that yields 3200 pools — adjust the bench names/comment to reflect the real pool count rather than mislabeling; do NOT claim 10k when the fixture builds 3.2k. The superlinear gate must compare the SAME shape doubled (10k→20k), so keep the `(200,25)` vs `(200,50)` pairing whatever absolute label is used. Verify `MarketDistances`, `run_macro_flow_at_tick`, `EconomyConfig.transport_cost_per_tile_unit`, and `MobilityPlugin`/`CorePlugin` paths are public from `sim_core::economy` / `sim_core::world` / `sim_core::mobility` before finalizing (the crate re-exports `pub use economy::*`).

- [ ] Register the bench in `backend/crates/sim-core/Cargo.toml` by adding after the existing `[[bench]]` blocks:

```toml
[[bench]]
name = "economy_tick"
harness = false
```

- [ ] Verify the bench compiles and runs (record numbers for the PR): `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh bench --manifest-path backend/Cargo.toml -p sim-core --bench economy_tick`. Confirm `macro_flow_2m_2g`, `macro_flow_10k_pools_scale`, `macro_flow_20k_pools_scale`, `macro_flow_superlinear_gate`, and `economy_tick/*` all run; the superlinear gate must not panic (ratio ≤ 3.0). If the gate is flaky on CI hardware, raise the tolerance to a documented linear bound (e.g. 3.5) — never disable it. Record the `macro_flow_2m_2g` baseline ns in the PR body; a >20% regression on it blocks merge.
- [ ] Run the full sim-core test suite once to confirm nothing regressed and `tests/mod.rs` references `mod macro_flow;`, `mod warm_flow;` is gone, and `grep -n WarmMarkets backend/crates/sim-core/src/economy/tests/lod.rs` is empty: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core`. Confirm GREEN.
- [ ] Commit: `git add -A && git commit -m "feat(economy): live-seed second good + macro flow benches with superlinear gate"`.
