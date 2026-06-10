use bevy_ecs::prelude::*;

use crate::economy::audit::{
    LedgerAuditCursor, commit_ledger_audit, init_ledger_audit_cursor, pending_ledger_audit,
};
use crate::economy::{
    EconomicActorId, EconomyEvent, GOOD_TOOLS, MarketId, Money, PERSISTED_LEDGER_TAIL, Quantity,
    TradeLedger,
};
use crate::mobility::resources::Tick;
use crate::persistence::InMemoryEconomyEventStore;

fn world_with_ledger(n: usize) -> World {
    let mut world = World::new();
    let events: Vec<EconomyEvent> = (0..n)
        .map(|i| EconomyEvent::CashLocked {
            actor: EconomicActorId(i as u64),
            amount: Money(1),
        })
        .collect();
    world.insert_resource(TradeLedger(events));
    world.insert_resource(LedgerAuditCursor(0));
    world.insert_resource(Tick(7));
    world
}

#[test]
fn economy_event_type_tags_are_stable() {
    assert_eq!(
        EconomyEvent::Trade {
            market: MarketId(1),
            good: GOOD_TOOLS,
            buyer: EconomicActorId(1),
            seller: EconomicActorId(2),
            qty: Quantity(1),
            price: Money(1),
            total: Money(1),
        }
        .event_type(),
        "trade"
    );
    assert_eq!(
        EconomyEvent::TransportPaid {
            actor: EconomicActorId(1),
            amount: Money(5),
        }
        .event_type(),
        "transport_paid"
    );
    assert_eq!(
        EconomyEvent::CashLocked {
            actor: EconomicActorId(1),
            amount: Money(1),
        }
        .event_type(),
        "cash_locked"
    );
}

#[test]
fn ledger_audit_flush_appends_new_then_trims_and_bounds() {
    let n = PERSISTED_LEDGER_TAIL + 6;
    let mut world = world_with_ledger(n);

    let (tick, pending) = pending_ledger_audit(&world);
    assert_eq!(tick, 7);
    assert_eq!(pending.len(), n, "all events pending initially");

    commit_ledger_audit(&mut world, pending.len());
    assert_eq!(
        world.resource::<TradeLedger>().0.len(),
        PERSISTED_LEDGER_TAIL,
        "live ledger bounded to the tail after flush"
    );
    assert_eq!(
        world.resource::<LedgerAuditCursor>().0,
        PERSISTED_LEDGER_TAIL
    );
    assert!(
        pending_ledger_audit(&world).1.is_empty(),
        "nothing pending right after a flush"
    );

    // New events accumulate; the next flush appends only them and stays bounded.
    world
        .resource_mut::<TradeLedger>()
        .0
        .push(EconomyEvent::TransportPaid {
            actor: EconomicActorId(99),
            amount: Money(5),
        });
    let (_, pending2) = pending_ledger_audit(&world);
    assert_eq!(pending2.len(), 1, "only the new event is pending");
    commit_ledger_audit(&mut world, pending2.len());
    assert_eq!(
        world.resource::<TradeLedger>().0.len(),
        PERSISTED_LEDGER_TAIL,
        "still bounded after the second flush"
    );
    assert_eq!(
        world.resource::<LedgerAuditCursor>().0,
        PERSISTED_LEDGER_TAIL
    );
}

#[test]
fn ledger_audit_init_skips_restored_tail() {
    let mut world = world_with_ledger(50); // simulate a restored ledger_tail of 50
    init_ledger_audit_cursor(&mut world);
    assert_eq!(world.resource::<LedgerAuditCursor>().0, 50);
    assert!(
        pending_ledger_audit(&world).1.is_empty(),
        "restored tail is treated as already-appended (no re-append on restart)"
    );
}

