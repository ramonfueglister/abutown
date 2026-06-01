use std::collections::{BTreeMap, BTreeSet};

use crate::economy::MarketDistances;
use crate::economy::macro_flow::synthetic_price;
use crate::economy::macro_flow::{Candidate, MacroBucket, build_candidates, build_macro_buckets};
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyConfig, GOOD_FOOD, InventoryBook,
    MarketGoodKey, MarketGoods, MarketId, Money, Quantity, SettlementPolicy, SupplyPool,
    SupplyPools,
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
    let mut cfg = EconomyConfig::default();
    cfg.transport_cost_per_tile_unit = Money(50);

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
    let mut cfg = EconomyConfig::default();
    cfg.transport_cost_per_tile_unit = Money(50);
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
    let mut cfg = EconomyConfig::default();
    cfg.transport_cost_per_tile_unit = Money(50);
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
    let mut cfg = EconomyConfig::default();
    cfg.transport_cost_per_tile_unit = Money(50);
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
