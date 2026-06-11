use bevy_ecs::prelude::*;

use crate::economy::production::{
    EXTRACTOR_FOOD_A, PRODUCER_TOOLS, ProductionPool, ProductionPools, RawDeposit, RawDeposits,
    Recipe,
};
use crate::economy::systems::EconomyConfig;
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyEvent, EconomyPlugin, GOOD_FOOD,
    GOOD_RAW, GOOD_TOOLS, HouseholdSector, MarketGoodKey, MarketGoodState, MarketGoods, MarketId,
    MarketSite, Markets, Money, Quantity, SupplyPool, SupplyPools, TradeLedger,
};
use crate::ids::AgentId;
use crate::mobility::MarketBinding;
use crate::mobility::components::{AgentMarker, StableAgentId};
use crate::mobility::resources::CitizenEconomicTargets;
use crate::routing::{Graph, Node, NodeId, NodeKind};
use crate::world::components::{ActiveChunk, ChunkCoordComp};
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;
use std::collections::BTreeMap;

/// Seeds the canonical two-extractor economy used by every harness in this module:
/// the TOOLS and FOOD_A extractors (raw deposit → recipe → market supply), two
/// household demand pools (one per finished good), the single `MarketId(1)` anchored
/// at `NodeId(0)`, the `HouseholdSector`, and mid-band seeded prices for both goods.
///
/// `opening_cash` is deposited into BOTH consumer accounts; `desired_qty_per_tick`
/// sets both demand pools' target. Returns `(tools_consumer, food_consumer)`.
fn seed_two_extractor_economy(
    world: &mut World,
    opening_cash: Money,
    desired_qty_per_tick: Quantity,
) -> (EconomicActorId, EconomicActorId) {
    let consumer = EconomicActorId(8_002);
    let food_consumer = EconomicActorId(8_012);
    let market = MarketId(1);

    // TOOLS chain.
    world.resource_mut::<RawDeposits>().0.insert(
        PRODUCER_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        PRODUCER_TOOLS,
        ProductionPool {
            actor: PRODUCER_TOOLS,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(10))],
                outputs: vec![(GOOD_TOOLS, Quantity(10))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        PRODUCER_TOOLS,
        SupplyPool {
            actor: PRODUCER_TOOLS,
            market,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, opening_cash)
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer,
            market,
            good: GOOD_TOOLS,
            desired_qty_per_tick,
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
        },
    );

    // FOOD chain.
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_FOOD_A,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_FOOD_A,
        ProductionPool {
            actor: EXTRACTOR_FOOD_A,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(10))],
                outputs: vec![(GOOD_FOOD, Quantity(10))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR_FOOD_A,
        SupplyPool {
            actor: EXTRACTOR_FOOD_A,
            market,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<AccountBook>()
        .deposit(food_consumer, opening_cash)
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        food_consumer,
        DemandPool {
            actor: food_consumer,
            market,
            good: GOOD_FOOD,
            desired_qty_per_tick,
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
        },
    );

    // Market + household.
    world.resource_mut::<Markets>().0.insert(
        market,
        MarketSite {
            id: market,
            node_id: crate::routing::NodeId(0),
            name: "M1".to_string(),
        },
    );
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1_i64), (food_consumer, 1_i64)]),
    });

    // Seed mid-band prices for both goods.
    for good in [GOOD_TOOLS, GOOD_FOOD] {
        let key = MarketGoodKey { market, good };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }

    (consumer, food_consumer)
}

/// Build the same minimal two-extractor economy used in `conservation_full_plugin_multi_tick`,
/// deriving the requested `CapitaFactor` value from live citizens (2c: `refresh_capita_factor_system`
/// now owns the factor). `capita_baseline=1` is used so that spawning exactly `factor` AgentMarker
/// entities yields `floor(factor/1) = factor`. Asserts `total_money` is byte-invariant throughout.
fn run_conservation_with_factor(factor: i64, n: u64) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    // Drive CapitaFactor via citizens: capita_baseline=1 → factor = live count.
    world.insert_resource(EconomyConfig {
        capita_baseline: 1,
        ..EconomyConfig::default()
    });
    for _ in 0..factor {
        world.spawn(AgentMarker);
    }

    let (_consumer, _food_consumer) =
        seed_two_extractor_economy(&mut world, Money(10_000_000), Quantity(10));

    let money_before = world.resource::<AccountBook>().total_money().unwrap();

    for i in 0..n {
        // MobilityPlugin's tick_increment_system advances Tick inside the schedule;
        // a manual increment here would double the stride and halve every
        // interval-gated cadence (see tests/economy_production_chain.rs run_tick).
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant at tick {i} with CapitaFactor({factor})"
        );
    }

    // Non-vacuity: at least some economy events must have fired.
    assert!(
        world
            .resource::<TradeLedger>()
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::Regenerated { .. })),
        "goods flowed (CapitaFactor({factor}))"
    );
}

