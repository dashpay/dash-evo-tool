//! Per-`(identity, token)` balance cache seam (T6).
//!
//! [`TokenBalanceView`] is the doorway DET code uses to read or write the
//! raw `u64` Platform token balance for an `(identity, token)` pair.
//!
//! ## Why a seam, and why the cache is still active
//!
//! At 08b0ed9 `PlatformWalletManager` exposes no public per-`(identity,
//! token)` balance reader (the only public balance accessors are the
//! core-chain `WalletBalance` and the platform-address credit balance —
//! neither is the DPP token balance DET shows on the My Tokens screen).
//! So DET keeps caching token balances itself, under
//! `det:token_balance:<identity>:<token>` in the per-network wallet k/v
//! store, via the ACTIVE [`KvCachedTokenBalances`] impl — zero behaviour
//! change.
//!
//! [`UpstreamTokenBalances`] is the reserved swap target: once upstream
//! adds a public per-token balance reader, implement it for real, make it
//! ACTIVE, and delete `det:token_balance`. Until then its read body is
//! stubbed (platform todo `f5897abd`).
//!
//! The view speaks DET / SDK domain types ([`Identifier`]); no upstream
//! `ObjectId` / `WalletId` / `PlatformWalletManager` type crosses the
//! seam.

use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;

use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};

const TOKEN_BALANCE_KEY_PREFIX: &str = "det:token_balance:";

fn token_balance_key(identity_id: &Identifier, token_id: &Identifier) -> String {
    format!(
        "{}{}:{}",
        TOKEN_BALANCE_KEY_PREFIX,
        identity_id.to_string(Encoding::Base58),
        token_id.to_string(Encoding::Base58),
    )
}

fn parse_token_balance_key(key: &str) -> Option<(Identifier, Identifier)> {
    let suffix = key.strip_prefix(TOKEN_BALANCE_KEY_PREFIX)?;
    let (identity_b58, token_b58) = suffix.split_once(':')?;
    let identity = Identifier::from_string(identity_b58, Encoding::Base58).ok()?;
    let token = Identifier::from_string(token_b58, Encoding::Base58).ok()?;
    Some((identity, token))
}

/// Read/write the raw per-`(identity, token)` token balance. Registry
/// decoration (alias / config / order list) stays with the caller; this
/// view owns only the balance slot.
pub trait TokenBalanceView: Send + Sync {
    /// Balance for one `(identity, token)` pair, or `Ok(None)` if absent.
    fn get(
        &self,
        identity_id: &Identifier,
        token_id: &Identifier,
    ) -> Result<Option<u64>, KvAdapterError>;

    /// Upsert the balance for one `(identity, token)` pair.
    fn set(
        &self,
        identity_id: &Identifier,
        token_id: &Identifier,
        balance: u64,
    ) -> Result<(), KvAdapterError>;

    /// Delete the balance for one `(identity, token)` pair. Idempotent.
    fn delete(&self, identity_id: &Identifier, token_id: &Identifier)
    -> Result<(), KvAdapterError>;

    /// Every stored `(identity, token, balance)` triple. Malformed keys
    /// are skipped (logged at warn) so one bad row cannot block the list.
    fn list(&self) -> Result<Vec<(Identifier, Identifier, u64)>, KvAdapterError>;
}

/// ACTIVE impl: balances live in the per-network wallet k/v store under
/// `det:token_balance:*` in the global scope.
pub struct KvCachedTokenBalances {
    kv: DetKv,
}

impl KvCachedTokenBalances {
    pub fn new(kv: DetKv) -> Self {
        Self { kv }
    }
}

impl TokenBalanceView for KvCachedTokenBalances {
    fn get(
        &self,
        identity_id: &Identifier,
        token_id: &Identifier,
    ) -> Result<Option<u64>, KvAdapterError> {
        self.kv
            .get(DetScope::Global, &token_balance_key(identity_id, token_id))
    }

    fn set(
        &self,
        identity_id: &Identifier,
        token_id: &Identifier,
        balance: u64,
    ) -> Result<(), KvAdapterError> {
        self.kv.put(
            DetScope::Global,
            &token_balance_key(identity_id, token_id),
            &balance,
        )
    }

    fn delete(
        &self,
        identity_id: &Identifier,
        token_id: &Identifier,
    ) -> Result<(), KvAdapterError> {
        self.kv
            .delete(DetScope::Global, &token_balance_key(identity_id, token_id))
    }

    fn list(&self) -> Result<Vec<(Identifier, Identifier, u64)>, KvAdapterError> {
        let keys = self
            .kv
            .list(DetScope::Global, Some(TOKEN_BALANCE_KEY_PREFIX))?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let Some((identity_id, token_id)) = parse_token_balance_key(&key) else {
                tracing::warn!(key = %key, "Skipping malformed token-balance key");
                continue;
            };
            let balance: u64 = match self.kv.get(DetScope::Global, &key) {
                Ok(Some(v)) => v,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(key = %key, error = ?e, "Skipping unreadable token balance");
                    continue;
                }
            };
            out.push((identity_id, token_id, balance));
        }
        Ok(out)
    }
}

