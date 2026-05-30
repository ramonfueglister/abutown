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
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = production.0.keys().copied().collect();
    for actor in actors {
        let pool = production.0[&actor].clone();
        if !interval_elapsed(pool.last_generated_tick, current_tick, pool.interval_ticks) {
            continue;
        }
        // All inputs must be covered before consuming any (atomic per pool).
        let can_produce = pool
            .recipe
            .inputs
            .iter()
            .all(|(good, qty)| inventory.balance(actor, *good).available >= *qty);
        if can_produce {
            for (good, qty) in &pool.recipe.inputs {
                inventory.consume(actor, *good, *qty)?;
                ledger.0.push(EconomyEvent::Consumed {
                    actor,
                    good: *good,
                    qty: *qty,
                });
            }
            for (good, qty) in &pool.recipe.outputs {
                inventory.deposit(actor, *good, *qty)?;
                ledger.0.push(EconomyEvent::Produced {
                    actor,
                    good: *good,
                    qty: *qty,
                });
            }
        }
        if let Some(p) = production.0.get_mut(&actor) {
            p.last_generated_tick = Some(current_tick);
        }
    }
    Ok(())
}
