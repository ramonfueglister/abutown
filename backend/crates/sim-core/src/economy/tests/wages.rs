use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::EconomyPlugin;
use crate::economy::auction::SettlementPolicy;
use crate::economy::macro_flow::PlannedFlow;
use crate::economy::{
    AccountBook, BuyerOutlays, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId,
    EconomyConfig, EconomyEvent, EconomyPersistSnapshot, GOOD_FOOD, GOOD_TOOLS, HOUSEHOLD_SECTOR,
    HouseholdSector, InputPools, InventoryBook, MarketChunks, MarketDistances, MarketGoodKey,
    MarketGoodState, MarketGoods, MarketId, Money, NextOrderId, OrderBook, ProducerPolicies,
    Quantity, SellerReceipts, SupplyPool, SupplyPools, TradeLedger, WageTelemetry,
    apply_into_world, clear_market_good_with_receipts, create_ask, create_bid, extract_from_world,
    run_pay_wages_at_tick, settle_flow_with_receipts,
};
use crate::ids::ChunkCoord;
use crate::mobility::resources::Tick;
use crate::world::components::{AsleepChunk, ChunkCoordComp};
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;

fn seeded_state(market: MarketId) -> MarketGoodState {
    MarketGoodState {
        key: MarketGoodKey {
            market,
            good: GOOD_FOOD,
        },
        last_settlement_price: Money(1_100),
        ewma_reference_price: Money(1_100),
        traded_qty_last_tick: Quantity(0),
        unmet_demand_last_tick: Quantity(0),
        unsold_supply_last_tick: Quantity(0),
        consumed_qty_last_tick: Quantity::ZERO,
        dirty: true,
        last_cleared_tick: 0,
    }
}

#[test]
fn auction_captures_seller_revenue_into_receipts() {
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let market = MarketId(1);
    let key = MarketGoodKey {
        market,
        good: GOOD_FOOD,
    };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_state(market));
    accounts.deposit(buyer, Money(10_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(2_000))
        .unwrap();
    create_bid(
        &mut accounts,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        buyer,
        market,
        GOOD_FOOD,
        Quantity(1_000),
        Money(1_500),
        10,
    )
    .unwrap();
    create_ask(
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        seller,
        market,
        GOOD_FOOD,
        Quantity(1_000),
        Money(1_000),
        10,
    )
    .unwrap();

    let before = accounts.total_money().unwrap();
    let mut receipts = SellerReceipts::default();
    let mut outlays = BuyerOutlays::default();
    clear_market_good_with_receipts(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        key,
        2,
        SettlementPolicy::Anchored,
        &mut receipts.0,
        &mut outlays.0,
    )
    .unwrap();

    assert_eq!(accounts.total_money().unwrap(), before, "money conserved");
    assert_eq!(
        receipts.0.get(&(seller, market)).copied(),
        Some(Money(1_100))
    );
    assert_eq!(
        receipts.0.get(&(buyer, market)).copied(),
        None,
        "buyers are not credited"
    );
}

#[test]
fn auction_no_fills_produces_no_receipts() {
    let market = MarketId(7);
    let key = MarketGoodKey {
        market,
        good: GOOD_FOOD,
    };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_state(market));
    let mut receipts = SellerReceipts::default();
    let mut outlays = BuyerOutlays::default();
    clear_market_good_with_receipts(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        key,
        1,
        SettlementPolicy::Anchored,
        &mut receipts.0,
        &mut outlays.0,
    )
    .unwrap();
    assert!(receipts.0.is_empty(), "no fills → no receipts");
}

#[test]
fn settle_flow_captures_seller_revenue_into_receipts() {
    let seller = EconomicActorId(2);
    let buyer = EconomicActorId(1);
    let src = MarketId(10);
    let dst = MarketId(11);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut goods = MarketGoods::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    let flow = PlannedFlow {
        good: GOOD_FOOD,
        src,
        dst,
        q: 10,
        p_src: Money(1_000),
        p_dst: Money(1_200),
        dist: 0,
    };
    let config = EconomyConfig::default();
    let before = accounts.total_money().unwrap();
    let mut receipts = SellerReceipts::default();
    let mut outlays = BuyerOutlays::default();
    settle_flow_with_receipts(
        &mut accounts,
        &mut inventory,
        &mut goods,
        &flow,
        &[(seller, 10)],
        &[(buyer, 10)],
        10,
        10,
        10,
        10,
        &config,
        1,
        false,
        false,
        &mut receipts.0,
        &mut outlays.0,
    )
    .unwrap();
    assert_eq!(accounts.total_money().unwrap(), before, "money conserved");
    // src_revenue = value(1_000, 10) = 1_000*10/ECONOMY_SCALE(=1_000) = 10
    assert_eq!(receipts.0.get(&(seller, src)).copied(), Some(Money(10)));
}

#[test]
fn auction_settle_records_buyer_outlay_at_actual_cost() {
    // Same setup as `auction_captures_seller_revenue_into_receipts`: 1 bid, 1 ask, 1 fill.
    // Settlement price is anchored to last_settlement_price=1_100 (clamped into [1_000,1_500]).
    // actual_cost = value(1_100, 1_000) = 1_100*1_000/ECONOMY_SCALE = 1_100.
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let market = MarketId(1);
    let key = MarketGoodKey {
        market,
        good: GOOD_FOOD,
    };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_state(market));
    accounts.deposit(buyer, Money(10_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(2_000))
        .unwrap();
    create_bid(
        &mut accounts,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        buyer,
        market,
        GOOD_FOOD,
        Quantity(1_000),
        Money(1_500),
        10,
    )
    .unwrap();
    create_ask(
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        seller,
        market,
        GOOD_FOOD,
        Quantity(1_000),
        Money(1_000),
        10,
    )
    .unwrap();

    let before = accounts.total_money().unwrap();
    let mut receipts = SellerReceipts::default();
    let mut outlays = BuyerOutlays::default();
    clear_market_good_with_receipts(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        key,
        2,
        SettlementPolicy::Anchored,
        &mut receipts.0,
        &mut outlays.0,
    )
    .unwrap();

    assert_eq!(accounts.total_money().unwrap(), before, "money conserved");
    let actual_cost = Money(1_100); // settlement=1_100, qty=1_000, value=1_100
    assert_eq!(
        receipts.0.get(&(seller, market)).copied(),
        Some(actual_cost),
        "seller receipt == actual_cost"
    );
    assert_eq!(
        outlays.0.get(&(buyer, market)).copied(),
        Some(actual_cost),
        "buyer outlay == actual_cost"
    );
    assert_eq!(
        outlays.0.get(&(seller, market)).copied(),
        None,
        "sellers are not charged as buyers"
    );
}

#[test]
fn flow_settle_records_buyer_outlays_including_transport() {
    // Same setup as `settle_flow_captures_seller_revenue_into_receipts` but with dist > 0
    // so transport_total > 0. dst_payment = src_revenue + transport_total.
    // Use q=200 so transport is non-zero:
    //   per_tile = checked_order_value(rate=5, qty=200) = 5*200/1_000 = 1
    //   transport = per_tile * dist = 1 * 20 = 20
    //   src_revenue = value(1_000, 200) = 1_000*200/1_000 = 200
    //   dst_payment = 200 + 20 = 220
    let seller = EconomicActorId(2);
    let buyer = EconomicActorId(1);
    let src = MarketId(10);
    let dst = MarketId(11);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut goods = MarketGoods::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(10_000))
        .unwrap();
    let flow = PlannedFlow {
        good: GOOD_FOOD,
        src,
        dst,
        q: 200,
        p_src: Money(1_000),
        p_dst: Money(1_200),
        dist: 20,
    };
    let config = EconomyConfig::default(); // transport_cost_per_tile_unit = Money(5)
    let before = accounts.total_money().unwrap();
    let mut receipts = SellerReceipts::default();
    let mut outlays = BuyerOutlays::default();
    settle_flow_with_receipts(
        &mut accounts,
        &mut inventory,
        &mut goods,
        &flow,
        &[(seller, 200)],
        &[(buyer, 200)],
        200,
        200,
        200,
        200,
        &config,
        1,
        false,
        false,
        &mut receipts.0,
        &mut outlays.0,
    )
    .unwrap();
    assert_eq!(accounts.total_money().unwrap(), before, "money conserved");
    // src_revenue = value(1_000, 200) = 200; transport = transport_cost(20, 200, 5) = 20
    // dst_payment = 200 + 20 = 220
    assert_eq!(receipts.0.get(&(seller, src)).copied(), Some(Money(200)));
    assert_eq!(
        outlays.0.get(&(buyer, dst)).copied(),
        Some(Money(220)),
        "buyer outlay == dst_payment (src_revenue + transport)"
    );
}

