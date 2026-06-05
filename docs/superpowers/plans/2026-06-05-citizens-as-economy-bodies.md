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

### Task 3: Assign `MarketBinding` at SEED (in the spawn path), inherit at BIRTH

**Why assignment lives in the spawn function, not `seed.rs` (corrected after tracing the runtime):** pedestrians are NOT seeded directly into the live world. The runtime builds them in a temp world WITHOUT the economy (`initial_mobility_snapshot_for_base_world` → `mobility::seed::from_base_world_bundle`, which installs no `EconomyPlugin` and no markets), extracts a snapshot (records with `home_market == 0`), then `apply_into_world` spawns them into the LIVE world — which DOES have markets (`seed_from_markets_layer` ran at `sim-server/src/runtime/mod.rs:221`, before the `apply_into_world` at line 230). So assigning inside `seed_pedestrians_from_bundle` would never fire (no `Markets` there). Instead, assign inside the **single spawn function** `spawn_agent_from_record_with_position` when the record is still unassigned (`home_market == 0`) AND `Markets` is present. That one location covers: initial seed (via snapshot apply, records are 0 → assigned), restore (saved records carry real ids ≠ 0 → preserved, NOT recomputed from a since-moved position), and birth (child inherits mother's binding ≠ 0 → preserved).

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/market_binding.rs` (add `markets_with_positions` helper)
- Modify: `backend/crates/sim-core/src/mobility/api.rs` (assign-when-unassigned inside `spawn_agent_from_record_with_position`)
- Modify: `backend/crates/sim-core/src/population/mod.rs:158-247` (birth inheritance)

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

- [ ] **Step 2: Assign binding inside the spawn function when unassigned**

In `backend/crates/sim-core/src/mobility/api.rs`, inside `spawn_agent_from_record_with_position`, REPLACE Task 2's unconditional component values with a computed-when-unassigned version. Task 2 destructured `home_market`/`work_market` locals from `record`; after the spawn position `(px, py)` is known and BEFORE the `world.spawn((...))` tuple, recompute them when unassigned:

```rust
        // Assign the market binding from spawn position the first time only
        // (record carries 0 = unassigned at initial seed). On restore/birth the
        // record already carries real ids (>= 9001), which we PRESERVE — never
        // recompute from a since-moved position.
        let (home_market, work_market) = if home_market == 0 {
            let markets = crate::mobility::market_binding::markets_with_positions(world);
            crate::mobility::market_binding::assign_binding((px, py), &markets)
                .map(|b| (b.home_market, b.work_market))
                .unwrap_or((home_market, work_market))
        } else {
            (home_market, work_market)
        };
```

The bundle then inserts `crate::mobility::MarketBinding { home_market, work_market }` using these (possibly-reassigned) locals.

> Borrow discipline: read `markets_with_positions(world)` (immutable borrow, released before `world.spawn`). If `home_market`/`work_market` were destructured by value and `record` is partially moved later, capture them into `let mut` locals as needed so the recompute compiles. `markets_with_positions` returns empty when the economy isn't installed (the temp seed world) — there the binding stays the record's value (0), and gets assigned later when the snapshot is applied to the market-bearing live world.

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

- [ ] **Step 4: Add an inline assign-in-spawn test (`api.rs`)**

Add a `#[cfg(test)]` test in `api.rs` that builds a World with a routing `Graph` + a seeded `Markets` resource (REUSE the existing economy seed fixture — `economy/tests/seed.rs` builds a routed world and calls `seed_from_markets_layer`; or take `from_base_world_bundle(&bundle)` then `crate::economy::seed_from_markets_layer(&mut world, &bundle.markets)` so `Markets` + `Graph` both exist). Then assert TWO behaviors:

1. **Assign-when-unassigned:** spawn an `AgentRecord` with `home_market = 0` whose spawn position is near a known market anchor; assert the spawned entity's `MarketBinding.home_market` equals that nearest market id (and is `>= 9001`, i.e. ≠ 0).
2. **Preserve-when-assigned (restore safety):** spawn an `AgentRecord` with `home_market = 9003, work_market = 9004` whose spawn position is nearest a DIFFERENT market; assert the spawned `MarketBinding` is still `{9003, 9004}` (NOT recomputed).

Use the real fixture; do not invent a world builder. If positioning a spawn precisely near a chosen market is awkward through `AgentRecord` state, assert behavior 1 more loosely as "binding is assigned to some real market id (`>= 9001`)" and keep behavior 2 (the exact preserve assertion) strict — behavior 2 is the one that proves restore-safety.

> Note: `agent_record_from_entity` keeps its `unwrap_or(0)` from Task 2 — the `MarketBinding` component is always inserted for citizens spawned via this path, but raw-spawn test entities and temp-world (no-market) citizens legitimately carry/lack a binding with value `0`. `0` is a meaningful "unassigned" sentinel here, not a guard against an unreachable state, so it stays.

- [ ] **Step 5: Add an inline birth-inheritance test (`population/mod.rs`)**

Add at the end of `backend/crates/sim-core/src/population/mod.rs` a `#[cfg(test)]` test that: spawns one female citizen with `MarketBinding{home_market:9001, work_market:9002}`, advances `population_monthly_system` far enough to force one birth (reuse the existing population test fixture/harness in this file — search its current `#[cfg(test)]` module for the helper that drives a birth), then asserts the newborn entity (the one with `ParentId(Some(mother))`) carries the SAME binding. Use the existing harness; do not invent a new clock/world builder.

- [ ] **Step 6: Run the tests**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core market_binding -- --nocapture` (covers the assign-in-spawn test in api.rs and the helper)
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population -- --nocapture`
Expected: both PASS (spawn assigns when unassigned + preserves when assigned; newborn inherits mother's binding). Also keep the broader spawn/persist tests green: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core persist -- --nocapture`.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/mobility/api.rs backend/crates/sim-core/src/mobility/market_binding.rs backend/crates/sim-core/src/population/mod.rs
git commit -m "feat: assign MarketBinding in spawn (when unassigned) and inherit at birth"
```

---

### Task 4: Persist the binding + one-time mobility reset (snapshot round-trip + documented DELETE)

The binding is already in `AgentRecord` as REQUIRED fields (Task 2), and the AgentRecord-level required-field tests already exist (`agent_record_rejects_missing_market_binding`, plus round-trips, Task 2). `AgentRecord` derives `Serialize/Deserialize`, so bindings persist automatically. This task adds a **snapshot-level** round-trip test and documents the operational one-time reset.

**Verified persistence facts (do not re-derive):**
- The live server uses `PostgresMobilitySnapshotStore` (`sim-server/src/app/mod.rs:434`). Its load (`postgres_mobility.rs:199-224`) selects the row `WHERE world_id = $1 AND base_world_schema_version = $3`, then `serde_json::from_value::<MobilityPersistSnapshot>(payload)`, returning a `MobilitySnapshotStoreError::unavailable(...)` (an error, fail-fast) if the payload won't deserialize.
- The `SnapshotProvider::schema_version()`/`migrate()` trait (`world/persistence.rs`) is **NOT wired into the mobility postgres load path** — it is used only by tests and the economy/chunk provider abstraction. Therefore **bumping `MobilitySnapshotProvider::schema_version()` would be inert** and must NOT be done (it would be misleading dead config).
- Adding the required binding fields means pre-existing mobility snapshots (lacking the fields) will fail `from_value` on load → fail-fast `unavailable`.
- The codebase's clean reset for this (matching the established `DELETE FROM economy_snapshots` practice and the approved spec's "one-time mobility-state reset; existing dev saves lose accumulated ages/births") is a one-time **`DELETE FROM mobility_snapshots`** before deploying this slice. We do NOT add a serde-default shim, NOT a `migrate_legacy_agent_birth_ticks`-style SQL backfill (that is a heal-on-restore the project rule forbids), and NOT the inert provider schema bump.

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/persist_snapshot.rs` (add a snapshot-level round-trip test)
- Docs: deploy note (this plan + the PR) for the one-time `DELETE FROM mobility_snapshots`.

- [ ] **Step 1: (already verified above) — no code; the load gate is `base_world_schema_version` + `from_value`, and the reset is the documented DELETE. Proceed to the test.**

- [ ] **Step 2: Add a snapshot-level round-trip test (inline in `persist_snapshot.rs`)**

The AgentRecord-level required-field + round-trip tests already exist (Task 2: `agent_record_rejects_missing_market_binding`, `market_binding_round_trips_through_spawn_and_extract`). This adds the FULL snapshot cycle: a citizen with a real binding survives `extract_from_world` → serialize → `from_value` → `apply_into_world`.

Add a `#[cfg(test)]` test in `persist_snapshot.rs` that REUSES the existing persistence round-trip harness in `backend/crates/sim-core/src/tests/mobility_persistence_round_trip.rs` (or the `extract_from_world`/`apply_into_world` pattern used there). Concretely:
1. Build a market-bearing source world (seed routing + `seed_from_markets_layer` + spawn at least one citizen so it gets a real binding `>= 9001`), OR construct a `MobilityPersistSnapshot` whose `agents` map contains an `AgentRecord` with `home_market = 9003, work_market = 9004`.
2. `serde_json::to_value(&snap)` then `serde_json::from_value::<MobilityPersistSnapshot>(value)` and assert the agent record's `home_market`/`work_market` survive (proves payload-level persistence — the exact path `postgres_mobility.rs` uses).
3. Then `apply_into_world(&mut fresh_world, snap)` into a freshly-installed mobility world that ALSO has markets seeded, and assert the spawned citizen entity carries the SAME `MarketBinding` (proves hydrate preserves it, and the spawn's assign-when-0 does NOT clobber an already-assigned binding).
Use the real harness/fixtures; do not invent a world builder. If full `apply_into_world` setup is heavy, the `to_value`→`from_value` payload assertion (step 2) is the must-have; the `apply_into_world` assertion (step 3) is the strong form — include it if the existing harness makes it straightforward.

- [ ] **Step 3: Run the test + the existing persistence suite**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core persist -- --nocapture`
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core mobility_persistence -- --nocapture`
Expected: PASS, including the new snapshot round-trip and all existing persistence tests (which Task 2 already updated for the required fields). Keep fmt + scoped clippy clean.

- [ ] **Step 4: Document the one-time reset (no code shim)**

Do NOT bump `MobilitySnapshotProvider::schema_version()` (inert for the postgres load path — see verified facts above). Instead, add a deploy note to this plan's final section and to the PR body:

> **Deploy step (one-time):** before deploying this slice, run `DELETE FROM mobility_snapshots;` (per-world: `DELETE FROM mobility_snapshots WHERE world_id = '<id>';`). Existing mobility snapshots predate the required `home_market`/`work_market` fields and will fail to deserialize on load (`PostgresMobilitySnapshotStore` returns `unavailable`, fail-fast). This is the approved one-time mobility-state reset (existing dev saves lose accumulated ages/births). No serde-default shim, no SQL backfill, no schema-version bump.

(There is nothing to implement in this step beyond the documentation; the fail-fast behavior is the intended surfacing of the consequence.)

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/mobility/persist_snapshot.rs docs/superpowers/plans/2026-06-05-citizens-as-economy-bodies.md
git commit -m "test(persistence): snapshot-level MarketBinding round-trip; document one-time mobility-snapshots reset"
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

**BORROW DISCIPLINE (critical):** this is an exclusive `&mut World` system that needs BOTH immutable resource reads (`Graph`, `Markets`, `MarketGoods`, `WageTelemetry`, `EconomyConfig`) AND a `world.query_filtered` over citizen components (which needs `&mut World`) AND a final `world.resource_mut::<CitizenEconomicTargets>()`. These borrows CANNOT overlap. Structure it as sequential scopes, each releasing its borrow by cloning owned data out (the same shape the deleted capture systems used). Do NOT hold a `markets`/`graph` reference across the citizen query or the final write.

```rust
use bevy_ecs::world::World;

/// Exclusive system (EconomySet::Attribution). Reads realized consumption
/// (`MarketGoods.consumed_qty_last_tick`, valid after Consume) and wages
/// (`WageTelemetry`, valid after PayWages); restricts to observed markets (those
/// whose market node is in an Active/Hot chunk — identical test to the former
/// capture systems); selects the attributed cohort from observed, bound citizens;
/// and writes their economic target node into `CitizenEconomicTargets`. READ-ONLY
/// over economy state — mints and moves NO money (the `#78` audit is unaffected).
pub fn run_citizen_attribution_system(world: &mut World) {
    use crate::economy::{EconomyConfig, MarketGoods, Markets, WageTelemetry};
    use crate::mobility::resources::CitizenEconomicTargets;
    use crate::world::components::{ActiveChunk, ChunkCoord, ChunkCoordComp, HotChunk};
    use bevy_ecs::prelude::{Or, With};
    use std::collections::{BTreeMap, BTreeSet};

    // Resources may be absent in narrow tests; no-op then.
    if world.get_resource::<Markets>().is_none()
        || world.get_resource::<crate::routing::Graph>().is_none()
        || world.get_resource::<CitizenEconomicTargets>().is_none()
    {
        return;
    }

    // (1) Observed chunks — query borrow released after collect.
    let observed_chunks: BTreeSet<ChunkCoord> = {
        let mut q =
            world.query_filtered::<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>();
        q.iter(world).map(|c| c.0).collect()
    };

    // (2) Observed markets + target nodes + realized telemetry + config — all
    //     immutable resource borrows, cloned into owned locals before release.
    let (observed_markets, market_nodes, consumed_by_market, wage_by_market, config) = {
        let graph = world.resource::<crate::routing::Graph>();
        let markets = world.resource::<Markets>();
        let observed_markets: BTreeSet<u32> = markets
            .0
            .iter()
            .filter(|(_, site)| {
                let pos = graph.node(site.node_id).position;
                observed_chunks.contains(&crate::mobility::chunk_of(pos.0, pos.1, 32))
            })
            .map(|(id, _)| id.0)
            .collect();
        let market_nodes: BTreeMap<u32, crate::routing::NodeId> =
            markets.0.iter().map(|(id, site)| (id.0, site.node_id)).collect();
        let mut consumed_by_market: BTreeMap<u32, i64> = BTreeMap::new();
        for (key, st) in world.resource::<MarketGoods>().0.iter() {
            if observed_markets.contains(&key.market.0) {
                *consumed_by_market.entry(key.market.0).or_default() +=
                    st.consumed_qty_last_tick.0;
            }
        }
        let wage_by_market: BTreeMap<u32, i64> = world
            .resource::<WageTelemetry>()
            .0
            .iter()
            .filter(|(m, _)| observed_markets.contains(&m.0))
            .map(|(m, w)| (m.0, w.0))
            .collect();
        let config = *world.resource::<EconomyConfig>();
        (observed_markets, market_nodes, consumed_by_market, wage_by_market, config)
    };

    if observed_markets.is_empty() {
        world.resource_mut::<CitizenEconomicTargets>().0.clear();
        return;
    }

    // (3) Candidate citizens per market — query borrow released after collect.
    //     shop candidates ← home_market binding; work candidates ← work_market.
    let (shop_candidates, work_candidates) = {
        let mut shop: BTreeMap<u32, Vec<crate::ids::AgentId>> = BTreeMap::new();
        let mut work: BTreeMap<u32, Vec<crate::ids::AgentId>> = BTreeMap::new();
        let mut q = world.query_filtered::<(
            &crate::mobility::components::StableAgentId,
            &crate::mobility::MarketBinding,
        ), With<crate::mobility::components::AgentMarker>>();
        for (id, binding) in q.iter(world) {
            if observed_markets.contains(&binding.home_market) {
                shop.entry(binding.home_market).or_default().push(id.0.clone());
            }
            if observed_markets.contains(&binding.work_market) {
                work.entry(binding.work_market).or_default().push(id.0.clone());
            }
        }
        for v in shop.values_mut() {
            v.sort();
        }
        for v in work.values_mut() {
            v.sort();
        }
        (shop, work)
    };

    // (4) Compute targets — pure, no world borrow.
    let mut targets: BTreeMap<crate::ids::AgentId, crate::routing::NodeId> = BTreeMap::new();
    for (market_id, realized) in consumed_by_market {
        let Some(&node) = market_nodes.get(&market_id) else {
            continue;
        };
        let cands = shop_candidates.get(&market_id).cloned().unwrap_or_default();
        let res = attribute_channel(
            realized,
            config.shoppers_per_unit,
            config.max_shoppers_per_market,
            &cands,
        );
        for id in res.attributed {
            targets.insert(id, node);
        }
    }
    for (market_id, realized) in wage_by_market {
        let Some(&node) = market_nodes.get(&market_id) else {
            continue;
        };
        let cands = work_candidates.get(&market_id).cloned().unwrap_or_default();
        let res = attribute_channel(
            realized,
            config.commuters_per_wage_unit,
            config.max_commuters_per_market,
            &cands,
        );
        for id in res.attributed {
            // Shop leg wins ties (consumption attributed first): only fill if absent.
            targets.entry(id).or_insert(node);
        }
    }

    // (5) Write.
    world.resource_mut::<CitizenEconomicTargets>().0 = targets;
}
```

> If `ChunkCoord` / `ChunkCoordComp` / `ActiveChunk` / `HotChunk` live at a slightly different path, match the deleted capture system's imports (it used `crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk}` and `BTreeSet<ChunkCoord>`). `EconomyConfig` is `Copy` (the capture systems did `let config = *world.resource::<EconomyConfig>();`). Confirm field types: `shoppers_per_unit: i64`, `max_shoppers_per_market: usize`, `commuters_per_wage_unit: i64`, `max_commuters_per_market: usize` — they line up with `attribute_channel(realized: i64, per_unit: i64, cap: usize, ...)`.

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
