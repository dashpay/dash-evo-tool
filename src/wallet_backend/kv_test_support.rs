//! Shared in-memory [`KvStore`] test fake — the canonical fixture every
//! `DetKv`-backed test wires against instead of hand-rolling its own copy.
//!
//! Was independently duplicated across 14 files (`wallet_backend/kv.rs` plus
//! 12 other test modules in `wallet_backend/`, `context/`, and
//! `backend_task/migration/`) with byte-identical `get`/`put`/`delete`
//! bodies. Consolidated here following the `leak_test_support` pattern.

use std::sync::Mutex;

use platform_wallet_storage::{KvError, KvStore, ObjectId};

/// In-memory `KvStore` implementation for adapter/view tests.
///
/// Models every [`ObjectId`] scope FK-free (no parent-existence checks) so
/// callers can be exercised without a real `SqlitePersister`:
/// - each scope is an independent slot;
/// - `put` is upsert;
/// - `delete` is idempotent;
/// - `list_keys` supports an optional prefix and returns sorted keys.
///
/// Upstream `ObjectId` is not `Ord`, so the backing store is a flat `Vec`
/// scanned by `PartialEq` rather than a map. LIKE-pattern escaping is
/// irrelevant here — colon separators are not pattern metacharacters — so
/// prefix matching is plain `str::starts_with`.
#[derive(Default)]
pub(crate) struct InMemoryKv {
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

    fn list_keys(&self, scope: &ObjectId, prefix: Option<&str>) -> Result<Vec<String>, KvError> {
        let pred = |k: &str| -> bool { prefix.is_none_or(|p| k.starts_with(p)) };
        let mut keys: Vec<String> = self
            .slots
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, k, _)| s == scope && pred(k))
            .map(|(_, k, _)| k.clone())
            .collect();
        keys.sort();
        Ok(keys)
    }
}