fn consumer_pool(actor: EconomicActorId, market: MarketId) -> DemandPool {
    DemandPool {
        actor,
        market,
        good: GOOD_TOOLS,
        desired_qty_per_tick: Quantity(10),
        max_price: Money(2_000),
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

fn fixture(
    firm_revenues: &[(EconomicActorId, MarketId, Money)],
    consumers: &[EconomicActorId],
) -> (
    AccountBook,
    SellerReceipts,
    DemandPools,
    HouseholdSector,
    EconomyConfig,
) {
    let mut accounts = AccountBook::default();
    let mut receipts = SellerReceipts::default();
    for (firm, market, rev) in firm_revenues {
        accounts.deposit(*firm, *rev).unwrap();
        let slot = receipts.0.entry((*firm, *market)).or_insert(Money::ZERO);
        *slot = slot.checked_add(*rev).unwrap();
    }
    let mut demand = DemandPools::default();
    let mut weights = BTreeMap::new();
    for c in consumers {
        demand.0.insert(*c, consumer_pool(*c, MarketId(9_002)));
        weights.insert(*c, 1);
    }
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: weights,
    };
    (
        accounts,
        receipts,
        demand,
        household,
        EconomyConfig::default(),
    )
}

#[test]
fn pay_wages_conserves_money_and_nets_sentinel_to_zero() {
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let c2 = EconomicActorId(8_012);
    let (mut accounts, receipts, mut demand, household, config) =
        fixture(&[(f1, MarketId(9_001), Money(1_000))], &[c1, c2]);
    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();

    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
    )
    .unwrap();

    assert_eq!(
        accounts.total_money().unwrap(),
        before,
        "byte-invariant total money"
    );
    assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "sentinel nets to zero"
    );
    assert_eq!(accounts.account(HOUSEHOLD_SECTOR).locked, Money::ZERO);
    // wage = 1000 * 6000/10000 = 600. firm keeps 400. consumers split 600 (300/300).
    assert_eq!(accounts.account(f1).available, Money(400));
    let inc: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
    assert_eq!(
        inc, 600,
        "Σ income == wage bill (== Σ firm→household transfers)"
    );
    assert_eq!(demand.0[&c1].income_last_tick, Money(300));
    assert_eq!(demand.0[&c2].income_last_tick, Money(300));
    assert_eq!(wage_tel.0.get(&MarketId(9_001)).copied(), Some(Money(600)));
}

#[test]
fn pay_wages_no_overdraft_and_income_equals_transfers() {
    let f1 = EconomicActorId(8_001);
    let f2 = EconomicActorId(8_011);
    let c1 = EconomicActorId(8_002);
    let (mut accounts, receipts, mut demand, household, config) = fixture(
        &[
            (f1, MarketId(9_001), Money(1_000)),
            (f2, MarketId(9_003), Money(500)),
        ],
        &[c1],
    );
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
    )
    .unwrap();
    assert!(accounts.account(f1).available.0 >= 0);
    assert!(accounts.account(f2).available.0 >= 0);
    // wage1=600, wage2=300 → Σ=900 all to the single consumer.
    assert_eq!(demand.0[&c1].income_last_tick, Money(900));
    let firms: Vec<EconomicActorId> = ledger
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::WagePaid { firm, .. } => Some(*firm),
            _ => None,
        })
        .collect();
    assert_eq!(firms, vec![f1, f2], "WagePaid emitted in ascending firm id");
}

#[test]
fn pay_wages_zero_receipts_is_noop() {
    let c1 = EconomicActorId(8_002);
    let (mut accounts, _r, mut demand, household, config) = fixture(&[], &[c1]);
    let receipts = SellerReceipts::default();
    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
    )
    .unwrap();
    assert_eq!(accounts.total_money().unwrap(), before);
    assert_eq!(demand.0[&c1].income_last_tick, Money::ZERO);
    assert!(wage_tel.0.is_empty());
}

#[test]
fn pay_wages_wage_bill_smaller_than_pools_floors_some_to_zero() {
    // wage_bill = floor(2 * 0.6) = 1, split across 3 equal pools → 1/0/0 (largest-remainder).
    let f1 = EconomicActorId(8_001);
    let (c1, c2, c3) = (
        EconomicActorId(8_002),
        EconomicActorId(8_012),
        EconomicActorId(8_022),
    );
    let (mut accounts, receipts, mut demand, household, config) =
        fixture(&[(f1, MarketId(9_001), Money(2))], &[c1, c2, c3]);
    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
    )
    .unwrap();
    assert_eq!(accounts.total_money().unwrap(), before);
    assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO);
    let total_income: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
    assert_eq!(
        total_income, 1,
        "Σ income == wage bill even when some pools floor to 0"
    );
    assert_eq!(
        demand.0[&c1].income_last_tick,
        Money(1),
        "lowest index wins the single unit"
    );
    assert_eq!(demand.0[&c2].income_last_tick, Money::ZERO);
}

#[test]
fn pay_wages_full_labor_share_pays_all_revenue() {
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let (mut accounts, receipts, mut demand, household, mut config) =
        fixture(&[(f1, MarketId(9_001), Money(1_000))], &[c1]);
    config.labor_share_bps = 10_000;
    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
    )
    .unwrap();
    assert_eq!(accounts.total_money().unwrap(), before);
    assert_eq!(
        accounts.account(f1).available,
        Money::ZERO,
        "labor_share=1.0 → firm pays all"
    );
    assert_eq!(demand.0[&c1].income_last_tick, Money(1_000));
}

#[test]
fn pay_wages_all_zero_weights_skips_first_leg() {
    // Σ weights == 0 ⇒ wage bill must NOT strand in the sentinel; first leg skipped.
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let (mut accounts, receipts, mut demand, mut household, config) =
        fixture(&[(f1, MarketId(9_001), Money(1_000))], &[c1]);
    household.pool_weights.insert(c1, 0);
    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
    )
    .unwrap();
    assert_eq!(accounts.total_money().unwrap(), before);
    assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "no strand"
    );
    assert_eq!(
        accounts.account(f1).available,
        Money(1_000),
        "firm keeps all (no payout target)"
    );
    assert_eq!(demand.0[&c1].income_last_tick, Money::ZERO);
}

#[test]
fn pay_wages_population_million_max_revenue_no_overflow() {
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let big = Money(i64::MAX / 2);
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, big).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, MarketId(9_001)), big);
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    let mut weights = BTreeMap::new();
    weights.insert(c1, 1);
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: weights,
    };
    let config = EconomyConfig::default();
    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
    )
    .unwrap();
    assert_eq!(
        accounts.total_money().unwrap(),
        before,
        "no mint under huge revenue"
    );
    assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO);
}

