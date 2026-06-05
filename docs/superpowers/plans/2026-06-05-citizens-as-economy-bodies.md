# Citizens as the Economy's Bodies — Implementation Plan (Slice 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the persistent, aging ECS citizens the visible bodies of the mean-field economy — routed to their bound markets by realized demand/wages via a conservation-exact attribution — and delete the ephemeral shopper/commuter shadow agents, with the macro economy and the `#78` money-audit unchanged.

**Architecture:** The macro economy stays the sole authority (`O(sectors)`, viewport-independent, byte-invariant `#78` audit). A new read-only **attribution** step partitions each tick's realized consumption/wages onto observed, market-bound citizens — moving no money — and publishes a per-tick `AgentId → target NodeId` map. The citizen routing system reads that map and overrides the geometric destination of the economic leg. Citizens gain a static, seed-assigned `MarketBinding {home_market, work_market}` (inherited at birth, persisted). The shopper/commuter shadow machinery is removed; flow-traders stay.

**Tech Stack:** Rust (bevy_ecs, `sim-core` crate), TypeScript/Vite frontend, Playwright e2e. All cargo runs go through `scripts/cargo-serial.sh` locally (CLAUDE.md). Base branch: `feat/citizens-as-economy-bodies` off `origin/main` (worktree `/Users/ramonfuglister/Coding/abutown-citizens-economy`).

---

## File Structure

**Create:**
- `backend/crates/sim-core/src/mobility/market_binding.rs` — `MarketBinding` component + pure `assign_binding` helper (home = nearest market, work = nearest *other* market) + unit tests.
- `backend/crates/sim-core/src/economy/attribution.rs` — pure `attribute_citizens` core (magnitude-bounded, conservation-exact) + `run_citizen_attribution_system` exclusive system + unit tests.

**Modify:**
- `backend/crates/sim-core/src/mobility/mod.rs` — declare `pub mod market_binding;` and re-export `MarketBinding`.
- `backend/crates/sim-core/src/mobility/resources.rs` — add `CitizenEconomicTargets` resource (ephemeral `BTreeMap<AgentId, NodeId>`).
- `backend/crates/sim-core/src/mobility/mod.rs` (MobilityPlugin install) — insert `CitizenEconomicTargets::default()`.
- `backend/crates/sim-core/src/mobility/seed.rs:554-585` — assign `MarketBinding` after each seed spawn.
- `backend/crates/sim-core/src/population/mod.rs:218-247` — inherit mother's `MarketBinding` at birth.
- `backend/crates/sim-core/src/mobility/records.rs:81-96` — add `home_market`/`work_market` to `AgentRecord` (required, no serde-default).
- `backend/crates/sim-core/src/mobility/api.rs:457-474` — insert `MarketBinding` in the spawn bundle; `agent_record_from_entity` reads it.
- `backend/crates/sim-core/src/mobility/persist_snapshot.rs` — extract/apply already flow through `AgentRecord`; verify round-trip.
- `backend/crates/sim-core/src/mobility/snapshot_provider.rs:18-20,34-39` — bump `schema_version` 1→2; `migrate` refuses `from < 2` (one-time reset, no shim).
- `backend/crates/sim-core/src/economy/mod.rs` — declare `pub mod attribution;`; (Task 8) drop `shoppers`/`commuters` modules + resource inserts.
- `backend/crates/sim-core/src/economy/systems.rs` — add `EconomySet::Attribution` + register the system; (Task 8) remove `ShopperCapture`/`CommuterCapture` + their systems + config fields.
- `backend/crates/sim-core/src/mobility/systems/routing.rs:148-212` — economic destination override in `route_assignment_system`.

**Delete (Task 8):**
- `backend/crates/sim-core/src/economy/shoppers.rs`, `commuters.rs`
- `backend/crates/sim-core/src/economy/tests/shoppers.rs`, `tests/commuters.rs`
- shopper/commuter blocks in `economy/materialize.rs` + the shopper/commuter test in `tests/materialize.rs` and the ordering test in `tests/systems.rs`.

**Conventions for this plan:** all new Rust tests are `#[cfg(test)]` modules co-located in the file under test (or inside the new file), so no test-module wiring is guessed. Run a single test with:
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <FILTER> -- --nocapture`

---

### Task 1: `MarketBinding` component + pure assignment helper

**Files:**
- Create: `backend/crates/sim-core/src/mobility/market_binding.rs`
- Modify: `backend/crates/sim-core/src/mobility/mod.rs`

- [ ] **Step 1: Create the file with the component, the pure helper, and failing tests**

Create `backend/crates/sim-core/src/mobility/market_binding.rs`:

```rust
//! Static citizen↔market binding: each citizen shops at `home_market` and earns
//! wages at `work_market`. Assigned deterministically at seed from market anchor
//! positions; inherited by newborns; persisted in `AgentRecord`. Market ids are
//! raw `u32` (matching `MarketSpec.id` and the persisted record) so this mobility
//! module carries no dependency on `economy::MarketId`.

use bevy_ecs::prelude::Component;

/// The two markets a citizen is bound to. `home_market` is the shopping
/// destination (realized consumption); `work_market` is the wage commute target.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketBinding {
    pub home_market: u32,
    pub work_market: u32,
}

