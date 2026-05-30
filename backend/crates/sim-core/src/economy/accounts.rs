use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{EconomicActorId, EconomyError, Money};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MoneyAccount {
    pub available: Money,
    pub locked: Money,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct AccountBook {
    pub accounts: BTreeMap<EconomicActorId, MoneyAccount>,
}

impl AccountBook {
    pub fn account(&self, actor: EconomicActorId) -> MoneyAccount {
        self.accounts.get(&actor).copied().unwrap_or_default()
    }

    pub fn deposit(&mut self, actor: EconomicActorId, amount: Money) -> Result<(), EconomyError> {
        if amount.0 < 0 {
            return Err(EconomyError::NegativeMoney);
        }
        let mut account = self.account(actor);
        account.available = account.available.checked_add(amount)?;
        self.accounts.insert(actor, account);
        Ok(())
    }

    pub fn lock_cash(&mut self, actor: EconomicActorId, amount: Money) -> Result<(), EconomyError> {
        if amount.0 < 0 {
            return Err(EconomyError::NegativeMoney);
        }
        let mut account = self.account(actor);
        if account.available < amount {
            return Err(EconomyError::InsufficientFunds);
        }
        account.available = account.available.checked_sub(amount)?;
        account.locked = account.locked.checked_add(amount)?;
        self.accounts.insert(actor, account);
        Ok(())
    }

    pub fn release_cash(
        &mut self,
        actor: EconomicActorId,
        amount: Money,
    ) -> Result<(), EconomyError> {
        let mut account = self.account(actor);
        if account.locked < amount {
            return Err(EconomyError::InsufficientFunds);
        }
        account.locked = account.locked.checked_sub(amount)?;
        account.available = account.available.checked_add(amount)?;
        self.accounts.insert(actor, account);
        Ok(())
    }

    pub fn debit_locked(
        &mut self,
        actor: EconomicActorId,
        amount: Money,
    ) -> Result<(), EconomyError> {
        let mut account = self.account(actor);
        if account.locked < amount {
            return Err(EconomyError::InsufficientFunds);
        }
        account.locked = account.locked.checked_sub(amount)?;
        self.accounts.insert(actor, account);
        Ok(())
    }

    pub fn transfer(
        &mut self,
        from: EconomicActorId,
        to: EconomicActorId,
        amount: Money,
    ) -> Result<(), EconomyError> {
        if amount.0 < 0 {
            return Err(EconomyError::NegativeMoney);
        }
        let mut f = self.account(from);
        if f.available < amount {
            return Err(EconomyError::InsufficientFunds);
        }
        f.available = f.available.checked_sub(amount)?;
        let mut t = self.account(to);
        t.available = t.available.checked_add(amount)?;
        self.accounts.insert(from, f);
        self.accounts.insert(to, t);
        Ok(())
    }

    pub fn total_money(&self) -> Result<Money, EconomyError> {
        self.accounts
            .values()
            .try_fold(Money::ZERO, |sum, account| {
                sum.checked_add(account.available)?
                    .checked_add(account.locked)
            })
    }
}
