use crate::economy::{
    EconomicActorId, EconomyEvent, GOOD_IRON, GOOD_TOOLS, GOOD_WOOD, InventoryBook, ProductionPool,
    ProductionPools, Quantity, Recipe, TradeLedger, run_production_at_tick,
};

fn tools_recipe() -> Recipe {
    Recipe {
        inputs: vec![(GOOD_WOOD, Quantity(2_000)), (GOOD_IRON, Quantity(1_000))],
        outputs: vec![(GOOD_TOOLS, Quantity(1_000))],
    }
}

fn seed(actor: EconomicActorId, interval: u64) -> ProductionPools {
    let mut p = ProductionPools::default();
    p.0.insert(
        actor,
        ProductionPool {
            actor,
            recipe: tools_recipe(),
            interval_ticks: interval,
            last_generated_tick: None,
        },
    );
    p
}

#[test]
fn production_consumes_inputs_and_produces_outputs() {
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap();
    inv.deposit(actor, GOOD_IRON, Quantity(1_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 1);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 5).unwrap();
    assert_eq!(inv.balance(actor, GOOD_WOOD).available, Quantity(0));
    assert_eq!(inv.balance(actor, GOOD_IRON).available, Quantity(0));
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(1_000));
    assert!(ledger.0.contains(&EconomyEvent::Consumed {
        actor,
        good: GOOD_WOOD,
        qty: Quantity(2_000)
    }));
    assert!(ledger.0.contains(&EconomyEvent::Consumed {
        actor,
        good: GOOD_IRON,
        qty: Quantity(1_000)
    }));
    assert!(ledger.0.contains(&EconomyEvent::Produced {
        actor,
        good: GOOD_TOOLS,
        qty: Quantity(1_000)
    }));
}

#[test]
fn production_skips_when_inputs_insufficient() {
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap(); // no IRON
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 1);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 5).unwrap();
    assert_eq!(inv.balance(actor, GOOD_WOOD).available, Quantity(2_000)); // unchanged
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(0));
    assert!(ledger.0.is_empty());
    assert_eq!(prod.0[&actor].last_generated_tick, Some(5)); // cadence still advances
}

#[test]
fn production_respects_interval() {
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(4_000)).unwrap();
    inv.deposit(actor, GOOD_IRON, Quantity(2_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 10);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0).unwrap(); // produces (last=None)
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 3).unwrap(); // interval not elapsed → skip
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(1_000)); // only one batch
}

#[test]
fn production_conserves_money() {
    use crate::economy::{AccountBook, Money};
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap();
    inv.deposit(actor, GOOD_IRON, Quantity(1_000)).unwrap();
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(5_000)).unwrap();
    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 1);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 1).unwrap();
    assert_eq!(accounts.total_money().unwrap(), before); // production never touches money
}

#[test]
fn production_is_deterministic() {
    let run = || {
        let a1 = EconomicActorId(2);
        let a2 = EconomicActorId(1);
        let mut inv = InventoryBook::default();
        for a in [a1, a2] {
            inv.deposit(a, GOOD_WOOD, Quantity(2_000)).unwrap();
            inv.deposit(a, GOOD_IRON, Quantity(1_000)).unwrap();
        }
        let mut ledger = TradeLedger::default();
        let mut prod = ProductionPools::default();
        for a in [a1, a2] {
            prod.0.insert(
                a,
                ProductionPool {
                    actor: a,
                    recipe: tools_recipe(),
                    interval_ticks: 1,
                    last_generated_tick: None,
                },
            );
        }
        run_production_at_tick(&mut inv, &mut ledger, &mut prod, 1).unwrap();
        ledger.0
    };
    assert_eq!(run(), run());
}

#[test]
fn good_raw_is_the_next_free_good_id_and_distinct() {
    use crate::economy::{GOOD_FOOD, GOOD_IRON, GOOD_RAW, GOOD_TOOLS, GOOD_WOOD, GoodId};
    assert_eq!(GOOD_RAW, GoodId(5));
    for g in [GOOD_FOOD, GOOD_WOOD, GOOD_IRON, GOOD_TOOLS] {
        assert_ne!(
            g, GOOD_RAW,
            "GOOD_RAW must not collide with a tradable good"
        );
    }
}

#[test]
fn regenerated_event_type_tag_is_stable() {
    use crate::economy::{EconomicActorId, EconomyEvent, GOOD_RAW, Quantity};
    let e = EconomyEvent::Regenerated {
        actor: EconomicActorId(8_031),
        good: GOOD_RAW,
        qty: Quantity(100),
    };
    assert_eq!(e.event_type(), "regenerated");
}

#[test]
fn regen_deposits_faucet_on_interval_and_stamps_cursor() {
    use crate::economy::production::{EXTRACTOR, RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{EconomyEvent, GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );

    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 5).unwrap();

    assert_eq!(inv.balance(EXTRACTOR, GOOD_RAW).available, Quantity(100));
    assert_eq!(deposits.0[&EXTRACTOR].last_regen_tick, Some(5));
    assert!(ledger.0.contains(&EconomyEvent::Regenerated {
        actor: EXTRACTOR,
        good: GOOD_RAW,
        qty: Quantity(100),
    }));
}

#[test]
fn regen_skips_within_interval_but_does_not_advance_cursor_on_skip() {
    use crate::economy::production::{EXTRACTOR, RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 10,
            last_regen_tick: None,
        },
    );
    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 0).unwrap(); // fires (last=None)
    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 3).unwrap(); // interval not elapsed
    assert_eq!(
        inv.balance(EXTRACTOR, GOOD_RAW).available,
        Quantity(100),
        "only one deposit within the interval"
    );
    // On a skip the gate returns BEFORE stamping, so the cursor stays at the firing tick.
    assert_eq!(deposits.0[&EXTRACTOR].last_regen_tick, Some(0));
}

#[test]
fn regen_is_flow_capped_not_capacity_capped() {
    use crate::economy::production::{EXTRACTOR, RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    // No recipe consuming RAW here: deposits stack unboundedly per interval (faucet,
    // not a level-capped reservoir). The recipe is what bounds RAW in the live loop.
    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    for t in 0..3 {
        run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, t).unwrap();
    }
    assert_eq!(inv.balance(EXTRACTOR, GOOD_RAW).available, Quantity(300));
}

#[test]
fn regen_is_deterministic_keys_first() {
    use crate::economy::production::{RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{EconomicActorId, GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    let run = || {
        let mut inv = InventoryBook::default();
        let mut ledger = TradeLedger::default();
        let mut deposits = RawDeposits(BTreeMap::new());
        // Insert out of ascending order to prove keys-first iteration.
        for a in [EconomicActorId(9), EconomicActorId(2)] {
            deposits.0.insert(
                a,
                RawDeposit {
                    good: GOOD_RAW,
                    qty_per_interval: Quantity(50),
                    interval_ticks: 1,
                    last_regen_tick: None,
                },
            );
        }
        run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 1).unwrap();
        ledger.0
    };
    assert_eq!(run(), run());
}