#[test]
fn capita_factor_1_conserves_total_money() {
    run_conservation_with_factor(1, 20);
}

#[test]
fn capita_factor_2_conserves_total_money() {
    run_conservation_with_factor(2, 20);
}

#[test]
fn capita_factor_10_conserves_total_money() {
    run_conservation_with_factor(10, 20);
}

/// Run the full economy schedule for `n` ticks with the given `factor` and a FIXED
/// opening_cash of `Money(1_000_000)` (the realistic seeded amount). Returns:
/// `(total_final_consumed, total_trade_events)` across the run.
///
/// Since 2c, `CapitaFactor` is derived by `refresh_capita_factor_system` from the
/// live citizen count. `capita_baseline=1` is used so that spawning exactly `factor`
/// AgentMarker entities yields `floor(factor/1) = factor`.
fn run_solvency_scenario(factor: i64, n: u64) -> (i64, usize) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    // Drive CapitaFactor via citizens: capita_baseline=1 → factor = live count.
    world.insert_resource(EconomyConfig {
        capita_baseline: 1,
        ..EconomyConfig::default()
    });
    for _ in 0..factor {
        world.spawn(AgentMarker);
    }

    // Realistic opening cash: 1_000_000 (unchanged from seed — the solvency question).
    let (_consumer, _food_consumer) =
        seed_two_extractor_economy(&mut world, Money(1_000_000), Quantity(0));

    let money_before = world.resource::<AccountBook>().total_money().unwrap();

    for i in 0..n {
        // tick_increment_system advances Tick inside the schedule — no manual
        // increment (it would double the stride; see run_tick in
        // tests/economy_production_chain.rs).
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant at tick {i} with CapitaFactor({factor})"
        );
    }

    let ledger = world.resource::<TradeLedger>();
    let total_final_consumed: i64 = ledger
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::FinalConsumed { qty, .. } => Some(qty.0),
            _ => None,
        })
        .sum();
    let total_trade_events = ledger
        .0
        .iter()
        .filter(|e| matches!(e, EconomyEvent::Trade { .. }))
        .count();

    (total_final_consumed, total_trade_events)
}

/// Solvency + throughput scaling at factor 30 (~300 citizens / baseline 10).
///
/// After production-chain scaling (regen + recipe both scale by capita_factor), the
/// extractor faucet deposits 300 RAW/tick at factor 30 (vs 10 at factor 1), the recipe
/// consumes 300 RAW→300 goods/tick, and the supply offer places 300 units/tick on the
/// market. Throughput therefore ACTUALLY scales ~30×.
///
/// (a) audit stays byte-invariant every tick — asserted inside run_solvency_scenario
///     via .expect in run_tick_audit_system (a conservation violation would panic).
/// (b) FinalConsumed and Trade events keep firing at factor 30 (loop not starved).
/// (c) THROUGHPUT SCALES: consumed_30 >= consumed_1 * 10 (robust ~30× assertion).
///
/// SOLVENCY VERDICT: SOLVENT at factor 30 / opening_cash=1_000_000.
/// Cash circulates via wages → no InsufficientFunds collapse at 30× throughput.
/// Seed scaling unnecessary at factor 30 — no economy_snapshots migration.
#[test]
fn factor_30_solvency_at_fixed_opening_cash() {
    let (consumed_30, trades_30) = run_solvency_scenario(30, 50);
    let (consumed_1, trades_1) = run_solvency_scenario(1, 50);

    // (b) Demand must not have collapsed: both Trade and FinalConsumed must have fired.
    assert!(
        trades_30 > 0,
        "factor-30: Trade events must have fired (demand not starved); got trades_30={trades_30}"
    );
    assert!(
        consumed_30 > 0,
        "factor-30: FinalConsumed events must have fired; got consumed_30={consumed_30}"
    );

    // (c) Throughput must ACTUALLY SCALE: production chain (regen + recipe) both scale by
    // capita_factor, so factor-30 should clear ~30× as many goods as factor-1.
    // Assert >= 10× (robust lower bound, tolerant of warm-up and round-off).
    assert!(
        consumed_30 >= consumed_1 * 10,
        "factor-30 throughput must be materially larger than factor-1 (~30×, assert >=10×): \
         consumed_30={consumed_30}, consumed_1={consumed_1}, \
         trades_30={trades_30}, trades_1={trades_1}"
    );

    // Emit diagnostic so the throughput evidence and solvency verdict are readable.
    println!(
        "THROUGHPUT SCALING VERIFIED factor=30 / opening_cash=1_000_000: \
         consumed_30={consumed_30} consumed_1={consumed_1} \
         (ratio={:.1}×) trades_30={trades_30} trades_1={trades_1}; \
         SOLVENCY VERDICT: SOLVENT — seed scaling unnecessary at factor 30 — \
         no economy_snapshots migration.",
        if consumed_1 > 0 {
            consumed_30 as f64 / consumed_1 as f64
        } else {
            0.0
        }
    );
}