#[test]
fn pending_ledger_audit_does_not_mutate() {
    let world = world_with_ledger(5);
    let before_len = world.resource::<TradeLedger>().0.len();
    let before_cursor = world.resource::<LedgerAuditCursor>().0;
    let _ = pending_ledger_audit(&world);
    let _ = pending_ledger_audit(&world);
    assert_eq!(world.resource::<TradeLedger>().0.len(), before_len);
    assert_eq!(world.resource::<LedgerAuditCursor>().0, before_cursor);
}

#[tokio::test]
async fn in_memory_event_store_appends_in_order() {
    use crate::persistence::EconomyEventStore;
    let mut store = InMemoryEconomyEventStore::default();
    store
        .append(
            "w",
            1,
            &[EconomyEvent::CashLocked {
                actor: EconomicActorId(1),
                amount: Money(1),
            }],
        )
        .await
        .unwrap();
    store
        .append(
            "w",
            2,
            &[EconomyEvent::TransportPaid {
                actor: EconomicActorId(1),
                amount: Money(5),
            }],
        )
        .await
        .unwrap();

    let evs = store.events("w");
    assert_eq!(evs.len(), 2);
    assert_eq!((evs[0].0, evs[0].1.event_type()), (1, "cash_locked"));
    assert_eq!((evs[1].0, evs[1].1.event_type()), (2, "transport_paid"));
    assert_eq!(store.len("other"), 0);
}

#[test]
fn macro_flow_event_type_is_macro_flow() {
    use crate::economy::{EconomyEvent, GoodId, MarketId, Money, Quantity};
    let ev = EconomyEvent::MacroFlow {
        from_market: MarketId(1),
        to_market: MarketId(2),
        good: GoodId(4),
        qty: Quantity(10),
        price: Money(1_000),
        transport: Money(50),
    };
    assert_eq!(ev.event_type(), "macro_flow");
}

#[test]
fn final_consumed_event_tag() {
    let e = EconomyEvent::FinalConsumed {
        actor: EconomicActorId(1),
        good: GOOD_TOOLS,
        qty: Quantity(3),
    };
    assert_eq!(e.event_type(), "final_consumed");
}

#[test]
fn sfc_audit_primitives_exist() {
    use crate::economy::audit::LastTickMoney;
    use crate::economy::{EconomyError, EconomyEvent, Money};
    // The new honest error variant.
    let _ = EconomyError::ConservationViolation;
    // The TickAudit event + its stable tag.
    let e = EconomyEvent::TickAudit {
        tick: 7,
        total_money: Money(12_345),
    };
    assert_eq!(e.event_type(), "tick_audit");
    // The ephemeral baseline defaults to None.
    assert_eq!(LastTickMoney::default().0, None);
}

#[test]
fn tick_audit_emits_event_and_tracks_baseline_when_conserved() {
    use crate::economy::audit::{LastTickMoney, run_tick_audit_at_tick};
    use crate::economy::{AccountBook, EconomicActorId, EconomyEvent, Money, TradeLedger};
    let mut accounts = AccountBook::default();
    accounts.deposit(EconomicActorId(1), Money(1_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut last = LastTickMoney::default();

    // First tick: no prior baseline → initialize, emit event, no check.
    run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, 0).unwrap();
    assert_eq!(last.0, Some(Money(1_000)));
    assert_eq!(
        ledger.0,
        vec![EconomyEvent::TickAudit {
            tick: 0,
            total_money: Money(1_000)
        }]
    );

    // Second tick, unchanged total → Ok, second event, baseline unchanged.
    run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, 1).unwrap();
    assert_eq!(last.0, Some(Money(1_000)));
    assert_eq!(ledger.0.len(), 2);
}

