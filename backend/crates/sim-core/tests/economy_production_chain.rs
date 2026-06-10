//! Task 6 — production-chain integration tests on the REAL abutopia seed.
//!
//! The chain under test: extractor 8041 faucets RAW→WOOD at market 9003, the
//! WOOD travels 9003→9001 via the macro flow (baked distance pair), producer
//! 8031 buys it through its Leontief `InputPool` (participation-bound bid),
//! converts WOOD→TOOLS, pays value-added wages, and distributes a θ-dividend
//! above its capita-scaled working-capital target. Everything runs at the
//! live-world per-capita scale: 300 citizens / authored `capita_baseline = 10`
//! → factor 30 (seed cash ×30, order targets ×30, wc_target ×30).
//!
//! Spec: docs/superpowers/specs/2026-06-10-economy-production-chains-design.md
//! (§8 test plan: conservation, hydrate-path resume, long-run stationarity).

use sim_core::base_world::BaseWorldBundle;
use sim_core::economy::production::PRODUCER_TOOLS;
use sim_core::economy::{
    AccountBook, EconomyConfig, EconomyEvent, GOOD_TOOLS, GOOD_WOOD, HOUSEHOLD_SECTOR, InputPools,
    InventoryBook, MarketDistances, MarketGoodKey, MarketGoods, MarketId, Markets, Money,
    ProducerPolicies, TradeLedger, apply_into_world, extract_from_world, participation_bound,
    seed_from_markets_layer, wc_target,
};
use sim_core::mobility::components::AgentMarker;
use sim_core::mobility::resources::Tick;
use sim_core::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};
use sim_core::world::plugin::CorePlugin;
use sim_core::world::schedule::SimPlugin;

/// The live abutopia scale: the authored pedestrian seed is 300 citizens and
/// markets.json authors `capita_baseline = 10`, so the runtime capita factor —
/// and the seed-time mint/inventory scale — is 30.
const CITIZENS: usize = 300;
const EXPECTED_CAPITA_FACTOR: i64 = 30;

fn abutopia_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("workspace root")
        .join("data/worlds/abutopia")
}

fn node(id: u32, x: f32, y: f32) -> Node {
    Node {
        id: NodeId(id),
        position: (x, y),
        kind: NodeKind::Intersection,
        legacy_id: None,
    }
}

/// Install the full runnable stack (CorePlugin + MobilityPlugin + EconomyPlugin)
/// with the four reference footway nodes at the abutopia market anchors —
/// the same harness shape as the in-crate `abutopia_price_stability` long-run
/// test — but WITHOUT seeding the economy (test 3 hydrates instead of seeding).
/// Spawns 300 bare `AgentMarker` citizens FIRST so both the seed-time scale and
/// the per-tick `CapitaFactor` refresh see the live-world population.
fn build_unseeded_world() -> (bevy_ecs::world::World, bevy_ecs::schedule::Schedule) {
    let mut world = bevy_ecs::world::World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    sim_core::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    sim_core::economy::EconomyPlugin.install(&mut world, &mut schedule);

    let nodes = vec![
        node(0, 2.0, 3.0),     // market 9001 anchor
        node(1, 111.5, 64.51), // market 9002 anchor
        node(2, 16.0, 48.0),   // market 9003 anchor
        node(3, 208.0, 48.0),  // market 9004 anchor
    ];
    world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
    world.insert_resource(Graph::new(nodes, vec![]));

    for _ in 0..CITIZENS {
        world.spawn(AgentMarker);
    }
    (world, schedule)
}

/// Unseeded world + the real abutopia markets layer (fresh-seed path).
fn build_chain_economy() -> (bevy_ecs::world::World, bevy_ecs::schedule::Schedule) {
    let (mut world, schedule) = build_unseeded_world();
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("abutopia bundle loads");
    seed_from_markets_layer(&mut world, &bundle.markets);
    world
        .resource::<Markets>()
        .0
        .get(&MarketId(9001))
        .expect("seed populated the markets (graph snap succeeded)");
    (world, schedule)
}

