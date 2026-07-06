//! Ledger audit flush bookkeeping. The sim-server persistence loop drains new
//! `TradeLedger` events into the durable `EconomyEventStore` on the snapshot
//! cadence (append), then bounds the live ledger. These helpers own the cursor +
//! trim so the flush is robust if the economy appended more events mid-flush.

use bevy_ecs::prelude::*;

use crate::clock::WorldClock;
use crate::econ::{AccountBook, EconomyError, EconomyEvent, PERSISTED_LEDGER_TAIL, TradeLedger};

/// Index into `TradeLedger.0` of the first event NOT yet durably appended to the
/// audit store. Everything before it is durable.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LedgerAuditCursor(pub usize);

/// The previous tick's `total_money`, for the per-tick byte-invariance check. EPHEMERAL —
/// NOT persisted (re-initialized from the restored, conserved `total_money` on the first audit
/// tick after a hydrate, so it stays consistent across restarts without a snapshot field).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LastTickMoney(pub Option<crate::econ::Money>);

/// Per-tick SFC conservation audit (pure over its refs). Reads `total_money` (= Σ available+locked,
/// which is byte-CONSTANT after seed since every runtime money move is a conservative transfer),
/// asserts it equals the prior tick's value (drift ⇒ `Err(ConservationViolation)`), emits a
/// `TickAudit` heartbeat event, and updates the ephemeral baseline. Moves NO money. Deterministic.
pub fn run_tick_audit_at_tick(
    accounts: &AccountBook,
    ledger: &mut TradeLedger,
    last: &mut LastTickMoney,
    current_tick: u64,
) -> Result<(), EconomyError> {
    let total = accounts.total_money()?;
    if let Some(prev) = last.0
        && total != prev
    {
        return Err(EconomyError::ConservationViolation);
    }
    ledger.0.push(EconomyEvent::TickAudit {
        tick: current_tick,
        total_money: total,
    });
    last.0 = Some(total);
    Ok(())
}

/// The durable subset of a flush batch: drops transient events
/// (`!is_audit_durable`) and collapses the per-tick `TickAudit` heartbeats to
/// the LAST one of the batch — the durable conservation trace lands on the
/// flush cadence (~5 s), not the tick cadence, which would otherwise dominate
/// the table again at full tick speed. Order is preserved. The caller still
/// commits the FULL batch length so the cursor advances past transient events.
pub fn durable_audit_subset(batch: &[EconomyEvent]) -> Vec<EconomyEvent> {
    let last_tick_audit = batch
        .iter()
        .rposition(|event| matches!(event, EconomyEvent::TickAudit { .. }));
    batch
        .iter()
        .enumerate()
        .filter(|(index, event)| {
            event.is_audit_durable()
                && (!matches!(event, EconomyEvent::TickAudit { .. })
                    || Some(*index) == last_tick_audit)
        })
        .map(|(_, event)| event.clone())
        .collect()
}

/// The un-appended tail of the ledger + the current tick. Non-mutating: only
/// `commit_ledger_audit` advances the cursor / trims, so a failed append (no
/// commit) leaves all state unchanged — the events are retried next cycle.
pub fn pending_ledger_audit(world: &World) -> (u64, Vec<EconomyEvent>) {
    let tick = world
        .get_resource::<WorldClock>()
        .map_or(0, |c| c.world_tick);
    let cursor = world.get_resource::<LedgerAuditCursor>().map_or(0, |c| c.0);
    let pending = world
        .get_resource::<TradeLedger>()
        .map(|l| l.0.get(cursor..).unwrap_or(&[]).to_vec())
        .unwrap_or_default();
    (tick, pending)
}

/// After a successful append of `appended` events, advance the cursor past them
/// and trim the live ledger to the last `PERSISTED_LEDGER_TAIL` events — but never
/// trim un-appended events (the trim is clamped to the cursor). Robust if the
/// economy pushed more events while the async append was in flight: those stay
/// pending for the next cycle.
pub fn commit_ledger_audit(world: &mut World, appended: usize) {
    let mut cursor = world.get_resource::<LedgerAuditCursor>().map_or(0, |c| c.0);
    if let Some(mut ledger) = world.get_resource_mut::<TradeLedger>() {
        let len = ledger.0.len();
        cursor = (cursor + appended).min(len);
        if len > PERSISTED_LEDGER_TAIL {
            let drain = (len - PERSISTED_LEDGER_TAIL).min(cursor);
            ledger.0.drain(0..drain);
            cursor -= drain;
        }
    } else {
        cursor += appended;
    }
    world.insert_resource(LedgerAuditCursor(cursor));
}

/// Initialize the cursor to the current ledger length. Called once after economy
/// hydration so the restored `ledger_tail` (already durably appended before
/// shutdown) is not re-appended (best-effort: a crash between the last flush and
/// shutdown loses at most the un-flushed window, acceptable for an audit log).
pub fn init_ledger_audit_cursor(world: &mut World) {
    let len = world.get_resource::<TradeLedger>().map_or(0, |l| l.0.len());
    world.insert_resource(LedgerAuditCursor(len));
}
