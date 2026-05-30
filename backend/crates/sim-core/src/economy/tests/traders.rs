use std::collections::BTreeSet;

use crate::economy::{
    AccountBook, DirtyMarketGoods, EconomicActorId, EconomyEvent, GOOD_TOOLS, InventoryBook,
    MarketGoods, MarketId, Money, NextOrderId, OrderBook, Quantity, TRANSPORT_OPERATOR,
    TradeLedger, Trader, TraderState, Traders, adjust_price, run_traders_at_tick, transport_ticks,
};

fn trader(actor: EconomicActorId) -> Trader {
    Trader {
        actor,
        good: GOOD_TOOLS,
        source: MarketId(1),
        dest: MarketId(2),
        distance_tiles: 10,
        batch_qty: Quantity(1_000),
        buy_premium_bps: 0,
        sell_discount_bps: 0,
        order_ttl_ticks: 100,
        state: TraderState::Buying { order: None },
    }
}

#[test]
fn adjust_price_applies_bps() {
    assert_eq!(adjust_price(Money(1_000), 0).unwrap(), Money(1_000));
    assert_eq!(adjust_price(Money(1_000), 2_500).unwrap(), Money(1_250)); // +25%
    assert_eq!(adjust_price(Money(1_000), -2_000).unwrap(), Money(800)); // -20%
}

#[test]
fn transport_ticks_is_at_least_one() {
    let cfg = crate::economy::EconomyConfig {
        trader_tiles_per_tick: 4,
        ..Default::default()
    };
    assert_eq!(transport_ticks(10, &cfg), 2); // 10/4 = 2
    assert_eq!(transport_ticks(1, &cfg), 1); // floor 0 -> max(1)
}

#[test]
fn buying_places_a_bid_when_short() {
    let actor = EconomicActorId(1);
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(100_000)).unwrap();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let goods = MarketGoods::default();
    let mut traders = Traders::default();
    traders.0.insert(actor, trader(actor));
    let cfg = crate::economy::EconomyConfig::default();

    run_traders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &goods,
        &mut traders,
        &cfg,
        0,
        &BTreeSet::new(),
    )
    .unwrap();

    assert_eq!(orders.bids.len(), 1); // placed a buy bid at source
    assert!(matches!(
        traders.0[&actor].state,
        TraderState::Buying { order: Some(_) }
    ));
}

#[test]
fn acquired_goods_trigger_travel_and_transport_payment() {
    let actor = EconomicActorId(1);
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(100_000)).unwrap();
    let mut inventory = InventoryBook::default();
    inventory
        .deposit(actor, GOOD_TOOLS, Quantity(1_000))
        .unwrap(); // already holds a batch
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let goods = MarketGoods::default();
    let mut traders = Traders::default();
    let mut t = trader(actor);
    t.state = TraderState::Buying {
        order: Some(crate::economy::OrderId(1)),
    };
    traders.0.insert(actor, t);
    let cfg = crate::economy::EconomyConfig::default(); // transport_cost_per_tile_unit = Money(5)

    let before = accounts.total_money().unwrap();
    run_traders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &goods,
        &mut traders,
        &cfg,
        0,
        &BTreeSet::new(),
    )
    .unwrap();

    assert!(matches!(
        traders.0[&actor].state,
        TraderState::ToDest { .. }
    ));
    // transport cost = transport_cost(10, Quantity(1000), Money(5)) = (5*1000/1000)*10 = 50
    assert_eq!(accounts.account(TRANSPORT_OPERATOR).available, Money(50));
    assert!(ledger.0.iter().any(|e| matches!(
        e,
        EconomyEvent::TransportPaid { amount, .. } if *amount == Money(50)
    )));
    assert_eq!(accounts.total_money().unwrap(), before); // conserved (trader -> operator)
}

#[test]
fn travel_counts_down_then_sells() {
    let actor = EconomicActorId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    inventory
        .deposit(actor, GOOD_TOOLS, Quantity(1_000))
        .unwrap();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let goods = MarketGoods::default();
    let mut traders = Traders::default();
    let mut t = trader(actor);
    t.state = TraderState::ToDest { remaining: 2 };
    traders.0.insert(actor, t);
    let cfg = crate::economy::EconomyConfig::default();
    let tick_once = |accounts: &mut AccountBook,
                     inventory: &mut InventoryBook,
                     orders: &mut OrderBook,
                     ledger: &mut TradeLedger,
                     dirty: &mut DirtyMarketGoods,
                     next: &mut NextOrderId,
                     goods: &MarketGoods,
                     traders: &mut Traders,
                     cfg: &crate::economy::EconomyConfig| {
        run_traders_at_tick(
            accounts,
            inventory,
            orders,
            ledger,
            dirty,
            next,
            goods,
            traders,
            cfg,
            0,
            &BTreeSet::new(),
        )
        .unwrap()
    };
    tick_once(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &goods,
        &mut traders,
        &cfg,
    );
    assert!(matches!(
        traders.0[&actor].state,
        TraderState::ToDest { remaining: 1 }
    ));
    tick_once(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &goods,
        &mut traders,
        &cfg,
    );
    assert!(matches!(
        traders.0[&actor].state,
        TraderState::Selling { .. }
    ));
    tick_once(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &goods,
        &mut traders,
        &cfg,
    ); // Selling: holds goods -> places ask
    assert_eq!(orders.asks.len(), 1);
}

#[test]
fn traders_are_deterministic() {
    let build = || {
        let mut accounts = AccountBook::default();
        let mut inventory = InventoryBook::default();
        let mut orders = OrderBook::default();
        let mut ledger = TradeLedger::default();
        let mut dirty = DirtyMarketGoods::default();
        let mut next = NextOrderId::default();
        let goods = MarketGoods::default();
        let mut traders = Traders::default();
        for id in [2u64, 1u64] {
            let a = EconomicActorId(id);
            accounts.deposit(a, Money(100_000)).unwrap();
            traders.0.insert(a, trader(a));
        }
        let cfg = crate::economy::EconomyConfig::default();
        run_traders_at_tick(
            &mut accounts,
            &mut inventory,
            &mut orders,
            &mut ledger,
            &mut dirty,
            &mut next,
            &goods,
            &mut traders,
            &cfg,
            0,
            &BTreeSet::new(),
        )
        .unwrap();
        ledger.0
    };
    assert_eq!(build(), build());
}
