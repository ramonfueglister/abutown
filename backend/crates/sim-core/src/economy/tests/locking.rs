use crate::economy::{
    AccountBook, EconomicActorId, EconomyError, GOOD_FOOD, InventoryBook, Money, Quantity,
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
