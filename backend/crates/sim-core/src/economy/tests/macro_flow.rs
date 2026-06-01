use std::collections::{BTreeMap, BTreeSet};

use crate::economy::MarketDistances;
use crate::economy::macro_flow::synthetic_price;
use crate::economy::macro_flow::{Candidate, MacroBucket, build_candidates, build_macro_buckets};
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyConfig, GOOD_FOOD, GoodId,
    InventoryBook, MarketGoodKey, MarketGoods, MarketId, Money, Quantity, SettlementPolicy,
    SupplyPool, SupplyPools,
};

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

    let buckets =
        build_macro_buckets(&accounts, &inventory, &demand, &supply, &mg, &dormant, &cfg).unwrap();
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