#[test]
fn pay_wages_weighted_split_zero_weight_pool_gets_nothing() {
    // Three pools with weights {c1:3, c2:1, c3:0}. Firm revenue 1_000 → wage 600 (at
    // labor_share_bps=6_000). Largest-remainder split of 600 across [3,1,0]:
    //   base = floor(600*3/4)=450, floor(600*1/4)=150, 0 → total=600, no remainder.
    // c3 has weight 0 → share MUST be zero; Σincome == wage_bill; sentinel nets to zero.
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let c2 = EconomicActorId(8_012);
    let c3 = EconomicActorId(8_022);
    let market = MarketId(9_001);

    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(1_000)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, market), Money(1_000));

    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    demand.0.insert(c2, consumer_pool(c2, MarketId(9_002)));
    demand.0.insert(c3, consumer_pool(c3, MarketId(9_002)));

    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 3), (c2, 1), (c3, 0)]),
    };
    let config = EconomyConfig::default(); // labor_share_bps = 6_000

    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();

    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
    )
    .unwrap();

    // money byte-invariant
    assert_eq!(
        accounts.total_money().unwrap(),
        before,
        "byte-invariant total money"
    );
    // HOUSEHOLD_SECTOR nets to zero
    assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "sentinel nets to zero"
    );
    // per-pool exact apportionment
    assert_eq!(
        demand.0[&c1].income_last_tick,
        Money(450),
        "c1 weight=3 → 450"
    );
    assert_eq!(
        demand.0[&c2].income_last_tick,
        Money(150),
        "c2 weight=1 → 150"
    );
    assert_eq!(
        demand.0[&c3].income_last_tick,
        Money::ZERO,
        "zero-weight pool gets nothing"
    );
    // Σincome == wage_bill
    let total_income: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
    assert_eq!(total_income, 600, "Σincome == wage_bill");
}

#[test]
fn pay_wages_property_conserves_over_random_inputs() {
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as i64
    };
    for _ in 0..400 {
        let labor = (next().rem_euclid(10_001)) as u16;
        let n_firms = 1 + next().rem_euclid(4);
        let n_pools = 1 + next().rem_euclid(3);
        let mut accounts = AccountBook::default();
        let mut receipts = SellerReceipts::default();
        let mut total_wage_expected: i64 = 0;
        for k in 0..n_firms {
            let firm = EconomicActorId(8_001 + k as u64 * 10);
            let rev = Money(next().rem_euclid(1_000_000));
            if rev.0 > 0 {
                accounts.deposit(firm, rev).unwrap();
                let slot = receipts
                    .0
                    .entry((firm, MarketId(9_000 + k as u32)))
                    .or_insert(Money::ZERO);
                *slot = slot.checked_add(rev).unwrap();
                total_wage_expected += (rev.0 as i128 * labor as i128 / 10_000) as i64;
            }
        }
        let mut demand = DemandPools::default();
        let mut weights = BTreeMap::new();
        for p in 0..n_pools {
            let c = EconomicActorId(8_002 + p as u64 * 10);
            demand.0.insert(c, consumer_pool(c, MarketId(9_002)));
            weights.insert(c, 1);
        }
        let household = HouseholdSector {
            population: 1_000_000,
            pool_weights: weights,
        };
        let config = EconomyConfig {
            labor_share_bps: labor,
            ..EconomyConfig::default()
        };
        let before = accounts.total_money().unwrap();
        let mut wage_tel = WageTelemetry::default();
        let mut ledger = TradeLedger::default();
        run_pay_wages_at_tick(
            &mut accounts,
            &receipts,
            &mut demand,
            &household,
            &mut wage_tel,
            &mut ledger,
            &config,
            &BuyerOutlays::default(),
        )
        .unwrap();
        assert_eq!(
            accounts.total_money().unwrap(),
            before,
            "money conserved (labor={labor})"
        );
        assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO);
        let inc: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
        assert_eq!(
            inc, total_wage_expected,
            "Σ income == Σ wages (labor={labor})"
        );
        for acc in accounts.accounts.values() {
            assert!(
                acc.available.0 >= 0 && acc.locked.0 >= 0,
                "no negative balance"
            );
        }
    }
}

// ── Full-tick schedule-level tests ────────────────────────────────────────────

/// Build a full CorePlugin+MobilityPlugin+EconomyPlugin world with a live schedule.
/// `Tick(0)` is inserted by MobilityPlugin. No markets or actors are seeded here;
/// callers add their fixtures. Mirrors the pattern from tests/macro_flow.rs.
fn full_economy_world() -> (World, bevy_ecs::schedule::Schedule) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);
    (world, schedule)
}

/// Run the schedule once and advance the tick counter (mirrors all wired tests).
fn tick_world(world: &mut World, schedule: &mut bevy_ecs::schedule::Schedule) {
    schedule.run(world);
    world.resource_mut::<Tick>().0 += 1;
}

/// Auction path: a supplier sells TOOLS via SupplyPool (active market → auction
/// path), a consumer has cash and a DemandPool at the same market (m=1), a
/// HouseholdSector paying the consumer. Run 6 ticks. Assert:
///   1. total_money byte-invariant (no mint, no burn).
///   2. HOUSEHOLD_SECTOR available == ZERO (sentinel cleared every tick).
///   3. consumer earned income (income_last_tick > 0 at least once over 6 ticks).
#[test]
fn full_tick_wage_loop_conserves_total_money_auction_path() {
    let (mut world, mut schedule) = full_economy_world();

    let supplier = EconomicActorId(8_001);
    let consumer = EconomicActorId(8_002);
    let market = MarketId(1);

    // Fund the supplier with goods and the consumer with cash.
    world
        .resource_mut::<crate::economy::InventoryBook>()
        .deposit(supplier, GOOD_TOOLS, Quantity(1_000_000))
        .unwrap();
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(1_000_000))
        .unwrap();

    // SupplyPool: supplier offers TOOLS at market 1.
    world.resource_mut::<SupplyPools>().0.insert(
        supplier,
        SupplyPool {
            actor: supplier,
            market,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    // DemandPool: consumer bids for TOOLS at market 1 (SAME market — so the
    // auction path can clear, capturing seller revenue into SellerReceipts).
    world
        .resource_mut::<DemandPools>()
        .0
        .insert(consumer, consumer_pool(consumer, market));

    // HouseholdSector pays the consumer.
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1)]),
    });

    // The market has NO MarketChunks entry, so it is never marked dormant and the
    // auction clears it every tick (the auction settle path → SellerReceipts → PayWages).
    world.resource_mut::<crate::economy::Markets>().0.insert(
        market,
        crate::economy::MarketSite {
            id: market,
            node_id: crate::routing::NodeId(0),
            name: "Test Market".to_string(),
        },
    );

    // Seed the opening reference price for the consumer's (market, good) so that
    // run_consumption_update_at_tick (which .expects a real price) does not panic.
    {
        let key = MarketGoodKey {
            market,
            good: GOOD_TOOLS,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let state = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        state.ewma_reference_price = Money(1_000);
        state.last_settlement_price = Money(1_000);
    }

    let before = world.resource::<AccountBook>().total_money().unwrap();

    let mut earned_income = false;
    for _ in 0..6 {
        tick_world(&mut world, &mut schedule);
        // Check sentinel clears every tick.
        assert_eq!(
            world
                .resource::<AccountBook>()
                .account(HOUSEHOLD_SECTOR)
                .available,
            Money::ZERO,
            "HOUSEHOLD_SECTOR must net to zero after every tick"
        );
        if world
            .resource::<DemandPools>()
            .0
            .get(&consumer)
            .map(|p| p.income_last_tick.0 > 0)
            .unwrap_or(false)
        {
            earned_income = true;
        }
    }

    assert_eq!(
        world.resource::<AccountBook>().total_money().unwrap(),
        before,
        "total_money byte-invariant over 6 ticks (auction path)"
    );
    assert!(earned_income, "consumer earned income at least once");

    // Non-vacuity: auction settle path must have emitted WagePaid events.
    let total_wages_paid: i64 = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::WagePaid { amount, .. } => Some(amount.0),
            _ => None,
        })
        .sum();
    assert!(
        total_wages_paid > 0,
        "auction settle path produced wages through PayWages"
    );
}