/// Deterministically choose (home_market, work_market) for a citizen at `pos`
/// from `markets` (each `(market_id, market_position)`).
///
/// - `home_market` = the market whose anchor is nearest `pos` (tie-break: lower id).
/// - `work_market` = the nearest market that is NOT `home_market` (tie-break: lower
///   id); if only one market exists, `work_market == home_market`.
///
/// Returns `None` only when `markets` is empty. Pure: no RNG, no wall-clock.
pub fn assign_binding(pos: (f32, f32), markets: &[(u32, (f32, f32))]) -> Option<MarketBinding> {
    fn dist2(a: (f32, f32), b: (f32, f32)) -> f32 {
        let dx = a.0 - b.0;
        let dy = a.1 - b.1;
        dx * dx + dy * dy
    }
    // Sort candidates by (distance, id) deterministically.
    let mut ranked: Vec<(u32, f32)> = markets.iter().map(|(id, mp)| (*id, dist2(pos, *mp))).collect();
    ranked.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let home_market = ranked.first()?.0;
    let work_market = ranked
        .iter()
        .find(|(id, _)| *id != home_market)
        .map(|(id, _)| *id)
        .unwrap_or(home_market);
    Some(MarketBinding {
        home_market,
        work_market,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_nearest_work_is_second_nearest() {
        // pos near market 9001; 9002 is the second nearest.
        let markets = vec![
            (9001u32, (2.0f32, 3.0f32)),
            (9002u32, (13.0, 3.0)),
            (9004u32, (208.0, 48.0)),
        ];
        let b = assign_binding((3.0, 3.0), &markets).unwrap();
        assert_eq!(b.home_market, 9001);
        assert_eq!(b.work_market, 9002);
    }

    #[test]
    fn single_market_makes_work_equal_home() {
        let markets = vec![(9001u32, (2.0f32, 3.0f32))];
        let b = assign_binding((100.0, 100.0), &markets).unwrap();
        assert_eq!(b.home_market, 9001);
        assert_eq!(b.work_market, 9001);
    }

    #[test]
    fn empty_markets_is_none() {
        assert!(assign_binding((0.0, 0.0), &[]).is_none());
    }

    #[test]
    fn deterministic_tie_break_by_id() {
        // Two markets equidistant from pos → lower id is home.
        let markets = vec![(9002u32, (0.0f32, 1.0f32)), (9001u32, (0.0, -1.0))];
        let b = assign_binding((0.0, 0.0), &markets).unwrap();
        assert_eq!(b.home_market, 9001, "equal distance → lower id wins");
        assert_eq!(b.work_market, 9002);
    }
}
```

- [ ] **Step 2: Declare the module + re-export in `mobility/mod.rs`**

Add to `backend/crates/sim-core/src/mobility/mod.rs` (with the other `pub mod` lines near the top):

```rust
pub mod market_binding;
```

And with the other re-exports:

```rust
pub use market_binding::MarketBinding;
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core market_binding -- --nocapture`
Expected: 4 tests pass (`home_is_nearest_work_is_second_nearest`, `single_market_makes_work_equal_home`, `empty_markets_is_none`, `deterministic_tie_break_by_id`).

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-core/src/mobility/market_binding.rs backend/crates/sim-core/src/mobility/mod.rs
git commit -m "feat(mobility): MarketBinding component + deterministic assign_binding helper"
```

---

### Task 2: Carry `MarketBinding` through `AgentRecord` and the spawn bundle

This wires the binding into the one spawn path used by BOTH seed and birth (`spawn_agent_from_record_with_position`), and into the persisted record. Assignment values come in later tasks; here the field plumbing + component insertion is added with a default-free required field.

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/records.rs:81-131`
- Modify: `backend/crates/sim-core/src/mobility/api.rs:457-481` (spawn bundle) and `agent_record_from_entity`

- [ ] **Step 1: Add required fields to `AgentRecord` (no serde-default — see Task 4 for the reset)**

In `backend/crates/sim-core/src/mobility/records.rs`, add to the `AgentRecord` struct (after `cyclic`):

```rust
    pub home_market: u32,
    pub work_market: u32,
```

In `AgentRecord::new_born_at` (lines ~112-131), initialize them to a sentinel `0` (callers overwrite before spawn):

```rust
            home_market: 0,
            work_market: 0,
```

> Note: `0` is never a real market id (`markets.json` ids start at `9001`); it means "unassigned" until a caller sets it. Task 4 makes these fields required in serialization (no `#[serde(default)]`), and old snapshots are discarded via the schema bump.

- [ ] **Step 2: Insert the `MarketBinding` component in the spawn bundle**

In `backend/crates/sim-core/src/mobility/api.rs`, inside `spawn_agent_from_record_with_position`, the bundle currently ends at `SpriteKey(sprite_key),` (line ~472). Add `MarketBinding` to the spawned tuple:

```rust
            SpriteKey(sprite_key),
            crate::mobility::MarketBinding {
                home_market: record.home_market,
                work_market: record.work_market,
            },
```

(If the `record` is moved into locals before `.spawn`, capture `home_market`/`work_market` into locals alongside the others first.)

- [ ] **Step 3: Read the binding back in `agent_record_from_entity`**

Find `agent_record_from_entity` in `backend/crates/sim-core/src/mobility/api.rs` (the extract counterpart, ~line 528 per the code-map). When it constructs the `AgentRecord`, populate the new fields from the component:

```rust
    let binding = world.get::<crate::mobility::MarketBinding>(entity);
    // ...inside the AgentRecord { ... } construction:
        home_market: binding.map(|b| b.home_market).unwrap_or(0),
        work_market: binding.map(|b| b.work_market).unwrap_or(0),
```

- [ ] **Step 4: Add a round-trip unit test (inline in `api.rs`)**

Add at the end of `backend/crates/sim-core/src/mobility/api.rs`:

```rust
#[cfg(test)]
mod market_binding_roundtrip_tests {
    use super::*;

    #[test]
    fn spawn_then_extract_preserves_binding() {
        // Build a minimal mobility world via the crate's existing test harness.
        let mut world = crate::mobility::api::test_world_with_minimal_graph();
        let mut rec = crate::mobility::AgentRecord::new_born_at(
            crate::ids::AgentId("agent:test:1".to_string()),
            crate::mobility::AgentMobilityState::Walking {
                link_id: "link:walk:corridor:0".to_string(),
                progress: 0.0,
            },
            vec![],
            0.05,
            0,
        );
        rec.home_market = 9001;
        rec.work_market = 9002;
        let entity = crate::mobility::api::spawn_agent_from_record(&mut world, rec);
        let binding = world
            .get::<crate::mobility::MarketBinding>(entity)
            .expect("spawned citizen has MarketBinding");
        assert_eq!(binding.home_market, 9001);
        assert_eq!(binding.work_market, 9002);
    }
}
```

> If `test_world_with_minimal_graph` does not exist, reuse the helper the existing `api.rs`/`seed.rs` tests use to build a test World (search `#[cfg(test)]` in `api.rs` for the established fixture) and call it instead — do not invent a new world builder.

- [ ] **Step 5: Run the test**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core market_binding_roundtrip -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Build the crate to confirm every `AgentRecord` literal compiles**

Run: `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-core`
Expected: compiles. If any other `AgentRecord { .. }` struct-literal exists (not using `new_born_at`), the compiler will name it — add `home_market: 0, work_market: 0` there.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/mobility/records.rs backend/crates/sim-core/src/mobility/api.rs
git commit -m "feat(mobility): carry MarketBinding through AgentRecord + spawn bundle"
```

---

### Task 3: Assign `MarketBinding` at SEED, inherit at BIRTH

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/seed.rs:554-585`
- Modify: `backend/crates/sim-core/src/population/mod.rs:158-247`

- [ ] **Step 1: Helper to read markets-with-positions from the World**

Add to `backend/crates/sim-core/src/mobility/market_binding.rs`:

```rust
/// Collect `(market_id, anchor_position)` for every seeded market, reading the
/// economy `Markets` resource and the routing `Graph` for each market node's
/// position. Returns an empty vec if the economy is not installed.
pub fn markets_with_positions(world: &bevy_ecs::world::World) -> Vec<(u32, (f32, f32))> {
    let Some(markets) = world.get_resource::<crate::economy::Markets>() else {
        return Vec::new();
    };
    let Some(graph) = world.get_resource::<crate::routing::Graph>() else {
        return Vec::new();
    };
    markets
        .0
        .iter()
        .map(|(id, site)| (id.0, graph.node(site.node_id).position))
        .collect()
}
```

- [ ] **Step 2: Assign binding after each SEED spawn**

In `backend/crates/sim-core/src/mobility/seed.rs`, the loop calls `api::spawn_agent_from_record(world, rec);` (line ~583). Replace that call with one that captures the entity and assigns the binding from the spawned position:

```rust
            let entity = api::spawn_agent_from_record(world, rec);
            let markets = crate::mobility::market_binding::markets_with_positions(world);
            if !markets.is_empty()
                && let Some(pos) = world
                    .get::<crate::mobility::components::Position>(entity)
                    .map(|p| (p.x, p.y))
                && let Some(binding) = crate::mobility::market_binding::assign_binding(pos, &markets)
            {
                world.entity_mut(entity).insert(binding);
            }
```

> The spawn computes the citizen's initial `Position` from corridor progress (code-map: `initial_agent_position`), so it is available immediately after spawn. `seed_from_markets_layer` runs before pedestrian seeding (runtime ordering verified), so `Markets` is populated.

- [ ] **Step 3: Inherit the mother's binding at BIRTH**

In `backend/crates/sim-core/src/population/mod.rs`, the `BirthCandidate` struct (lines ~158-165) must also carry the mother's binding. Add a field:

```rust
    mother_binding: Option<crate::mobility::MarketBinding>,
```

Where candidates are collected (the loop reading the mother entity's components, ~line 188), read the binding:

```rust
            mother_binding: world
                .get::<crate::mobility::MarketBinding>(*entity)
                .copied(),
```

Then in the spawn loop (lines ~231-246), set the child record's binding before spawning:

```rust
            if let Some(b) = candidate.mother_binding {
                child_record.home_market = b.home_market;
                child_record.work_market = b.work_market;
            }
```

(`spawn_agent_from_record_at_position` then inserts the `MarketBinding` component via the Task-2 bundle change.)

- [ ] **Step 4: Add an inline seed-binding integration test (`seed.rs`)**

Add at the end of `backend/crates/sim-core/src/mobility/seed.rs`:

```rust
#[cfg(test)]
mod seed_binding_tests {
    use super::*;

    #[test]
    fn seeded_citizens_get_a_market_binding() {
        // Build the abutopia base world bundle + seed it the way the runtime does.
        let bundle = crate::base_world::BaseWorldBundle::load_abutopia()
            .expect("load abutopia bundle");
        let (mut world, _schedule) =
            crate::mobility::seed::from_base_world_bundle(&bundle).expect("seed world");
        // Seed the economy markets the way the runtime does (RoutingPlugin already
        // populated NodeSpatialIndex inside from_base_world_bundle).
        crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);
        // Re-seed pedestrians is NOT needed; assert at least one seeded walker has a binding.
        let mut q = world.query_filtered::<&crate::mobility::MarketBinding, bevy_ecs::prelude::With<crate::mobility::components::AgentMarker>>();
        let count = q.iter(&world).count();
        assert!(count > 0, "at least one seeded citizen carries a MarketBinding");
        for b in q.iter(&world) {
            assert!(b.home_market >= 9001, "home_market is a real market id");
        }
    }
}
```

> If `from_base_world_bundle` seeds pedestrians BEFORE `seed_from_markets_layer` can run here (so bindings are empty), instead mirror the runtime order: use the lower-level seed entry points so markets seed first. Confirm the order against `sim-server/src/runtime/mod.rs:207-221` (markets seed at line 221, pedestrians after) and replicate it. Do NOT assert a binding on citizens seeded before markets existed.

- [ ] **Step 5: Add an inline birth-inheritance test (`population/mod.rs`)**

Add at the end of `backend/crates/sim-core/src/population/mod.rs` a `#[cfg(test)]` test that: spawns one female citizen with `MarketBinding{home_market:9001, work_market:9002}`, advances `population_monthly_system` far enough to force one birth (reuse the existing population test fixture/harness in this file — search its current `#[cfg(test)]` module for the helper that drives a birth), then asserts the newborn entity (the one with `ParentId(Some(mother))`) carries the SAME binding. Use the existing harness; do not invent a new clock/world builder.

- [ ] **Step 6: Run both tests**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_binding_tests -- --nocapture`
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population -- --nocapture`
Expected: both PASS (seeded citizens bound; newborn inherits mother's binding).

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/mobility/seed.rs backend/crates/sim-core/src/mobility/market_binding.rs backend/crates/sim-core/src/population/mod.rs
git commit -m "feat: assign MarketBinding at seed (nearest markets) and inherit at birth"
```

---

### Task 4: Persist the binding + one-time mobility reset (schema bump, no shim)

The binding is already in `AgentRecord` (Task 2). `AgentRecord` derives `Serialize/Deserialize`, so the new fields serialize automatically. Per project rule (no serde-default legacy shim, no heal-on-restore), the fields are **required**, and a schema bump discards old (v1) snapshots.

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/snapshot_provider.rs:14-40`
- Verify: `backend/crates/sim-core/src/mobility/persist_snapshot.rs` round-trip (extract→apply via `AgentRecord`).

- [ ] **Step 1: Confirm the host's version-gate behavior (read-only)**

Read `backend/crates/sim-core/src/world/persistence.rs` (the `SnapshotProvider` trait + `MigrationError`) and the sim-server snapshot store that loads snapshots (search for `schema_version` / `migrate(` usage in `backend/crates/sim-server/`). Confirm: when a stored item's `schema_version` is below the provider's current version, the host calls `migrate(raw, from_version)`; an `Err` from `migrate` means the snapshot is discarded and the world reseeds fresh. Note the exact `MigrationError` variant to return for "cannot migrate, reset."

- [ ] **Step 2: Write a failing serialization round-trip test (inline in `persist_snapshot.rs`)**

Add at the end of `backend/crates/sim-core/src/mobility/persist_snapshot.rs`:

```rust
#[cfg(test)]
mod binding_persistence_tests {
    use super::*;

    #[test]
    fn agent_record_roundtrips_market_binding() {
        let mut rec = crate::mobility::AgentRecord::new_born_at(
            crate::ids::AgentId("agent:walk:7".to_string()),
            crate::mobility::AgentMobilityState::Walking {
                link_id: "link:walk:corridor:0".to_string(),
                progress: 0.0,
            },
            vec![],
            0.05,
            0,
        );
        rec.home_market = 9003;
        rec.work_market = 9004;
        let json = serde_json::to_string(&rec).unwrap();
        let back: crate::mobility::AgentRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.home_market, 9003);
        assert_eq!(back.work_market, 9004);
    }

    #[test]
    fn old_v1_record_without_binding_fails_to_deserialize() {
        // A v1 AgentRecord JSON lacks home_market/work_market. Without serde-default,
        // deserialization MUST fail — proving we rely on the schema bump, not a shim.
        let v1 = r#"{"id":"agent:walk:7","state":{"Walking":{"link_id":"l","progress":0.0}},"plan":[],"plan_cursor":0,"walk_speed_per_tick":0.05,"birth_tick":0}"#;
        let parsed: Result<crate::mobility::AgentRecord, _> = serde_json::from_str(v1);
        assert!(parsed.is_err(), "v1 record must NOT deserialize (no serde-default shim)");
    }
}
```

- [ ] **Step 3: Run the test to verify it passes (fields required, v1 rejected)**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core binding_persistence_tests -- --nocapture`
Expected: both PASS. If `old_v1_record_without_binding_fails_to_deserialize` FAILS, a `#[serde(default)]` leaked onto the new fields — remove it (the fields must be required).

- [ ] **Step 4: Bump the mobility schema version and refuse v1 migration**

In `backend/crates/sim-core/src/mobility/snapshot_provider.rs`:

```rust
    fn schema_version(&self) -> u32 {
        2
    }
```

In `collect`, change the hardcoded `schema_version: 1` in the emitted `SnapshotItem` to `2`.

In `migrate`, refuse anything older than 2 so the host discards it (use the variant confirmed in Step 1; shown here as `Unsupported`):

```rust
    fn migrate(&self, raw: SnapshotItem, from: u32) -> Result<SnapshotItem, MigrationError> {
        if from >= 2 {
            Ok(raw)
        } else {
            // v1 mobility snapshots predate MarketBinding. No shim, no heal-on-restore:
            // discard and reseed fresh (one-time mobility-state reset).
            Err(MigrationError::Unsupported { from, to: 2 })
        }
    }
```

> If the host does NOT auto-discard on `migrate` error, instead perform the documented one-time `DELETE FROM mobility_snapshots` in the sim-server store load path before accepting v2. Pick whichever the store actually supports (confirmed in Step 1). Either way: no serde-default, no heal-on-restore.

- [ ] **Step 5: Run the snapshot-provider tests**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core snapshot_provider -- --nocapture`
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core persist -- --nocapture`
Expected: PASS (existing persistence tests still green; provider reports version 2).

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/mobility/snapshot_provider.rs backend/crates/sim-core/src/mobility/persist_snapshot.rs
git commit -m "feat(persistence): persist MarketBinding; bump mobility schema v1→2 with one-time reset (no shim)"
```

---

### Task 5: Attribution core — conservation-exact, magnitude-bounded

A pure function that, per market, selects the attributed citizen cohort and proves `attributed_qty + unobserved == realized` exactly.

**Files:**
- Create: `backend/crates/sim-core/src/economy/attribution.rs`
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (add `pub mod attribution;`)

- [ ] **Step 1: Create `attribution.rs` with the pure core + failing tests**

Create `backend/crates/sim-core/src/economy/attribution.rs`:

```rust
//! Conservation-exact attribution of the macro's realized consumption/wages onto
//! observed, market-bound citizens. READ-ONLY over economy quantities: it mints
//! and moves NO money, so the `#78` tick audit is unaffected. It only SELECTS
//! which citizens are economically targeted this tick and proves the partition
//! identity `attributed + unobserved == realized`.

/// One market's attribution outcome for a single channel (shopping OR wages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelAttribution {
    /// Citizens selected to represent the realized activity, in deterministic order.
    pub attributed: Vec<crate::ids::AgentId>,
    /// `attributed.len() as i64 * per_unit` — the quantity the visible citizens depict.
    pub attributed_amount: i64,
    /// `realized - attributed_amount` (>= 0) — the part no visible citizen depicts.
    pub unobserved_amount: i64,
}

/// Select up to `min(realized / per_unit, cap, candidates.len())` citizens from
/// `candidates` (already sorted deterministically by the caller, e.g. by AgentId),
/// each representing `per_unit` units. Pure; no RNG.
///
/// `realized` is the macro's realized quantity (consumed goods, or wage Money).
/// Guarantees `attributed_amount + unobserved_amount == realized` exactly.
pub fn attribute_channel(
    realized: i64,
    per_unit: i64,
    cap: usize,
    candidates: &[crate::ids::AgentId],
) -> ChannelAttribution {
    let per_unit = per_unit.max(1);
    let by_magnitude = (realized / per_unit).max(0) as usize;
    let count = by_magnitude.min(cap).min(candidates.len());
    let attributed: Vec<crate::ids::AgentId> = candidates.iter().take(count).cloned().collect();
    let attributed_amount = (count as i64) * per_unit;
    let unobserved_amount = realized - attributed_amount;
    ChannelAttribution {
        attributed,
        attributed_amount,
        unobserved_amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::AgentId;

    fn ids(n: usize) -> Vec<AgentId> {
        (0..n).map(|i| AgentId(format!("agent:walk:{i}"))).collect()
    }

    #[test]
    fn count_is_min_of_magnitude_cap_and_candidates() {
        // realized 9, per_unit 3 → magnitude 3; cap 4; candidates 10 → count 3.
        let c = attribute_channel(9, 3, 4, &ids(10));
        assert_eq!(c.attributed.len(), 3);
        assert_eq!(c.attributed_amount, 9);
        assert_eq!(c.unobserved_amount, 0);
    }

    #[test]
    fn cap_bounds_the_cohort_and_leaves_unobserved_remainder() {
        // realized 100, per_unit 3 → magnitude 33; cap 4 → count 4; 4*3=12 attributed.
        let c = attribute_channel(100, 3, 4, &ids(10));
        assert_eq!(c.attributed.len(), 4, "absolute cap, never scales with population");
        assert_eq!(c.attributed_amount, 12);
        assert_eq!(c.unobserved_amount, 88);
        assert_eq!(c.attributed_amount + c.unobserved_amount, 100, "conservation identity");
    }

    #[test]
    fn fewer_candidates_than_magnitude_caps_at_candidates() {
        // realized 9, per_unit 3 → magnitude 3, but only 2 observed citizens bound here.
        let c = attribute_channel(9, 3, 4, &ids(2));
        assert_eq!(c.attributed.len(), 2);
        assert_eq!(c.attributed_amount, 6);
        assert_eq!(c.unobserved_amount, 3);
        assert_eq!(c.attributed_amount + c.unobserved_amount, 9);
    }

    #[test]
    fn zero_realized_attributes_nobody() {
        let c = attribute_channel(0, 3, 4, &ids(10));
        assert!(c.attributed.is_empty());
        assert_eq!(c.attributed_amount, 0);
        assert_eq!(c.unobserved_amount, 0);
    }

    #[test]
    fn selection_is_deterministic_prefix() {
        let c = attribute_channel(9, 3, 4, &ids(10));
        assert_eq!(
            c.attributed,
            vec![AgentId("agent:walk:0".into()), AgentId("agent:walk:1".into()), AgentId("agent:walk:2".into())],
        );
    }
}
```

- [ ] **Step 2: Declare the module in `economy/mod.rs`**

Add near the other `pub mod` lines in `backend/crates/sim-core/src/economy/mod.rs`:

```rust
pub mod attribution;
```

- [ ] **Step 3: Run the tests**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core attribution::tests -- --nocapture`
Expected: 5 tests pass, including the two conservation-identity assertions.

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-core/src/economy/attribution.rs backend/crates/sim-core/src/economy/mod.rs
git commit -m "feat(economy): conservation-exact, magnitude-bounded attribution core"
```

---

### Task 6: Attribution system + `CitizenEconomicTargets` + `EconomySet::Attribution`

Wire the pure core into an exclusive system that reads realized telemetry + observed markets + bound citizens, and publishes the per-tick `AgentId → target NodeId` map for routing.

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/resources.rs` (new resource)
- Modify: `backend/crates/sim-core/src/mobility/mod.rs` (MobilityPlugin inserts the resource)
- Modify: `backend/crates/sim-core/src/economy/attribution.rs` (the system)
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (new `EconomySet::Attribution` phase + registration)

- [ ] **Step 1: Add the `CitizenEconomicTargets` resource**

In `backend/crates/sim-core/src/mobility/resources.rs`, add (near `AgentIdIndex`):

```rust
/// Per-tick economic destination override for citizens, written by the economy
/// attribution system and read by `route_assignment_system`. Maps a citizen's
/// stable `AgentId` to the routing node it should walk its economic leg toward
/// this tick. Ephemeral: cleared and repopulated every tick; never persisted.
#[derive(bevy_ecs::prelude::Resource, Debug, Default, Clone)]
pub struct CitizenEconomicTargets(pub std::collections::BTreeMap<crate::ids::AgentId, crate::routing::NodeId>);
```

- [ ] **Step 2: Insert the resource in `MobilityPlugin::install`**

In `backend/crates/sim-core/src/mobility/mod.rs`, find `MobilityPlugin`'s `install` and add (with the other `insert_resource` calls):

```rust
        world.insert_resource(crate::mobility::resources::CitizenEconomicTargets::default());
```

> It lives in the mobility plugin (read by routing) so it always exists even when the economy is absent; the attribution system below only *populates* it.

- [ ] **Step 3: Add the exclusive attribution system to `attribution.rs`**

Append to `backend/crates/sim-core/src/economy/attribution.rs`:

```rust
use bevy_ecs::world::World;

/// Exclusive system (EconomySet::Attribution). Reads realized consumption
/// (`MarketGoods.consumed_qty_last_tick`, valid after Consume) and wages
/// (`WageTelemetry`, valid after PayWages); restricts to observed markets (those
/// whose market node is in an Active/Hot chunk — identical test to the former
/// capture systems); selects the attributed cohort from observed, bound citizens;
/// and writes their economic target node into `CitizenEconomicTargets`. Mints and
/// moves NO money.
pub fn run_citizen_attribution_system(world: &mut World) {
    use crate::economy::{EconomyConfig, MarketGoods, Markets, WageTelemetry};
    use crate::mobility::resources::CitizenEconomicTargets;
    use std::collections::{BTreeMap, BTreeSet};

    // Resources may be absent in narrow tests; no-op then.
    if world.get_resource::<Markets>().is_none()
        || world.get_resource::<crate::routing::Graph>().is_none()
        || world.get_resource::<CitizenEconomicTargets>().is_none()
    {
        return;
    }

    let computed: BTreeMap<crate::ids::AgentId, crate::routing::NodeId> = {
        use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk};
        let observed_chunks: BTreeSet<crate::world::components::ChunkCoord> = {
            let mut q = world
                .query_filtered::<&ChunkCoordComp, bevy_ecs::prelude::Or<(bevy_ecs::prelude::With<ActiveChunk>, bevy_ecs::prelude::With<HotChunk>)>>();
            q.iter(world).map(|c| c.0).collect()
        };
        let graph = world.resource::<crate::routing::Graph>();
        let markets = world.resource::<Markets>();
        // Observed markets: market node chunk currently observed (same as old capture).
        let observed_markets: BTreeSet<u32> = markets
            .0
            .iter()
            .filter(|(_, site)| {
                let pos = graph.node(site.node_id).position;
                observed_chunks.contains(&crate::mobility::chunk_of(pos.0, pos.1, 32))
            })
            .map(|(id, _)| id.0)
            .collect();
        if observed_markets.is_empty() {
            world.resource_mut::<CitizenEconomicTargets>().0.clear();
            return;
        }

        // Realized consumption per market (sum across goods).
        let mut consumed_by_market: BTreeMap<u32, i64> = BTreeMap::new();
        for (key, st) in world.resource::<MarketGoods>().0.iter() {
            if observed_markets.contains(&key.market.0) {
                *consumed_by_market.entry(key.market.0).or_default() += st.consumed_qty_last_tick.0;
            }
        }
        // Realized wage per market.
        let wage_by_market: BTreeMap<u32, i64> = world
            .resource::<WageTelemetry>()
            .0
            .iter()
            .filter(|(m, _)| observed_markets.contains(&m.0))
            .map(|(m, w)| (m.0, w.0))
            .collect();

        // Candidate citizens per market, sorted by AgentId (deterministic).
        // shop candidates → bound by home_market; work candidates → by work_market.
        let mut shop_candidates: BTreeMap<u32, Vec<crate::ids::AgentId>> = BTreeMap::new();
        let mut work_candidates: BTreeMap<u32, Vec<crate::ids::AgentId>> = BTreeMap::new();
        {
            let mut q = world.query_filtered::<(&crate::mobility::components::StableAgentId, &crate::mobility::MarketBinding), bevy_ecs::prelude::With<crate::mobility::components::AgentMarker>>();
            for (id, binding) in q.iter(world) {
                if observed_markets.contains(&binding.home_market) {
                    shop_candidates.entry(binding.home_market).or_default().push(id.0.clone());
                }
                if observed_markets.contains(&binding.work_market) {
                    work_candidates.entry(binding.work_market).or_default().push(id.0.clone());
                }
            }
        }
        for v in shop_candidates.values_mut() {
            v.sort();
        }
        for v in work_candidates.values_mut() {
            v.sort();
        }

        let config = *world.resource::<EconomyConfig>();
        let mut targets: BTreeMap<crate::ids::AgentId, crate::routing::NodeId> = BTreeMap::new();
        // Shopping: attribute consumption to home-bound citizens; target = home market node.
        for (market_id, realized) in consumed_by_market {
            let Some(site) = markets.0.get(&crate::economy::MarketId(market_id)) else {
                continue;
            };
            let cands = shop_candidates.get(&market_id).cloned().unwrap_or_default();
            let res = attribute_channel(realized, config.shoppers_per_unit, config.max_shoppers_per_market, &cands);
            for id in res.attributed {
                targets.insert(id, site.node_id);
            }
        }
        // Wages: attribute wages to work-bound citizens; target = work market node.
        for (market_id, realized) in wage_by_market {
            let Some(site) = markets.0.get(&crate::economy::MarketId(market_id)) else {
                continue;
            };
            let cands = work_candidates.get(&market_id).cloned().unwrap_or_default();
            let res = attribute_channel(realized, config.commuters_per_wage_unit, config.max_commuters_per_market, &cands);
            for id in res.attributed {
                // A citizen already shopping keeps the shop target (shop wins ties
                // deterministically: home/consumption leg first). Only insert if absent.
                targets.entry(id).or_insert(site.node_id);
            }
        }
        targets
    };

    let mut out = world.resource_mut::<CitizenEconomicTargets>();
    out.0 = computed;
}
```

> Uses the existing config fields `shoppers_per_unit`/`max_shoppers_per_market`/`commuters_per_wage_unit`/`max_commuters_per_market` (Task 8 keeps these — they move from "shadow tuning" to "attribution tuning"; the shadow *systems* are deleted, the magnitude constants stay).

- [ ] **Step 4: Add the `EconomySet::Attribution` phase + register the system**

In `backend/crates/sim-core/src/economy/systems.rs`, add `Attribution` to the `EconomySet` enum immediately after `Consume`:

```rust
    Consume,
    Attribution,
    ShopperCapture,
```

Add it to the `configure_sets((...).chain())` tuple in the same position (after `EconomySet::Consume`, before `EconomySet::ShopperCapture`).

Register the system (exclusive, like the capture systems), after the consume registration and before the shopper registration:

```rust
    schedule.add_systems(
        crate::economy::attribution::run_citizen_attribution_system
            .in_set(EconomySet::Attribution)
            .before(crate::mobility::systems::tick_increment_system),
    );
```

- [ ] **Step 5: Add an inline system test (`attribution.rs`)**

Append a `#[cfg(test)]` test to `attribution.rs` that builds a World with: `Graph` + one `MarketSite` (node in an Active chunk), `MarketGoods` with `consumed_qty_last_tick = 9` for that market, `EconomyConfig::default()`, `CitizenEconomicTargets::default()`, and 5 citizens with `MarketBinding{home_market: that_market, work_market: that_market}` placed so the market chunk is observed. Run `run_citizen_attribution_system`; assert exactly `min(9/3, 4, 5) = 3` citizens appear in `CitizenEconomicTargets` mapped to the market node. Reuse the World/graph/chunk fixture the existing economy tests use (`economy/tests/materialize.rs::routed_shipment_world` is the closest template for a routed world with markets + observed chunks) — adapt it; do not invent a new graph builder.

- [ ] **Step 6: Run the test**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core attribution -- --nocapture`
Expected: PASS (3 citizens attributed to the market node).

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/attribution.rs backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/mobility/resources.rs backend/crates/sim-core/src/mobility/mod.rs
git commit -m "feat(economy): citizen attribution system + CitizenEconomicTargets + EconomySet::Attribution phase"
```

---

### Task 7: Economic destination override in `route_assignment_system`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/systems/routing.rs:30-49,148-212`

- [ ] **Step 1: Add the resource to the routing system signature**

In `route_assignment_system` (line ~148), add a parameter (after `waypoints`):

```rust
    targets: Option<Res<crate::mobility::resources::CitizenEconomicTargets>>,
```

- [ ] **Step 2: Override the resolved destination for the economic leg**

Immediately after the `destination` is resolved (the `let Some(destination) = destination_for_stage(...)` block ends at line ~212), insert an override. The economic leg is the `activity:destination` stage; the `home` leg stays geometric:

```rust
        // Economic override: when this citizen is in the tick's attributed cohort,
        // its economic leg (activity:destination) walks to the bound market node
        // instead of the geometric corridor endpoint. The home leg is untouched.
        let mut destination = destination;
        if let PlanStage::WalkToActivity { activity_id, .. } = &stage
            && activity_id == "activity:destination"
            && let Some(targets) = targets.as_deref()
            && let Some(node) = targets.0.get(&stable.0)
        {
            destination = *node;
        }
```

(Change the original `let Some(destination) = ...` binding so the subsequent `let mut destination` shadow compiles — i.e. resolve into a temporary, then `let mut destination = resolved;` before the override. Keep the existing failure path: if `destination_for_stage` returns `None`, still `stats.failed += 1; continue;`.)

- [ ] **Step 3: Write an inline routing-override test (`routing.rs`)**

Add a `#[cfg(test)]` test that builds a routed mobility World (reuse the existing routing/seed test fixture in this module — search the file's current `#[cfg(test)]` for the graph+agent builder), spawns one citizen on a `Walking` link whose `plan.cursor` points at a `WalkToActivity{activity_id:"activity:destination"}` stage, inserts `CitizenEconomicTargets` mapping that citizen's `AgentId` to a specific `NodeId`, runs `route_assignment_system` once, and asserts the agent's assigned `ActiveRoute`/corridor heads toward the overridden node (not the geometric waypoint). If asserting the full route is heavy, factor the override into a tiny pure helper `economic_destination(stage, agent_id, geometric, targets) -> NodeId` and unit-test THAT directly (preferred — keeps the test fast and deterministic), then call it from the system.

> Recommended: extract the override into a pure helper and unit-test it:
> ```rust
> pub(crate) fn economic_destination(
>     stage: &PlanStage,
>     agent: &crate::ids::AgentId,
>     geometric: crate::routing::NodeId,
>     targets: Option<&crate::mobility::resources::CitizenEconomicTargets>,
> ) -> crate::routing::NodeId {
>     if let PlanStage::WalkToActivity { activity_id, .. } = stage
>         && activity_id == "activity:destination"
>         && let Some(t) = targets
>         && let Some(node) = t.0.get(agent)
>     {
>         return *node;
>     }
>     geometric
> }
> ```
> Test: home-leg stage → geometric unchanged; destination-leg with a target → overridden node; destination-leg without a target → geometric.

- [ ] **Step 4: Run the test**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core routing -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Confirm + document the economy→routing ordering**

Read how `EconomySet` and the mobility routing set are ordered in the schedule (search `route_assignment_system` registration + `tick_increment_system` ordering). Add a one-line code comment above the override stating whether `CitizenEconomicTargets` is read same-tick (economy precedes routing) or with a deterministic one-tick lag. Both are correct (the economy's telemetry is itself last-tick); just record which it is.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/mobility/systems/routing.rs
git commit -m "feat(mobility): economic destination override for attributed citizens"
```

---

### Task 8: Delete the shopper/commuter shadow machinery

Now that citizens carry the economic bodies, remove the shadows. Flow-traders stay. Per the code-map deletion surface.

**Files:**
- Delete: `backend/crates/sim-core/src/economy/shoppers.rs`, `commuters.rs`, `tests/shoppers.rs`, `tests/commuters.rs`
- Modify: `economy/mod.rs`, `economy/systems.rs`, `economy/materialize.rs`, `economy/tests/mod.rs`, `economy/tests/materialize.rs`, `economy/tests/systems.rs`

- [ ] **Step 1: Remove module declarations, re-exports, and resource inserts (`economy/mod.rs`)**

Delete `mod shoppers;` / `mod commuters;` (and any `pub mod`) and `pub use shoppers::*;` / `pub use commuters::*;`. Delete the 4 resource inserts in `EconomyPlugin::install` (lines ~83-86: `ShopperVisits`, `NextShopperId`, `CommuterTrips`, `NextCommuterId`).

- [ ] **Step 2: Remove the capture phases, systems, and tuning fields (`economy/systems.rs`)**

- Delete `EconomySet::ShopperCapture` and `EconomySet::CommuterCapture` from the enum and from the `configure_sets((...).chain())` tuple (keep `EconomySet::Attribution` from Task 6).
- Delete `run_shopper_capture_system` (lines ~281-353) and `run_commuter_capture_system` (lines ~712-782) and their `schedule.add_systems(...)` registrations (lines ~241-253).
- In `EconomyConfig` keep the magnitude constants (`shoppers_per_unit`, `max_shoppers_per_market`, `commuters_per_wage_unit`, `max_commuters_per_market`) — they now tune **attribution**. Delete only `shopper_radius_tiles` (used solely by the deleted capture origin-picker) from the struct AND from `EconomyConfig::default()`.

> Rename note (optional, do NOT do in this task): the fields could later be renamed `attributed_*`; keep names stable here to minimize churn and keep the ported tests valid.

- [ ] **Step 3: Delete the shopper/commuter files**

```bash
git rm backend/crates/sim-core/src/economy/shoppers.rs backend/crates/sim-core/src/economy/commuters.rs
git rm backend/crates/sim-core/src/economy/tests/shoppers.rs backend/crates/sim-core/src/economy/tests/commuters.rs
```

Remove `mod shoppers;` / `mod commuters;` from `economy/tests/mod.rs`.

- [ ] **Step 4: Simplify `id_prefix` and drop the shopper/commuter render blocks (`economy/materialize.rs`)**

- Replace `id_prefix` (lines ~77-85) with the trivial form:

```rust
pub(crate) fn id_prefix(_actor: EconomicActorId) -> &'static str {
    "trader:"
}
```

- Delete `rendering_shopper_ids` (lines ~357-366) and `rendering_commuter_ids` (lines ~372-380).
- In `materialize_traders_system`: delete the pre-apply shopper/commuter expiry block (lines ~400-411 — keep the `FlowShipments` `expire_arrived` call), the `ShopperVisits` iteration (lines ~464-477), the `CommuterTrips` iteration (lines ~483-496), and the post-apply shopper/commuter expiry block (lines ~526-537 — again keep the `FlowShipments` part).

- [ ] **Step 5: Remove the shadow-dependent tests (`economy/tests/`)**

- In `tests/materialize.rs`: delete `materialize_does_not_touch_money_or_goods_with_active_shipment`'s shopper/commuter insertions (or the whole test if it is shopper/commuter-specific), delete `materialize_renders_shopper_then_despawns_on_arrival` (lines ~353-474), and remove the `ShopperVisits`/`CommuterTrips` initialization in the `routed_shipment_world` fixture (lines ~224-225).
- In `tests/systems.rs`: delete `shopper_capture_set_runs_after_macro_flow_before_materialize` (lines ~319-370).

- [ ] **Step 6: Build + clippy to confirm the deletion compiles clean**

Run: `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-core`
Run: `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-core --all-targets -- -D warnings`
Expected: compiles with zero warnings. Fix any remaining references the compiler names (e.g. a `use` of a deleted symbol).

- [ ] **Step 7: Run the economy test suite**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy -- --nocapture`
Expected: PASS (no shopper/commuter tests remain; attribution + flow-trader + audit tests green).

- [ ] **Step 8: Commit**

```bash
git add -A backend/crates/sim-core/src/economy
git commit -m "refactor(economy): delete shopper/commuter shadow machinery (citizens are the bodies now); flow-traders retained"
```

---

### Task 9: Whole-system verification (audit, smoke, full CI gate)

**Files:** none new — verification of the integrated slice.

- [ ] **Step 1: `#78` money-conservation audit stays byte-invariant with citizens as bodies**

Run the economy audit/conservation tests:
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core audit -- --nocapture`
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core conservation -- --nocapture`
Expected: PASS. (Attribution moves no money, so `total_money` byte-invariance and the `HOUSEHOLD_SECTOR` net-zero sentinels are unchanged. If anything here fails, attribution wrote to `AccountBook`/`InventoryBook` — it must not; revisit Task 6.)

- [ ] **Step 2: Full Rust gate**

Run: `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all`
Run: `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
Run: `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
Expected: all green. (Workspace test run covers sim-server integration that installs the economy beside mobility/population.)

- [ ] **Step 3: Frontend gate (no frontend code changed, but the agent stream did)**

Run: `npm run typecheck`
Run: `npm test`
Run: `npm run build`
Expected: all green. The render-smoke vitest/e2e **300-pedestrian pin** must still hold: it filters out `trader:`/`shopper:`/`commuter:` id-prefixes, and citizens keep their `agent:walk:*`/`agent:born:*` ids — so removing shadows leaves the count at 300. If the count changed, a citizen id was accidentally reprefixed — fix that, do NOT edit the pin.

- [ ] **Step 4: Browser smoke (mandatory — frontend↔backend agent stream changed)**

Run the e2e render-smoke (launches the dev stack + headless chromium):
Run: `npm run test:e2e`
Expected: PASS — citizens stream and render as pedestrians; no `shopper:`/`commuter:` agents appear on the wire anymore. If `test:e2e` is heavy, additionally adapt `scripts/smoke-7a.mjs` to log WS agent frames and confirm citizens (and, when a market is observed, citizens routed toward a market node) appear, while no `shopper:`/`commuter:` ids do.

- [ ] **Step 5: Manual liveness sanity (honest "correct but sparse")**

Confirm by observation that at demo scale most citizens keep their geometric routine and only a small attributed cohort heads to markets when consumption/wages are realized in an observed chunk. This is the intended Slice-1 behavior (density arrives in Slice 2 / per-capita). Note this in the PR description so reviewers don't read sparseness as a bug.

- [ ] **Step 6: Final commit / branch ready**

```bash
git add -A
git commit -m "test: verify citizens-as-economy-bodies slice — audit byte-invariant, smoke green, 300-pin holds" || echo "nothing to commit"
```

The branch `feat/citizens-as-economy-bodies` is now ready for the finishing-a-development-branch flow (PR to `origin/main`). Deploy note for the PR: this slice bumps the **mobility** snapshot schema (v1→2, one-time reset; existing dev saves lose accumulated ages/births) and introduces **no** `economy_snapshots` change (no `DELETE FROM economy_snapshots` needed).

---

## Self-Review (author checklist — completed)

**Spec coverage:** Hybrid causality (macro authoritative, no money moved) → Tasks 5–6 + Task 9 Step 1. Static seed binding + birth inheritance → Tasks 1–3. Persistence + one-time reset (no shim) → Task 4. Movement augmentation (not replacement) → Task 7. Shadow deletion, flow-traders retained → Task 8. Conservation-exact attribution identity → Task 5 tests. Render (citizens already pedestrians; no change) + render-smoke 300 holds → Task 9 Step 3. Browser-smoke mandate + full CI gate → Task 9. Slice-2/per-capita explicitly out of scope.

**Placeholder scan:** No "TBD/handle errors/similar to". Two steps intentionally say "confirm against the actual fixture/host then implement X" (Task 4 Step 1 host version-gate; test-fixture reuse in Tasks 3/6/7) — these name the exact file to read and the exact fallback, because the host's delete-on-mismatch and the crate's test-fixture helpers were not captured verbatim in the code-map and must not be invented.

**Type consistency:** `MarketBinding{home_market:u32, work_market:u32}` consistent across component, `AgentRecord`, attribution candidate collection, and routing. `CitizenEconomicTargets(BTreeMap<AgentId, NodeId>)` written in Task 6, read in Task 7. `attribute_channel` signature in Task 5 matches its call sites in Task 6. `EconomySet::Attribution` added in Task 6, survives Task 8's deletion of `ShopperCapture`/`CommuterCapture`. Config fields `shoppers_per_unit`/`max_shoppers_per_market`/`commuters_per_wage_unit`/`max_commuters_per_market` retained in Task 8 (used by attribution); only `shopper_radius_tiles` removed.
