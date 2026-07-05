use crate::econ::{
    AccountBook, EconomicActorId, EconomyError, GOOD_FOOD, GOOD_WOOD, InventoryBook, Money,
    Quantity,
};

#[test]
fn bid_locks_cash() {
    let actor = EconomicActorId(1);
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(1_000)).unwrap();

    accounts.lock_cash(actor, Money(700)).unwrap();

    let balance = accounts.account(actor);
    assert_eq!(balance.available, Money(300));
    assert_eq!(balance.locked, Money(700));
}

#[test]
fn ask_locks_goods() {
    let actor = EconomicActorId(2);
    let mut inventory = InventoryBook::default();
    inventory
        .deposit(actor, GOOD_FOOD, Quantity(5_000))
        .unwrap();

    inventory
        .lock_goods(actor, GOOD_FOOD, Quantity(2_000))
        .unwrap();

    let balance = inventory.balance(actor, GOOD_FOOD);
    assert_eq!(balance.available, Quantity(3_000));
    assert_eq!(balance.locked, Quantity(2_000));
}

#[test]
fn cannot_bid_without_available_cash() {
    let actor = EconomicActorId(1);
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(100)).unwrap();

    assert_eq!(
        accounts.lock_cash(actor, Money(200)),
        Err(EconomyError::InsufficientFunds)
    );
}

#[test]
fn cannot_ask_without_available_goods() {
    let actor = EconomicActorId(2);
    let mut inventory = InventoryBook::default();
    inventory.deposit(actor, GOOD_FOOD, Quantity(100)).unwrap();

    assert_eq!(
        inventory.lock_goods(actor, GOOD_FOOD, Quantity(200)),
        Err(EconomyError::InsufficientGoods)
    );
}

#[test]
fn cannot_double_lock_cash() {
    let actor = EconomicActorId(1);
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(1_000)).unwrap();
    accounts.lock_cash(actor, Money(700)).unwrap();

    assert_eq!(
        accounts.lock_cash(actor, Money(400)),
        Err(EconomyError::InsufficientFunds)
    );
}

#[test]
fn cannot_double_lock_goods() {
    let actor = EconomicActorId(2);
    let mut inventory = InventoryBook::default();
    inventory
        .deposit(actor, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    inventory
        .lock_goods(actor, GOOD_FOOD, Quantity(700))
        .unwrap();

    assert_eq!(
        inventory.lock_goods(actor, GOOD_FOOD, Quantity(400)),
        Err(EconomyError::InsufficientGoods)
    );
}

#[test]
fn consume_debits_available() {
    let actor = EconomicActorId(3);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(5_000)).unwrap();
    inv.consume(actor, GOOD_WOOD, Quantity(2_000)).unwrap();
    assert_eq!(inv.balance(actor, GOOD_WOOD).available, Quantity(3_000));
    assert_eq!(inv.balance(actor, GOOD_WOOD).locked, Quantity(0));
}

#[test]
fn cannot_consume_more_than_available() {
    let actor = EconomicActorId(3);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(1_000)).unwrap();
    assert_eq!(
        inv.consume(actor, GOOD_WOOD, Quantity(2_000)),
        Err(EconomyError::InsufficientGoods)
    );
}

#[test]
fn cannot_consume_negative() {
    let mut inv = InventoryBook::default();
    assert_eq!(
        inv.consume(EconomicActorId(3), GOOD_WOOD, Quantity(-1)),
        Err(EconomyError::NegativeQuantity)
    );
}

#[test]
fn transfer_moves_available_and_conserves_total() {
    let a = EconomicActorId(1);
    let b = EconomicActorId(2);
    let mut acc = AccountBook::default();
    acc.deposit(a, Money(1_000)).unwrap();
    let before = acc.total_money().unwrap();
    acc.transfer(a, b, Money(400)).unwrap();
    assert_eq!(acc.account(a).available, Money(600));
    assert_eq!(acc.account(b).available, Money(400));
    assert_eq!(acc.total_money().unwrap(), before);
}
#[test]
fn cannot_transfer_more_than_available() {
    let mut acc = AccountBook::default();
    acc.deposit(EconomicActorId(1), Money(100)).unwrap();
    assert_eq!(
        acc.transfer(EconomicActorId(1), EconomicActorId(2), Money(200)),
        Err(EconomyError::InsufficientFunds)
    );
}
#[test]
fn cannot_transfer_negative() {
    let mut acc = AccountBook::default();
    assert_eq!(
        acc.transfer(EconomicActorId(1), EconomicActorId(2), Money(-1)),
        Err(EconomyError::NegativeMoney)
    );
}
