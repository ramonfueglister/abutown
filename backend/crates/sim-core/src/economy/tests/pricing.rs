use crate::economy::pricing::run_adjust_reservation_prices_at_tick;
use crate::economy::systems::EconomyConfig;
use crate::economy::{
    DemandPool, DemandPools, EconomicActorId, GOOD_TOOLS, MarketGoodKey, MarketGoodState,
    MarketGoods, MarketId, Money, Quantity, SupplyPool, SupplyPools,
};
use std::collections::BTreeMap;

fn state(market: MarketId, unmet: i64, unsold: i64) -> MarketGoodState {
    let key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    let mut s = MarketGoodState::new(key);
    s.unmet_demand_last_tick = Quantity(unmet);
    s.unsold_supply_last_tick = Quantity(unsold);
    s
}
fn demand_pool(actor: u64, market: MarketId, max_price: i64) -> DemandPool {
    DemandPool {
        actor: EconomicActorId(actor),
        market,
        good: GOOD_TOOLS,
        desired_qty_per_tick: Quantity(10),
        max_price: Money(max_price),
        urgency_bps: 0,
        elasticity_bps: 0,
        interval_ticks: 1,
        last_generated_tick: None,
        last_consumed_tick: None,
        income_last_tick: Money::ZERO,
        mpc_bps: 8_000,
        autonomous: Money(5_000),
    }
}
fn supply_pool(actor: u64, market: MarketId, min_price: i64) -> SupplyPool {
    SupplyPool {
        actor: EconomicActorId(actor),
        market,
        good: GOOD_TOOLS,
        offered_qty_per_tick: Quantity(10),
        min_price: Money(min_price),
        interval_ticks: 1,
        last_generated_tick: None,
    }
}

#[test]
fn shortage_raises_glut_lowers_balanced_unchanged_both_walls_translate() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default(); // k=500, max_step=100, floor=1, ceiling=100_000
    let run = |unmet: i64, unsold: i64| {
        let mut d = DemandPools(BTreeMap::from([(
            EconomicActorId(1),
            demand_pool(1, m, 2_000),
        )]));
        let mut s = SupplyPools(BTreeMap::from([(
            EconomicActorId(2),
            supply_pool(2, m, 500),
        )]));
        let mut g = MarketGoods::default();
        g.0.insert(
            MarketGoodKey {
                market: m,
                good: GOOD_TOOLS,
            },
            state(m, unmet, unsold),
        );
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        (
            d.0[&EconomicActorId(1)].max_price.0,
            s.0[&EconomicActorId(2)].min_price.0,
        )
    };
    let (max_up, min_up) = run(100, 0);
    assert!(
        max_up > 2_000 && min_up > 500,
        "shortage raises both walls; got max={max_up} min={min_up}"
    );
    let (max_dn, min_dn) = run(0, 100);
    assert!(
        max_dn < 2_000 && min_dn < 500,
        "glut lowers both walls; got max={max_dn} min={min_dn}"
    );
    let (max_eq, min_eq) = run(50, 50);
    assert_eq!(
        (max_eq, min_eq),
        (2_000, 500),
        "no net imbalance → no nudge"
    );
    for (mx, mn) in [(max_up, min_up), (max_dn, min_dn)] {
        assert!(mn < mx, "min<max preserved");
    }
}

#[test]
fn step_is_speed_limited_regardless_of_signal_magnitude() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default(); // max_step=100 bps = 1%
    let mut d = DemandPools(BTreeMap::from([(
        EconomicActorId(1),
        demand_pool(1, m, 10_000),
    )]));
    let mut s = SupplyPools::default();
    let mut g = MarketGoods::default();
    g.0.insert(
        MarketGoodKey {
            market: m,
            good: GOOD_TOOLS,
        },
        state(m, 1_000_000, 0),
    );
    run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
    assert_eq!(
        d.0[&EconomicActorId(1)].max_price.0,
        10_100,
        "1%/interval cap binds for any huge imbalance"
    );
}

#[test]
fn guardrails_clamp_and_never_zero() {
    let m = MarketId(1);
    let cfg = EconomyConfig {
        price_ceiling: Money(2_010),
        ..EconomyConfig::default()
    };
    let mut d = DemandPools(BTreeMap::from([(
        EconomicActorId(1),
        demand_pool(1, m, 2_000),
    )]));
    let mut s = SupplyPools(BTreeMap::from([(
        EconomicActorId(2),
        supply_pool(2, m, 500),
    )]));
    let mut g = MarketGoods::default();
    g.0.insert(
        MarketGoodKey {
            market: m,
            good: GOOD_TOOLS,
        },
        state(m, 1_000, 0),
    );
    run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
    assert!(
        d.0[&EconomicActorId(1)].max_price.0 <= 2_010,
        "clamped to ceiling"
    );
    assert!(
        s.0[&EconomicActorId(2)].min_price.0 >= 1,
        "never below floor (>0)"
    );
}

#[test]
fn no_state_means_no_nudge_not_a_default() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default();
    let mut d = DemandPools(BTreeMap::from([(
        EconomicActorId(1),
        demand_pool(1, m, 2_000),
    )]));
    let mut s = SupplyPools::default();
    let g = MarketGoods::default(); // no state for (m, TOOLS)
    run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
    assert_eq!(
        d.0[&EconomicActorId(1)].max_price.0,
        2_000,
        "no signal → price unchanged (not defaulted)"
    );
}

