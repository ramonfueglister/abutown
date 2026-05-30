use crate::economy::prorata_distribute;
use crate::economy::{
    AccountBook, DirtyMarketGoods, GOOD_FOOD as FOOD, InventoryBook, MarketGoodState, MarketGoods,
    NextOrderId, OrderBook, TradeLedger, clear_market_good, create_ask, create_bid,
};
use crate::economy::{
    Ask, Bid, EconomicActorId, GOOD_FOOD, MarketGoodKey, MarketId, Money, OrderId, Quantity,
    build_clearing_plan,
};

fn rbid(id: u64, max_price: Money, qty: Quantity, created_tick: u64) -> Bid {
    Bid {
        id: OrderId(id),
        owner: EconomicActorId(10 + id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: qty,
        max_price,
        cash_locked_remaining: Money(max_price.0 * qty.0 / 1_000),
        created_tick,
        expires_tick: 100,
    }
}

fn rask(id: u64, min_price: Money, qty: Quantity, created_tick: u64) -> Ask {
    Ask {
        id: OrderId(id),
        owner: EconomicActorId(20 + id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: qty,
        min_price,
        goods_locked_remaining: qty,
        created_tick,
        expires_tick: 100,
    }
}

fn key() -> MarketGoodKey {
    MarketGoodKey {
        market: MarketId(1),
        good: GOOD_FOOD,
    }
}

// Sum of fill qty attributed to a given bid / ask id.
fn filled_for_bid(plan: &crate::economy::ClearingPlan, id: u64) -> i64 {
    plan.fills
        .iter()
        .filter(|f| f.bid == OrderId(id))
        .map(|f| f.qty.0)
        .sum()
}
fn filled_for_ask(plan: &crate::economy::ClearingPlan, id: u64) -> i64 {
    plan.fills
        .iter()
        .filter(|f| f.ask == OrderId(id))
        .map(|f| f.qty.0)
        .sum()
}

#[test]
fn prorata_exact_division() {
    assert_eq!(prorata_distribute(&[10, 10], 10), vec![5, 5]);
}

#[test]
fn prorata_proportional() {
    assert_eq!(prorata_distribute(&[30, 10], 20), vec![15, 5]);
}

#[test]
fn prorata_leftover_to_largest_remainder_then_index() {
    // total 2 across three equal weights: floors are [0,0,0], 2 leftover units go
    // to the two largest remainders; all remainders equal -> lowest indices win.
    assert_eq!(prorata_distribute(&[1, 1, 1], 2), vec![1, 1, 0]);
}

#[test]
fn prorata_odd_split_is_deterministic() {
    // 1001 across two equal weights -> 501 / 500 (extra unit to index 0).
    assert_eq!(prorata_distribute(&[1000, 1000], 1001), vec![501, 500]);
}

#[test]
fn prorata_total_at_or_above_sum_returns_weights() {
    assert_eq!(prorata_distribute(&[3, 7], 10), vec![3, 7]);
    assert_eq!(prorata_distribute(&[3, 7], 100), vec![3, 7]);
}

#[test]
fn prorata_zero_total_is_zeros() {
    assert_eq!(prorata_distribute(&[5, 5], 0), vec![0, 0]);
}

#[test]
fn prorata_never_exceeds_a_weight() {
    let weights = [2, 2, 2];
    for total in 0..=6 {
        let out = prorata_distribute(&weights, total);
        assert_eq!(out.iter().sum::<i64>(), total.min(6));
        for (o, w) in out.iter().zip(weights.iter()) {
            assert!(*o <= *w, "alloc {o} exceeded weight {w} at total {total}");
        }
    }
}

#[test]
fn marginal_tier_is_rationed_pro_rata() {
    // Two equal-price bids (1000 each @ 1000) compete for one ask of 1000 @ 1000.
    // Time-priority would give bid 1 all 1000; pro-rata gives each 500.
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_000), Quantity(1_000), 1),
            rbid(2, Money(1_000), Quantity(1_000), 2),
        ],
        &[rask(3, Money(1_000), Quantity(1_000), 1)],
        Money(1_000),
    )
    .unwrap();
    assert_eq!(filled_for_bid(&plan, 1), 500);
    assert_eq!(filled_for_bid(&plan, 2), 500);
    assert_eq!(filled_for_ask(&plan, 3), 1_000);
    assert_eq!(plan.settlement_price, Some(Money(1_000)));
}

#[test]
fn marginal_tier_pro_rata_is_size_weighted() {
    // Bids 1500 and 500 (both @ 1000) vs one ask of 1000 @ 1000 -> 750 / 250.
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_000), Quantity(1_500), 1),
            rbid(2, Money(1_000), Quantity(500), 2),
        ],
        &[rask(3, Money(1_000), Quantity(1_000), 1)],
        Money(1_000),
    )
    .unwrap();
    assert_eq!(filled_for_bid(&plan, 1), 750);
    assert_eq!(filled_for_bid(&plan, 2), 250);
}