// ── Slice 2b density + safety harness ────────────────────────────────────────

/// Report returned by `run_density_scenario`.
struct DensityReport {
    /// Maximum `CitizenEconomicTargets.0.len()` observed across all ticks.
    max_routed: usize,
    /// Sum of `FinalConsumed` qty over the FIRST half of ticks.  Used by the safety
    /// test to prove the second half did not decay relative to warm-up.
    final_consumed_first_half: i64,
    /// Sum of `FinalConsumed` qty over the SECOND half of ticks.
    final_consumed_second_half: i64,
    /// Total count of `EconomyEvent::Trade` events over the full run.
    total_trades: usize,
}

/// Full economy harness with ATTRIBUTION wiring: a real `Graph`, an `ActiveChunk`,
/// and citizens spawned with `(AgentMarker, StableAgentId, MarketBinding)`.  Unlike
/// `run_solvency_scenario`, the attribution path is live — the Graph contains the
/// market's `NodeId(0)`, the chunk entity marks the market's chunk as Active, and
/// every citizen is bound to `MarketId(1)`.  This lets `run_citizen_attribution_system`
/// actually route citizens, so `CitizenEconomicTargets` fills up every tick.
///
/// `capita_baseline` drives the factor: 300 citizens / 10 baseline → factor 30;
/// 300 citizens / 1_000_000 baseline → factor 1 (identity).
fn run_density_scenario(capita_baseline: i64, n_citizens: u64, n_ticks: u64) -> DensityReport {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    // ── Config: drive factor via capita_baseline ───────────────────────────
    world.insert_resource(EconomyConfig {
        capita_baseline,
        ..EconomyConfig::default()
    });

    // ── Citizens: AgentMarker + StableAgentId + MarketBinding ─────────────
    // These three components wire the attribution path (run_citizen_attribution_system
    // queries `With<AgentMarker>` filtering on `(StableAgentId, MarketBinding)`).
    // The bare AgentMarker also feeds `refresh_capita_factor_system` (counts live citizens).
    for i in 0..n_citizens {
        world.spawn((
            AgentMarker,
            StableAgentId(AgentId(format!("agent:walk:{i}"))),
            MarketBinding {
                home_market: 1,
                work_market: 1,
            },
        ));
    }

    // ── Economy setup: two extractors (shared helper) ───────
    let (_consumer, _food_consumer) =
        seed_two_extractor_economy(&mut world, Money(10_000_000), Quantity(10));

    // ── Attribution wiring: real Graph + ActiveChunk ───────────────────────
    // The market's NodeId(0) is at position (1.0, 1.0), which maps to chunk (0,0)
    // via chunk_of(1, 1, 32) = ChunkCoord { x:0, y:0 }.
    // Override the empty default Graph installed by the plugins with one that
    // contains NodeId(0) so the attribution bounds guard passes.
    world.insert_resource(Graph::new(
        vec![Node {
            id: NodeId(0),
            position: (1.0, 1.0),
            kind: NodeKind::Intersection,
            legacy_id: None,
        }],
        vec![],
    ));
    // Mark chunk (0,0) Active so run_citizen_attribution_system considers it observed.
    world.spawn((
        ChunkCoordComp(crate::ids::ChunkCoord { x: 0, y: 0 }),
        ActiveChunk,
    ));

    // ── Per-tick loop ──────────────────────────────────────────────────────
    let money_before = world.resource::<AccountBook>().total_money().unwrap();
    let mid = n_ticks / 2;
    let config = *world.resource::<EconomyConfig>();

    let mut max_routed: usize = 0;
    let mut mid_consumed: i64 = 0;

    for i in 0..n_ticks {
        // tick_increment_system advances Tick inside the schedule — no manual
        // increment (it would double the stride; see run_tick in
        // tests/economy_production_chain.rs).
        schedule.run(&mut world);

        // (a) Money-conservation: byte-invariant every tick.
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant at tick {i} (capita_baseline={capita_baseline})"
        );

        // (b) Price-band: every MarketGoodState must stay within [price_floor, price_ceiling].
        for (key, st) in world.resource::<MarketGoods>().0.iter() {
            assert!(
                st.ewma_reference_price >= config.price_floor
                    && st.ewma_reference_price <= config.price_ceiling,
                "price out of band at tick {i}: market={:?} good={:?} price={:?} \
                 (floor={:?} ceil={:?})",
                key.market,
                key.good,
                st.ewma_reference_price,
                config.price_floor,
                config.price_ceiling,
            );
        }

        // (c) Record routed cohort size.
        let routed = world.resource::<CitizenEconomicTargets>().0.len();
        if routed > max_routed {
            max_routed = routed;
        }

        // (d) Snapshot mid-point FinalConsumed sum.
        // i == mid-1 → after `mid` schedule runs: captures FinalConsumed over the first half.
        if i + 1 == mid {
            mid_consumed = world
                .resource::<TradeLedger>()
                .0
                .iter()
                .filter_map(|e| match e {
                    EconomyEvent::FinalConsumed { qty, .. } => Some(qty.0),
                    _ => None,
                })
                .sum();
        }
    }

    // ── Final ledger tallies ───────────────────────────────────────────────
    let ledger = world.resource::<TradeLedger>();
    let end_consumed: i64 = ledger
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::FinalConsumed { qty, .. } => Some(qty.0),
            _ => None,
        })
        .sum();
    let total_trades = ledger
        .0
        .iter()
        .filter(|e| matches!(e, EconomyEvent::Trade { .. }))
        .count();

    DensityReport {
        max_routed,
        final_consumed_first_half: mid_consumed,
        final_consumed_second_half: end_consumed - mid_consumed,
        total_trades,
    }
}

