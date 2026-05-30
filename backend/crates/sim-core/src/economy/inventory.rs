use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{EconomicActorId, EconomyError, GoodId, Quantity};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InventoryBalance {
    pub available: Quantity,
    pub locked: Quantity,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct InventoryBook {
    pub balances: BTreeMap<(EconomicActorId, GoodId), InventoryBalance>,
}

impl InventoryBook {
    pub fn balance(&self, actor: EconomicActorId, good: GoodId) -> InventoryBalance {
        self.balances
            .get(&(actor, good))
            .copied()
            .unwrap_or_default()
    }

    pub fn deposit(
        &mut self,
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    ) -> Result<(), EconomyError> {
        if qty.0 < 0 {
            return Err(EconomyError::NegativeQuantity);
        }
        let mut balance = self.balance(actor, good);
        balance.available = balance.available.checked_add(qty)?;
        self.balances.insert((actor, good), balance);
        Ok(())
    }

    pub fn lock_goods(
        &mut self,
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    ) -> Result<(), EconomyError> {
        if qty.0 < 0 {
            return Err(EconomyError::NegativeQuantity);
        }
        let mut balance = self.balance(actor, good);
        if balance.available < qty {
            return Err(EconomyError::InsufficientGoods);
        }
        balance.available = balance.available.checked_sub(qty)?;
        balance.locked = balance.locked.checked_add(qty)?;
        self.balances.insert((actor, good), balance);
        Ok(())
    }

    pub fn release_goods(
        &mut self,
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    ) -> Result<(), EconomyError> {
        let mut balance = self.balance(actor, good);
        if balance.locked < qty {
            return Err(EconomyError::InsufficientGoods);
        }
        balance.locked = balance.locked.checked_sub(qty)?;
        balance.available = balance.available.checked_add(qty)?;
        self.balances.insert((actor, good), balance);
        Ok(())
    }

    pub fn debit_locked_goods(
        &mut self,
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    ) -> Result<(), EconomyError> {
        let mut balance = self.balance(actor, good);
        if balance.locked < qty {
            return Err(EconomyError::InsufficientGoods);
        }
        balance.locked = balance.locked.checked_sub(qty)?;
        self.balances.insert((actor, good), balance);
        Ok(())
    }

    pub fn consume(
        &mut self,
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    ) -> Result<(), EconomyError> {
        if qty.0 < 0 {
            return Err(EconomyError::NegativeQuantity);
        }
        let mut balance = self.balance(actor, good);
        if balance.available < qty {
            return Err(EconomyError::InsufficientGoods);
        }
        balance.available = balance.available.checked_sub(qty)?;
        self.balances.insert((actor, good), balance);
        Ok(())
    }

    pub fn total_good(&self, good: GoodId) -> Result<Quantity, EconomyError> {
        self.balances
            .iter()
            .filter(|((_, item_good), _)| *item_good == good)
            .map(|(_, balance)| *balance)
            .try_fold(Quantity::ZERO, |sum, balance| {
                sum.checked_add(balance.available)?
                    .checked_add(balance.locked)
            })
    }
}