#[test]
fn invalid_config_is_honest_err_no_silent_default() {
    let m = MarketId(1);
    let cfg = EconomyConfig {
        price_floor: Money(0),
        ..EconomyConfig::default()
    };
    let mut d = DemandPools(BTreeMap::from([(
        EconomicActorId(1),
        demand_pool(1, m, 2_000),
    )]));
    let mut s = SupplyPools::default();
    let mut g = MarketGoods::default();
    g.0.insert(
        MarketGoodKey {
            market: m,
            good: GOOD_TOOLS,
        },
        state(m, 100, 0),
    );
    assert!(
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).is_err(),
        "floor<=0 → Err"
    );
    assert_eq!(
        d.0[&EconomicActorId(1)].max_price.0,
        2_000,
        "no partial mutation on config Err"
    );
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
        let mut s = SupplyPools(BTreeMap::from([(
            EconomicActorId(5),
            supply_pool(5, m, 500),
        )]));
        let mut g = MarketGoods::default();
        g.0.insert(
            MarketGoodKey {
                market: m,
                good: GOOD_TOOLS,
            },
            state(m, 70, 10),
        );
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        (
            d.0[&EconomicActorId(9)].max_price.0,
            d.0[&EconomicActorId(2)].max_price.0,
            s.0[&EconomicActorId(5)].min_price.0,
        )
    };
    assert_eq!(run(), run());
}

#[test]
fn sustained_shortage_raises_price_monotonically_and_bounded_over_intervals() {
    use crate::economy::{
        DemandPools, EconomicActorId, GOOD_TOOLS, MarketGoodKey, MarketGoods, MarketId, SupplyPools,
    };
    let m = MarketId(1);
    let cfg = EconomyConfig::default();
    let mut d = DemandPools(std::collections::BTreeMap::from([(
        EconomicActorId(1),
        demand_pool(1, m, 2_000),
    )]));
    let mut s = SupplyPools::default();
    let mut g = MarketGoods::default();
    let key = MarketGoodKey {
        market: m,
        good: GOOD_TOOLS,
    };
    g.0.insert(key, state(m, 100, 0)); // sustained shortage every interval

    let mut prices = Vec::new();
    for _ in 0..8 {
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        prices.push(d.0[&EconomicActorId(1)].max_price.0);
    }
    for w in prices.windows(2) {
        assert!(
            w[1] >= w[0],
            "monotone rise under sustained shortage: {prices:?}"
        );
    }
    assert!(prices[0] > 2_000, "rose on the first interval");
    // Speed-limited: <=1%/interval, so after 8 intervals well under 2000*1.01^8 (~2165).
    assert!(
        *prices.last().unwrap() <= 2_000 + 2_000 * 9 / 100,
        "rise is speed-limited: {prices:?}"
    );
    assert!(
        *prices.last().unwrap() <= cfg.price_ceiling.0,
        "never exceeds ceiling"
    );

    // Glut variant: sustained unsold lowers the price monotonically, never below floor.
    let mut d2 = DemandPools(std::collections::BTreeMap::from([(
        EconomicActorId(1),
        demand_pool(1, m, 2_000),
    )]));
    let mut g2 = MarketGoods::default();
    g2.0.insert(key, state(m, 0, 100));
    let mut down = Vec::new();
    for _ in 0..8 {
        run_adjust_reservation_prices_at_tick(&mut d2, &mut SupplyPools::default(), &g2, &cfg)
            .unwrap();
        down.push(d2.0[&EconomicActorId(1)].max_price.0);
    }
    for w in down.windows(2) {
        assert!(w[1] <= w[0], "monotone fall under sustained glut: {down:?}");
    }
    assert!(
        *down.last().unwrap() >= cfg.price_floor.0,
        "never below floor"
    );
}

#[test]
fn cross_market_source_sink_gap_is_logged_and_stays_bounded() {
    // Pure-core model of the cross-market topology: a pure SOURCE market m_a (supply only →
    // post-flow glut → unsold>0) and a pure SINK market m_b (demand only → import shortfall →
    // unmet>0). Under the LOCAL-signal nudge: source min_price falls (glut), sink max_price
    // rises (shortage) — so the spatial gap WIDENS, NOT converges. This is the honest, spec-
    // disclosed limitation: we LOG the gap and assert ONLY boundedness, never convergence.
    use crate::economy::{
        DemandPools, EconomicActorId, GOOD_TOOLS, MarketGoodKey, MarketGoods, MarketId, SupplyPools,
    };
    let cfg = EconomyConfig::default();
    let m_a = MarketId(1); // source
    let m_b = MarketId(2); // sink
    let mut d = DemandPools(std::collections::BTreeMap::from([(
        EconomicActorId(1),
        demand_pool(1, m_b, 2_000),
    )]));
    let mut s = SupplyPools(std::collections::BTreeMap::from([(
        EconomicActorId(2),
        supply_pool(2, m_a, 500),
    )]));
    let mut g = MarketGoods::default();
    g.0.insert(
        MarketGoodKey {
            market: m_b,
            good: GOOD_TOOLS,
        },
        state(m_b, 100, 0),
    ); // sink: unmet
    g.0.insert(
        MarketGoodKey {
            market: m_a,
            good: GOOD_TOOLS,
        },
        state(m_a, 0, 100),
    ); // source: unsold

    for i in 0..6 {
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        let sink = d.0[&EconomicActorId(1)].max_price.0;
        let src = s.0[&EconomicActorId(2)].min_price.0;
        println!(
            "interval {i}: sink_max={sink} src_min={src} gap={}",
            sink - src
        );
        assert!(
            src >= cfg.price_floor.0 && src <= cfg.price_ceiling.0,
            "src in band"
        );
        assert!(
            sink >= cfg.price_floor.0 && sink <= cfg.price_ceiling.0,
            "sink in band"
        );
    }
    assert!(
        d.0[&EconomicActorId(1)].max_price.0 > 2_000,
        "sink price rose under shortage"
    );
    assert!(
        s.0[&EconomicActorId(2)].min_price.0 < 500,
        "source price fell under glut"
    );
}