/// MacroFlow path: dormant supply@src / demand@dst pair — the auction never clears
/// it (both markets are anchored to AsleepChunk so they stay dormant), but
/// `run_macro_flow_at_tick` settles it each interval, crediting SellerReceipts →
/// PayWages. Mirrored verbatim from `macro_flow_replays_across_restart` fixture in
/// tests/macro_flow.rs (ChunkCoord-anchored AsleepChunk entities for each market).
#[test]
fn full_tick_macro_flow_feeds_pay_wages_and_conserves() {
    let (mut world, mut schedule) = full_economy_world();

    let supplier = EconomicActorId(50);
    let consumer = EconomicActorId(60);
    let m_src = MarketId(9_401);
    let m_dst = MarketId(9_402);
    let chunk_src = ChunkCoord { x: 5, y: 5 };
    let chunk_dst = ChunkCoord { x: 9, y: 5 };

    // Fund actors.
    world
        .resource_mut::<crate::economy::InventoryBook>()
        .deposit(supplier, GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(1_000_000_000))
        .unwrap();

    // Pools.
    world.resource_mut::<SupplyPools>().0.insert(
        supplier,
        SupplyPool {
            actor: supplier,
            market: m_src,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(200),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer,
            market: m_dst,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(200),
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

    // Anchor each market to an AsleepChunk → refresh_dormant_markets_system puts
    // both in DormantMarkets → macro-flow fires; auction does NOT fire for them.
    // (Mirrored exactly from macro_flow_replays_across_restart in macro_flow.rs.)
    world
        .resource_mut::<MarketChunks>()
        .0
        .insert(m_src, chunk_src);
    world
        .resource_mut::<MarketChunks>()
        .0
        .insert(m_dst, chunk_dst);

    let mut dist = MarketDistances(BTreeMap::new());
    dist.0.insert((m_src, m_dst), 4);
    dist.0.insert((m_dst, m_src), 4);
    world.insert_resource(dist);

    world
        .resource_mut::<EconomyConfig>()
        .transport_cost_per_tile_unit = Money(50);

    // Spawn AsleepChunk entities for each market chunk — these make
    // refresh_dormant_markets_system classify them as dormant.
    world.spawn((ChunkCoordComp(chunk_src), AsleepChunk));
    world.spawn((ChunkCoordComp(chunk_dst), AsleepChunk));

    // HouseholdSector paying the consumer.
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1)]),
    });

    // Seed the opening reference price for the consumer's (m_dst, GOOD_FOOD) so that
    // run_consumption_update_at_tick (which .expects a real price) does not panic.
    // The first UpdateConsumption runs at end of tick 0, before any macro-flow settle.
    {
        let key = MarketGoodKey {
            market: m_dst,
            good: GOOD_FOOD,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let state = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        state.ewma_reference_price = Money(1_000);
        state.last_settlement_price = Money(1_000);
    }

    let before = world.resource::<AccountBook>().total_money().unwrap();

    let macro_interval = world.resource::<EconomyConfig>().macro_flow_interval_ticks;
    let n_ticks = macro_interval + 2;

    for _ in 0..n_ticks {
        tick_world(&mut world, &mut schedule);
        // sentinel clears every tick
        assert_eq!(
            world
                .resource::<AccountBook>()
                .account(HOUSEHOLD_SECTOR)
                .available,
            Money::ZERO,
            "HOUSEHOLD_SECTOR nets to zero each tick"
        );
    }

    // money byte-invariant
    assert_eq!(
        world.resource::<AccountBook>().total_money().unwrap(),
        before,
        "total_money byte-invariant (macro-flow path)"
    );

    // no account ever negative
    for acct in world.resource::<AccountBook>().accounts.values() {
        assert!(
            acct.available.0 >= 0,
            "no account goes negative: {:?}",
            acct
        );
    }

    // Σ WagePaid amounts > 0: proves the MacroFlow path produced wages through PayWages.
    let total_wages_paid: i64 = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::WagePaid { amount, .. } => Some(amount.0),
            _ => None,
        })
        .sum();
    assert!(
        total_wages_paid > 0,
        "WagePaid events must be emitted via the MacroFlow settle path (got 0)"
    );
}

#[test]
fn closed_loop_bootstraps_from_autonomous_and_lags_one_tick() {
    // Tick 0: income=0 ⇒ desired_qty driven by autonomous only (set by UpdateConsumption
    // at end of tick 0) ⇒ subsequent ticks bid the autonomous floor ⇒ trade ⇒ wage ⇒
    // income>0. Asserts the loop is self-starting and conservative.
    let (mut world, mut schedule) = full_economy_world();
    let supplier = EconomicActorId(8_001);
    let consumer = EconomicActorId(8_002);
    let m = MarketId(1);
    world
        .resource_mut::<InventoryBook>()
        .deposit(supplier, GOOD_TOOLS, Quantity(1_000_000))
        .unwrap();
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(1_000_000))
        .unwrap();
    world.resource_mut::<SupplyPools>().0.insert(
        supplier,
        SupplyPool {
            actor: supplier,
            market: m,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(1_000),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    let mut pool = consumer_pool(consumer, m);
    pool.desired_qty_per_tick = Quantity(0);
    pool.good = GOOD_TOOLS;
    world.resource_mut::<DemandPools>().0.insert(consumer, pool);
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1_i64)]),
    });

    // Seed the opening reference price for the consumer's (m, GOOD_TOOLS) so that
    // run_consumption_update_at_tick (which .expects a real price) does not panic.
    {
        let key = MarketGoodKey {
            market: m,
            good: GOOD_TOOLS,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let state = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        state.ewma_reference_price = Money(1_000);
        state.last_settlement_price = Money(1_000);
    }

    let before = world.resource::<AccountBook>().total_money().unwrap();

    // Tick 0: UpdateConsumption sets desired_qty from autonomous (income still 0).
    schedule.run(&mut world);
    let dq0 = world.resource::<DemandPools>().0[&consumer]
        .desired_qty_per_tick
        .0;
    assert!(
        dq0 > 0,
        "autonomous term sets a positive desired_qty at tick 0 (bootstrap)"
    );

    let mut saw_income = false;
    for _ in 0..7 {
        schedule.run(&mut world);
        if world.resource::<DemandPools>().0[&consumer]
            .income_last_tick
            .0
            > 0
        {
            saw_income = true;
        }
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            before,
            "money conserved"
        );
        assert_eq!(
            world
                .resource::<AccountBook>()
                .account(HOUSEHOLD_SECTOR)
                .available,
            Money::ZERO
        );
    }
    assert!(
        saw_income,
        "the wage→income→consumption loop closed (income became positive)"
    );
}

#[test]
fn profit_and_rebate_event_type_tags_are_stable() {
    use crate::economy::{EconomicActorId, EconomyEvent, MarketId, Money};
    let p = EconomyEvent::ProfitDistributed {
        firm: EconomicActorId(8_001),
        market: MarketId(9_001),
        amount: Money(400),
    };
    assert_eq!(p.event_type(), "profit_distributed");
    let r = EconomyEvent::TransportRebate { amount: Money(123) };
    assert_eq!(r.event_type(), "transport_rebate");
}

#[test]
fn dividend_share_default_is_full_and_validates_bounds() {
    use crate::economy::{EconomyConfig, EconomyError};
    let cfg = EconomyConfig::default();
    assert_eq!(
        cfg.dividend_share_bps, 10_000,
        "default is full distribution"
    );
    assert_eq!(cfg.validated_dividend_share_bps().unwrap(), 10_000_i128);
    let bad = EconomyConfig {
        dividend_share_bps: 10_001,
        ..EconomyConfig::default()
    };
    assert_eq!(
        bad.validated_dividend_share_bps(),
        Err(EconomyError::InvalidOrder),
        "share > 10_000 is a config bug"
    );
    let zero = EconomyConfig {
        dividend_share_bps: 0,
        ..EconomyConfig::default()
    };
    assert_eq!(zero.validated_dividend_share_bps().unwrap(), 0_i128);
}

