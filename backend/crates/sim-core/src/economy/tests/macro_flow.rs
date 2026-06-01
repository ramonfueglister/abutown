use std::collections::BTreeSet;

use crate::economy::macro_flow::synthetic_price;
use crate::economy::macro_flow::{MacroBucket, build_macro_buckets};
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyConfig, GOOD_FOOD, InventoryBook,
    MarketGoodKey, MarketGoods, MarketId, Money, Quantity, SettlementPolicy, SupplyPool,
    SupplyPools,
};

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