/// One sim tick: `MobilityPlugin`'s `tick_increment_system` advances `Tick` by
/// exactly one per `schedule.run`, so no manual increment (a manual `+= 1` on
/// top would double-step the tick and halve every interval cadence).
fn run_tick(world: &mut bevy_ecs::world::World, schedule: &mut bevy_ecs::schedule::Schedule) {
    schedule.run(world);
}

/// Σ of an event-quantity selector over `events`.
fn sum_qty(events: &[EconomyEvent], select: impl Fn(&EconomyEvent) -> Option<i64>) -> i64 {
    events.iter().filter_map(&select).sum()
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Money conservation, byte-exact, on the live seed with the active chain.
// ─────────────────────────────────────────────────────────────────────────────

/// 400 ticks on the real abutopia seed at capita factor 30: after EVERY tick
/// `total_money` equals the post-seed baseline byte-exactly (every runtime money
/// move is a conservative transfer; the #78 tick-audit would panic the schedule
/// on a violation — this proves it on the real seed with the chain trading) and
/// the `HOUSEHOLD_SECTOR` clearing sentinel nets to zero (wages + dividends are
/// fully apportioned out within the tick; stranded sentinel cash would be a
/// conservation violation).
#[test]
fn chain_conserves_money_byte_exact_over_400_ticks() {
    let (mut world, mut schedule) = build_chain_economy();

    // Pin the capita-scaled mint: (3 demand actors + 1 producer) × 1_000_000 × 30.
    let baseline = world.resource::<AccountBook>().total_money().unwrap();
    assert_eq!(
        baseline,
        Money(4 * 1_000_000 * EXPECTED_CAPITA_FACTOR),
        "post-seed total_money must be the authored opening cash × capita factor 30 \
         (3 consumers + producer 8031, 1M each)"
    );

    // 400 (not 200) ticks: the macro flow's dormant SUPPLY quantities are
    // capita-blind (offered_qty unscaled — the known capita-flow gap, next
    // slice), so WOOD arrives ~10/cadence while a scaled batch needs 300; the
    // first Produced(TOOLS) lands ~tick 290. The conservation assertion itself
    // is unchanged and checked after EVERY tick.
    for tick in 0..400_u64 {
        run_tick(&mut world, &mut schedule);
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            baseline,
            "total_money must be byte-invariant after every tick (tick {tick}): money is \
             only ever moved by conservative transfers, never minted/destroyed at runtime"
        );
        let sentinel = world.resource::<AccountBook>().account(HOUSEHOLD_SECTOR);
        assert_eq!(
            (sentinel.available, sentinel.locked),
            (Money::ZERO, Money::ZERO),
            "HOUSEHOLD_SECTOR must net to zero at end of tick {tick}: the wage/dividend \
             clearing account is fully apportioned to consumer pools within the tick"
        );
    }

    // Capita factor sanity: the run really happened at the live-world scale.
    assert_eq!(
        world
            .resource::<sim_core::economy::capita::CapitaFactor>()
            .0,
        EXPECTED_CAPITA_FACTOR,
        "300 citizens / authored capita_baseline 10 must yield factor 30"
    );

    // Non-vacuity: the chain actually traded and produced under the invariant.
    let ledger = &world.resource::<TradeLedger>().0;
    assert!(
        ledger.iter().any(|e| matches!(e,
            EconomyEvent::Produced { actor, good, .. }
            if *actor == PRODUCER_TOOLS && *good == GOOD_TOOLS)),
        "producer 8031 must have produced TOOLS during the run (chain alive, not a \
         vacuous conservation pass)"
    );
    assert!(
        ledger.iter().any(|e| matches!(e,
            EconomyEvent::Consumed { actor, good, .. }
            if *actor == PRODUCER_TOOLS && *good == GOOD_WOOD)),
        "producer 8031 must have consumed bought WOOD (the firms-as-buyers leg ran)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Goods-side ledger identity (the goods analog of money conservation).
// ─────────────────────────────────────────────────────────────────────────────

/// Over an accumulated multi-tick window, the event ledger must explain every
/// unit of WOOD and TOOLS exactly:
///   Σ Produced(g) + Σ Regenerated(g) − Σ Consumed(g) − Σ FinalConsumed(g)
///     == Δ total inventory of g (available + locked, all actors).
/// Trades and macro flows move goods BETWEEN actors (flow shipments are a
/// render-only projection, not a stock), so creation/destruction is exactly
/// the four event kinds above.
#[test]
fn chain_conserves_goods_ledger_identity() {
    let (mut world, mut schedule) = build_chain_economy();

    let total = |world: &bevy_ecs::world::World, good| {
        world
            .resource::<InventoryBook>()
            .total_good(good)
            .unwrap()
            .0
    };
    let wood_before = total(&world, GOOD_WOOD);
    let tools_before = total(&world, GOOD_TOOLS);
    let cursor = world.resource::<TradeLedger>().0.len();

    // 400 (not 120) ticks: the capita-flow gap (dormant supply unscaled →
    // ~10 WOOD/cadence vs the 300-unit scaled batch) puts the first
    // Produced(TOOLS) at ~tick 290 — the window must contain at least one
    // batch for the non-vacuity assert. The ledger identity is unchanged.
    for _ in 0..400_u64 {
        run_tick(&mut world, &mut schedule);
    }

    let ledger = &world.resource::<TradeLedger>().0;
    let window = &ledger[cursor..];
    for (name, good, before) in [
        ("WOOD", GOOD_WOOD, wood_before),
        ("TOOLS", GOOD_TOOLS, tools_before),
    ] {
        let produced = sum_qty(window, |e| match e {
            EconomyEvent::Produced { good: g, qty, .. } if *g == good => Some(qty.0),
            _ => None,
        });
        let regenerated = sum_qty(window, |e| match e {
            EconomyEvent::Regenerated { good: g, qty, .. } if *g == good => Some(qty.0),
            _ => None,
        });
        let consumed = sum_qty(window, |e| match e {
            EconomyEvent::Consumed { good: g, qty, .. } if *g == good => Some(qty.0),
            _ => None,
        });
        let final_consumed = sum_qty(window, |e| match e {
            EconomyEvent::FinalConsumed { good: g, qty, .. } if *g == good => Some(qty.0),
            _ => None,
        });
        let delta = total(&world, good) - before;
        assert_eq!(
            produced + regenerated - consumed - final_consumed,
            delta,
            "{name} ledger identity violated over 400 ticks: produced({produced}) + \
             regenerated({regenerated}) − consumed({consumed}) − final_consumed({final_consumed}) \
             must equal the inventory delta ({delta}) — a mismatch means goods were \
             created or destroyed outside the audited event kinds"
        );
        assert!(
            produced > 0,
            "{name} must actually have been produced in the window (non-vacuous identity)"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Persistence round trip mid-run, via the RUNTIME re-apply path.
// ─────────────────────────────────────────────────────────────────────────────

/// 350 ticks → `extract_from_world` → FRESH world → the runtime hydrate sequence
/// (`apply_into_world` THEN `seed_from_markets_layer`, exactly like
/// `sim-server/src/runtime/mod.rs::hydrate_from_stores` — the #86 lesson: seeding
/// runs AFTER apply, and the hydrate path re-applies authored config/policies) →
/// 50 more ticks. The `InputPool` cursor must survive byte-exactly, the
/// `ProducerPolicies` (authored config, never persisted) must be re-applied from
/// the layer, and production must CONTINUE after the resume (frozen-time model).
///
/// Phase lengths are 350 (not 50) ticks: the capita-flow gap (dormant supply
/// unscaled → ~10 WOOD/cadence vs the 300-unit scaled batch) puts a batch at
/// ~tick 290 and the next at ~tick 590, so each phase must span one full batch
/// cycle for its "produced TOOLS" liveness assert. All round-trip/byte-exactness
/// assertions are unchanged.
#[test]
fn chain_survives_persistence_round_trip_mid_run() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("abutopia bundle loads");

    // Phase 1 — fresh seed, 350 ticks of live chain (covers the ~tick-290 batch).
    let (mut world, mut schedule) = build_chain_economy();
    for _ in 0..350_u64 {
        run_tick(&mut world, &mut schedule);
    }
    let tick_at_snapshot = world.resource::<Tick>().0;
    assert_eq!(tick_at_snapshot, 350, "harness advances one tick per run");
    let snap = extract_from_world(&world);
    let pre_pool = world.resource::<InputPools>().0[&PRODUCER_TOOLS];
    assert!(
        pre_pool.last_generated_tick.is_some(),
        "the input-order cursor must have been stamped before the snapshot \
         (interval 1: it generates every tick)"
    );
    let pre_total = world.resource::<AccountBook>().total_money().unwrap();
    assert!(
        world.resource::<TradeLedger>().0.iter().any(|e| matches!(e,
            EconomyEvent::Produced { actor, good, .. }
            if *actor == PRODUCER_TOOLS && *good == GOOD_TOOLS)),
        "pre-snapshot run must have produced TOOLS (chain alive before the restart)"
    );

    // Phase 2 — fresh world, runtime hydrate order: apply, THEN seed (re-apply).
    let (mut resumed, mut resumed_schedule) = build_unseeded_world();
    apply_into_world(&mut resumed, &snap);
    seed_from_markets_layer(&mut resumed, &bundle.markets);
    // The real runtime restores Tick from the mobility snapshot (frozen-time
    // model); mirror that — the cursors are stamped against pre-restart ticks.
    resumed.resource_mut::<Tick>().0 = tick_at_snapshot;

    // Cursor preserved byte-exactly through snapshot + re-apply (the re-apply
    // must not reset persisted STATE).
    assert_eq!(
        resumed.resource::<InputPools>().0[&PRODUCER_TOOLS],
        pre_pool,
        "InputPool (incl. last_generated_tick cursor + discovered max_price) must \
         survive the persistence round trip byte-exactly"
    );
    // ProducerPolicies are authored CONFIG (not persisted): the hydrate path must
    // have rebuilt them from markets.json (#83-class config-revert guard).
    let policy = resumed.resource::<ProducerPolicies>().0[&PRODUCER_TOOLS];
    assert_eq!(
        (policy.theta_bps, policy.batches_target),
        (8_000, 2),
        "ProducerPolicies must be re-applied from the authored layer on hydrate"
    );
    assert_eq!(
        resumed.resource::<Markets>().0.len(),
        4,
        "hydrate path must not double-seed the markets"
    );
    assert_eq!(
        resumed.resource::<AccountBook>().total_money().unwrap(),
        pre_total,
        "total_money must survive the round trip byte-exactly"
    );

    // Phase 3 — 350 more ticks (covers the next ~tick-590 batch): production
    // continues, conservation holds.
    let resume_cursor = resumed.resource::<TradeLedger>().0.len();
    for _ in 0..350_u64 {
        run_tick(&mut resumed, &mut resumed_schedule);
        assert_eq!(
            resumed.resource::<AccountBook>().total_money().unwrap(),
            pre_total,
            "total_money byte-invariant across the resumed run"
        );
    }
    let resumed_events = &resumed.resource::<TradeLedger>().0[resume_cursor..];
    assert!(
        resumed_events.iter().any(|e| matches!(e,
            EconomyEvent::Produced { actor, good, .. }
            if *actor == PRODUCER_TOOLS && *good == GOOD_TOOLS)),
        "producer 8031 must keep producing TOOLS after the hydrate-path resume \
         (a reset cursor or missing policy would stall the chain)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Long-run stationarity: derived-demand input pricing, bounded cash DRIFT,
//    live sink.
// ─────────────────────────────────────────────────────────────────────────────

/// 1_200 ticks on the real seed (120 macro-flow cadences). The chain reaches a
/// self-consistent fixed point by ~tick 600 and never moves, so 1_200 leaves a
/// wide margin and the 500-tick tail sits entirely inside the stationary
/// regime (an earlier 2_600-tick draft chased a LoOP band the input leg can
/// never reach — see (a) below). Then:
/// (a) DERIVED-DEMAND INPUT PRICING (spec §8, corrected during Task 6): a
///     single-firm input market prices at the firm's §5.4 participation bound —
///     the only buyer bids its marginal revenue product per input unit net of
///     the labor share (Marshallian derived demand / MRP input pricing). The
///     #85 spatial-LoOP band `p_src + rate·dist` presupposes consumer-sink
///     arbitrage feedback: InputPools are structurally outside the flow-margin
///     feedback (pricing.rs iterates Demand/SupplyPools only), the dormant
///     demand-only bucket settles AT the bid ceiling == the bound
///     (macro_flow.rs synthetic_price), and the bound is rewritten from the
///     TOOLS ewma every cadence — so the input leg settles at the bound BY
///     CONSTRUCTION, and that is the economically correct fixed point. The
///     LoOP band remains the right anchor for CONSUMER sinks, covered by the
///     in-crate `abutopia_prices_stay_in_band_and_9002_consumes_over_long_run`
///     test. Asserted as:
///     (a1) `last_settlement_price(WOOD@9001)` == the participation bound
///          recomputed from the live end-of-run resources (TOOLS ewma, labor
///          share, recipe quantities) via the production `participation_bound`;
///     (a2) `ewma(WOOD@9001)` within the integer-EWMA freeze gap of the bound
///          (`integer_ewma` floors, so the ewma stalls once
///          `alpha·gap < 1` ⇔ `gap < 10_000/alpha_bps`; observed gap 4 at
///          alpha 2_000);
///     (a3) viability: the trade stays strictly profitable below the bound —
///          every tail flow's realized landed unit cost
///          (`p_src + ceil(transport/qty)` from the MacroFlow events) is < the
///          bound, the WOOD route ships at least once per cadence in the tail
///          (flows alive == profitable), and `ewma(WOOD@9003)` holds the
///          authored opening 50 (the extractor side never re-prices);
/// (b1) the θ-dividend FIRES stationarily: `ProfitDistributed` events from firm
///     8031 occur within the last 500 ticks — a silent dividend stall (lost
///     policy, never-distributable cash, dead profit base) would show zero;
/// (b2) firm cash DRIFT is bounded: `max(end-of-tick cash over the tail) −
///     cash(tail start) ≤ wc_target` at the live pool/policy/capita. NOTE an
///     ABSOLUTE cash bound is unsatisfiable by design (spec §5.3): the seed
///     endowment (opening_cash 1M × capita 30 = 30M) never drains because the
///     per-tick payout caps at θ·profit, and (1−θ) of each interval's profit is
///     retained legitimately as firm net worth. The stationarity guarantee is
///     the DRIFT: observed ≈ +49 over 500 ticks vs wc_target ≈ 166, while a
///     dividend stall would drift by the full profit stream (thousands) — the
///     one-wc_target bound discriminates without being tautological;
/// (c) Σ FinalConsumed(TOOLS) over the last 500 ticks > 0 — the chain delivers
///     stationarily, not just during a transient.
#[test]
fn wood_input_leg_prices_at_participation_bound_and_cash_drift_is_bounded() {
    const N: u64 = 1_200;
    const TAIL: u64 = 500;
    let (mut world, mut schedule) = build_chain_economy();
    let config = *world.resource::<EconomyConfig>();

    let mut tail_ledger_cursor = 0usize;
    let mut tail_start_cash: i64 = 0;
    let mut max_tail_cash: i64 = i64::MIN;
    let mut cash_trace: Vec<(u64, i64)> = Vec::new();

    for tick in 0..N {
        if tick == N - TAIL {
            tail_ledger_cursor = world.resource::<TradeLedger>().0.len();
        }
        run_tick(&mut world, &mut schedule);

        if tick >= N - TAIL {
            // End-of-tick (post-dividend) firm cash, locked bid cash included —
            // it is all working capital the firm holds.
            let acct = world.resource::<AccountBook>().account(PRODUCER_TOOLS);
            let cash = acct.available.0 + acct.locked.0;
            if tick == N - TAIL {
                tail_start_cash = cash;
            }
            max_tail_cash = max_tail_cash.max(cash);
            if tick % 100 == 0 || tick == N - 1 {
                cash_trace.push((tick, cash));
            }
        }
    }

    // The code's own working-capital target at the live end-of-run state
    // (participation bound, policy, capita factor) — no hardcoded magic number.
    let pool = world.resource::<InputPools>().0[&PRODUCER_TOOLS];
    let policy = world.resource::<ProducerPolicies>().0[&PRODUCER_TOOLS];
    let factor = world
        .resource::<sim_core::economy::capita::CapitaFactor>()
        .0;
    let wc = wc_target(policy, &pool, factor)
        .expect("wc_target computable on a priced live pool")
        .0;
    let drift = max_tail_cash - tail_start_cash;

    // (a) Derived-demand pricing of the input leg 9003→9001 (doc comment above).
    let market_good = |market: u32, good| {
        world.resource::<MarketGoods>().0[&MarketGoodKey {
            market: MarketId(market),
            good,
        }]
            .clone()
    };
    let wood_sink = market_good(9001, GOOD_WOOD);
    let p_sink_settle = wood_sink.last_settlement_price.0;
    let p_sink_ewma = wood_sink.ewma_reference_price.0;
    let p_src = market_good(9003, GOOD_WOOD).ewma_reference_price.0;
    let tools_ewma = market_good(9001, GOOD_TOOLS).ewma_reference_price;
    let labor_share = config
        .validated_labor_share_bps()
        .expect("authored labor share is valid");
    let bound = participation_bound(tools_ewma, labor_share, pool.out_qty, pool.in_qty)
        .expect("participation bound computable on the priced live TOOLS market")
        .0;
    let dist = world.resource::<MarketDistances>().0[&(MarketId(9003), MarketId(9001))];
    println!(
        "CHAIN STATIONARITY: p_wood_9001_settle={p_sink_settle} ewma={p_sink_ewma} \
         bound={bound} p_wood_9003={p_src} dist={dist} \
         idealized_loop_cost={} tail_start_cash={tail_start_cash} \
         max_tail_cash={max_tail_cash} drift={drift} wc_target={wc} \
         cash_trace={cash_trace:?}",
        p_src + config.transport_cost_per_tile_unit.0 * dist
    );

    // (a1) The settle price IS the participation bound at the fixed point: the
    // single buyer's bid ceiling, rewritten from the TOOLS ewma every cadence,
    // is exactly where the dormant demand-only bucket settles.
    assert_eq!(
        p_sink_settle, bound,
        "last_settlement_price(WOOD@9001) must equal the live participation bound \
         (derived-demand/MRP input pricing): settle {p_sink_settle} vs bound {bound} \
         (ewma(TOOLS@9001)={}, labor_share_bps={labor_share}, out/in = {}/{})",
        tools_ewma.0, pool.out_qty.0, pool.in_qty.0
    );

    // (a2) ewma(WOOD@9001) freezes a hair below the bound: `integer_ewma` floors
    // each step, so the ewma stalls once alpha·gap < 1, i.e. gap < 10_000/alpha_bps
    // (= 5 at the default alpha 2_000; observed gap 4).
    let ewma_freeze_gap = 10_000 / i64::from(config.ewma_alpha_bps);
    assert!(
        (p_sink_ewma - bound).abs() <= ewma_freeze_gap,
        "ewma(WOOD@9001) must sit within the integer-EWMA freeze gap of the bound: \
         |{p_sink_ewma} − {bound}| > {ewma_freeze_gap} (= 10_000/alpha_bps)"
    );

    // (a3) Viability: trading at the bound is strictly profitable. The extractor
    // side never re-prices (authored opening 50), every realized tail flow lands
    // below the bound, and the route ships every cadence (flows alive == the
    // firm keeps finding the trade worth taking).
    assert_eq!(
        p_src, 50,
        "ewma(WOOD@9003) must hold the authored opening price 50 (supply side of \
         the input leg is a faucet extractor; a moving p_src re-prices the story)"
    );
    let tail_events = &world.resource::<TradeLedger>().0[tail_ledger_cursor..];
    let tail_flows: Vec<(i64, i64)> = tail_events
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::MacroFlow {
                from_market,
                to_market,
                good,
                qty,
                transport,
                ..
            } if *from_market == MarketId(9003)
                && *to_market == MarketId(9001)
                && *good == GOOD_WOOD =>
            {
                Some((qty.0, transport.0))
            }
            _ => None,
        })
        .collect();
    let cadences_in_tail = TAIL / config.macro_flow_interval_ticks;
    assert!(
        tail_flows.len() as u64 >= cadences_in_tail,
        "the WOOD route 9003→9001 must ship at least once per macro-flow cadence \
         in the tail: {} flows over {cadences_in_tail} cadences — a starved route \
         means the bound stopped clearing the landed cost",
        tail_flows.len()
    );
    for &(qty, transport) in &tail_flows {
        // Realized landed unit cost: buyers pay p_src·qty + transport for qty
        // units (macro_flow settle: dst_payment = src_revenue + transport).
        let landed_unit = p_src + (transport + qty - 1) / qty;
        assert!(
            bound > landed_unit,
            "trade at the participation bound must stay strictly profitable: \
             bound {bound} ≤ realized landed unit cost {landed_unit} \
             (p_src={p_src}, qty={qty}, transport={transport})"
        );
    }

    // (b1) θ-dividends fire in the stationary tail (spec §5.3 mechanism alive).
    let dividends = tail_events
        .iter()
        .filter(|e| {
            matches!(e,
                EconomyEvent::ProfitDistributed { firm, .. } if *firm == PRODUCER_TOOLS)
        })
        .count();
    assert!(
        dividends > 0,
        "firm 8031 must emit ProfitDistributed events within the last {TAIL} ticks: \
         zero dividends in the stationary regime means the θ-payout silently \
         stalled (lost policy / dead profit base)"
    );

    // (b2) Tail cash DRIFT bounded by one working-capital target — the retained
    // (1−θ) trickle, not the profit stream (which a dividend stall would leave).
    assert!(
        drift <= wc,
        "firm 8031's end-of-tick cash drift over the last {TAIL} ticks must stay \
         within one live wc_target: drift {drift} (max {max_tail_cash} − start \
         {tail_start_cash}) > wc_target {wc} — a drift of this size means profit \
         is accumulating at the firm beyond the designed (1−θ) retention \
         (trace: {cash_trace:?})"
    );

    // (c) The TOOLS sink keeps consuming in the stationary regime.
    let tools_sunk = sum_qty(tail_events, |e| match e {
        EconomyEvent::FinalConsumed { good, qty, .. } if *good == GOOD_TOOLS => Some(qty.0),
        _ => None,
    });
    assert!(
        tools_sunk > 0,
        "Σ FinalConsumed(TOOLS) over the last {TAIL} ticks must be > 0: the chain \
         must deliver TOOLS to the 9002 sink stationarily, not only during a transient"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Pin the REAL baked graph distance of the WOOD route.
// ─────────────────────────────────────────────────────────────────────────────

/// The participation story has TWO distance ceilings: the bound 400 vs landed
/// cost 50 + 5·d requires d ≤ 69 for the route to be viable at all, and the
/// authored tick-1 sink price 380 > 50 + 5·d requires d ≤ 65 for the chain to
/// trade from the first cadence. The seeded anchors give d = 59. A re-snapped
/// or re-authored anchor that silently drifts d outside [55, 65] would change
/// the story (dead chain or no tick-1 trade) without any other test failing —
/// pin it.
#[test]
fn wood_route_distance_stays_within_participation_headroom() {
    let (world, _schedule) = build_chain_economy();
    let distances = world.resource::<MarketDistances>();
    let d_fwd = distances.0[&(MarketId(9003), MarketId(9001))];
    let d_rev = distances.0[&(MarketId(9001), MarketId(9003))];
    assert_eq!(d_fwd, d_rev, "baked distances must be symmetric");
    assert!(
        (55..=65).contains(&d_fwd),
        "WOOD route distance 9003↔9001 is {d_fwd}, outside [55, 65]: the chain's \
         participation headroom needs d ≤ 69 (TOOLS bound 400 ≥ landed 50 + 5·d) and \
         tick-1 trade needs d ≤ 65 (authored sink price 380 > 50 + 5·d); the lower \
         lip 55 catches an anchor re-snap that silently shortens the route and \
         re-prices the whole convergence story"
    );
    assert_eq!(
        d_fwd, 59,
        "the snapped abutopia anchors bake exactly 59 tiles"
    );
}
