use crate::economy::pricing::run_adjust_reservation_prices_at_tick;
use crate::economy::systems::EconomyConfig;
use crate::economy::{
    DemandPool, DemandPools, EconomicActorId, GOOD_TOOLS, MarketGoodKey, MarketGoodState,
    MarketGoods, MarketId, Money, Quantity, SupplyPool, SupplyPools,
};
use std::collections::BTreeMap;

fn state(market: MarketId, unmet: i64, unsold: i64) -> MarketGoodState {
    let key = MarketGoodKey { market, good: GOOD_TOOLS };
    let mut s = MarketGoodState::new(key);
    s.unmet_demand_last_tick = Quantity(unmet);
    s.unsold_supply_last_tick = Quantity(unsold);
    s
}
fn demand_pool(actor: u64, market: MarketId, max_price: i64) -> DemandPool {
    DemandPool {
        actor: EconomicActorId(actor), market, good: GOOD_TOOLS,
        desired_qty_per_tick: Quantity(10), max_price: Money(max_price),
        urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
        last_generated_tick: None, last_consumed_tick: None,
        income_last_tick: Money::ZERO, mpc_bps: 8_000, autonomous: Money(5_000),
    }
}
fn supply_pool(actor: u64, market: MarketId, min_price: i64) -> SupplyPool {
    SupplyPool {
        actor: EconomicActorId(actor), market, good: GOOD_TOOLS,
        offered_qty_per_tick: Quantity(10), min_price: Money(min_price),
        interval_ticks: 1, last_generated_tick: None,
    }
}

#[test]
fn shortage_raises_glut_lowers_balanced_unchanged_both_walls_translate() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default(); // k=500, max_step=100, floor=1, ceiling=100_000
    let run = |unmet: i64, unsold: i64| {
        let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 2_000))]));
        let mut s = SupplyPools(BTreeMap::from([(EconomicActorId(2), supply_pool(2, m, 500))]));
        let mut g = MarketGoods::default();
        g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, unmet, unsold));
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        (d.0[&EconomicActorId(1)].max_price.0, s.0[&EconomicActorId(2)].min_price.0)
    };
    let (max_up, min_up) = run(100, 0);
    assert!(max_up > 2_000 && min_up > 500, "shortage raises both walls; got max={max_up} min={min_up}");
    let (max_dn, min_dn) = run(0, 100);
    assert!(max_dn < 2_000 && min_dn < 500, "glut lowers both walls; got max={max_dn} min={min_dn}");
    let (max_eq, min_eq) = run(50, 50);
    assert_eq!((max_eq, min_eq), (2_000, 500), "no net imbalance → no nudge");
    for (mx, mn) in [(max_up, min_up), (max_dn, min_dn)] { assert!(mn < mx, "min<max preserved"); }
}

#[test]
fn step_is_speed_limited_regardless_of_signal_magnitude() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default(); // max_step=100 bps = 1%
    let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 10_000))]));
    let mut s = SupplyPools::default();
    let mut g = MarketGoods::default();
    g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, 1_000_000, 0));
    run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
    assert_eq!(d.0[&EconomicActorId(1)].max_price.0, 10_100, "1%/interval cap binds for any huge imbalance");
}

#[test]
fn guardrails_clamp_and_never_zero() {
    let m = MarketId(1);
    let cfg = EconomyConfig { price_ceiling: Money(2_010), ..EconomyConfig::default() };
    let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 2_000))]));
    let mut s = SupplyPools(BTreeMap::from([(EconomicActorId(2), supply_pool(2, m, 500))]));
    let mut g = MarketGoods::default();
    g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, 1_000, 0));
    run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
    assert!(d.0[&EconomicActorId(1)].max_price.0 <= 2_010, "clamped to ceiling");
    assert!(s.0[&EconomicActorId(2)].min_price.0 >= 1, "never below floor (>0)");
}

#[test]
fn no_state_means_no_nudge_not_a_default() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default();
    let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 2_000))]));
    let mut s = SupplyPools::default();
    let g = MarketGoods::default(); // no state for (m, TOOLS)
    run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
    assert_eq!(d.0[&EconomicActorId(1)].max_price.0, 2_000, "no signal → price unchanged (not defaulted)");
}

#[test]
fn invalid_config_is_honest_err_no_silent_default() {
    let m = MarketId(1);
    let cfg = EconomyConfig { price_floor: Money(0), ..EconomyConfig::default() };
    let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 2_000))]));
    let mut s = SupplyPools::default();
    let mut g = MarketGoods::default();
    g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, 100, 0));
    assert!(run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).is_err(), "floor<=0 → Err");
    assert_eq!(d.0[&EconomicActorId(1)].max_price.0, 2_000, "no partial mutation on config Err");
}

#[test]
fn nudge_is_deterministic() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default();
    let run = || {
        let mut d = DemandPools(BTreeMap::from([
            (EconomicActorId(9), demand_pool(9, m, 2_000)),
            (EconomicActorId(2), demand_pool(2, m, 2_000)),
        ]));
        let mut s = SupplyPools(BTreeMap::from([(EconomicActorId(5), supply_pool(5, m, 500))]));
        let mut g = MarketGoods::default();
        g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, 70, 10));
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        (d.0[&EconomicActorId(9)].max_price.0, d.0[&EconomicActorId(2)].max_price.0, s.0[&EconomicActorId(5)].min_price.0)
    };
    assert_eq!(run(), run());
}