#[test]
fn infra_marginal_bids_keep_price_priority() {
    // A higher bid (1200) fills fully before the marginal tier (1000) is rationed.
    // ask supply 1000 total: bid1@1200 x600 fills fully (600), remaining 400 split
    // pro-rata across the two 1000-priced bids (500 each -> 200 each).
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_200), Quantity(600), 1),
            rbid(2, Money(1_000), Quantity(500), 2),
            rbid(3, Money(1_000), Quantity(500), 3),
        ],
        &[rask(4, Money(1_000), Quantity(1_000), 1)],
        Money(1_100),
    )
    .unwrap();
    assert_eq!(
        filled_for_bid(&plan, 1),
        600,
        "infra-marginal bid fully filled"
    );
    assert_eq!(filled_for_bid(&plan, 2), 200);
    assert_eq!(filled_for_bid(&plan, 3), 200);
    assert_eq!(filled_for_ask(&plan, 4), 1_000);
}

#[test]
fn odd_contested_quantity_splits_deterministically() {
    // ask 1001 across two equal 1000-bids -> 501 / 500 (extra to lower index after
    // sort: bid 1, created_tick 1).
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_000), Quantity(1_000), 1),
            rbid(2, Money(1_000), Quantity(1_000), 2),
        ],
        &[rask(3, Money(1_000), Quantity(1_001), 1)],
        Money(1_000),
    )
    .unwrap();
    assert_eq!(filled_for_bid(&plan, 1) + filled_for_bid(&plan, 2), 1_001);
    assert_eq!(filled_for_bid(&plan, 1), 501);
    assert_eq!(filled_for_bid(&plan, 2), 500);
}

#[test]
fn clearing_plan_is_deterministic() {
    let mk = || {
        build_clearing_plan(
            key(),
            &[
                rbid(1, Money(1_000), Quantity(700), 1),
                rbid(2, Money(1_000), Quantity(300), 2),
            ],
            &[rask(3, Money(1_000), Quantity(900), 1)],
            Money(1_000),
        )
        .unwrap()
    };
    assert_eq!(mk(), mk());
}

#[test]
fn fills_balance_quantity_per_side() {
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_000), Quantity(1_000), 1),
            rbid(2, Money(1_000), Quantity(1_000), 2),
        ],
        &[
            rask(3, Money(1_000), Quantity(700), 1),
            rask(4, Money(1_000), Quantity(800), 2),
        ],
        Money(1_000),
    )
    .unwrap();
    let total: i64 = plan.fills.iter().map(|f| f.qty.0).sum();
    assert_eq!(
        total, 1_500,
        "matched quantity = min(2000 demand, 1500 supply)"
    );
    assert_eq!(filled_for_ask(&plan, 3) + filled_for_ask(&plan, 4), 1_500);
    assert_eq!(filled_for_bid(&plan, 1) + filled_for_bid(&plan, 2), 1_500);
}

#[test]
fn contested_clearing_conserves_and_rations() {
    let buyer_a = EconomicActorId(1);
    let buyer_b = EconomicActorId(2);
    let seller = EconomicActorId(3);
    let market = MarketId(1);
    let k = MarketGoodKey { market, good: FOOD };

    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    let mut state = MarketGoodState::new(k);
    state.last_settlement_price = Money(1_000);
    state.dirty = true;
    goods.0.insert(k, state);

    accounts.deposit(buyer_a, Money(10_000)).unwrap();
    accounts.deposit(buyer_b, Money(10_000)).unwrap();
    inventory.deposit(seller, FOOD, Quantity(1_000)).unwrap();

    // Two equal bids @1000 x1000 each, one ask @1000 x1000 -> 500/500 rationed.
    create_bid(
        &mut accounts,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        buyer_a,
        market,
        FOOD,
        Quantity(1_000),
        Money(1_000),
        10,
    )
    .unwrap();
    create_bid(
        &mut accounts,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        buyer_b,
        market,
        FOOD,
        Quantity(1_000),
        Money(1_000),
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
        FOOD,
        Quantity(1_000),
        Money(1_000),
        10,
    )
    .unwrap();

    let money_before = accounts.total_money().unwrap();
    let goods_before = inventory.total_good(FOOD).unwrap();

    clear_market_good(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        k,
        2,
    )
    .unwrap();

    assert_eq!(
        accounts.total_money().unwrap(),
        money_before,
        "money conserved"
    );
    assert_eq!(
        inventory.total_good(FOOD).unwrap(),
        goods_before,
        "goods conserved"
    );
    // Each buyer received a proportional 500 of FOOD.
    assert_eq!(inventory.balance(buyer_a, FOOD).available, Quantity(500));
    assert_eq!(inventory.balance(buyer_b, FOOD).available, Quantity(500));
    assert_eq!(inventory.balance(seller, FOOD).available, Quantity(0));
}
