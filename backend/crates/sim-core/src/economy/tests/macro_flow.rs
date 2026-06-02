use std::collections::{BTreeMap, BTreeSet};

use crate::economy::MarketDistances;
use crate::economy::macro_flow::synthetic_price;
use crate::economy::macro_flow::{
    Candidate, MacroBucket, build_candidates, build_macro_buckets, plan_flows,
    prune_unaffordable_buyers,
};
use crate::economy::{
    AccountBook, Ask, Bid, DemandPool, DemandPools, EconomicActorId, EconomyConfig, GOOD_FOOD,
    GoodId, InventoryBook, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money, OrderBook,
    OrderId, Quantity, SettlementPolicy, SupplyPool, SupplyPools,
};

// --- Section-C schedule-level harness (Task 13): reused BY NAME by Tasks 14-19. ---
use bevy_ecs::prelude::*;

use crate::economy::EconomyPlugin;
use crate::economy::transport::transport_cost;
use crate::economy::{EconomyError, MarketChunks, TRANSPORT_OPERATOR};
use crate::ids::ChunkCoord;
use crate::mobility::resources::Tick;
use crate::world::components::{AsleepChunk, ChunkCoordComp};
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;

fn bucket(price: Money, buyers: Vec<(u64, i64)>, sellers: Vec<(u64, i64)>) -> MacroBucket {
    MacroBucket {
        price,
        buyers: buyers
            .into_iter()
            .map(|(a, q)| (EconomicActorId(a), q))
            .collect(),
        sellers: sellers
            .into_iter()
            .map(|(a, q)| (EconomicActorId(a), q))
            .collect(),
        intra_cleared: false,
        buyer_orders: Vec::new(),
        buyer_max_prices: Vec::new(),
        seller_orders: Vec::new(),
    }
}

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
    let p2 = synthetic_price(
        true,
        true,
        Money(2_000),
        Money(500),
        Money(100),
        SettlementPolicy::Anchored,
    );
    assert_eq!(p2, Money(500));
}