#[test]
fn tick_audit_returns_err_on_money_drift() {
    use crate::economy::audit::{LastTickMoney, run_tick_audit_at_tick};
    use crate::economy::{AccountBook, EconomicActorId, EconomyError, Money, TradeLedger};
    let mut accounts = AccountBook::default();
    accounts.deposit(EconomicActorId(1), Money(1_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut last = LastTickMoney::default();
    run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, 0).unwrap(); // baseline = 1_000

    // Mint money (a conservation violation) → next audit detects drift.
    accounts.deposit(EconomicActorId(2), Money(500)).unwrap();
    let r = run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, 1);
    assert_eq!(r, Err(EconomyError::ConservationViolation));
}

#[test]
fn tick_audit_fires_every_tick_under_full_plugin() {
    use crate::economy::systems::{EconomySet, run_tick_audit_system};
    use crate::economy::{EconomyEvent, EconomyPlugin, TradeLedger};
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::prelude::*;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);
    world.insert_resource(Tick(0));

    for _ in 0..3 {
        schedule.run(&mut world);
        world.resource_mut::<Tick>().0 += 1;
    }
    let n_audit = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter(|e| matches!(e, EconomyEvent::TickAudit { .. }))
        .count();
    assert!(n_audit >= 3, "a TickAudit event per tick: {n_audit}");
    let _ = (run_tick_audit_system, EconomySet::TickAudit);
}

#[test]
#[should_panic(expected = "CONSERVATION VIOLATION")]
fn injected_money_drift_panics_the_audit() {
    use crate::economy::{AccountBook, EconomicActorId, EconomyPlugin, Money};
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::prelude::*;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);
    world.insert_resource(Tick(0));

    schedule.run(&mut world); // tick 0: audit sets the baseline
    world.resource_mut::<Tick>().0 += 1;
    // MINT money (a conservation violation) between ticks.
    world
        .resource_mut::<AccountBook>()
        .deposit(EconomicActorId(42), Money(1_000))
        .unwrap();
    schedule.run(&mut world); // tick 1: audit sees total changed → .expect panics
}

/// B1 (2026-06-10 tick-cost-and-event-retention design): the durable audit
/// store keeps the financially meaningful events; high-frequency intra-tick
/// mechanics stay in-memory/snapshot-tail only.
#[test]
fn is_audit_durable_classifies_every_variant() {
    use crate::economy::{EconomyError, GoodId, OrderId};
    let actor = EconomicActorId(1);
    let market = MarketId(1);
    let good = GoodId(1);

    let durable = [
        EconomyEvent::Trade {
            market,
            good,
            buyer: actor,
            seller: EconomicActorId(2),
            qty: Quantity(1),
            price: Money(1),
            total: Money(1),
        },
        EconomyEvent::FinalConsumed {
            actor,
            good,
            qty: Quantity(1),
        },
        EconomyEvent::OrderRejected {
            actor,
            market,
            good,
            reason: EconomyError::InsufficientFunds,
        },
        EconomyEvent::MarketClearFailed {
            market,
            good,
            reason: EconomyError::InsufficientFunds,
        },
        EconomyEvent::TransportPaid {
            actor,
            amount: Money(1),
        },
        EconomyEvent::MacroFlow {
            from_market: market,
            to_market: MarketId(2),
            good,
            qty: Quantity(1),
            price: Money(1),
            transport: Money(0),
        },
        EconomyEvent::WagePaid {
            firm: actor,
            market,
            amount: Money(1),
        },
        EconomyEvent::ProfitDistributed {
            firm: actor,
            market,
            amount: Money(1),
        },
        EconomyEvent::TransportRebate { amount: Money(1) },
        EconomyEvent::TickAudit {
            tick: 1,
            total_money: Money(1),
        },
    ];
    for event in &durable {
        assert!(
            event.is_audit_durable(),
            "{} must be audit-durable",
            event.event_type()
        );
    }

    let transient = [
        EconomyEvent::OrderCreated {
            order: OrderId(1),
            actor,
            market,
            good,
        },
        EconomyEvent::OrderExpired {
            order: OrderId(1),
            actor,
            market,
            good,
        },
        EconomyEvent::CashLocked {
            actor,
            amount: Money(1),
        },
        EconomyEvent::CashReleased {
            actor,
            amount: Money(1),
        },
        EconomyEvent::GoodsLocked {
            actor,
            good,
            qty: Quantity(1),
        },
        EconomyEvent::GoodsReleased {
            actor,
            good,
            qty: Quantity(1),
        },
        EconomyEvent::Produced {
            actor,
            good,
            qty: Quantity(1),
        },
        EconomyEvent::Consumed {
            actor,
            good,
            qty: Quantity(1),
        },
        EconomyEvent::Regenerated {
            actor,
            good,
            qty: Quantity(1),
        },
    ];
    for event in &transient {
        assert!(
            !event.is_audit_durable(),
            "{} must NOT be audit-durable",
            event.event_type()
        );
    }
}

