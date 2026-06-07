# Abutopia Live-Visible (Blockers 2+3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the abutopia economy↔demographics merge visible live (citizens shop at market 9002) by flipping the persistence pooler config + fresh-seeding, and ship a deterministic long-run pricing-stability test that proves (or disproves) market 9002 stays healthy.

**Architecture:** One shippable code artifact — a characterization test that runs the real abutopia economy ~2000 ticks and asserts money-conservation, in-band prices, and a sustained/non-pinned market 9002. Everything else is operational: a `.env` pooler-port flip (the SOTA spec's open operator item) + a one-time DB fresh seed + a live browser-smoke. No new gate code; no ad-hoc pricing code.

**Tech Stack:** Rust (bevy_ecs), `sim-core`. Cargo via `scripts/cargo-serial.sh` only (never two at once). Determinism: BTreeMap/sorted, no HashMap-iteration/RNG/wall-clock.

**Worktree:** `/Users/ramonfuglister/Coding/abutown-live`, branch `feat/abutopia-live-visible` (off `origin/main` `53cd2e3`). Spec: `docs/superpowers/specs/2026-06-07-abutopia-live-visible-design.md`.

---

## Background the implementer needs

- **The test is a *characterization* test, not red-green TDD.** It asserts the EXPECTED healthy behavior of market 9002. Expected outcome: PASS (9002 is supplied by 9001, net_gain ≈ +650/unit → it consumes and stays in-band). If it PASSES → commit it (it is both the evidence that there's no bug AND a regression guard). If it FAILS (9002's consumption collapses to 0 and/or its price pins at the ceiling) → that contradicts the free-prices spec's stability guarantee (Test #10) = a genuine bug → **STOP, do not commit a weakened test, report BLOCKED with the failure evidence** (it becomes a separate recovery-fix slice).
- **Why 9002 should be healthy:** the macro flow (`macro_flow_interval_ticks = 10`) ships goods 9001→9002 over the declared `9001↔9002` distance edge; `net_gain = max_price(2000) − min_price(500) − 5×dist(≈171) ≈ +650 > 0` → goods delivered → `consumed_qty_last_tick > 0` (consumption is inventory-driven) → unmet≈0 → no upward price nudge → price stable near its delivered level (~1855), far from the 100_000 ceiling.
- **Key symbols** (verified in `economy/tests/seed.rs` + `economy/tests/capita.rs`):
  - `node(id,x,y)` (in seed.rs) → `crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex}`.
  - Build a runnable abutopia economy = the `seed_world()` recipe (4-node graph at market anchors incl. `node(1, 111.5, 64.51)` for 9002 + `seed_from_markets_layer(&bundle.markets)`) **plus** keeping a `Schedule` and installing `CorePlugin` + `MobilityPlugin` so the schedule runs and `Tick` advances (the `capita.rs` run pattern). `seed_world()` itself only installs `EconomyPlugin` and discards the schedule, so it cannot be reused directly.
  - Tick loop: `schedule.run(&mut world); world.resource_mut::<crate::mobility::resources::Tick>().0 += 1;`
  - `world.resource::<crate::economy::AccountBook>().total_money().unwrap()` (byte-invariant).
  - `world.resource::<crate::economy::MarketGoods>().0` → `BTreeMap<MarketGoodKey, MarketGoodState>`; `MarketGoodKey { market: MarketId, good: GoodId }`; `MarketGoodState.ewma_reference_price: Money`, `.consumed_qty_last_tick: Quantity`.
  - `EconomyConfig.price_floor`, `.price_ceiling` (defaults `Money(1)`, `Money(100_000)`).
  - 9002 = `MarketId(9002)`; its demand goods = `GOOD_TOOLS` (4) + `GOOD_FOOD` (1).
  - No citizens are spawned → `CapitaFactor = 1`; pricing dynamics are scale-invariant (intensity is a ratio), so factor 1 is a faithful pricing test.

## File Structure

- **Create** `backend/crates/sim-core/src/economy/tests/abutopia_price_stability.rs` — the one test.
- **Modify** `backend/crates/sim-core/src/economy/tests/mod.rs` — register `mod abutopia_price_stability;`.

---

### Task 1: Long-run pricing-stability characterization test (the shippable artifact)

**Files:**
- Create: `backend/crates/sim-core/src/economy/tests/abutopia_price_stability.rs`
- Modify: `backend/crates/sim-core/src/economy/tests/mod.rs`

- [ ] **Step 1: Write the test**

Create `backend/crates/sim-core/src/economy/tests/abutopia_price_stability.rs`:

```rust
//! Blocker-2 evidence: run the REAL abutopia economy long enough to see whether the
//! free-price tâtonnement keeps the SUPPLIED demand market 9002 healthy (consuming,
//! price in-band) or lets it collapse to the ceiling. Extends the free-prices spec's
//! stability Test #10 to a long-run abutopia scenario. If 9002 collapses, that
//! contradicts the spec's stability guarantee and is a genuine bug (escalate).

use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, EconomyConfig, EconomyPlugin, GOOD_FOOD, GOOD_TOOLS, MarketGoods, MarketId,
};
use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};
use crate::world::plugin::CorePlugin;

fn node(id: u32, x: f32, y: f32) -> Node {
    Node { id: NodeId(id), position: (x, y), kind: NodeKind::Intersection, legacy_id: None }
}

/// Build the real abutopia economy with a RUNNABLE schedule (seed_world recipe + the
/// capita run pattern: CorePlugin + MobilityPlugin + EconomyPlugin so the schedule
/// advances and Tick exists). 4-node graph at the market anchors (9002 @ 111.5,64.51).
fn build_abutopia_economy() -> (World, bevy_ecs::schedule::Schedule) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let nodes = vec![
        node(0, 2.0, 3.0),
        node(1, 111.5, 64.51),
        node(2, 16.0, 48.0),
        node(3, 208.0, 48.0),
    ];
    world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
    world.insert_resource(Graph::new(nodes, vec![]));

    let bundle = crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
        .expect("abutopia bundle loads");
    crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);
    (world, schedule)
}

/// Sum market 9002's consumed_qty_last_tick across its two demand goods.
fn consumed_9002(world: &World) -> i64 {
    let goods = world.resource::<MarketGoods>();
    let mut total = 0i64;
    for g in [GOOD_TOOLS, GOOD_FOOD] {
        if let Some(st) = goods.0.get(&crate::economy::MarketGoodKey { market: MarketId(9002), good: g }) {
            total += st.consumed_qty_last_tick.0;
        }
    }
    total
}

/// Max ewma_reference_price across 9002's demand goods (the divergence signal).
fn price_9002(world: &World) -> i64 {
    let goods = world.resource::<MarketGoods>();
    let mut max = 0i64;
    for g in [GOOD_TOOLS, GOOD_FOOD] {
        if let Some(st) = goods.0.get(&crate::economy::MarketGoodKey { market: MarketId(9002), good: g }) {
            max = max.max(st.ewma_reference_price.0);
        }
    }
    max
}

#[test]
fn abutopia_prices_stay_in_band_and_9002_consumes_over_long_run() {
    const N: u64 = 2000; // 200 tâtonnement cadences (macro_flow_interval_ticks = 10)
    let (mut world, mut schedule) = build_abutopia_economy();

    let money_before = world.resource::<AccountBook>().total_money().unwrap();
    let config = *world.resource::<EconomyConfig>();

    let mut consumed_first_half = 0i64;
    let mut consumed_last_quarter = 0i64;
    let mut peak_price_9002 = 0i64;

    for i in 0..N {
        schedule.run(&mut world);
        world.resource_mut::<crate::mobility::resources::Tick>().0 += 1;

        // (a) Money-conservation byte-invariant every tick.
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant at tick {i}"
        );

        // (b) ALL prices stay within [floor, ceiling] every tick.
        for (key, st) in world.resource::<MarketGoods>().0.iter() {
            assert!(
                st.ewma_reference_price >= config.price_floor
                    && st.ewma_reference_price <= config.price_ceiling,
                "price out of band at tick {i}: {:?} {:?} price={:?}",
                key.market, key.good, st.ewma_reference_price
            );
        }

        let c = consumed_9002(&world);
        let p = price_9002(&world);
        peak_price_9002 = peak_price_9002.max(p);
        if i < N / 2 { consumed_first_half += c; }
        if i >= N - N / 4 { consumed_last_quarter += c; }
    }

    let final_price_9002 = price_9002(&world);
    println!(
        "ABUTOPIA STABILITY: consumed_first_half={consumed_first_half} \
         consumed_last_quarter={consumed_last_quarter} peak_price_9002={peak_price_9002} \
         final_price_9002={final_price_9002} ceiling={}",
        config.price_ceiling.0
    );

    // (c) 9002 keeps consuming in the last quarter (NOT collapsed to zero).
    assert!(
        consumed_last_quarter > 0,
        "market 9002 must keep consuming over the long run (no collapse); \
         consumed_last_quarter={consumed_last_quarter}, consumed_first_half={consumed_first_half}"
    );

    // (d) 9002's price does NOT ratchet toward the ceiling. A supplied market settles
    //     near its delivered level (~1855); a ceiling-bound ratchet would be far higher.
    //     Bound at ceiling/10 (10_000) — comfortably above the healthy equilibrium,
    //     far below the 100_000 ceiling; tune from the printed actuals if needed but
    //     keep it a real collapse-detector (must stay << ceiling).
    assert!(
        peak_price_9002 < config.price_ceiling.0 / 10,
        "market 9002 price must not ratchet toward the ceiling (no divergence); \
         peak_price_9002={peak_price_9002}, ceiling={}",
        config.price_ceiling.0
    );
}
```

Register it: add `mod abutopia_price_stability;` to `backend/crates/sim-core/src/economy/tests/mod.rs` (match the existing `mod <name>;` lines).

- [ ] **Step 2: Run it and read the result**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core abutopia_price_stability -- --nocapture`

Read the `ABUTOPIA STABILITY:` line and the outcome:
- **PASS** → expected (9002 supplied + stable). The test is the evidence (no bug) + a regression guard. Proceed to Step 3.
- **Compile errors** → fix imports/symbols against the real defs (grep `pub const GOOD_TOOLS`, `pub fn total_money`, `struct MarketGoodState`, `MarketGoodKey`, `pub struct Tick`); the structure is correct.
- **FAIL on (c) or (d)** (9002 collapses / ratchets) → **DO NOT weaken the assertion to make it pass.** This is the genuine-bug branch: report `BLOCKED` with the printed actuals (consumed_last_quarter, peak/final price). It contradicts the free-prices spec's stability guarantee and becomes a separate recovery-fix slice (its own brainstorm/spec). Stop here.
- **FAIL on (a)/(b)** (money or all-prices-in-band) → a deeper invariant break; report `BLOCKED` with details.
- If a threshold in (c)/(d) is *borderline but clearly healthy* (e.g. peak price ~1900, well under 10_000), the bound is fine; only adjust a threshold if the printed actuals show it's healthy yet just over an arbitrarily-tight bound — and never into vacuity.

- [ ] **Step 3: fmt + clippy**

- `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all` then `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check; echo "FMT_EXIT=$?"` → 0.
- `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-core --all-targets -- -D warnings 2>&1 | tail -5; echo "done"` → clean.

- [ ] **Step 4: Commit (only if Step 2 PASSED)**

```bash
git add backend/crates/sim-core/src/economy/tests/abutopia_price_stability.rs \
        backend/crates/sim-core/src/economy/tests/mod.rs
git commit -m "test(economy): long-run abutopia price-stability evidence (9002 stays in-band + consuming)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Gate config + fresh seed (controller-run, operational)

**Files:** none committed (the `.env` is gitignored). Run by the controller, with the user (fresh-seed authorized).

- [ ] **Step 1: Flip `.env` `DATABASE_URL` to the `:6543` transaction pooler**

Edit `/Users/ramonfuglister/Coding/abutown/.env`: change the active `DATABASE_URL`'s port from `:5432` (session pooler) to `:6543` (transaction pooler), same host/user/password/query. This is the SOTA spec's documented open operator item; `statement_cache_capacity(0)` (already in `db.rs`) makes `:6543` safe.

- [ ] **Step 2: Fresh-seed the abutopia world**

With `DATABASE_URL` exported from `.env`, run the two DELETEs against the remote DB (psql):
```bash
psql "$DATABASE_URL" -c "DELETE FROM economy_snapshots;" -c "DELETE FROM mobility_snapshots;"
```
(Scope is the single-world abutopia DB; these tables are one-row-per-world.)

- [ ] **Step 3: Restart the backend on the fresh-seeded world + confirm Healthy**

Start the stack from the `abutown-live` worktree (symlink `node_modules` if needed). Poll `GET /health`: expect `ok=true`, persistence status `healthy` (not `stale`), `world_id=abutopia`, mobility tick advancing. Confirm via the backend log: no panic, no persistence-stale, `economy::liveness` heartbeat appears.

---

### Task 3: Live demo — browser-smoke + screenshot (controller-run)

**Files:** none.

- [ ] **Step 1: Drive the headless browser at the frontend**

With backend Healthy on `:6543`, start the frontend, navigate a headless chromium to it, wait for the canvas to render (no "persistence stale" overlay — proving Blocker-3 resolved), pan/ensure the residential corridor chunk (3,2) is in view (where 9002 + the 300 pedestrians are).

- [ ] **Step 2: Capture evidence**

Screenshot the city. Read the backend log's `economy::liveness` routed count (expect `routed > 0` now that 9002 consumes + is observed). Present the screenshot + routed count to the user for the "looks alive" judgment.

---

### Task 4: Ship the test (gate + PR)

**Files:** none beyond Task 1.

- [ ] **Step 1: Full CI gate**
- Rust: `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check` (0); `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings` (clean); `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace` (all pass incl. the new test).
- Frontend: `npm run typecheck` (clean), `npm test` (vitest pass), `npm run build` (ok). (No frontend change here, but the gate requires it.)
- e2e: `CORS_ALLOWED_ORIGINS="http://127.0.0.1:5173" npm run test:e2e` → render-smoke 2/2 (no code change to the wire; confirms no regression).

- [ ] **Step 2: PR via finishing-a-development-branch**

Push + open a PR against `main`. The PR ships ONLY the pricing-stability test. Body must state: the gate is resolved operationally (`.env` `:6543` + fresh seed, the SOTA spec's open item — no code), the test is the in-repo evidence that 9002 stays healthy (or, if it failed, that finding triggers a separate recovery slice), and the live demo result. Wait for CI green, squash-merge, clean up.

---

## Self-Review

**1. Spec coverage:** Gate config (`.env` :6543) → Task 2 Step 1 (spec §1). Fresh seed → Task 2 Step 2 (spec §2). Pricing-stability evidence test → Task 1 (spec §3, with the PASS/FAIL branch matching the spec's evidence-first decision). Live demo → Task 3 (spec §4). Ship + gate → Task 4. All spec acceptance criteria covered.

**2. Placeholder scan:** No TBD/placeholders; Task 1 has complete test code + exact commands. Tasks 2–4 are operational runbooks with exact commands.

**3. Type consistency:** `build_abutopia_economy`, `consumed_9002`, `price_9002`, `node` are consistent across the test; symbol paths (`AccountBook::total_money`, `MarketGoods.0`, `MarketGoodKey`, `MarketGoodState.{ewma_reference_price,consumed_qty_last_tick}`, `EconomyConfig.price_{floor,ceiling}`, `Tick`, `GOOD_TOOLS/GOOD_FOOD`, `MarketId(9002)`) match the verified `seed.rs`/`capita.rs` usage. `9002`/`111.5,64.51` consistent with Blocker-1.