#[test]
fn determinism_same_snapshot_same_tick_yields_identical_desired_qty() {
    // Build a closed-loop world, run a few ticks to non-trivial income, snapshot it,
    // then run ONE more tick from the snapshot — twice, and across a serde round-trip —
    // and assert byte-identical desired_qty_per_tick per pool.
    fn build() -> (World, bevy_ecs::schedule::Schedule) {
        let (mut world, schedule) = full_economy_world();
        let supplier = EconomicActorId(8_001);
        let consumer = EconomicActorId(8_002);
        let m = MarketId(1);
        world
            .resource_mut::<InventoryBook>()
            .deposit(supplier, GOOD_TOOLS, Quantity(1_000_000))
            .unwrap();
        world
            .resource_mut::<AccountBook>()
            .deposit(consumer, Money(1_000_000))
            .unwrap();
        world.resource_mut::<SupplyPools>().0.insert(
            supplier,
            SupplyPool {
                actor: supplier,
                market: m,
                good: GOOD_TOOLS,
                offered_qty_per_tick: Quantity(1_000),
                min_price: Money(500),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
        let mut pool = consumer_pool(consumer, m);
        pool.good = GOOD_TOOLS;
        world.resource_mut::<DemandPools>().0.insert(consumer, pool);
        world.insert_resource(HouseholdSector {
            population: 1_000_000,
            pool_weights: BTreeMap::from([(consumer, 1_i64)]),
        });
        // Register the market so the auction settle path fires.
        world.resource_mut::<crate::economy::Markets>().0.insert(
            m,
            crate::economy::MarketSite {
                id: m,
                node_id: crate::routing::NodeId(0),
                name: "Test Market".to_string(),
            },
        );
        // Seed the opening reference price for the consumer's (m, GOOD_TOOLS) so that
        // run_consumption_update_at_tick (which .expects a real price) does not panic.
        {
            let key = MarketGoodKey {
                market: m,
                good: GOOD_TOOLS,
            };
            let mut goods = world.resource_mut::<MarketGoods>();
            let state = goods
                .0
                .entry(key)
                .or_insert_with(|| MarketGoodState::new(key));
            state.ewma_reference_price = Money(1_000);
            state.last_settlement_price = Money(1_000);
        }
        (world, schedule)
    }

    let (mut warm, mut warm_sched) = build();
    for _ in 0..5 {
        warm_sched.run(&mut warm);
    }
    let snap = extract_from_world(&warm);

    // Non-vacuity guard: the warm-up must have produced genuine wage income, otherwise
    // the determinism equality below would pass trivially on the pure-autonomous floor.
    let snap_income = snap
        .demand_pools
        .iter()
        .find(|(a, _)| *a == EconomicActorId(8_002))
        .map(|(_, p)| p.income_last_tick.0)
        .expect("consumer pool present in snapshot");
    assert!(
        snap_income > 0,
        "warm-up must produce non-zero wage income (got {snap_income}); else determinism is vacuous"
    );

    // Run one more tick from the snapshot, into a fresh fully-wired world.
    let run_one_more = |snap: &EconomyPersistSnapshot| -> BTreeMap<EconomicActorId, i64> {
        let mut world = World::new();
        let mut schedule = bevy_ecs::schedule::Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
        EconomyPlugin.install(&mut world, &mut schedule);
        apply_into_world(&mut world, snap);
        schedule.run(&mut world);
        world
            .resource::<DemandPools>()
            .0
            .iter()
            .map(|(a, p)| (*a, p.desired_qty_per_tick.0))
            .collect()
    };

    let a = run_one_more(&snap);
    let b = run_one_more(&snap);
    assert_eq!(a, b, "same snapshot + same tick → identical desired_qty");

    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let c = run_one_more(&decoded);
    assert_eq!(a, c, "serde round-trip preserves determinism");
}

#[test]
fn distribute_profit_conserves_money_and_drains_firm_to_zero() {
    use crate::economy::wages::run_distribute_profit_at_tick;
    use crate::economy::{EconomyConfig, EconomyError};
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let c2 = EconomicActorId(8_012);
    // Firm sold 1_000; PayWages already paid wage=600 and left 400 in the firm account.
    // Here we model the post-wage state: firm holds the 400 profit.
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(400)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, MarketId(9_001)), Money(1_000)); // gross revenue captured this tick
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    demand.0.insert(c2, consumer_pool(c2, MarketId(9_002)));
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1), (c2, 1)]),
    };
    let config = EconomyConfig::default(); // labor_share=6_000, dividend_share=10_000

    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
        &ProducerPolicies::default(),
        &InputPools::default(),
    )
    .unwrap();

    // wage = floor(1000*0.6)=600; profit = 1000-600 = 400; dividend = floor(400*1.0)=400.
    assert_eq!(
        accounts.total_money().unwrap(),
        before,
        "byte-invariant total money"
    );
    assert_eq!(
        accounts.account(f1).available,
        Money::ZERO,
        "full distribution drains the firm"
    );
    assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "sentinel nets to zero"
    );
    assert_eq!(demand.0[&c1].income_last_tick, Money(200));
    assert_eq!(demand.0[&c2].income_last_tick, Money(200));
    assert!(ledger.0.contains(&EconomyEvent::ProfitDistributed {
        firm: f1,
        market: MarketId(9_001),
        amount: Money(400),
    }));
    let _ = EconomyError::InvalidOrder; // import sanity
}

#[test]
fn distribute_profit_underfunded_firm_books_only_covered_and_audits() {
    use crate::economy::EconomyConfig;
    use crate::economy::wages::run_distribute_profit_at_tick;
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    // Receipts say revenue=1_000 (profit target = 400), but the firm only HOLDS 150
    // (it spent the rest buying inputs via macro_flow this tick). We must book 150 and
    // audit the shortfall — never panic, never silently skip.
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(150)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, MarketId(9_001)), Money(1_000));
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let config = EconomyConfig::default();

    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
        &ProducerPolicies::default(),
        &InputPools::default(),
    )
    .expect("the function itself returns Ok — the shortfall is surfaced via an audited event, not an Err");

    assert_eq!(
        accounts.total_money().unwrap(),
        before,
        "byte-invariant even under shortfall"
    );
    assert_eq!(
        accounts.account(f1).available,
        Money::ZERO,
        "all the firm HELD was distributed"
    );
    assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "sentinel nets to zero"
    );
    assert_eq!(
        demand.0[&c1].income_last_tick,
        Money(150),
        "only the covered amount reached the household"
    );
    // Audited shortfall event (MarketClearFailed-style), NOT a panic, NOT a silent drop.
    let audited = ledger.0.iter().any(|e| matches!(
        e,
        EconomyEvent::MarketClearFailed { market, reason, .. }
            if *market == MarketId(9_001) && *reason == crate::economy::EconomyError::InsufficientFunds
    ));
    assert!(
        audited,
        "an underfunded profit distribution must surface an audited event"
    );
    // The covered amount is still booked as ProfitDistributed.
    assert!(ledger.0.contains(&EconomyEvent::ProfitDistributed {
        firm: f1,
        market: MarketId(9_001),
        amount: Money(150),
    }));
}

#[test]
fn distribute_profit_zero_dividend_share_is_noop() {
    use crate::economy::EconomyConfig;
    use crate::economy::wages::run_distribute_profit_at_tick;
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(400)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, MarketId(9_001)), Money(1_000));
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let config = EconomyConfig {
        dividend_share_bps: 0,
        ..EconomyConfig::default()
    };
    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
        &ProducerPolicies::default(),
        &InputPools::default(),
    )
    .unwrap();
    assert_eq!(accounts.total_money().unwrap(), before);
    assert_eq!(
        accounts.account(f1).available,
        Money(400),
        "0 share retains profit at the firm"
    );
    assert_eq!(demand.0[&c1].income_last_tick, Money::ZERO);
}