/// Slice 2b density gate: a ramped `capita_baseline` (10 → factor 30 with 300 citizens)
/// routes materially MORE citizens than identity (baseline 1_000_000 → factor 1).
///
/// With `max_shoppers_per_market=4` and `CapitaFactor(30)` the shop cap is 4×30=120.
/// With `CapitaFactor(1)` the cap is 4×1=4. At factor 30 the attribution cohort is
/// therefore ~30× larger (bounded by the scaled cap of 120, with the 300
/// candidates as headroom).
#[test]
fn ramped_capita_is_denser_than_identity() {
    let ramped = run_density_scenario(10, 300, 60); // factor = floor(300/10) = 30
    let identity = run_density_scenario(1_000_000, 300, 60); // factor = floor(300/1_000_000) → 1

    println!(
        "DENSITY: ramped max_routed={} identity max_routed={}",
        ramped.max_routed, identity.max_routed
    );

    assert!(
        ramped.max_routed > identity.max_routed.max(1) * 10,
        "ramped capita (factor 30, cap 120) must route >10× as many citizens as identity \
         (factor 1, cap 4): ramped.max_routed={} identity.max_routed={}",
        ramped.max_routed,
        identity.max_routed,
    );
}

/// Slice 2b safety gate: a 60-tick run at factor 30 must remain solvent, price-stable,
/// and active (no demand collapse after warm-up).
///
/// The per-tick asserts INSIDE `run_density_scenario` are the primary safety checks —
/// any conservation violation or out-of-band price panics immediately.  This test
/// adds post-hoc non-vacuity: trades fired, and second-half consumption is positive
/// (the economy is still active past warm-up, not starved into all-InsufficientFunds).
#[test]
fn ramped_capita_stays_safe_over_long_run() {
    let report = run_density_scenario(10, 300, 60); // factor 30

    println!(
        "SAFETY: total_trades={} second_half_consumed={}",
        report.total_trades, report.final_consumed_second_half
    );

    assert!(
        report.total_trades > 0,
        "factor-30: Trade events must have fired (demand not starved)"
    );
    // Substantial churn, not a trickle: the second half must clear well above a
    // token amount (measured steady-state is ~18_000 over 30 ticks → ~600/tick).
    assert!(
        report.final_consumed_second_half > 1_000,
        "factor-30: second-half consumption must be substantial (no near-collapse): \
         second_half={}",
        report.final_consumed_second_half
    );
    // No decay relative to warm-up: the second half must be at least half the first
    // half. Guards against a slow starvation tail that a bare `> 0` would miss.
    assert!(
        report.final_consumed_second_half * 2 >= report.final_consumed_first_half,
        "factor-30: second half must not have decayed vs warm-up: \
         first_half={} second_half={}",
        report.final_consumed_first_half,
        report.final_consumed_second_half
    );
}