#[test]
fn synthetic_price_one_sided_is_reservation_pinned() {
    // supply-only: returns ask_floor regardless of a high prior.
    let s = synthetic_price(
        false,
        true,
        Money(0),
        Money(500),
        Money(9_999),
        SettlementPolicy::Anchored,
    );
    assert_eq!(
        s,
        Money(500),
        "supply-only pins to ask_floor, ignores prior"
    );
    // demand-only: returns bid_ceiling regardless of a low prior.
    let d = synthetic_price(
        true,
        false,
        Money(2_000),
        Money(0),
        Money(1),
        SettlementPolicy::Anchored,
    );
    assert_eq!(
        d,
        Money(2_000),
        "demand-only pins to bid_ceiling, ignores prior"
    );
}

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
    demand.0.insert(
        buyer,
        DemandPool {
            actor: buyer,
            market,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(100),
            max_price: Money(1_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    let mut supply = SupplyPools::default();
    supply.0.insert(
        seller,
        SupplyPool {
            actor: seller,
            market,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(100),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    let dormant: BTreeSet<MarketId> = [market].into_iter().collect();
    let mg = MarketGoods::default(); // never auctioned -> prior = default ref price
    let cfg = EconomyConfig::default();

    let buckets = build_macro_buckets(
        &accounts,
        &inventory,
        &demand,
        &supply,
        &mg,
        &dormant,
        &cfg,
        &crate::economy::OrderBook::default(),
        false,
    )
    .unwrap();
    let key = MarketGoodKey {
        market,
        good: GOOD_FOOD,
    };
    let b: &MacroBucket = buckets.get(&key).expect("bucket exists");
    // both-sided -> p_m = settlement_price_with_policy(prior=1000, bid=1000, ask=500)=1000.
    assert_eq!(b.price, Money(1_000));
    // effective demand = min(100 desired, affordable(30 cash / price 1000 -> 30)) = 30.
    assert_eq!(b.total_demand(), 30);
    // effective supply = min(100 offered, 20 on hand) = 20.
    assert_eq!(b.total_supply(), 20);
}

#[test]
fn build_macro_buckets_skips_zero_price_band() {
    let market = MarketId(1);
    let seller = EconomicActorId(2);
    let mut inventory = InventoryBook::default();
    inventory.deposit(seller, GOOD_FOOD, Quantity(50)).unwrap();
    let mut supply = SupplyPools::default();
    supply.0.insert(
        seller,
        SupplyPool {
            actor: seller,
            market,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(0),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    let dormant: BTreeSet<MarketId> = [market].into_iter().collect();
    let buckets = build_macro_buckets(
        &AccountBook::default(),
        &inventory,
        &DemandPools::default(),
        &supply,
        &MarketGoods::default(),
        &dormant,
        &EconomyConfig::default(),
        &crate::economy::OrderBook::default(),
        false,
    )
    .unwrap();
    assert!(
        buckets.is_empty(),
        "zero-price band market produces no bucket and no error"
    );
}

#[test]
fn classify_bucket_partitions_matched_surplus_deficit() {
    use crate::economy::macro_flow::classify_bucket;

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

#[test]
fn build_candidates_keeps_cross_edge_when_gap_exceeds_transport() {
    let a = MarketId(1); // cheap, supply-only
    let b = MarketId(2); // dear, demand-only
    let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
    buckets.insert(
        MarketGoodKey {
            market: a,
            good: GOOD_FOOD,
        },
        bucket(Money(500), vec![], vec![(10, 100)]),
    ); // p_src=500, surplus 100
    buckets.insert(
        MarketGoodKey {
            market: b,
            good: GOOD_FOOD,
        },
        bucket(Money(2_000), vec![(20, 100)], vec![]),
    ); // p_dst=2000, deficit 100
    let mut distances = MarketDistances::default();
    distances.0.insert((a, b), 1); // 1 tile
    distances.0.insert((b, a), 1);
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };

    let candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
    // q_cap = min(surplus 100, deficit 100) = 100.
    // src_revenue = 500*100/1000 = 50 ; dst_value = 2000*100/1000 = 200.
    // transport = (50*100/1000)*1 = 5. net_gain = 200 - 50 - 5 = 145 > 0 -> kept.
    let cross: Vec<&Candidate> = candidates.iter().filter(|c| c.src != c.dst).collect();
    assert_eq!(cross.len(), 1);
    assert_eq!((cross[0].src, cross[0].dst), (a, b));
    assert_eq!(cross[0].q_cap, 100);
    assert_eq!(cross[0].net_gain, 145);
    assert_eq!(cross[0].transport_total, Money(5));
}

#[test]
fn build_candidates_prunes_cross_edge_when_gap_not_above_transport() {
    let a = MarketId(1);
    let b = MarketId(2);
    let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
    buckets.insert(
        MarketGoodKey {
            market: a,
            good: GOOD_FOOD,
        },
        bucket(Money(1_900), vec![], vec![(10, 100)]),
    );
    buckets.insert(
        MarketGoodKey {
            market: b,
            good: GOOD_FOOD,
        },
        bucket(Money(2_000), vec![(20, 100)], vec![]),
    );
    let mut distances = MarketDistances::default();
    distances.0.insert((a, b), 3);
    distances.0.insert((b, a), 3);
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };
    // gap value = 200 - 190 = 10 ; transport = (50*100/1000)*3 = 15 ; net = -5 <= 0.
    let candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
    assert!(
        candidates.iter().all(|c| c.src == c.dst),
        "no cross-edge survives when net_gain <= 0; only self-edges (none here)"
    );
    assert!(
        candidates.is_empty(),
        "no matched overlap -> no self-edges either"
    );
}

#[test]
fn build_candidates_prunes_cross_edge_at_exact_break_even() {
    let a = MarketId(1);
    let b = MarketId(2);
    let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
    buckets.insert(
        MarketGoodKey {
            market: a,
            good: GOOD_FOOD,
        },
        bucket(Money(1_900), vec![], vec![(10, 100)]),
    );
    buckets.insert(
        MarketGoodKey {
            market: b,
            good: GOOD_FOOD,
        },
        bucket(Money(2_000), vec![(20, 100)], vec![]),
    );
    let mut distances = MarketDistances::default();
    distances.0.insert((a, b), 2);
    distances.0.insert((b, a), 2);
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };
    // gap value = 10 ; transport = (50*100/1000)*2 = 10 ; net = 0 -> NOT kept (strict >).
    let candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
    assert!(
        candidates.is_empty(),
        "net_gain == 0 is pruned (strict greater)"
    );
}

#[test]
fn build_candidates_drops_overflow_edge() {
    let a = MarketId(1);
    let b = MarketId(2);
    let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
    buckets.insert(
        MarketGoodKey {
            market: a,
            good: GOOD_FOOD,
        },
        bucket(Money(500), vec![], vec![(10, i64::MAX)]),
    ); // pathological surplus
    buckets.insert(
        MarketGoodKey {
            market: b,
            good: GOOD_FOOD,
        },
        bucket(Money(2_000), vec![(20, i64::MAX)], vec![]),
    ); // pathological deficit
    let mut distances = MarketDistances::default();
    distances.0.insert((a, b), i64::MAX); // pathological distance
    distances.0.insert((b, a), i64::MAX);
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };
    let candidates = build_candidates(&buckets, &distances, &cfg)
        .expect("gate overflow is pruned, never an Err");
    assert!(
        candidates.iter().all(|c| c.src == c.dst),
        "overflow cross-edge dropped, no candidate, no fault"
    );
}

#[test]
fn build_candidates_emits_gate_exempt_self_edge() {
    let m = MarketId(1);
    let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
    // both sides present: D=40, S=60 -> matched 40, surplus 20.
    buckets.insert(
        MarketGoodKey {
            market: m,
            good: GOOD_FOOD,
        },
        bucket(Money(1_000), vec![(20, 40)], vec![(10, 60)]),
    );
    let candidates = build_candidates(
        &buckets,
        &MarketDistances::default(),
        &EconomyConfig::default(),
    )
    .unwrap();
    let self_edges: Vec<&Candidate> = candidates.iter().filter(|c| c.src == c.dst).collect();
    assert_eq!(self_edges.len(), 1);
    assert_eq!(self_edges[0].q_cap, 40, "self-edge clears matched overlap");
    assert_eq!(self_edges[0].transport_total, Money::ZERO);
    assert_eq!(
        self_edges[0].net_gain, 0,
        "self-edge net_gain is identically 0, gate-exempt"
    );
}

#[test]
fn sort_candidates_total_order() {
    use crate::economy::macro_flow::sort_candidates;

    fn cand(good: u16, src: u32, dst: u32, net: i64) -> Candidate {
        Candidate {
            good: GoodId(good),
            src: MarketId(src),
            dst: MarketId(dst),
            q_cap: 10,
            p_src: Money(500),
            p_dst: Money(2_000),
            transport_total: Money(0),
            net_gain: net,
            dist: 1,
        }
    }

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

#[test]
fn plan_flows_consumes_disjoint_budgets_once() {
    use crate::economy::macro_flow::sort_candidates;
    use crate::economy::macro_flow::{PlannedFlow, plan_flows};

    let a = MarketId(1); // surplus 30, matched 0
    let b = MarketId(2); // deficit 20
    let c = MarketId(3); // deficit 50
    let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
    buckets.insert(
        MarketGoodKey {
            market: a,
            good: GOOD_FOOD,
        },
        bucket(Money(500), vec![], vec![(10, 30)]),
    ); // surplus 30
    buckets.insert(
        MarketGoodKey {
            market: b,
            good: GOOD_FOOD,
        },
        bucket(Money(2_000), vec![(20, 20)], vec![]),
    ); // deficit 20
    buckets.insert(
        MarketGoodKey {
            market: c,
            good: GOOD_FOOD,
        },
        bucket(Money(3_000), vec![(30, 50)], vec![]),
    ); // deficit 50 (higher net first)
    let mut distances = MarketDistances::default();
    for (x, y) in [(a, b), (a, c), (b, a), (c, a)] {
        distances.0.insert((x, y), 1);
    }
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };

    let mut candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
    sort_candidates(&mut candidates);
    let flows: Vec<PlannedFlow> = plan_flows(&candidates, &buckets);
    // a->c has higher net_gain (p_dst 3000 > 2000) so it fills first: q=min(surplus30, need50)=30.
    // a->b then sees remaining_surplus[a]=0 -> q=0 -> skipped.
    let total_from_a: i64 = flows.iter().filter(|f| f.src == a).map(|f| f.q).sum();
    assert_eq!(
        total_from_a, 30,
        "surplus consumed exactly once across cross-edges"
    );
    assert!(flows.iter().any(|f| f.src == a && f.dst == c && f.q == 30));
    assert!(
        !flows.iter().any(|f| f.src == a && f.dst == b),
        "second cross-edge gets nothing once surplus is spent"
    );
}

#[test]
fn plan_flows_self_and_cross_are_disjoint() {
    use crate::economy::macro_flow::plan_flows;
    use crate::economy::macro_flow::sort_candidates;

    let a = MarketId(1); // D=20, S=50 -> matched 20, surplus 30
    let b = MarketId(2); // deficit 40
    let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
    buckets.insert(
        MarketGoodKey {
            market: a,
            good: GOOD_FOOD,
        },
        bucket(Money(500), vec![(11, 20)], vec![(10, 50)]),
    );
    buckets.insert(
        MarketGoodKey {
            market: b,
            good: GOOD_FOOD,
        },
        bucket(Money(2_000), vec![(20, 40)], vec![]),
    );
    let mut distances = MarketDistances::default();
    distances.0.insert((a, b), 1);
    distances.0.insert((b, a), 1);
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };
    let mut candidates = build_candidates(&buckets, &distances, &cfg).unwrap();
    sort_candidates(&mut candidates);
    let flows = plan_flows(&candidates, &buckets);
    let self_q: i64 = flows
        .iter()
        .filter(|f| f.src == a && f.dst == a)
        .map(|f| f.q)
        .sum();
    let cross_q: i64 = flows
        .iter()
        .filter(|f| f.src == a && f.dst == b)
        .map(|f| f.q)
        .sum();
    assert_eq!(self_q, 20, "matched cleared locally");
    assert_eq!(cross_q, 30, "surplus exported; budgets never contend");
}

#[test]
fn plan_flows_tiebreak_is_stable_ascending_dst() {
    use crate::economy::macro_flow::plan_flows;
    use crate::economy::macro_flow::sort_candidates;

    let build = || {
        let a = MarketId(1); // surplus 30
        let b = MarketId(2); // deficit 30, p_dst 2000
        let c = MarketId(3); // deficit 30, p_dst 2000 (equal net_gain & dist -> tie)
        let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
        buckets.insert(
            MarketGoodKey {
                market: a,
                good: GOOD_FOOD,
            },
            bucket(Money(500), vec![], vec![(10, 30)]),
        );
        buckets.insert(
            MarketGoodKey {
                market: b,
                good: GOOD_FOOD,
            },
            bucket(Money(2_000), vec![(20, 30)], vec![]),
        );
        buckets.insert(
            MarketGoodKey {
                market: c,
                good: GOOD_FOOD,
            },
            bucket(Money(2_000), vec![(30, 30)], vec![]),
        );
        let mut distances = MarketDistances::default();
        for (x, y) in [(a, b), (a, c), (b, a), (c, a)] {
            distances.0.insert((x, y), 1);
        }
        let cfg = EconomyConfig {
            transport_cost_per_tile_unit: Money(50),
            ..Default::default()
        };
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

#[test]
fn settle_flow_conserves_and_credits_operator_exactly() {
    use crate::economy::macro_flow::PlannedFlow;
    use crate::economy::macro_flow::settle_flow;
    use crate::economy::{TRANSPORT_OPERATOR, TradeLedger};

    let a = MarketId(1);
    let b = MarketId(2);
    let seller = EconomicActorId(10);
    let buyer = EconomicActorId(20);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    let mut market_goods = MarketGoods::default();
    let mut ledger = TradeLedger::default();

    let flow = PlannedFlow {
        good: GOOD_FOOD,
        src: a,
        dst: b,
        q: 100,
        p_src: Money(500),
        p_dst: Money(2_000),
        dist: 1,
    };
    // buyers/sellers weight maps for the prorata (single seller / single buyer here).
    let sellers = vec![(seller, 100i64)];
    let buyers = vec![(buyer, 100i64)];

    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };

    let mut next_accounts = accounts.clone();
    let mut next_inventory = inventory.clone();
    let event = settle_flow(
        &mut next_accounts,
        &mut next_inventory,
        &mut market_goods,
        &flow,
        &sellers,
        &buyers,
        /*eff_demand_src=*/ 0,
        /*eff_supply_src=*/ 100,
        /*eff_demand_dst=*/ 100,
        /*eff_supply_dst=*/ 0,
        &cfg,
        /*current_tick=*/ 10,
        false,
        false,
    )
    .unwrap();
    accounts = next_accounts;
    inventory = next_inventory;
    ledger.0.push(event);

    // src_revenue = 500*100/1000 = 50 ; transport = (50*100/1000)*1 = 5 ; dst_payment = 55.
    assert_eq!(
        accounts.total_money().unwrap(),
        m0,
        "money conserved (transport is a transfer)"
    );
    assert_eq!(
        inventory.total_good(GOOD_FOOD).unwrap(),
        g0,
        "goods conserved"
    );
    assert_eq!(
        accounts.account(TRANSPORT_OPERATOR).available,
        Money(5),
        "operator credited exactly the transport total"
    );
    assert_eq!(inventory.balance(buyer, GOOD_FOOD).available, Quantity(100));
    assert_eq!(
        inventory.balance(seller, GOOD_FOOD).available,
        Quantity(900)
    );
    assert_eq!(
        accounts.account(seller).available,
        Money(50),
        "seller paid src_revenue"
    );
    // dst write-back: last_settlement_price = p_dst, traded_qty += q, residual demand 0.
    let st_b = market_goods
        .0
        .get(&MarketGoodKey {
            market: b,
            good: GOOD_FOOD,
        })
        .unwrap();
    assert_eq!(st_b.last_settlement_price, Money(2_000));
    assert_eq!(st_b.traded_qty_last_tick, Quantity(100));
    assert_eq!(st_b.last_cleared_tick, 10);
    let st_a = market_goods
        .0
        .get(&MarketGoodKey {
            market: a,
            good: GOOD_FOOD,
        })
        .unwrap();
    assert_eq!(st_a.last_settlement_price, Money(500));
}

#[test]
fn settle_flow_n_buyers_aggregate_floor_conserves() {
    use crate::economy::macro_flow::PlannedFlow;
    use crate::economy::macro_flow::settle_flow;
    use crate::economy::{EconomyEvent, TRANSPORT_OPERATOR};

    let a = MarketId(1);
    let b = MarketId(2);
    let seller = EconomicActorId(10);
    let buyer1 = EconomicActorId(20);
    let buyer2 = EconomicActorId(21);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    accounts.deposit(buyer1, Money(1_000_000)).unwrap();
    accounts.deposit(buyer2, Money(1_000_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    let mut market_goods = MarketGoods::default();
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };

    // q=3, p_src=500 -> src_revenue floor(1500/1000)=1 ; transport (50*3/1000)floor=0.
    let flow = PlannedFlow {
        good: GOOD_FOOD,
        src: a,
        dst: b,
        q: 3,
        p_src: Money(500),
        p_dst: Money(2_000),
        dist: 1,
    };
    let sellers = vec![(seller, 3i64)];
    let buyers = vec![(buyer1, 2i64), (buyer2, 1i64)];
    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();

    let mut na = accounts.clone();
    let mut ni = inventory.clone();
    let ev = settle_flow(
        &mut na,
        &mut ni,
        &mut market_goods,
        &flow,
        &sellers,
        &buyers,
        0,
        3,
        3,
        0,
        &cfg,
        10,
        false,
        false,
    )
    .unwrap();
    accounts = na;
    inventory = ni;

    assert_eq!(
        accounts.total_money().unwrap(),
        m0,
        "money conserved with N buyers"
    );
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
    // transport floored to 0 -> dst_payment == src_revenue == 1 ; Σ buyer charges == 1.
    let charged = m0.0
        - (accounts.account(buyer1).available.0 + accounts.account(buyer2).available.0)
        - accounts.account(TRANSPORT_OPERATOR).available.0;
    assert_eq!(
        charged,
        accounts.account(seller).available.0,
        "Σ buyer charges == seller revenue (no per-line floor leak)"
    );
    if let EconomyEvent::MacroFlow { transport, qty, .. } = ev {
        assert_eq!(transport, Money(0));
        assert_eq!(qty, Quantity(3));
    } else {
        panic!("expected MacroFlow");
    }
}

#[test]
fn settle_flow_conserves_when_per_unit_cash_exceeds_one_scale_unit() {
    // Regime the earlier conservation tests never exercised: p_src > 1.0
    // scale-unit (so per-unit cash > 1) AND positive transport. Here the old
    // `prorata_distribute(goods, cash)` clamp `min(cash, Σgoods)` would have
    // capped both the seller credit and the buyer charge at q, undercrediting
    // the seller and undercharging the buyer while the operator still received
    // the full transport -> money minted. With the exact (non-clamping)
    // apportionment the seller is credited src_revenue, buyers are charged
    // dst_payment, and total money is invariant.
    use crate::economy::macro_flow::PlannedFlow;
    use crate::economy::macro_flow::settle_flow;
    use crate::economy::{EconomyEvent, TRANSPORT_OPERATOR};

    let a = MarketId(1);
    let b = MarketId(2);
    let seller1 = EconomicActorId(10);
    let seller2 = EconomicActorId(11);
    let buyer1 = EconomicActorId(20);
    let buyer2 = EconomicActorId(21);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    accounts.deposit(buyer1, Money(1_000_000)).unwrap();
    accounts.deposit(buyer2, Money(1_000_000)).unwrap();
    inventory
        .deposit(seller1, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    inventory
        .deposit(seller2, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    let mut market_goods = MarketGoods::default();
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };

    // p_src=2000 (=2.0 scale-units), q=100, dist=1, rate=50:
    //   src_revenue = floor(2000*100/1000) = 200
    //   transport   = floor(50*100/1000)*1 = 5
    //   dst_payment = 205
    let flow = PlannedFlow {
        good: GOOD_FOOD,
        src: a,
        dst: b,
        q: 100,
        p_src: Money(2_000),
        p_dst: Money(3_000),
        dist: 1,
    };
    let sellers = vec![(seller1, 60i64), (seller2, 40i64)];
    let buyers = vec![(buyer1, 70i64), (buyer2, 30i64)];
    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();

    let mut na = accounts.clone();
    let mut ni = inventory.clone();
    let ev = settle_flow(
        &mut na,
        &mut ni,
        &mut market_goods,
        &flow,
        &sellers,
        &buyers,
        0,
        100,
        100,
        0,
        &cfg,
        10,
        false,
        false,
    )
    .unwrap();
    accounts = na;
    inventory = ni;

    assert_eq!(
        accounts.total_money().unwrap(),
        m0,
        "money conserved even when per-unit cash exceeds one scale-unit"
    );
    assert_eq!(
        inventory.total_good(GOOD_FOOD).unwrap(),
        g0,
        "goods conserved"
    );
    assert_eq!(
        accounts.account(TRANSPORT_OPERATOR).available,
        Money(5),
        "operator credited exactly the transport total"
    );
    // Σ seller credit == src_revenue (200), Σ buyer charge == dst_payment (205).
    let seller_credit =
        accounts.account(seller1).available.0 + accounts.account(seller2).available.0;
    assert_eq!(seller_credit, 200, "Σ seller credit == src_revenue");
    let buyer_charge =
        m0.0 - (accounts.account(buyer1).available.0 + accounts.account(buyer2).available.0);
    assert_eq!(buyer_charge, 205, "Σ buyer charge == dst_payment");
    if let EconomyEvent::MacroFlow { transport, qty, .. } = ev {
        assert_eq!(transport, Money(5));
        assert_eq!(qty, Quantity(100));
    } else {
        panic!("expected MacroFlow");
    }
}

#[test]
fn settle_flow_default_reference_price_with_transport_conserves() {
    // The production default reference price is Money(1_000) == 1.0 scale-unit,
    // which makes src_revenue == q exactly. With positive transport the buyer
    // charge dst_payment == q + transport > q. The old `min(cash, Σgoods)` clamp
    // capped the buyer charge at q while the operator received the full
    // transport -> exactly `transport` money minted on every default-priced
    // cross-market flow. This is the common production path.
    use crate::economy::macro_flow::PlannedFlow;
    use crate::economy::macro_flow::settle_flow;
    use crate::economy::{EconomyEvent, TRANSPORT_OPERATOR};

    let a = MarketId(1);
    let b = MarketId(2);
    let seller = EconomicActorId(10);
    let buyer = EconomicActorId(20);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    let mut market_goods = MarketGoods::default();
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };

    // p_src=1000 (default), q=100, dist=1, rate=50:
    //   src_revenue = floor(1000*100/1000) = 100 == q
    //   transport   = floor(50*100/1000)*1 = 5
    //   dst_payment = 105 (> q)
    let flow = PlannedFlow {
        good: GOOD_FOOD,
        src: a,
        dst: b,
        q: 100,
        p_src: Money(1_000),
        p_dst: Money(1_000),
        dist: 1,
    };
    let sellers = vec![(seller, 100i64)];
    let buyers = vec![(buyer, 100i64)];
    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();

    let mut na = accounts.clone();
    let mut ni = inventory.clone();
    let ev = settle_flow(
        &mut na,
        &mut ni,
        &mut market_goods,
        &flow,
        &sellers,
        &buyers,
        0,
        100,
        100,
        0,
        &cfg,
        10,
        false,
        false,
    )
    .unwrap();
    accounts = na;
    inventory = ni;

    assert_eq!(
        accounts.total_money().unwrap(),
        m0,
        "no money minted on the default-priced cross-market flow"
    );
    assert_eq!(
        inventory.total_good(GOOD_FOOD).unwrap(),
        g0,
        "goods conserved"
    );
    assert_eq!(
        accounts.account(TRANSPORT_OPERATOR).available,
        Money(5),
        "operator credited exactly the transport total"
    );
    assert_eq!(
        accounts.account(seller).available,
        Money(100),
        "seller credited src_revenue == q"
    );
    let buyer_charge = m0.0 - accounts.account(buyer).available.0;
    assert_eq!(
        buyer_charge, 105,
        "buyer charged dst_payment == q + transport"
    );
    if let EconomyEvent::MacroFlow { transport, .. } = ev {
        assert_eq!(transport, Money(5));
    } else {
        panic!("expected MacroFlow");
    }
}

#[test]
fn settle_flow_self_edge_clears_locally_transport_zero() {
    use crate::economy::macro_flow::PlannedFlow;
    use crate::economy::macro_flow::settle_flow;
    use crate::economy::{EconomyEvent, TRANSPORT_OPERATOR};

    let m = MarketId(1);
    let seller = EconomicActorId(10);
    let buyer = EconomicActorId(20);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    let mut market_goods = MarketGoods::default();
    let cfg = EconomyConfig::default();
    let flow = PlannedFlow {
        good: GOOD_FOOD,
        src: m,
        dst: m,
        q: 40,
        p_src: Money(1_000),
        p_dst: Money(1_000),
        dist: 0,
    };
    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();
    let mut na = accounts.clone();
    let mut ni = inventory.clone();
    let ev = settle_flow(
        &mut na,
        &mut ni,
        &mut market_goods,
        &flow,
        &[(seller, 40)],
        &[(buyer, 40)],
        40,
        40,
        40,
        40,
        &cfg,
        0,
        false,
        false,
    )
    .unwrap();
    accounts = na;
    inventory = ni;
    assert_eq!(accounts.total_money().unwrap(), m0);
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
    assert_eq!(accounts.account(TRANSPORT_OPERATOR).available, Money(0));
    if let EconomyEvent::MacroFlow {
        from_market,
        to_market,
        transport,
        ..
    } = ev
    {
        assert_eq!(from_market, to_market);
        assert_eq!(transport, Money(0));
    } else {
        panic!("expected MacroFlow");
    }
    // single write-back for the self market.
    let st = market_goods
        .0
        .get(&MarketGoodKey {
            market: m,
            good: GOOD_FOOD,
        })
        .unwrap();
    assert_eq!(st.traded_qty_last_tick, Quantity(40));
}

use crate::economy::macro_flow::run_macro_flow_at_tick;
use crate::economy::{DirtyMarketGoods, EconomyEvent, TradeLedger};

#[allow(clippy::type_complexity)]
fn surplus_deficit_world() -> (
    AccountBook,
    InventoryBook,
    TradeLedger,
    DemandPools,
    SupplyPools,
    MarketGoods,
    DirtyMarketGoods,
    BTreeSet<MarketId>,
    MarketDistances,
    EconomyConfig,
) {
    let a = MarketId(1);
    let b = MarketId(2);
    let seller = EconomicActorId(10);
    let buyer = EconomicActorId(20);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(
        buyer,
        DemandPool {
            actor: buyer,
            market: b,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(100),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    let mut supply = SupplyPools::default();
    supply.0.insert(
        seller,
        SupplyPool {
            actor: seller,
            market: a,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(100),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    let dormant: BTreeSet<MarketId> = [a, b].into_iter().collect();
    let mut distances = MarketDistances::default();
    distances.0.insert((a, b), 1);
    distances.0.insert((b, a), 1);
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };
    (
        accounts,
        inventory,
        TradeLedger::default(),
        demand,
        supply,
        MarketGoods::default(),
        DirtyMarketGoods::default(),
        dormant,
        distances,
        cfg,
    )
}

#[test]
fn macro_flow_only_fires_on_interval() {
    let (mut acc, mut inv, mut led, dem, sup, mut mg, dirty, dormant, dist, cfg) =
        surplus_deficit_world();
    // tick 3: not a multiple of 10 -> no flow, no events.
    run_macro_flow_at_tick(
        &mut acc,
        &mut inv,
        &mut led,
        &dem,
        &sup,
        &mut mg,
        &dirty,
        &dormant,
        &dist,
        &cfg,
        3,
        &mut crate::economy::FlowShipments::default(),
        &mut crate::economy::NextShipmentId::default(),
        &mut crate::economy::OrderBook::default(),
        &mut crate::economy::NextOrderId::default(),
    )
    .unwrap();
    assert!(led.0.is_empty(), "no flow off-interval");
    // tick 10: fires.
    run_macro_flow_at_tick(
        &mut acc,
        &mut inv,
        &mut led,
        &dem,
        &sup,
        &mut mg,
        &dirty,
        &dormant,
        &dist,
        &cfg,
        10,
        &mut crate::economy::FlowShipments::default(),
        &mut crate::economy::NextShipmentId::default(),
        &mut crate::economy::OrderBook::default(),
        &mut crate::economy::NextOrderId::default(),
    )
    .unwrap();
    assert!(
        led.0
            .iter()
            .any(|e| matches!(e, EconomyEvent::MacroFlow { .. })),
        "flow on interval tick"
    );
}

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
    run_macro_flow_at_tick(
        &mut acc,
        &mut inv,
        &mut led,
        &DemandPools::default(),
        &SupplyPools::default(),
        &mut mg,
        &DirtyMarketGoods::default(),
        &dormant,
        &MarketDistances::default(),
        &cfg,
        0,
        &mut crate::economy::FlowShipments::default(),
        &mut crate::economy::NextShipmentId::default(),
        &mut crate::economy::OrderBook::default(),
        &mut crate::economy::NextOrderId::default(),
    )
    .unwrap();
    assert_eq!(acc, before_acc, "books byte-identical on idle interval");
    assert_eq!(inv, before_inv);
    assert!(led.0.is_empty());
    assert!(mg.0.is_empty(), "no write-back on idle interval");
}

#[test]
fn macro_flow_settle_fault_isolates_and_conserves() {
    use crate::economy::MarketGoodState;

    // STEP H per-edge settle-fault isolation: one edge faults at SETTLEMENT time,
    // the system emits a single MarketClearFailed for it, skips it (its scratch
    // clone is discarded so the live books are byte-identical for that edge), and
    // every healthy edge still commits + emits a MacroFlow. Conservation holds
    // across the whole tick.
    //
    // NOTE ON THE FAULT MECHANISM. The spec/plan sketch a *cash* over-charge — a
    // deficit buyer targeted by two sources whose second edge's `lock_cash`
    // exceeds bucket-time affordability. That construction is UNREACHABLE against
    // this implementation: STEP A caps each buyer's effective demand to
    // `affordable_qty(cash, p_m)`, and the STEP-D transport gate guarantees, per
    // accepted import edge, `transport_i < (p_m - p_src_i) * q_i`. Summing over a
    // buyer's edges (self-edge at `p_src = p_dst = p_m`, imports at `p_src < p_m`):
    //   total_charge = Σ p_src_i·q_i + Σ transport_i + p_m·matched
    //                < Σ p_src_i·q_i + Σ(p_m - p_src_i)·q_i + p_m·matched
    //                = p_m·(deficit + matched) = p_m·effective_demand ≤ cash.
    // So `lock_cash` provably never under-funds. An exhaustive sweep over cash /
    // transport-rate / source-prices / demand / local-supply / distance produced
    // ZERO cash faults, confirming the proof. The genuine settlement-time fault
    // that STEP H isolates is therefore a *checked-op* fault — exactly the "…or
    // any checked op underflows" clause of the STEP H contract: here the
    // `traded_qty_last_tick` accumulator in `write_back` overflows `Quantity::
    // checked_add` for an import sink whose prior accumulated quantity is at
    // `i64::MAX`. The import to that sink errors with `Overflow`; the unrelated
    // healthy import commits.
    let a1 = MarketId(1); // cheap surplus source for the HEALTHY import
    let a2 = MarketId(2); // cheap surplus source for the FAULTING import
    let cdst = MarketId(3); // healthy deficit sink
    let bdst = MarketId(4); // faulting deficit sink (pre-seeded near i64::MAX)
    let s1 = EconomicActorId(10);
    let s2 = EconomicActorId(11);
    let cbuyer = EconomicActorId(20);
    let bbuyer = EconomicActorId(21);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    accounts.deposit(cbuyer, Money(1_000_000)).unwrap();
    accounts.deposit(bbuyer, Money(1_000_000)).unwrap();
    inventory.deposit(s1, GOOD_FOOD, Quantity(100)).unwrap();
    inventory.deposit(s2, GOOD_FOOD, Quantity(100)).unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(
        cbuyer,
        DemandPool {
            actor: cbuyer,
            market: cdst,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(100),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    demand.0.insert(
        bbuyer,
        DemandPool {
            actor: bbuyer,
            market: bdst,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(100),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    let mut supply = SupplyPools::default();
    supply.0.insert(
        s1,
        SupplyPool {
            actor: s1,
            market: a1,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(100),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    supply.0.insert(
        s2,
        SupplyPool {
            actor: s2,
            market: a2,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(100),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    let dormant: BTreeSet<MarketId> = [a1, a2, cdst, bdst].into_iter().collect();
    let mut distances = MarketDistances::default();
    for (x, y) in [(a1, cdst), (cdst, a1), (a2, bdst), (bdst, a2)] {
        distances.0.insert((x, y), 1);
    }
    let cfg = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };
    let mut market_goods = MarketGoods::default();
    // Pre-seed B's accumulator at i64::MAX so the import's write_back overflows
    // `Quantity::checked_add` -> settle returns Err(Overflow) -> the B edge faults.
    let bkey = MarketGoodKey {
        market: bdst,
        good: GOOD_FOOD,
    };
    let mut bstate = MarketGoodState::new(bkey);
    bstate.traded_qty_last_tick = Quantity(i64::MAX);
    bstate.last_settlement_price = Money(1_000);
    market_goods.0.insert(bkey, bstate);
    let mut ledger = TradeLedger::default();
    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();

    run_macro_flow_at_tick(
        &mut accounts,
        &mut inventory,
        &mut ledger,
        &demand,
        &supply,
        &mut market_goods,
        &DirtyMarketGoods::default(),
        &dormant,
        &distances,
        &cfg,
        0,
        &mut crate::economy::FlowShipments::default(),
        &mut crate::economy::NextShipmentId::default(),
        &mut crate::economy::OrderBook::default(),
        &mut crate::economy::NextOrderId::default(),
    )
    .unwrap();

    // Conservation holds across the whole tick (faulted edge left books unchanged for it).
    assert_eq!(accounts.total_money().unwrap(), m0);
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
    // At least one healthy MacroFlow AND exactly one MarketClearFailed for the faulted edge.
    assert!(
        ledger
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::MacroFlow { .. }))
    );
    assert_eq!(
        ledger
            .0
            .iter()
            .filter(|e| matches!(e, EconomyEvent::MarketClearFailed { .. }))
            .count(),
        1,
        "the faulting edge faults exactly once, others healthy"
    );
    // The faulted edge moved nothing: the B buyer never spent, the healthy C buyer did.
    assert_eq!(
        accounts.account(bbuyer).available,
        Money(1_000_000),
        "faulted-edge buyer untouched"
    );
    assert!(accounts.account(bbuyer).available.0 >= 0, "no overdraw");
}

/// Full Core+Mobility+Economy world so the wired `EconomySet` chain runs end to
/// end. Inserts an (initially empty) `MarketDistances` table + `Tick(0)`. Markets
/// are NOT anchored here (callers anchor + set distances), so the schedule-level
/// tests drive the real `run_macro_flow_system`. Reused by every Section-C test.
#[allow(dead_code)] // consumed by the schedule-level Section-C tests (Tasks 14-19).
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
#[allow(clippy::too_many_arguments)]
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
        supply.0.insert(
            EconomicActorId(actor),
            sp(actor, m_a, seller_qty, ask_floor),
        );
    }
    for c in 0..n_buyers {
        let actor = 200 + c;
        accounts
            .deposit(EconomicActorId(actor), Money(1_000_000_000))
            .unwrap();
        demand.0.insert(
            EconomicActorId(actor),
            dp(actor, m_b, buyer_qty, bid_ceiling),
        );
    }

    let mut distances = MarketDistances(BTreeMap::new());
    distances.0.insert((m_a, m_b), dist);
    distances.0.insert((m_b, m_a), dist);

    let config = EconomyConfig {
        transport_cost_per_tile_unit: rate,
        ..Default::default()
    };

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
        &mut crate::economy::FlowShipments::default(),
        &mut crate::economy::NextShipmentId::default(),
        &mut crate::economy::OrderBook::default(),
        &mut crate::economy::NextOrderId::default(),
    )
}

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

    assert_eq!(
        s.accounts.total_money().unwrap(),
        money_before,
        "money conserved"
    );
    assert_eq!(
        s.inventory.total_good(GOOD_FOOD).unwrap(),
        good_before,
        "goods conserved"
    );
    let op_after = s.accounts.account(TRANSPORT_OPERATOR).available;
    assert_eq!(
        op_after.0 - op_before.0,
        expected_transport.0,
        "operator gained exactly transport_total"
    );
    for acct in s.accounts.accounts.values() {
        assert!(acct.available.0 >= 0, "no negative available cash");
    }
}

#[test]
#[allow(non_snake_case)]
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

        inventory
            .deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000))
            .unwrap();
        supply
            .0
            .insert(EconomicActorId(100), sp(100, m_a, 100, 400));
        accounts
            .deposit(EconomicActorId(200), Money(1_000_000_000))
            .unwrap();
        demand
            .0
            .insert(EconomicActorId(200), dp(200, m_b, 100, 1800));
        accounts
            .deposit(EconomicActorId(201), Money(1_000_000_000))
            .unwrap();
        demand
            .0
            .insert(EconomicActorId(201), dp(201, m_c, 100, 1800));

        let mut distances = MarketDistances(BTreeMap::new());
        for (x, y) in [(m_a, m_b), (m_b, m_a), (m_a, m_c), (m_c, m_a)] {
            distances.0.insert((x, y), 3);
        }
        let config = EconomyConfig {
            transport_cost_per_tile_unit: Money(50),
            ..Default::default()
        };

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
    assert_eq!(
        a, b,
        "equidistant deficit split is byte-identical across runs"
    );

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

fn last_price(mg: &MarketGoods, market: MarketId) -> Money {
    mg.0.get(&MarketGoodKey {
        market,
        good: GOOD_FOOD,
    })
    .map(|s| s.last_settlement_price)
    .unwrap_or(Money::ZERO)
}

/// Both-sided pair: A is net-surplus & cheap (big supplier@low ask + small
/// consumer), B is net-deficit & dear (big consumer@high bid + small supplier).
/// Each market is price-DISCOVERING — it has both a demand and a supply side, so
/// its synthetic price is `settlement_price_with_policy(prior, bid_ceiling,
/// ask_floor)`, i.e. the Anchored clamp of the carried `prior` into that market's
/// own reservation band (§3 STEP A), NOT a one-sided reservation pin.
///
/// The two bands are deliberately ADJACENT around the shared default reference
/// price (`trader_default_ref_price = 1000`):
///   A band [ask 600, bid_ceiling 1001]   (cheap surplus; ceiling just above 1000)
///   B band [ask_floor 1002, bid 1800]     (dear deficit; floor just above 1000)
/// so the Law-of-One-Price clamp can actually pull the two realized prices TO
/// WITHIN transport of each other — `A.bid_ceiling (1001)` and `B.ask_floor
/// (1002)` straddle the common 1000 anchor. (Contrast the structurally-pinned
/// failure mode: bands 1000 apart with no overlap clamp to opposite edges and
/// the gap can never narrow below the band separation — that geometry cannot
/// demonstrate convergence and is exactly what this test must avoid.)
fn both_sided_pair(rate: Money, dist: i64) -> DormantScenario {
    let m_a = MarketId(1);
    let m_b = MarketId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();

    // A: big seller (300 @ ask 600) + small local buyer (20 @ bid 1001).
    inventory
        .deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    supply
        .0
        .insert(EconomicActorId(100), sp(100, m_a, 300, 600));
    accounts
        .deposit(EconomicActorId(110), Money(1_000_000_000))
        .unwrap();
    demand
        .0
        .insert(EconomicActorId(110), dp(110, m_a, 20, 1001));

    // B: big buyer (300 @ bid 1800) + small local seller (20 @ ask 1002).
    accounts
        .deposit(EconomicActorId(200), Money(1_000_000_000))
        .unwrap();
    demand
        .0
        .insert(EconomicActorId(200), dp(200, m_b, 300, 1800));
    inventory
        .deposit(EconomicActorId(210), GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    supply
        .0
        .insert(EconomicActorId(210), sp(210, m_b, 20, 1002));

    let mut distances = MarketDistances(BTreeMap::new());
    distances.0.insert((m_a, m_b), dist);
    distances.0.insert((m_b, m_a), dist);
    let config = EconomyConfig {
        transport_cost_per_tile_unit: rate,
        ..Default::default()
    };

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

    // Law of One Price (§1/§3 STEP A, §8 test 4): on price-DISCOVERING (both-
    // sided) markets the realized inter-market gap converges to within the
    // transport cost. Each market's synthetic price is the Anchored clamp of its
    // carried `prior` into its own reservation band; the bands here are adjacent
    // around the shared `trader_default_ref_price = 1000`, so the clamp pulls the
    // two prices together rather than pinning them to band edges 1000 apart.
    //
    // Trace (interval 0, prior = 1000 for both since nothing is auctioned yet):
    //   A: clamp(1000, [ask 600, bid 1001]) = 1000  (1000 lies inside A's band)
    //   B: clamp(1000, [ask 1002, bid 1800]) = 1002 (1000 < ask_floor → up to 1002)
    // Both markets self-clear their local overlap (matched = min(20, 300) = 20),
    // which is gate-exempt, so BOTH write their discovered price back this very
    // interval. The realized gap is |1002 - 1000| = 2.
    //
    // Each written-back price is its own market's band-clamp, so it is a fixed
    // point of the clamp (prior already inside band ⇒ unchanged next interval):
    // the gap is monotone non-increasing and settles at 2 = unit_transport + 1,
    // i.e. WITHIN transport. (Contrast the broken non-overlapping geometry whose
    // gap would floor at the 1000 band separation, never reaching transport.)
    //
    // unit_transport: transport is the fixed-point aggregate `(rate·q/SCALE)·dist`
    // floored to 0 below the SCALE threshold; the per-unit bound is that
    // aggregate over the reference fill `q` rounded up.
    let q_ref = Quantity(280);
    let agg_transport = transport_cost(dist, q_ref, rate).unwrap(); // (50*280/1000)*2 = 28
    assert_eq!(agg_transport, Money(28));
    let unit_transport = Money((agg_transport.0 + q_ref.0 - 1) / q_ref.0); // ceil(28/280) = 1

    let mut prev_gap = i64::MAX;
    for k in 0..40u64 {
        run_flow(&mut s, k * 10).unwrap();
        let pa = last_price(&s.market_goods, MarketId(1));
        let pb = last_price(&s.market_goods, MarketId(2));
        if pa.0 == 0 || pb.0 == 0 {
            continue; // not both priced yet
        }
        let gap = (pb.0 - pa.0).abs();
        assert!(
            gap <= prev_gap,
            "gap monotone non-increasing: {gap} <= {prev_gap}"
        );
        prev_gap = gap;
    }
    assert!(
        prev_gap <= unit_transport.0 + 1,
        "converged within transport: gap {prev_gap} <= unit_transport {} + 1",
        unit_transport.0
    );
}

#[test]
fn one_sided_pair_flows_goods_but_price_is_pinned() {
    // Pure source A (supply-only, ask 500) ↔ pure sink B (demand-only, bid 2000).
    // Reservation-pinned: p_a = ask_floor = 500, p_b = bid_ceiling = 2000 every
    // interval. Goods move each interval; the 1500 gap NEVER narrows.
    let mut s = surplus_deficit_scenario(1, 200, 500, 1, 200, 2000, 2, Money(50));
    let buyer_before = s
        .inventory
        .balance(EconomicActorId(200), GOOD_FOOD)
        .available;

    run_flow(&mut s, 0).unwrap();
    let pa0 = last_price(&s.market_goods, MarketId(1));
    let pb0 = last_price(&s.market_goods, MarketId(2));
    assert_eq!(pa0, Money(500), "supply-only price pinned to ask floor");
    assert_eq!(pb0, Money(2000), "demand-only price pinned to bid ceiling");
    let buyer_after_1 = s
        .inventory
        .balance(EconomicActorId(200), GOOD_FOOD)
        .available;
    assert!(
        buyer_after_1.0 > buyer_before.0,
        "goods flowed on interval 0"
    );

    for k in 1..5u64 {
        run_flow(&mut s, k * 10).unwrap();
        assert_eq!(
            last_price(&s.market_goods, MarketId(1)),
            Money(500),
            "still pinned"
        );
        assert_eq!(
            last_price(&s.market_goods, MarketId(2)),
            Money(2000),
            "still pinned"
        );
    }
}

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
    assert_eq!(
        seller_before.0 - seller_after.0,
        moved,
        "same q left surplus"
    );

    let cross: Vec<_> = s
        .ledger
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::MacroFlow {
                from_market,
                to_market,
                qty,
                ..
            } if from_market != to_market => Some((*from_market, *to_market, qty.0)),
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
    inventory
        .deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    supply
        .0
        .insert(EconomicActorId(100), sp(100, m_b, 200, 500)); // cheap supply at B
    accounts
        .deposit(EconomicActorId(200), Money(1_000_000_000))
        .unwrap();
    demand
        .0
        .insert(EconomicActorId(200), dp(200, m_a, 200, 2000)); // dear demand at A

    let mut distances = MarketDistances(BTreeMap::new());
    distances.0.insert((m_a, m_b), 4);
    distances.0.insert((m_b, m_a), 4);
    let config = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };

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
            EconomyEvent::MacroFlow {
                from_market,
                to_market,
                ..
            } if from_market != to_market => Some((*from_market, *to_market)),
            _ => None,
        })
        .collect();
    assert_eq!(
        cross,
        vec![(MarketId(2), MarketId(1))],
        "direction reversed"
    );
}

// --- Task 17: Edge cases (each its own #[test]). ---

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
    inventory
        .deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    supply.0.insert(EconomicActorId(100), sp(100, m, 100, 500));
    accounts
        .deposit(EconomicActorId(200), Money(1_000_000_000))
        .unwrap();
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
        config: EconomyConfig {
            transport_cost_per_tile_unit: Money(50),
            ..Default::default()
        },
    };
    let money_before = s.accounts.total_money().unwrap();
    let good_before = s.inventory.total_good(GOOD_FOOD).unwrap();
    run_flow(&mut s, 0).unwrap();
    assert_eq!(s.accounts.total_money().unwrap(), money_before);
    assert_eq!(s.inventory.total_good(GOOD_FOOD).unwrap(), good_before);
    assert!(
        s.ledger.0.iter().all(|e| match e {
            EconomyEvent::MacroFlow {
                from_market,
                to_market,
                ..
            } => from_market == to_market,
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
        s.accounts.account(TRANSPORT_OPERATOR).available,
        op_before,
        "zero distance → zero transport"
    );
}

#[test]
fn overflow_edge_is_pruned_not_faulted() {
    // A pathological distance forces the net_gain transport term to overflow i128
    // → the edge is PRUNED in STEP D (no candidate, no event, no panic). Books
    // unchanged. dist = i64::MAX with a large qty overflows transport_cost.
    let mut s = surplus_deficit_scenario(
        1,
        1_000_000,
        500,
        1,
        1_000_000,
        2000,
        i64::MAX,
        Money(i64::MAX),
    );
    let before = (s.accounts.clone(), s.inventory.clone());
    run_flow(&mut s, 0).expect("gate overflow is pruned, never an Err");
    assert_eq!(s.accounts, before.0, "pruned edge leaves books unchanged");
    assert_eq!(s.inventory, before.1);
    assert!(
        s.ledger
            .0
            .iter()
            .all(|e| !matches!(e, EconomyEvent::MacroFlow { .. })),
        "no MacroFlow event for a pruned edge"
    );
    assert!(
        s.ledger
            .0
            .iter()
            .all(|e| !matches!(e, EconomyEvent::MarketClearFailed { .. })),
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
    assert_eq!(
        s.accounts.total_money().unwrap(),
        money_before,
        "conserves with floored transport"
    );
    assert_eq!(s.inventory.total_good(GOOD_FOOD).unwrap(), good_before);
    assert_eq!(
        s.accounts.account(TRANSPORT_OPERATOR).available,
        op_before,
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
    inventory
        .deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    supply
        .0
        .insert(EconomicActorId(100), sp(100, m_a, 100, 500));
    accounts
        .deposit(EconomicActorId(200), Money(1_000_000_000))
        .unwrap();
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
        config: EconomyConfig {
            transport_cost_per_tile_unit: Money(50),
            ..Default::default()
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
    assert!(
        moved <= 50,
        "per-interval export bounded by offered_qty_per_tick (50), got {moved}"
    );
    assert!(moved > 0, "but it does export up to the cap");
}

#[test]
fn poisoning_market_does_not_abort_others() {
    // Spec §8 test #10. ONE edge faults at SETTLEMENT time; the system emits a
    // single `MarketClearFailed` for it, skips it (its scratch clone is discarded
    // so the live books are byte-identical for that edge), and the unrelated
    // healthy pair still commits + emits a `MacroFlow`. Conservation holds across
    // the whole tick.
    //
    // FAULT MECHANISM (spec §3 STEP H, line 121; §8 test #10, line 212): the
    // sketch of a *cash* over-charge — a deficit buyer affording the first import
    // but not the second — is PROVABLY UNREACHABLE here. STEP A
    // (`build_macro_buckets`) caps each buyer's effective demand to
    // `affordable_qty(cash, p_dst)`, and per accepted import edge `settle_flow`
    // charges `dst_payment = src_revenue + transport = p_src*q + transport`, NOT
    // `p_dst*q`. Since the STEP-D transport gate guarantees
    // `transport < (p_dst - p_src)*q`, summing a buyer's edges gives
    // `total_charge < p_dst * effective_demand <= cash`, so `lock_cash` never
    // under-funds — an exhaustive >1.5M-config sweep produced zero cash faults.
    // The genuine settle-time fault STEP H isolates is therefore a *checked-op*
    // fault: pre-seed the faulting sink's `traded_qty_last_tick` accumulator at
    // `i64::MAX` so the import's `write_back` overflows `Quantity::checked_add`
    // and that edge errors with `Overflow`. The unrelated healthy import commits.
    use crate::economy::MarketGoodState;

    // Healthy pair A(cheap surplus) -> B(dear deficit).
    let (m_a, m_b) = (MarketId(1), MarketId(2));
    // Poisoned pair C(cheap surplus) -> D(dear deficit); D's accumulator is at i64::MAX.
    let (m_c, m_d) = (MarketId(3), MarketId(4));
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();

    // Surpluses A and C: 200 each @ ask 500.
    inventory
        .deposit(EconomicActorId(100), GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    supply
        .0
        .insert(EconomicActorId(100), sp(100, m_a, 200, 500));
    inventory
        .deposit(EconomicActorId(101), GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    supply
        .0
        .insert(EconomicActorId(101), sp(101, m_c, 200, 500));
    // Deficit buyers B and D: 200 each @ bid 2000, fully funded.
    accounts
        .deposit(EconomicActorId(200), Money(1_000_000_000))
        .unwrap();
    demand
        .0
        .insert(EconomicActorId(200), dp(200, m_b, 200, 2000));
    accounts
        .deposit(EconomicActorId(201), Money(1_000_000_000))
        .unwrap();
    demand
        .0
        .insert(EconomicActorId(201), dp(201, m_d, 200, 2000));

    let mut distances = MarketDistances(BTreeMap::new());
    for (x, y) in [(m_a, m_b), (m_b, m_a), (m_c, m_d), (m_d, m_c)] {
        // dist 2, rate 50, q 200 -> transport (50*200/1000)*2 = 20 > 0 (real transfer).
        distances.0.insert((x, y), 2);
    }

    // Pre-seed D's market-good accumulator at i64::MAX so the C->D import's
    // write_back overflows `Quantity::checked_add` -> settle Err(Overflow) -> the
    // C->D edge faults. A->B is untouched and commits.
    let mut market_goods = MarketGoods::default();
    let dkey = MarketGoodKey {
        market: m_d,
        good: GOOD_FOOD,
    };
    let mut dstate = MarketGoodState::new(dkey);
    dstate.traded_qty_last_tick = Quantity(i64::MAX);
    dstate.last_settlement_price = Money(1_000);
    market_goods.0.insert(dkey, dstate);

    let mut s = DormantScenario {
        accounts,
        inventory,
        ledger: TradeLedger::default(),
        demand,
        supply,
        market_goods,
        dirty: crate::economy::DirtyMarketGoods::default(),
        dormant: [m_a, m_b, m_c, m_d].into_iter().collect(),
        distances,
        config: EconomyConfig {
            transport_cost_per_tile_unit: Money(50),
            ..Default::default()
        },
    };
    let money_before = s.accounts.total_money().unwrap();
    let good_before = s.inventory.total_good(GOOD_FOOD).unwrap();
    let a_seller = EconomicActorId(100);
    let b_buyer = EconomicActorId(200);
    let c_seller = EconomicActorId(101);
    let d_buyer = EconomicActorId(201);
    let a_before = s.inventory.balance(a_seller, GOOD_FOOD).available;
    let b_before = s.inventory.balance(b_buyer, GOOD_FOOD).available;
    let c_before = s.inventory.balance(c_seller, GOOD_FOOD).available;
    let d_before = s.inventory.balance(d_buyer, GOOD_FOOD).available;
    let d_cash_before = s.accounts.account(d_buyer).available;

    run_flow(&mut s, 0).unwrap();

    // Conservation holds across the whole tick (the faulted edge left the books
    // byte-identical for itself; the healthy edge committed atomically).
    assert_eq!(
        s.accounts.total_money().unwrap(),
        money_before,
        "money conserved"
    );
    assert_eq!(
        s.inventory.total_good(GOOD_FOOD).unwrap(),
        good_before,
        "goods conserved"
    );
    // The healthy A->B pair flowed: B received the same q that left A.
    let b_moved = s.inventory.balance(b_buyer, GOOD_FOOD).available.0 - b_before.0;
    assert!(b_moved > 0, "healthy A->B pair flowed");
    assert_eq!(
        a_before.0 - s.inventory.balance(a_seller, GOOD_FOOD).available.0,
        b_moved,
        "same q left surplus A as arrived at deficit B"
    );
    assert!(
        s.ledger
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::MacroFlow { .. })),
        "the healthy edge emitted a MacroFlow"
    );
    // The faulted C->D edge moved NOTHING: D's seller/buyer books are byte-identical.
    assert_eq!(
        s.inventory.balance(c_seller, GOOD_FOOD).available,
        c_before,
        "faulted edge: surplus C untouched"
    );
    assert_eq!(
        s.inventory.balance(d_buyer, GOOD_FOOD).available,
        d_before,
        "faulted edge: deficit D received nothing"
    );
    assert_eq!(
        s.accounts.account(d_buyer).available,
        d_cash_before,
        "faulted edge: deficit D buyer never spent"
    );
    // Exactly one MarketClearFailed — for the faulting C->D edge, and no other.
    assert_eq!(
        s.ledger
            .0
            .iter()
            .filter(|e| matches!(e, EconomyEvent::MarketClearFailed { .. }))
            .count(),
        1,
        "the poisoning edge faults exactly once; the healthy edge does not"
    );
}

#[tokio::test]
async fn macro_flow_emits_auditable_events() {
    use crate::economy::audit::{LedgerAuditCursor, commit_ledger_audit, pending_ledger_audit};
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
        s.ledger
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::MacroFlow { .. })),
        "flow produced at least one MacroFlow event"
    );

    let mut world = World::new();
    world.insert_resource(Tick(0));
    world.insert_resource(s.ledger.clone());
    world.insert_resource(LedgerAuditCursor(0));

    let (tick, pending) = pending_ledger_audit(&world);
    assert_eq!(tick, 0);
    assert!(!pending.is_empty());

    let mut store = InMemoryEconomyEventStore::default();
    store.append("w", tick, &pending).await.unwrap();
    commit_ledger_audit(&mut world, pending.len());
    assert!(
        pending_ledger_audit(&world).1.is_empty(),
        "cursor advanced past appended events"
    );

    let stored = store.events("w");
    let mf = stored
        .iter()
        .find_map(|(t, e)| match e {
            EconomyEvent::MacroFlow {
                from_market,
                to_market,
                ..
            } => Some((*t, *from_market, *to_market)),
            _ => None,
        })
        .expect("a MacroFlow row survived the store round-trip");
    assert_eq!(
        mf,
        (0, MarketId(1), MarketId(2)),
        "tick + from/to round-trip via serde jsonb"
    );
}

#[test]
fn macro_flow_replays_across_restart() {
    use crate::economy::{apply_into_world, extract_from_world};

    // Build a wired world with two asleep-anchored markets that flow; reuse the
    // lod-style assembly inline (macro_flow_world has no anchors). We anchor here.
    fn wired_flow_world() -> (World, bevy_ecs::schedule::Schedule) {
        let world = macro_flow_world();
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
            .deposit(EconomicActorId(50), GOOD_FOOD, Quantity(1_000_000))
            .unwrap();
        w2.resource_mut::<AccountBook>()
            .deposit(EconomicActorId(60), Money(1_000_000_000))
            .unwrap();
        w2.resource_mut::<SupplyPools>().0.insert(
            EconomicActorId(50),
            SupplyPool {
                actor: EconomicActorId(50),
                market: m_a,
                good: GOOD_FOOD,
                offered_qty_per_tick: Quantity(200),
                min_price: Money(500),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
        w2.resource_mut::<DemandPools>().0.insert(
            EconomicActorId(60),
            DemandPool {
                actor: EconomicActorId(60),
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
        w2.resource_mut::<MarketChunks>()
            .0
            .insert(m_a, ChunkCoord { x: 5, y: 5 });
        w2.resource_mut::<MarketChunks>()
            .0
            .insert(m_b, ChunkCoord { x: 9, y: 5 });
        let mut dist = MarketDistances(BTreeMap::new());
        dist.0.insert((m_a, m_b), 4);
        dist.0.insert((m_b, m_a), 4);
        w2.insert_resource(dist);
        w2.resource_mut::<EconomyConfig>()
            .transport_cost_per_tile_unit = Money(50);
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
    // EconomyConfig is intentionally NOT persisted (design spec: static tuning,
    // default-constructed by EconomyPlugin::install each boot). A real restart
    // re-applies the operator's static config to the freshly-installed world, so
    // mirror the live world's rate here — otherwise the macro flow would charge
    // the default rate (5) and diverge from the rate-50 continuation.
    restart
        .resource_mut::<EconomyConfig>()
        .transport_cost_per_tile_unit = Money(50);
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
    assert_eq!(
        tail(&world),
        tail(&restart),
        "ledger tail identical across restart"
    );
}

#[test]
fn drain_active_residual_defaults_off() {
    // S1 lands the config surface dark: S1+S2 must not change behavior, so the
    // drain flag is FALSE by default; S3 flips it. This guards that safety property.
    assert!(!EconomyConfig::default().drain_active_residual);
}

#[test]
fn macro_flow_threads_orderbook_and_counter_unchanged() {
    // S1 behavior-neutral threading: a populated OrderBook + a non-zero NextOrderId
    // ride through the macro-flow atomic boundary UNTOUCHED, while the dormant flow
    // still executes its cross-edge. Guards the clone topology S3 will mutate.
    let mut s = surplus_deficit_scenario(1, 200, 500, 1, 200, 2000, 4, Money(50));

    // Arbitrary residual orders the dormant-flow path must ignore entirely in S1.
    let mut orders = crate::economy::OrderBook::default();
    orders.bids.insert(
        crate::economy::OrderId(7),
        crate::economy::Bid {
            id: crate::economy::OrderId(7),
            owner: EconomicActorId(99),
            market: MarketId(9_001),
            good: GOOD_FOOD,
            qty_remaining: Quantity(5),
            max_price: Money(1_500),
            cash_locked_remaining: Money(7_500),
            created_tick: 0,
            expires_tick: 100,
        },
    );
    orders.asks.insert(
        crate::economy::OrderId(8),
        crate::economy::Ask {
            id: crate::economy::OrderId(8),
            owner: EconomicActorId(98),
            market: MarketId(9_002),
            good: GOOD_FOOD,
            qty_remaining: Quantity(5),
            min_price: Money(400),
            goods_locked_remaining: Quantity(5),
            created_tick: 0,
            expires_tick: 100,
        },
    );
    let mut next_oid = crate::economy::NextOrderId(42);

    let orders_before = orders.clone();
    let oid_before = next_oid;

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
        0,
        &mut crate::economy::FlowShipments::default(),
        &mut crate::economy::NextShipmentId::default(),
        &mut orders,
        &mut next_oid,
    )
    .unwrap();

    assert_eq!(
        orders, orders_before,
        "OrderBook must round-trip unchanged in S1"
    );
    assert_eq!(
        next_oid, oid_before,
        "NextOrderId must round-trip unchanged in S1"
    );
    assert!(
        s.ledger
            .0
            .iter()
            .any(|e| matches!(e, crate::economy::EconomyEvent::MacroFlow { .. })),
        "the dormant flow still executed its cross-edge while the OrderBook was carried through"
    );
}

/// S3 active-bucket constructor: one entry per residual order (DECISION-1).
/// `buyers`: (actor, qty_remaining, order_id, max_price); `sellers`: (actor, qty_remaining, order_id).
fn active_bucket(
    price: Money,
    buyers: Vec<(u64, i64, u64, i64)>,
    sellers: Vec<(u64, i64, u64)>,
) -> MacroBucket {
    MacroBucket {
        price,
        buyers: buyers
            .iter()
            .map(|(a, q, _, _)| (EconomicActorId(*a), *q))
            .collect(),
        sellers: sellers
            .iter()
            .map(|(a, q, _)| (EconomicActorId(*a), *q))
            .collect(),
        intra_cleared: true,
        buyer_orders: buyers.iter().map(|(_, _, o, _)| OrderId(*o)).collect(),
        buyer_max_prices: buyers.iter().map(|(_, _, _, mp)| Money(*mp)).collect(),
        seller_orders: sellers.iter().map(|(_, _, o)| OrderId(*o)).collect(),
    }
}

#[test]
fn build_candidates_suppresses_active_self_edge() {
    // A both-sided NON-CROSSING active market: residual bids (D) AND residual asks (S)
    // the auction left unmatched. D==S -> classify_bucket matched>0, but the self-edge
    // must be SUPPRESSED (re-clearing units the auction refused on price = double-clear).
    let key = MarketGoodKey {
        market: MarketId(1),
        good: GOOD_FOOD,
    };
    let mut active = BTreeMap::new();
    active.insert(
        key,
        active_bucket(Money(1_000), vec![(10, 5, 1, 900)], vec![(20, 5, 2)]),
    );
    let cands = build_candidates(
        &active,
        &MarketDistances(BTreeMap::new()),
        &EconomyConfig::default(),
    )
    .unwrap();
    assert!(
        cands.iter().all(|c| c.src != c.dst),
        "active bucket emits no self-edge"
    );

    // Control: the SAME quantities as a DORMANT bucket DO emit exactly one self-edge —
    // proving it is the intra_cleared flag (not the quantities) that suppressed it.
    let mut dormant = BTreeMap::new();
    dormant.insert(key, bucket(Money(1_000), vec![(10, 5)], vec![(20, 5)]));
    let dcands = build_candidates(
        &dormant,
        &MarketDistances(BTreeMap::new()),
        &EconomyConfig::default(),
    )
    .unwrap();
    assert_eq!(
        dcands.iter().filter(|c| c.src == c.dst).count(),
        1,
        "dormant bucket keeps its self-edge"
    );
}

#[test]
fn plan_flows_forces_zero_matched_for_active_bucket() {
    // Defense-in-depth: even if a self-edge Candidate reaches plan_flows for an active
    // bucket, the forced matched=0 budget makes it plan no flow.
    let key = MarketGoodKey {
        market: MarketId(1),
        good: GOOD_FOOD,
    };
    let mut active = BTreeMap::new();
    active.insert(
        key,
        active_bucket(Money(1_000), vec![(10, 5, 1, 900)], vec![(20, 5, 2)]),
    );
    let self_cand = Candidate {
        good: GOOD_FOOD,
        src: MarketId(1),
        dst: MarketId(1),
        q_cap: 5,
        p_src: Money(1_000),
        p_dst: Money(1_000),
        transport_total: Money::ZERO,
        net_gain: 0,
        dist: 0,
    };
    let flows = plan_flows(&[self_cand], &active);
    assert!(
        flows.is_empty(),
        "active bucket's forced matched=0 plans no self-flow"
    );
}

#[test]
fn prune_keeps_single_affordable_buyer() {
    let cfg = EconomyConfig::default();
    let r = prune_unaffordable_buyers(
        &[(EconomicActorId(1), 5)],
        &[OrderId(1)],
        &[Money(2_000)],
        5,
        0,
        Money(1_000),
        &cfg,
    )
    .unwrap();
    assert_eq!(r.buyers, vec![(EconomicActorId(1), 5)]);
    assert_eq!(r.orders, vec![OrderId(1)]);
    assert_eq!(r.q_prime, 5);
}

#[test]
fn prune_drops_single_unaffordable_buyer_to_empty() {
    // max_price 500 < landed p_dst 1000 -> value(500,5)=2 < charge 5 -> dropped.
    let cfg = EconomyConfig::default();
    let r = prune_unaffordable_buyers(
        &[(EconomicActorId(1), 5)],
        &[OrderId(1)],
        &[Money(500)],
        5,
        0,
        Money(1_000),
        &cfg,
    )
    .unwrap();
    assert!(r.buyers.is_empty());
    assert_eq!(
        r.q_prime, 0,
        "below-price buyer dropped -> q'==0 -> edge skipped"
    );
}

#[test]
fn prune_fixpoint_drops_unaffordable_then_stabilizes() {
    // 3 bids; C (max 500) unaffordable at p_dst 1000 -> dropped pass 1; A,B (max 2000)
    // stable pass 2 with the recomputed q' (15 -> 10 after C leaves).
    let cfg = EconomyConfig::default();
    let r = prune_unaffordable_buyers(
        &[
            (EconomicActorId(1), 5),
            (EconomicActorId(2), 5),
            (EconomicActorId(3), 5),
        ],
        &[OrderId(1), OrderId(2), OrderId(3)],
        &[Money(2_000), Money(2_000), Money(500)],
        15,
        0,
        Money(1_000),
        &cfg,
    )
    .unwrap();
    assert_eq!(
        r.buyers,
        vec![(EconomicActorId(1), 5), (EconomicActorId(2), 5)]
    );
    assert_eq!(r.orders, vec![OrderId(1), OrderId(2)]);
    assert_eq!(r.max_prices, vec![Money(2_000), Money(2_000)]);
    assert_eq!(r.q_prime, 10, "q' recomputed over survivors after the drop");
}

#[test]
fn prune_empty_input_is_empty() {
    let cfg = EconomyConfig::default();
    let r = prune_unaffordable_buyers(&[], &[], &[], 0, 0, Money(1_000), &cfg).unwrap();
    assert!(r.buyers.is_empty());
    assert_eq!(r.q_prime, 0);
}

#[test]
fn prune_is_deterministic() {
    let cfg = EconomyConfig::default();
    let run = || {
        prune_unaffordable_buyers(
            &[
                (EconomicActorId(1), 5),
                (EconomicActorId(2), 5),
                (EconomicActorId(3), 5),
            ],
            &[OrderId(1), OrderId(2), OrderId(3)],
            &[Money(2_000), Money(2_000), Money(500)],
            15,
            0,
            Money(1_000),
            &cfg,
        )
        .unwrap()
    };
    assert_eq!(run(), run());
}

#[test]
fn build_macro_buckets_active_sources_residual_orders_and_drops_below_price() {
    // ACTIVE (non-dormant) market m: two residual bids (one affordable @2000, one
    // below-price @1100 -> Stage-1 dropped) + one residual ask. The bucket is sourced
    // from qty_remaining (NOT available), priced at the auction's last_settlement_price
    // (authoritative, 1200 — distinct from the default ref 1000), intra_cleared=true.
    let m = MarketId(1);
    let key = MarketGoodKey {
        market: m,
        good: GOOD_FOOD,
    };
    let mut mg = MarketGoods::default();
    let mut state = MarketGoodState::new(key);
    state.last_settlement_price = Money(1_200);
    mg.0.insert(key, state);

    let mut orders = OrderBook::default();
    orders.bids.insert(
        OrderId(1),
        Bid {
            id: OrderId(1),
            owner: EconomicActorId(10),
            market: m,
            good: GOOD_FOOD,
            qty_remaining: Quantity(5),
            max_price: Money(2_000),
            cash_locked_remaining: Money(10),
            created_tick: 0,
            expires_tick: 100,
        },
    );
    orders.bids.insert(
        OrderId(3),
        Bid {
            id: OrderId(3),
            owner: EconomicActorId(11),
            market: m,
            good: GOOD_FOOD,
            qty_remaining: Quantity(4),
            max_price: Money(1_100), // below price 1200 -> Stage-1 dropped
            cash_locked_remaining: Money(4),
            created_tick: 0,
            expires_tick: 100,
        },
    );
    orders.asks.insert(
        OrderId(2),
        Ask {
            id: OrderId(2),
            owner: EconomicActorId(20),
            market: m,
            good: GOOD_FOOD,
            qty_remaining: Quantity(7),
            min_price: Money(400),
            goods_locked_remaining: Quantity(7),
            created_tick: 0,
            expires_tick: 100,
        },
    );

    let buckets = build_macro_buckets(
        &AccountBook::default(),
        &InventoryBook::default(),
        &DemandPools::default(),
        &SupplyPools::default(),
        &mg,
        &BTreeSet::new(),
        &EconomyConfig::default(),
        &orders,
        true,
    )
    .unwrap();

    let b = &buckets[&key];
    assert!(b.intra_cleared);
    assert_eq!(
        b.price,
        Money(1_200),
        "active price = auction last_settlement_price (not the default ref)"
    );
    assert_eq!(
        b.buyers,
        vec![(EconomicActorId(10), 5)],
        "below-price bid (1100<1200) dropped at Stage-1"
    );
    assert_eq!(b.buyer_orders, vec![OrderId(1)]);
    assert_eq!(b.buyer_max_prices, vec![Money(2_000)]);
    assert_eq!(
        b.sellers,
        vec![(EconomicActorId(20), 7)],
        "seller weight = qty_remaining (no available cap on the active path)"
    );
    assert_eq!(b.seller_orders, vec![OrderId(2)]);
}

#[test]
fn build_macro_buckets_flag_false_ignores_orders() {
    let m = MarketId(1);
    let key = MarketGoodKey {
        market: m,
        good: GOOD_FOOD,
    };
    let mut orders = OrderBook::default();
    orders.asks.insert(
        OrderId(2),
        Ask {
            id: OrderId(2),
            owner: EconomicActorId(20),
            market: m,
            good: GOOD_FOOD,
            qty_remaining: Quantity(7),
            min_price: Money(400),
            goods_locked_remaining: Quantity(7),
            created_tick: 0,
            expires_tick: 100,
        },
    );
    let buckets = build_macro_buckets(
        &AccountBook::default(),
        &InventoryBook::default(),
        &DemandPools::default(),
        &SupplyPools::default(),
        &MarketGoods::default(),
        &BTreeSet::new(),
        &EconomyConfig::default(),
        &orders,
        false, // flag OFF -> dark
    )
    .unwrap();
    assert!(
        !buckets.contains_key(&key),
        "flag off -> residual orders produce no active bucket"
    );
}

#[test]
fn write_back_preserves_active_price_but_updates_traded_and_residual() {
    let key = MarketGoodKey {
        market: MarketId(1),
        good: GOOD_FOOD,
    };
    let mut mg = MarketGoods::default();
    let mut state = MarketGoodState::new(key);
    state.last_settlement_price = Money(1_234); // auction-discovered, authoritative
    mg.0.insert(key, state);

    // preserve_price = true (active endpoint): price KEPT; traded/unmet/unsold updated.
    crate::economy::macro_flow::write_back(&mut mg, key, Money(999), 5, 3, 2, 7, true).unwrap();
    let s = &mg.0[&key];
    assert_eq!(
        s.last_settlement_price,
        Money(1_234),
        "active price preserved — auction authoritative"
    );
    assert_eq!(s.traded_qty_last_tick, Quantity(5));
    assert_eq!(s.unmet_demand_last_tick, Quantity(3));
    assert_eq!(s.unsold_supply_last_tick, Quantity(2));
    assert_eq!(s.last_cleared_tick, 7);

    // preserve_price = false (dormant endpoint): price OVERWRITTEN; traded accumulates.
    crate::economy::macro_flow::write_back(&mut mg, key, Money(999), 2, 1, 0, 8, false).unwrap();
    let s = &mg.0[&key];
    assert_eq!(
        s.last_settlement_price,
        Money(999),
        "dormant price overwritten by the flow"
    );
    assert_eq!(s.traded_qty_last_tick, Quantity(7), "5 + 2 accumulated");
    assert_eq!(s.unmet_demand_last_tick, Quantity(1));
}