#[test]
fn distribute_profit_does_not_reset_income() {
    // Profit distribution ADDS to income_last_tick (wages credited it first); it must not
    // zero it. Seed a non-zero income and assert it accumulates.
    use crate::economy::EconomyConfig;
    use crate::economy::wages::run_distribute_profit_at_tick;
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(400)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, MarketId(9_001)), Money(1_000));
    let mut demand = DemandPools::default();
    let mut pool = consumer_pool(c1, MarketId(9_002));
    pool.income_last_tick = Money(600); // wages already credited this tick
    demand.0.insert(c1, pool);
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let config = EconomyConfig::default();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
        &ProducerPolicies::default(),
        &InputPools::default(),
    )
    .unwrap();
    assert_eq!(
        demand.0[&c1].income_last_tick,
        Money(1_000),
        "wage 600 + dividend 400, accumulated"
    );
}

/// Builds the shared θ-dividend fixture: policy θ=8_000, batches_target=2, and an
/// InputPool with in_qty=1_000 @ max_price=300 → `wc_target = 300·2_000/1_000 = 600`
/// (settle-identical scale arithmetic, see `producers::wc_target`).
fn theta_policy_fixture(
    firm: EconomicActorId,
    market: MarketId,
) -> (crate::economy::ProducerPolicies, crate::economy::InputPools) {
    use crate::economy::producers::{InputPool, InputPools, ProducerPolicies, ProducerPolicy};
    let mut policies = ProducerPolicies::default();
    policies.0.insert(
        firm,
        ProducerPolicy {
            theta_bps: 8_000,
            batches_target: 2,
        },
    );
    let mut input_pools = InputPools::default();
    input_pools.0.insert(
        firm,
        InputPool {
            actor: firm,
            market,
            good: GOOD_FOOD,
            in_qty: Quantity(1_000),
            out_qty: Quantity(500),
            out_good: GOOD_TOOLS,
            interval_ticks: 1,
            last_generated_tick: None,
            max_price: Money(300),
        },
    );
    (policies, input_pools)
}

#[test]
fn dividend_theta_caps_at_working_capital_target() {
    // policy θ=8_000, batches=2 → wc_target=600; cash=800; revenue=1_000 →
    // wage=600, profit=400, intended=floor(400·0.8)=320 > cash − wc_target = 200
    // → dividend == cash − wc_target = 200 (Kappung greift), Firma behält wc_target.
    use crate::economy::EconomyConfig;
    use crate::economy::wages::run_distribute_profit_at_tick;
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let market = MarketId(9_001);
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(800)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, market), Money(1_000));
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let config = EconomyConfig::default(); // labor_share=6_000, dividend_share=10_000
    let (policies, input_pools) = theta_policy_fixture(f1, market);

    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
        &policies,
        &input_pools,
    )
    .unwrap();

    assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant");
    assert_eq!(
        accounts.account(f1).available,
        Money(600),
        "the firm retains exactly wc_target = 600"
    );
    assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "sentinel nets to zero"
    );
    assert_eq!(
        demand.0[&c1].income_last_tick,
        Money(200),
        "households receive the capped dividend cash − wc_target"
    );
    assert!(ledger.0.contains(&EconomyEvent::ProfitDistributed {
        firm: f1,
        market,
        amount: Money(200),
    }));
    // Policy capping is WANTED retention, not an anomaly — no audit event.
    assert!(
        !ledger
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::MarketClearFailed { .. })),
        "working-capital capping must NOT push an audited shortfall event"
    );
}

#[test]
fn dividend_zero_when_cash_below_target() {
    // cash=100 < wc_target=600 → distributable=0 → dividend == 0, kein Transfer,
    // kein Event (Aufbau des Liquiditätspuffers ist Policy-Verhalten). intended=320
    // exceeds even the firm's full cash — the policy still explains the retention.
    use crate::economy::EconomyConfig;
    use crate::economy::wages::run_distribute_profit_at_tick;
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let market = MarketId(9_001);
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(100)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, market), Money(1_000));
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let config = EconomyConfig::default();
    let (policies, input_pools) = theta_policy_fixture(f1, market);

    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
        &policies,
        &input_pools,
    )
    .unwrap();

    assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant");
    assert_eq!(
        accounts.account(f1).available,
        Money(100),
        "cash below wc_target → the firm retains everything (buffer build-up)"
    );
    assert_eq!(
        demand.0[&c1].income_last_tick,
        Money::ZERO,
        "no dividend transfer"
    );
    assert!(
        ledger.0.is_empty(),
        "no ProfitDistributed and no MarketClearFailed — retention is policy, got {:?}",
        ledger.0
    );
}

#[test]
fn actors_without_policy_distribute_like_before() {
    // f1 has NO ProducerPolicies / InputPools entry (another actor's entries must not
    // leak) → θ = config dividend_share (10_000), wc_target = 0 → byte-identical to
    // the #75 full-payout behavior: firm drains to zero.
    use crate::economy::EconomyConfig;
    use crate::economy::wages::run_distribute_profit_at_tick;
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let market = MarketId(9_001);
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(400)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, market), Money(1_000));
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let config = EconomyConfig::default();
    // Entries for a DIFFERENT firm: the lookup must miss for f1.
    let (policies, input_pools) = theta_policy_fixture(EconomicActorId(8_009), market);

    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
        &BuyerOutlays::default(),
        &policies,
        &input_pools,
    )
    .unwrap();

    // wage=600, profit=400, θ=10_000 → dividend=400: exactly the #75 behavior.
    assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant");
    assert_eq!(
        accounts.account(f1).available,
        Money::ZERO,
        "no policy → full distribution drains the firm (#75 regression)"
    );
    assert_eq!(demand.0[&c1].income_last_tick, Money(400));
    assert!(ledger.0.contains(&EconomyEvent::ProfitDistributed {
        firm: f1,
        market,
        amount: Money(400),
    }));
}

#[test]
fn mismatched_policy_pool_state_errors_loudly() {
    // ProducerPolicies and InputPools are only seeded/re-applied together. A one-sided
    // state is a silent config-revert (#83 class) that would otherwise drain working
    // capital silently. Both mismatch arms must return Err(InvalidOrder).
    use crate::economy::EconomyConfig;
    use crate::economy::wages::run_distribute_profit_at_tick;
    use crate::economy::producers::{InputPool, InputPools, ProducerPolicies, ProducerPolicy};
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let market = MarketId(9_001);

    let base_setup = || -> (AccountBook, SellerReceipts, DemandPools, HouseholdSector, EconomyConfig) {
        let mut accounts = AccountBook::default();
        accounts.deposit(f1, Money(400)).unwrap();
        let mut receipts = SellerReceipts::default();
        receipts.0.insert((f1, market), Money(1_000));
        let mut demand = DemandPools::default();
        demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
        let household = HouseholdSector {
            population: 1_000_000,
            pool_weights: BTreeMap::from([(c1, 1)]),
        };
        (accounts, receipts, demand, household, EconomyConfig::default())
    };

    // Case 1: policy present, input-pool absent → Err(InvalidOrder)
    {
        let (mut accounts, receipts, mut demand, household, config) = base_setup();
        let mut policies = ProducerPolicies::default();
        policies.0.insert(f1, ProducerPolicy { theta_bps: 8_000, batches_target: 2 });
        let input_pools = InputPools::default(); // no entry for f1
        let mut ledger = TradeLedger::default();
        let result = run_distribute_profit_at_tick(
            &mut accounts, &receipts, &mut demand, &household, &mut ledger,
            &config, &BuyerOutlays::default(), &policies, &input_pools,
        );
        assert_eq!(
            result,
            Err(crate::economy::EconomyError::InvalidOrder),
            "policy present but no InputPool → must fail fast (got {:?})", result
        );
    }

    // Case 2: input-pool present, policy absent → Err(InvalidOrder)
    {
        let (mut accounts, receipts, mut demand, household, config) = base_setup();
        let policies = ProducerPolicies::default(); // no entry for f1
        let mut input_pools = InputPools::default();
        input_pools.0.insert(
            f1,
            InputPool {
                actor: f1,
                market,
                good: GOOD_FOOD,
                in_qty: Quantity(1_000),
                out_qty: Quantity(500),
                out_good: GOOD_TOOLS,
                interval_ticks: 1,
                last_generated_tick: None,
                max_price: Money(300),
            },
        );
        let mut ledger = TradeLedger::default();
        let result = run_distribute_profit_at_tick(
            &mut accounts, &receipts, &mut demand, &household, &mut ledger,
            &config, &BuyerOutlays::default(), &policies, &input_pools,
        );
        assert_eq!(
            result,
            Err(crate::economy::EconomyError::InvalidOrder),
            "InputPool present but no policy → must fail fast (got {:?})", result
        );
    }
}

