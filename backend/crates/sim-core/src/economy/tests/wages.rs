use std::collections::BTreeMap;

use crate::economy::auction::SettlementPolicy;
use crate::economy::{
    AccountBook, DirtyMarketGoods, EconomicActorId, EconomyConfig, GOOD_FOOD, InventoryBook,
    MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money, NextOrderId, OrderBook, Quantity,
    SellerReceipts, TradeLedger, clear_market_good_with_receipts, create_ask, create_bid,
    settle_flow_with_receipts,
};
use crate::economy::macro_flow::PlannedFlow;

fn seeded_state(market: MarketId) -> MarketGoodState {
    MarketGoodState {
        key: MarketGoodKey { market, good: GOOD_FOOD },
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
    let key = MarketGoodKey { market, good: GOOD_FOOD };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_state(market));
    accounts.deposit(buyer, Money(10_000)).unwrap();
    inventory.deposit(seller, GOOD_FOOD, Quantity(2_000)).unwrap();
    create_bid(&mut accounts, &mut orders, &mut ledger, &mut dirty, &mut next, 1, buyer, market, GOOD_FOOD, Quantity(1_000), Money(1_500), 10).unwrap();
    create_ask(&mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next, 1, seller, market, GOOD_FOOD, Quantity(1_000), Money(1_000), 10).unwrap();

    let before = accounts.total_money().unwrap();
    let mut receipts = SellerReceipts::default();
    clear_market_good_with_receipts(
        &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut goods, key, 2,
        SettlementPolicy::Anchored, &mut receipts.0,
    )
    .unwrap();

    assert_eq!(accounts.total_money().unwrap(), before, "money conserved");
    assert_eq!(receipts.0.get(&(seller, market)).copied(), Some(Money(1_100)));
    assert_eq!(receipts.0.get(&(buyer, market)).copied(), None, "buyers are not credited");
}

#[test]
fn auction_no_fills_produces_no_receipts() {
    let market = MarketId(7);
    let key = MarketGoodKey { market, good: GOOD_FOOD };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_state(market));
    let mut receipts = SellerReceipts::default();
    clear_market_good_with_receipts(
        &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut goods, key, 1,
        SettlementPolicy::Anchored, &mut receipts.0,
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
    inventory.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
    let flow = PlannedFlow { good: GOOD_FOOD, src, dst, q: 10, p_src: Money(1_000), p_dst: Money(1_200), dist: 0 };
    let config = EconomyConfig::default();
    let before = accounts.total_money().unwrap();
    let mut receipts = SellerReceipts::default();
    settle_flow_with_receipts(
        &mut accounts, &mut inventory, &mut goods, &flow,
        &[(seller, 10)], &[(buyer, 10)],
        10, 10, 10, 10, &config, 1, false, false, &mut receipts.0,
    ).unwrap();
    assert_eq!(accounts.total_money().unwrap(), before, "money conserved");
    // src_revenue = value(1_000, 10) = 1_000*10/ECONOMY_SCALE(=1_000) = 10
    assert_eq!(receipts.0.get(&(seller, src)).copied(), Some(Money(10)));
}
