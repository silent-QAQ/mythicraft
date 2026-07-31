use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

pub const ECONOMY_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Transaction {
    pub id: Uuid,
    pub player: Uuid,
    pub reason: String,
    pub amount: i64,
    pub before: i64,
    pub after: i64,
    pub tick: u64,
    pub config_hash: String,
    pub committed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Transfer {
    pub id: Uuid,
    pub from: Uuid,
    pub to: Uuid,
    pub amount: i64,
    pub from_before: i64,
    pub from_after: i64,
    pub to_before: i64,
    pub to_after: i64,
    pub reason: String,
    pub tick: u64,
    pub config_hash: String,
    pub committed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EconomyState {
    pub schema_version: u32,
    pub currency_name: String,
    pub balances: HashMap<Uuid, i64>,
    pub transactions: HashMap<Uuid, Transaction>,
    pub transfers: HashMap<Uuid, Transfer>,
}

#[derive(Debug, Error, PartialEq)]
pub enum EconomyStateError {
    #[error("unsupported economy state schema {0}")]
    UnsupportedSchema(u32),
    #[error("invalid economy state: {0}")]
    Invalid(String),
    #[error("invalid economy state JSON: {0}")]
    Json(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum EconomyError {
    #[error("insufficient funds")]
    InsufficientFunds,
    #[error("transaction already exists")]
    DuplicateTransaction,
    #[error("amount must be positive")]
    InvalidAmount,
    #[error("source and destination must differ")]
    SameAccount,
    #[error("balance overflow")]
    BalanceOverflow,
}

#[derive(Debug)]
pub struct Economy {
    currency_name: String,
    balances: HashMap<Uuid, i64>,
    transactions: HashMap<Uuid, Transaction>,
    transfers: HashMap<Uuid, Transfer>,
}
impl Default for Economy {
    fn default() -> Self {
        Self {
            currency_name: "coins".into(),
            balances: HashMap::new(),
            transactions: HashMap::new(),
            transfers: HashMap::new(),
        }
    }
}
impl Economy {
    pub fn new(currency_name: impl Into<String>) -> Self {
        Self {
            currency_name: currency_name.into(),
            ..Default::default()
        }
    }
    pub fn currency_name(&self) -> &str {
        &self.currency_name
    }
    pub fn balance(&self, player: Uuid) -> i64 {
        *self.balances.get(&player).unwrap_or(&0)
    }

    /// Restores a player's balance from the native persistence layer.
    ///
    /// This deliberately does not create a synthetic transaction: loading a
    /// save is state hydration, not an economic operation or audit event.
    pub fn restore_balance(&mut self, player: Uuid, balance: i64) -> Result<(), EconomyStateError> {
        if balance < 0 {
            return Err(EconomyStateError::Invalid(
                "balance cannot be negative".into(),
            ));
        }
        self.balances.insert(player, balance);
        Ok(())
    }
    pub fn deposit(
        &mut self,
        id: Uuid,
        player: Uuid,
        amount: i64,
        reason: &str,
        tick: u64,
        config_hash: &str,
    ) -> Result<Transaction, EconomyError> {
        if amount <= 0 {
            return Err(EconomyError::InvalidAmount);
        }
        self.apply(id, player, amount, reason, tick, config_hash)
    }
    pub fn withdraw(
        &mut self,
        id: Uuid,
        player: Uuid,
        amount: i64,
        reason: &str,
        tick: u64,
        config_hash: &str,
    ) -> Result<Transaction, EconomyError> {
        if amount <= 0 {
            return Err(EconomyError::InvalidAmount);
        }
        if self.balance(player) < amount {
            return Err(EconomyError::InsufficientFunds);
        }
        self.apply(id, player, -amount, reason, tick, config_hash)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn transfer(
        &mut self,
        id: Uuid,
        from: Uuid,
        to: Uuid,
        amount: i64,
        reason: &str,
        tick: u64,
        config_hash: &str,
    ) -> Result<Transfer, EconomyError> {
        if amount <= 0 {
            return Err(EconomyError::InvalidAmount);
        }
        if from == to {
            return Err(EconomyError::SameAccount);
        }
        if self.used_id(id) {
            return Err(EconomyError::DuplicateTransaction);
        }
        let from_before = self.balance(from);
        let to_before = self.balance(to);
        if from_before < amount {
            return Err(EconomyError::InsufficientFunds);
        }
        let from_after = from_before
            .checked_sub(amount)
            .ok_or(EconomyError::BalanceOverflow)?;
        let to_after = to_before
            .checked_add(amount)
            .ok_or(EconomyError::BalanceOverflow)?;
        let transfer = Transfer {
            id,
            from,
            to,
            amount,
            from_before,
            from_after,
            to_before,
            to_after,
            reason: reason.into(),
            tick,
            config_hash: config_hash.into(),
            committed: true,
        };
        self.balances.insert(from, from_after);
        self.balances.insert(to, to_after);
        self.transfers.insert(id, transfer.clone());
        Ok(transfer)
    }
    pub fn audit(&self, id: Uuid) -> Option<&Transaction> {
        self.transactions.get(&id)
    }
    pub fn audit_transfer(&self, id: Uuid) -> Option<&Transfer> {
        self.transfers.get(&id)
    }

    pub fn snapshot(&self) -> EconomyState {
        EconomyState {
            schema_version: ECONOMY_STATE_VERSION,
            currency_name: self.currency_name.clone(),
            balances: self.balances.clone(),
            transactions: self.transactions.clone(),
            transfers: self.transfers.clone(),
        }
    }

    pub fn from_snapshot(state: EconomyState) -> Result<Self, EconomyStateError> {
        if state.schema_version != ECONOMY_STATE_VERSION {
            return Err(EconomyStateError::UnsupportedSchema(state.schema_version));
        }
        if state.currency_name.trim().is_empty() {
            return Err(EconomyStateError::Invalid(
                "currency name must not be empty".into(),
            ));
        }
        if state.balances.values().any(|balance| *balance < 0) {
            return Err(EconomyStateError::Invalid(
                "balance cannot be negative".into(),
            ));
        }
        if state
            .transactions
            .iter()
            .any(|(id, transaction)| id != &transaction.id)
            || state
                .transfers
                .iter()
                .any(|(id, transfer)| id != &transfer.id)
        {
            return Err(EconomyStateError::Invalid(
                "audit map key does not match record id".into(),
            ));
        }
        Ok(Self {
            currency_name: state.currency_name,
            balances: state.balances,
            transactions: state.transactions,
            transfers: state.transfers,
        })
    }

    pub fn to_json(&self) -> Result<String, EconomyStateError> {
        serde_json::to_string_pretty(&self.snapshot())
            .map_err(|error| EconomyStateError::Json(error.to_string()))
    }

    pub fn from_json(source: &str) -> Result<Self, EconomyStateError> {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let state: EconomyState = serde_json::from_str(source)
            .map_err(|error| EconomyStateError::Json(error.to_string()))?;
        Self::from_snapshot(state)
    }

    fn used_id(&self, id: Uuid) -> bool {
        self.transactions.contains_key(&id) || self.transfers.contains_key(&id)
    }
    fn apply(
        &mut self,
        id: Uuid,
        player: Uuid,
        delta: i64,
        reason: &str,
        tick: u64,
        config_hash: &str,
    ) -> Result<Transaction, EconomyError> {
        if self.used_id(id) {
            return Err(EconomyError::DuplicateTransaction);
        }
        let before = self.balance(player);
        let after = before
            .checked_add(delta)
            .ok_or(EconomyError::BalanceOverflow)?;
        let tx = Transaction {
            id,
            player,
            reason: reason.into(),
            amount: delta.unsigned_abs().min(i64::MAX as u64) as i64,
            before,
            after,
            tick,
            config_hash: config_hash.into(),
            committed: true,
        };
        self.balances.insert(player, after);
        self.transactions.insert(id, tx.clone());
        Ok(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_is_atomic_and_idempotent() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        let id = Uuid::new_v4();
        let mut economy = Economy::new("gold");
        economy
            .deposit(Uuid::new_v4(), from, 100, "seed", 1, "cfg")
            .unwrap();
        let transfer = economy
            .transfer(id, from, to, 40, "reward", 2, "cfg")
            .unwrap();
        assert_eq!(economy.currency_name(), "gold");
        assert_eq!(economy.balance(from), 60);
        assert_eq!(economy.balance(to), 40);
        assert_eq!(economy.audit_transfer(id), Some(&transfer));
        assert_eq!(
            economy.transfer(id, from, to, 40, "retry", 3, "cfg"),
            Err(EconomyError::DuplicateTransaction)
        );
        assert_eq!(economy.balance(from), 60);
        assert_eq!(economy.balance(to), 40);
    }
    #[test]
    fn failed_transfer_does_not_change_balances() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        let mut economy = Economy::default();
        assert_eq!(
            economy.transfer(Uuid::new_v4(), from, to, 1, "x", 0, "cfg"),
            Err(EconomyError::InsufficientFunds)
        );
        assert_eq!(economy.balance(from), 0);
        assert_eq!(economy.balance(to), 0);
    }
}