#[test]
fn full_tick_wage_profit_rebate_all_net_household_sector_to_zero() {
    // One world where a firm sells (auction path → revenue → wage + profit) AND transport
    // accrues (a dormant macro-flow pair → operator fee). Run past a macro-flow interval so
    // wage, profit, AND rebate all fire. Assert: total_money byte-invariant every tick AND
    // HOUSEHOLD_SECTOR == 0 after every tick (each of the three legs nets it independently).
    let (mut world, mut schedule) = full_economy_world();

    // Auction-path firm: supplier sells TOOLS at active market m1, consumer buys there.
    let supplier = EconomicActorId(8_001);
    let consumer = EconomicActorId(8_002);
    let m1 = MarketId(1);
    world
        .resource_mut::<InventoryBook>()
        .deposit(supplier, GOOD_TOOLS, Quantity(1_000_000))
        .unwrap();
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(10_000_000))
        .unwrap();
    world.resource_mut::<SupplyPools>().0.insert(
        supplier,
        SupplyPool {
            actor: supplier,
            market: m1,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<DemandPools>()
        .0
        .insert(consumer, consumer_pool(consumer, m1));
    world.resource_mut::<crate::economy::Markets>().0.insert(
        m1,
        crate::economy::MarketSite {
            id: m1,
            node_id: crate::routing::NodeId(0),
            name: "M1".to_string(),
        },
    );

    // Dormant macro-flow pair (GOOD_FOOD) → accrues a TRANSPORT_OPERATOR fee each interval.
    // Use ids in the 8_0xx labor band for consistency with the rest of the economy.
    let f_supplier = EconomicActorId(8_041);
    let f_consumer = EconomicActorId(8_042);
    let m_src = MarketId(9_401);
    let m_dst = MarketId(9_402);
    let chunk_src = ChunkCoord { x: 5, y: 5 };
    let chunk_dst = ChunkCoord { x: 9, y: 5 };
    world
        .resource_mut::<InventoryBook>()
        .deposit(f_supplier, GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    world
        .resource_mut::<AccountBook>()
        .deposit(f_consumer, Money(1_000_000_000))
        .unwrap();
    world.resource_mut::<SupplyPools>().0.insert(
        f_supplier,
        SupplyPool {
            actor: f_supplier,
            market: m_src,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(200),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<DemandPools>().0.insert(
        f_consumer,
        DemandPool {
            actor: f_consumer,
            market: m_dst,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(200),
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
    world
        .resource_mut::<MarketChunks>()
        .0
        .insert(m_src, chunk_src);
    world
        .resource_mut::<MarketChunks>()
        .0
        .insert(m_dst, chunk_dst);
    let mut dist = MarketDistances(BTreeMap::new());
    dist.0.insert((m_src, m_dst), 4);
    dist.0.insert((m_dst, m_src), 4);
    world.insert_resource(dist);
    world
        .resource_mut::<EconomyConfig>()
        .transport_cost_per_tile_unit = Money(50);
    world.spawn((ChunkCoordComp(chunk_src), AsleepChunk));
    world.spawn((ChunkCoordComp(chunk_dst), AsleepChunk));

    // Household pays BOTH consumers (the labor households).
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1), (f_consumer, 1)]),
    });

    // Seed opening prices for both consumer (market, good)s.
    for (mk, good) in [(m1, GOOD_TOOLS), (m_dst, GOOD_FOOD)] {
        let key = MarketGoodKey { market: mk, good };
        let mut goods = world.resource_mut::<MarketGoods>();
        let state = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        state.ewma_reference_price = Money(1_000);
        state.last_settlement_price = Money(1_000);
    }

    let before = world.resource::<AccountBook>().total_money().unwrap();
    let interval = world.resource::<EconomyConfig>().macro_flow_interval_ticks;

    // Run enough loop iterations that at least one rebate-boundary tick is reached. Because
    // tick_world DOUBLE-increments Tick (schedule's own tick_increment + the helper's +=1),
    // Tick.0 advances by 2 per iteration, so the loop counter is NOT Tick.0. We therefore
    // pin every boundary assertion against the REAL Tick.0 read inside the loop, never the
    // loop index. Run generously past one interval.
    let mut saw_operator_drained_on_boundary = false;
    for _ in 0..(interval as usize + 4) {
        tick_world(&mut world, &mut schedule);
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            before,
            "total_money byte-invariant across wage+profit+rebate legs"
        );
        assert_eq!(
            world
                .resource::<AccountBook>()
                .account(HOUSEHOLD_SECTOR)
                .available,
            Money::ZERO,
            "HOUSEHOLD_SECTOR nets to zero after the full tick (all three legs)"
        );
        // The rebate system gates on Tick.0 % interval == 0 (phase-locked to the operator
        // credit). On any such boundary tick the operator must read zero AFTER the tick.
        let now = world.resource::<crate::mobility::resources::Tick>().0;
        if interval != 0 && now % interval == 0 {
            assert_eq!(
                world
                    .resource::<AccountBook>()
                    .account(crate::economy::TRANSPORT_OPERATOR)
                    .available,
                Money::ZERO,
                "operator drained at the interval boundary Tick.0={now}"
            );
            saw_operator_drained_on_boundary = true;
        }
    }
    assert!(
        saw_operator_drained_on_boundary,
        "the run must cross at least one rebate boundary (Tick.0 multiple of {interval})"
    );

    // Non-vacuity: all three event kinds must have fired at least once.
    let ev = &world.resource::<TradeLedger>().0;
    assert!(
        ev.iter()
            .any(|e| matches!(e, EconomyEvent::WagePaid { .. })),
        "wages fired"
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, EconomyEvent::ProfitDistributed { .. })),
        "profit fired"
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, EconomyEvent::TransportRebate { .. })),
        "rebate fired"
    );
}

#[test]
fn nonzero_household_sentinel_is_release_grade_err() {
    use crate::economy::systems::EconomyConfig;
    use crate::economy::wages::{
        HOUSEHOLD_SECTOR, HouseholdSector, SellerReceipts, WageTelemetry, run_pay_wages_at_tick,
    };
    use crate::economy::{AccountBook, DemandPools, EconomyError, Money, TradeLedger};
    use std::collections::BTreeMap;
    // No receipts → no wage transfers → both legs skip; we pre-strand cash in the sentinel so
    // the net-zero check at the end is violated.
    let mut accounts = AccountBook::default();
    accounts.deposit(HOUSEHOLD_SECTOR, Money(123)).unwrap(); // stranded sentinel cash
    let receipts = SellerReceipts::default();
    let mut demand = DemandPools::default();
    let household = HouseholdSector {
        population: 1,
        pool_weights: BTreeMap::new(),
    };
    let mut wt = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    let cfg = EconomyConfig::default();
    let r = run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wt,
        &mut ledger,
        &cfg,
        &BuyerOutlays::default(),
    );
    assert_eq!(
        r,
        Err(EconomyError::ConservationViolation),
        "non-zero sentinel → release-grade Err, not a debug_assert"
    );
}