/// The flush-batch filter keeps durable events in order and collapses the
/// per-tick `TickAudit` heartbeats to the LAST one of the batch — the durable
/// conservation trace lands on the flush cadence (~5 s), not the tick cadence.
#[test]
fn durable_audit_subset_filters_and_keeps_last_tick_audit() {
    use crate::economy::audit::durable_audit_subset;

    let batch = vec![
        EconomyEvent::TickAudit {
            tick: 1,
            total_money: Money(100),
        },
        EconomyEvent::CashLocked {
            actor: EconomicActorId(1),
            amount: Money(1),
        },
        EconomyEvent::WagePaid {
            firm: EconomicActorId(2),
            market: MarketId(1),
            amount: Money(5),
        },
        EconomyEvent::TickAudit {
            tick: 2,
            total_money: Money(100),
        },
        EconomyEvent::Produced {
            actor: EconomicActorId(3),
            good: GOOD_TOOLS,
            qty: Quantity(1),
        },
        EconomyEvent::TickAudit {
            tick: 3,
            total_money: Money(100),
        },
    ];

    let durable = durable_audit_subset(&batch);
    assert_eq!(
        durable,
        vec![
            EconomyEvent::WagePaid {
                firm: EconomicActorId(2),
                market: MarketId(1),
                amount: Money(5),
            },
            EconomyEvent::TickAudit {
                tick: 3,
                total_money: Money(100),
            },
        ],
        "transient events dropped; only the last TickAudit of the batch survives"
    );

    assert!(
        durable_audit_subset(&[]).is_empty(),
        "empty batch stays empty"
    );
}

/// B2 (2026-06-10 design): the audit store is prunable to a rolling row cap —
/// `prune(world, keep_last)` retains only the most recent `keep_last` events.
#[tokio::test]
async fn in_memory_event_store_prune_keeps_last_n() {
    use crate::persistence::EconomyEventStore;

    let mut store = InMemoryEconomyEventStore::default();
    let world = "test:prune";
    for tick in 0..10u64 {
        store
            .append(
                world,
                tick,
                &[EconomyEvent::WagePaid {
                    firm: EconomicActorId(tick),
                    market: MarketId(1),
                    amount: Money(1),
                }],
            )
            .await
            .unwrap();
    }
    store
        .append(
            "test:other-world",
            1,
            &[EconomyEvent::TransportRebate { amount: Money(1) }],
        )
        .await
        .unwrap();

    let deleted = store.prune(world, 3).await.unwrap();
    assert_eq!(deleted, 7, "prune reports the number of deleted rows");
    let kept: Vec<u64> = store.events(world).iter().map(|(tick, _)| *tick).collect();
    assert_eq!(
        kept,
        vec![7, 8, 9],
        "only the newest keep_last events remain"
    );
    assert_eq!(
        store.len("test:other-world"),
        1,
        "prune is scoped to the given world"
    );

    // Under-cap prune is a no-op.
    let deleted = store.prune(world, 100).await.unwrap();
    assert_eq!(deleted, 0);
    assert_eq!(store.len(world), 3);
}