/// Reserved swap target: read token balances from upstream instead of
/// DET's cache. Not selected — read methods stubbed until upstream exposes
/// a public per-`(identity, token)` balance reader on
/// `PlatformWalletManager` (platform todo `f5897abd`).
pub struct UpstreamTokenBalances;

impl TokenBalanceView for UpstreamTokenBalances {
    fn get(
        &self,
        _identity_id: &Identifier,
        _token_id: &Identifier,
    ) -> Result<Option<u64>, KvAdapterError> {
        // TODO(f5897abd): public per-(identity, token) balance reader.
        unimplemented!("pending platform todo f5897abd — public per-token balance reader")
    }

    fn set(
        &self,
        _identity_id: &Identifier,
        _token_id: &Identifier,
        _balance: u64,
    ) -> Result<(), KvAdapterError> {
        // Writes become no-ops once balances are read straight from upstream.
        // TODO(f5897abd): drop with the cache.
        Ok(())
    }

    fn delete(
        &self,
        _identity_id: &Identifier,
        _token_id: &Identifier,
    ) -> Result<(), KvAdapterError> {
        // TODO(f5897abd): native cascade replaces explicit deletion.
        Ok(())
    }

    fn list(&self) -> Result<Vec<(Identifier, Identifier, u64)>, KvAdapterError> {
        // TODO(f5897abd): public per-token balance enumeration.
        unimplemented!("pending platform todo f5897abd — public per-token balance reader")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::DetKv;
    use platform_wallet_storage::{KvError, KvStore, ObjectId};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct InMemoryKv {
        slots: Mutex<Vec<(ObjectId, String, Vec<u8>)>>,
    }

    impl KvStore for InMemoryKv {
        fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            Ok(self
                .slots
                .lock()
                .unwrap()
                .iter()
                .find(|(s, k, _)| s == scope && k == key)
                .map(|(_, _, v)| v.clone()))
        }
        fn put(&self, scope: &ObjectId, key: &str, value: &[u8]) -> Result<(), KvError> {
            let mut slots = self.slots.lock().unwrap();
            if let Some(slot) = slots.iter_mut().find(|(s, k, _)| s == scope && k == key) {
                slot.2 = value.to_vec();
            } else {
                slots.push((scope.clone(), key.to_string(), value.to_vec()));
            }
            Ok(())
        }
        fn delete(&self, scope: &ObjectId, key: &str) -> Result<(), KvError> {
            self.slots
                .lock()
                .unwrap()
                .retain(|(s, k, _)| !(s == scope && k == key));
            Ok(())
        }
        fn list_keys(
            &self,
            scope: &ObjectId,
            prefix: Option<&str>,
        ) -> Result<Vec<String>, KvError> {
            let pred = |k: &str| -> bool { prefix.is_none_or(|p| k.starts_with(p)) };
            Ok(self
                .slots
                .lock()
                .unwrap()
                .iter()
                .filter(|(s, k, _)| s == scope && pred(k))
                .map(|(_, k, _)| k.clone())
                .collect())
        }
    }

    fn view() -> KvCachedTokenBalances {
        KvCachedTokenBalances::new(DetKv::from_store(Arc::new(InMemoryKv::default())))
    }

    fn id(b: u8) -> Identifier {
        Identifier::from([b; 32])
    }

    /// TB1: a balance round-trips through the active cache.
    #[test]
    fn balance_round_trips() {
        let v = view();
        v.set(&id(1), &id(2), 5_000).unwrap();
        assert_eq!(v.get(&id(1), &id(2)).unwrap(), Some(5_000));
    }

    /// TB2: `list` returns every stored triple; the `(identity, token)`
    /// pair survives the key round-trip.
    #[test]
    fn list_returns_all_triples() {
        let v = view();
        v.set(&id(1), &id(2), 10).unwrap();
        v.set(&id(3), &id(4), 20).unwrap();
        let mut got = v.list().unwrap();
        got.sort_by_key(|(_, _, bal)| *bal);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], (id(1), id(2), 10));
        assert_eq!(got[1], (id(3), id(4), 20));
    }

    /// TB3: delete drops only the targeted pair.
    #[test]
    fn delete_is_targeted() {
        let v = view();
        v.set(&id(1), &id(2), 1).unwrap();
        v.set(&id(1), &id(9), 2).unwrap();
        v.delete(&id(1), &id(2)).unwrap();
        assert_eq!(v.get(&id(1), &id(2)).unwrap(), None);
        assert_eq!(v.get(&id(1), &id(9)).unwrap(), Some(2));
    }
}