// ── Value-added wages tests ───────────────────────────────────────────────────

/// receipts[(firm, m)] = 1000, outlays[(firm, m)] = 400 → value_added = 600
/// → wage = floor(0.6 * 600) = 360 (not 600 as revenue-based would give)
#[test]
fn wage_basis_is_value_added_when_firm_bought_inputs() {
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let market = MarketId(9_001);
    let (mut accounts, receipts, mut demand, household, config) =
        fixture(&[(f1, market, Money(1_000))], &[c1]);
    // The firm also spent 400 buying inputs this tick → value_added = 1000 - 400 = 600
    let mut outlays = BuyerOutlays::default();
    outlays.0.insert((f1, market), Money(400));

    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &outlays,
    )
    .unwrap();

    // wage = floor(600 * 6000 / 10_000) = floor(360) = 360
    assert_eq!(
        accounts.total_money().unwrap(),
        before,
        "byte-invariant total money"
    );
    assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "sentinel nets to zero"
    );
    assert_eq!(
        accounts.account(f1).available,
        Money(640),
        "firm keeps revenue - wage = 1000 - 360 = 640"
    );
    let inc: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
    assert_eq!(inc, 360, "Σ income == value-added wage bill (360)");
}

/// receipts[(firm, m)] = 100, outlays[(firm, m)] = 400 → value_added = 0 (floored)
/// → wage = 0, no transfer at all
#[test]
fn negative_value_added_pays_zero_wage() {
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let market = MarketId(9_001);
    let (mut accounts, receipts, mut demand, household, config) =
        fixture(&[(f1, market, Money(100))], &[c1]);
    let mut outlays = BuyerOutlays::default();
    outlays.0.insert((f1, market), Money(400));

    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &outlays,
    )
    .unwrap();

    // value_added = max(0, 100 - 400) = 0 → wage = 0 → no transfer
    assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant");
    assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "sentinel nets to zero (no strand)"
    );
    assert_eq!(
        accounts.account(f1).available,
        Money(100),
        "firm keeps all revenue (no wage paid)"
    );
    let inc: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
    assert_eq!(inc, 0, "no income when value_added ≤ 0");
    // No WagePaid event must be emitted
    assert!(
        ledger
            .0
            .iter()
            .all(|e| !matches!(e, EconomyEvent::WagePaid { .. })),
        "no WagePaid event when wage == 0"
    );
}

/// Actors without outlays (e.g. pure extractors) must get the same wage as before:
/// wage = floor(revenue * labor_share). This is a regression guard.
#[test]
fn actors_without_outlays_unchanged() {
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let market = MarketId(9_001);
    let (mut accounts, receipts, mut demand, household, config) =
        fixture(&[(f1, market, Money(1_000))], &[c1]);
    // No outlays → value_added == revenue → wage identical to old behavior
    let outlays = BuyerOutlays::default();

    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &outlays,
    )
    .unwrap();

    // wage = floor(1000 * 6000 / 10_000) = 600 (unchanged from revenue-based)
    assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant");
    assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO);
    assert_eq!(
        accounts.account(f1).available,
        Money(400),
        "firm keeps 1000 - 600 = 400"
    );
    let inc: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
    assert_eq!(inc, 600, "Σ income == 600 (identical to old revenue-based)");
}

/// Two settles that hit the same (buyer, market) key must ACCUMULATE in BuyerOutlays,
/// not overwrite. This is a regression guard for entry().or_insert + checked_add vs. insert.
#[test]
fn outlays_accumulate_across_fills_for_same_buyer_market() {
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let market = MarketId(1);
    let key = MarketGoodKey {
        market,
        good: GOOD_FOOD,
    };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_state(market));
    accounts.deposit(buyer, Money(100_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(10_000))
        .unwrap();

    // Create two bid+ask pairs and clear twice, each time hitting (buyer, market).
    // First pair: qty=1_000, settlement ~1_100
    create_bid(
        &mut accounts,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        buyer,
        market,
        GOOD_FOOD,
        Quantity(1_000),
        Money(1_500),
        10,
    )
    .unwrap();
    create_ask(
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        seller,
        market,
        GOOD_FOOD,
        Quantity(1_000),
        Money(1_000),
        10,
    )
    .unwrap();

    let mut receipts = SellerReceipts::default();
    let mut outlays = BuyerOutlays::default();
    clear_market_good_with_receipts(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        key,
        2,
        SettlementPolicy::Anchored,
        &mut receipts.0,
        &mut outlays.0,
    )
    .unwrap();

    let first_outlay = outlays
        .0
        .get(&(buyer, market))
        .copied()
        .unwrap_or(Money::ZERO);

    // Second pair: qty=2_000
    create_bid(
        &mut accounts,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        3,
        buyer,
        market,
        GOOD_FOOD,
        Quantity(2_000),
        Money(1_500),
        10,
    )
    .unwrap();
    create_ask(
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        3,
        seller,
        market,
        GOOD_FOOD,
        Quantity(2_000),
        Money(1_000),
        10,
    )
    .unwrap();

    clear_market_good_with_receipts(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        key,
        4,
        SettlementPolicy::Anchored,
        &mut receipts.0,
        &mut outlays.0,
    )
    .unwrap();

    let total_outlay = outlays
        .0
        .get(&(buyer, market))
        .copied()
        .unwrap_or(Money::ZERO);

    // Settlement price derivation (Anchored, seeded_state sets last=1_100):
    //   Round 1: last=1_100 ∈ [ask=1_000, bid=1_500] → price = last = 1_100
    //            outlay = price * qty / SCALE = 1_100 * 1_000 / 1_000 = 1_100
    //   Round 2: last=1_100 ∈ [ask=1_000, bid=1_500] → price = last = 1_100
    //            outlay = 1_100 * 2_000 / 1_000 = 2_200
    // Accumulation must yield first + second = 1_100 + 2_200 = 3_300.
    // An insert/overwrite bug would yield only the second fill's outlay (2_200),
    // so the exact-sum assertion catches it while `total > first` would not.
    let expected_first = Money(1_100);
    let expected_second = Money(2_200);
    assert_eq!(
        first_outlay, expected_first,
        "first settle outlay must be price*qty/SCALE = 1_100"
    );
    assert_eq!(
        total_outlay,
        Money(expected_first.0 + expected_second.0),
        "second settle must ACCUMULATE onto the existing outlay, not overwrite: \
         expected {}, got {}",
        expected_first.0 + expected_second.0,
        total_outlay.0
    );
}

/// When buyer == seller in the same market (self-trade), the value added for that actor
/// is zero (receipts == outlays), so the wage is zero. Explicit test of the property,
/// not just inference.
#[test]
fn self_trade_nets_value_added_to_zero() {
    let actor = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let market = MarketId(9_001);
    // The actor is both seller (receipt=500) and buyer (outlay=500) in the same market.
    let (mut accounts, receipts, mut demand, household, config) =
        fixture(&[(actor, market, Money(500))], &[c1]);
    let mut outlays = BuyerOutlays::default();
    outlays.0.insert((actor, market), Money(500));

    let before = accounts.total_money().unwrap();
    let mut wage_tel = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_tel,
        &mut ledger,
        &config,
        &outlays,
    )
    .unwrap();

    // value_added = 500 - 500 = 0 → wage = 0
    assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant");
    assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO);
    assert_eq!(
        accounts.account(actor).available,
        Money(500),
        "self-trade: actor keeps all (no wage)"
    );
    let inc: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
    assert_eq!(inc, 0, "self-trade: value_added == 0 → no wage income");
}
