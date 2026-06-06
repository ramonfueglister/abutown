use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{
    EconomicActorId, EconomyError, EconomyEvent, GoodId, InventoryBook, Quantity, TradeLedger,
    pools::interval_elapsed,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Recipe {
    pub inputs: Vec<(GoodId, Quantity)>,
    pub outputs: Vec<(GoodId, Quantity)>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProductionPool {
    pub actor: EconomicActorId,
    pub recipe: Recipe,
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ProductionPools(pub BTreeMap<EconomicActorId, ProductionPool>);

pub fn run_production_at_tick(
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    production: &mut ProductionPools,
    current_tick: u64,
    capita_factor: i64,
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = production.0.keys().copied().collect();
    for actor in actors {
        let pool = production.0[&actor].clone();
        if !interval_elapsed(pool.last_generated_tick, current_tick, pool.interval_ticks) {
            continue;
        }
        // All inputs must be covered before consuming any (atomic per pool).
        // Scale each qty by capita_factor (factor 1 is byte-identical to pre-scaling).
        // Scale the inputs ONCE and reuse for both the can_produce check and the
        // consume loop, so check == consume by construction (no recompute drift).
        let factor = capita_factor.max(1);
        let scaled_inputs: Vec<(GoodId, Quantity)> = pool
            .recipe
            .inputs
            .iter()
            .map(|(good, qty)| {
                let scaled = i64::try_from((qty.0 as i128) * (factor as i128))
                    .map_err(|_| EconomyError::Overflow)?;
                Ok((*good, Quantity(scaled)))
            })
            .collect::<Result<Vec<_>, EconomyError>>()?;
        let can_produce = scaled_inputs
            .iter()
            .all(|(good, scaled_qty)| inventory.balance(actor, *good).available >= *scaled_qty);
        if can_produce {
            for (good, scaled_qty) in &scaled_inputs {
                inventory.consume(actor, *good, *scaled_qty)?;
                ledger.0.push(EconomyEvent::Consumed {
                    actor,
                    good: *good,
                    qty: *scaled_qty,
                });
            }
            for (good, qty) in &pool.recipe.outputs {
                let scaled_qty = Quantity(
                    i64::try_from((qty.0 as i128) * (factor as i128))
                        .map_err(|_| EconomyError::Overflow)?,
                );
                inventory.deposit(actor, *good, scaled_qty)?;
                ledger.0.push(EconomyEvent::Produced {
                    actor,
                    good: *good,
                    qty: scaled_qty,
                });
            }
        }
        if let Some(p) = production.0.get_mut(&actor) {
            p.last_generated_tick = Some(current_tick);
        }
    }
    Ok(())
}

/// The single named primary-resource extractor. ONE faucet (not N scattered ones),
/// adjacent to the other seeded actor ids (8_001..8_022) but well clear of them.
pub const EXTRACTOR_TOOLS: EconomicActorId = EconomicActorId(8_031);

/// FOOD self-sufficiency: one continuous RAW->FOOD extractor co-located at each FOOD
/// supply market. `_A` sits at m_a (backs finite supplier 8_011), `_FA` at m_fa (backs
/// finite flow supplier 8_021). Adjacent to EXTRACTOR_TOOLS (8_031), clear of 8_001..8_022.
pub const EXTRACTOR_FOOD_A: EconomicActorId = EconomicActorId(8_032);
pub const EXTRACTOR_FOOD_FA: EconomicActorId = EconomicActorId(8_033);

/// A standing raw-goods faucet for one actor. PERSISTED (mirrors `ProductionPool`).
/// `last_regen_tick` is the interval cursor (gates deposits, persists for free since
/// `Option<u64>: Copy` keeps `RawDeposit` `Copy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawDeposit {
    pub good: GoodId,
    pub qty_per_interval: Quantity,
    pub interval_ticks: u64,
    pub last_regen_tick: Option<u64>,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct RawDeposits(pub BTreeMap<EconomicActorId, RawDeposit>);

/// Flow-capped faucet: for each deposit whose `interval_ticks` have elapsed, deposit
/// `qty_per_interval` of `good` into the actor's inventory (goods-only — NEVER touches
/// money) and emit `Regenerated`. Deterministic, keys-first (ascending `EconomicActorId`).
/// Honest wording: deposits unconditionally on the interval (does NOT read the raw stock),
/// so RAW grows without a level cap here — the `RAW->good` recipe in `run_production_at_tick`
/// bounds it (RAW stays `<= 2*qty_per_interval` in the live loop). Stamps `last_regen_tick`
/// ONLY when the deposit fires (the gate returns before stamping on a skip), so a within-
/// interval skip is a true no-op.
pub fn run_regen_at_tick(
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    deposits: &mut RawDeposits,
    current_tick: u64,
    capita_factor: i64,
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = deposits.0.keys().copied().collect();
    for actor in actors {
        let dep = deposits.0[&actor];
        if !interval_elapsed(dep.last_regen_tick, current_tick, dep.interval_ticks) {
            continue;
        }
        let scaled = Quantity(
            i64::try_from((dep.qty_per_interval.0 as i128) * (capita_factor.max(1) as i128))
                .map_err(|_| EconomyError::Overflow)?,
        );
        inventory.deposit(actor, dep.good, scaled)?;
        ledger.0.push(EconomyEvent::Regenerated {
            actor,
            good: dep.good,
            qty: scaled,
        });
        // NO-FALLBACK: the actor came from keys() of the same map this iteration with no
        // removal, so get_mut is infallible. Use .expect (fail loud on the impossible) NOT
        // `if let Some {…}` — a silent skip would drop the cursor stamp and cause a
        // double-deposit next tick.
        deposits
            .0
            .get_mut(&actor)
            .expect("regen deposit actor from keys() must still exist")
            .last_regen_tick = Some(current_tick);
    }
    Ok(())
}
