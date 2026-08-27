//! Shared in-memory [`KvStore`] test fake — the canonical fixture every
//! `DetKv`-backed test wires against instead of hand-rolling its own copy.
//!
//! Was independently duplicated across 14 files (`wallet_backend/kv.rs` plus
//! 12 other test modules in `wallet_backend/`, `context/`, and
//! `backend_task/migration/`) with byte-identical `get`/`put`/`delete`
//! bodies. Consolidated here following the `leak_test_support` pattern.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};

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

/// An [`InMemoryKv`] whose reads can be made to fail on demand, counting every
/// `put` that reaches the store.
///
/// Models a transient backing-store failure (a poisoned persister lock, a SQLite
/// hiccup) so callers can be held to the rule a failed read imposes: never write
/// a value back over a blob you could not read. `put_count` is the assertion
/// handle — a caller that "recovers" from a read error by persisting defaults
/// shows up as an extra put.
#[derive(Default)]
pub(crate) struct FailingKv {
    inner: InMemoryKv,
    fail_reads: AtomicBool,
    fail_deletes: AtomicBool,
    puts: AtomicUsize,
}

impl FailingKv {
    /// Make every subsequent `get` fail with [`KvError::LockPoisoned`] (`true`),
    /// or restore normal reads (`false`). Stored values are never touched, so a
    /// read armed to fail and then restored still yields the original blob.
    pub(crate) fn fail_reads(&self, fail: bool) {
        self.fail_reads.store(fail, Ordering::Relaxed);
    }

    /// Make every subsequent `delete` fail with [`KvError::LockPoisoned`]
    /// (`true`), or restore normal deletes (`false`). The stored value is left
    /// in place, which is what a real failed delete leaves behind.
    pub(crate) fn fail_deletes(&self, fail: bool) {
        self.fail_deletes.store(fail, Ordering::Relaxed);
    }

    /// How many `put` calls have reached the store.
    pub(crate) fn put_count(&self) -> usize {
        self.puts.load(Ordering::Relaxed)
    }
}

impl KvStore for FailingKv {
    fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
        if self.fail_reads.load(Ordering::Relaxed) {
            return Err(KvError::LockPoisoned);
        }
        self.inner.get(scope, key)
    }

    fn put(&self, scope: &ObjectId, key: &str, value: &[u8]) -> Result<(), KvError> {
        // Counted before delegating: an attempted write is what the assertions
        // are about, whether or not the store would have accepted it.
        self.puts.fetch_add(1, Ordering::Relaxed);
        self.inner.put(scope, key, value)
    }

    fn delete(&self, scope: &ObjectId, key: &str) -> Result<(), KvError> {
        if self.fail_deletes.load(Ordering::Relaxed) {
            return Err(KvError::LockPoisoned);
        }
        self.inner.delete(scope, key)
    }

    fn list_keys(&self, scope: &ObjectId, prefix: Option<&str>) -> Result<Vec<String>, KvError> {
        self.inner.list_keys(scope, prefix)
    }
}

/// An [`InMemoryKv`] that stalls *after* each read has taken its snapshot.
///
/// Widens the window in which two concurrent read-modify-write mutations of
/// one key both observe the pre-mutation state, so an unserialized mutation
/// *usually* loses its peer's update.
///
/// Only usually: elapsed time establishes no happens-before between threads.
/// A delayed thread can wake and write before its peer has even reached the
/// read, in which case the peer observes the completed write and nothing is
/// lost — so a test built on this fake can pass against code whose
/// serialization was removed. Use it to make a race *likely* (a scheduling
/// probe); use [`RendezvousKv`] when a test has to be the standing guard for
/// an invariant, since only that one makes the interleaving certain.
#[derive(Default)]
pub(crate) struct StallingReadKv {
    inner: InMemoryKv,
}

impl KvStore for StallingReadKv {
    fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
        let value = self.inner.get(scope, key);
        std::thread::sleep(std::time::Duration::from_millis(200));
        value
    }

    fn put(&self, scope: &ObjectId, key: &str, value: &[u8]) -> Result<(), KvError> {
        self.inner.put(scope, key, value)
    }

    fn delete(&self, scope: &ObjectId, key: &str) -> Result<(), KvError> {
        self.inner.delete(scope, key)
    }

    fn list_keys(&self, scope: &ObjectId, prefix: Option<&str>) -> Result<Vec<String>, KvError> {
        self.inner.list_keys(scope, prefix)
    }
}

/// How long a [`RendezvousKv`] reader waits for its peers before giving up.
///
/// Reached only when the code under test is correctly serialized — the peers
/// are blocked on its lock and can never arrive — so this bounds how long such
/// a test runs and nothing else. It cannot change an outcome: what makes the
/// second read observe the first write is the lock, not the clock.
const RENDEZVOUS_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

/// An [`InMemoryKv`] that holds every read until all armed readers have taken
/// their snapshot.
///
/// The standing guard for lost-update invariants, and the reason it is not a
/// sleep: arming for `readers` makes the interleaving *certain* rather than
/// likely. Every armed reader is released only once all of them hold the
/// pre-mutation snapshot, so an unserialized read-modify-write always loses
/// its peer's update and the test fails every time, on every runner. A
/// correctly serialized caller never satisfies the rendezvous at all — its
/// peers are still queued behind its lock — and proceeds after
/// [`RENDEZVOUS_TIMEOUT`], observing each other's writes in order.
///
/// Reads are unrestricted until [`Self::arm`] is called, so a test can seed
/// state through the same store before the concurrent phase begins.
#[derive(Default)]
pub(crate) struct RendezvousKv {
    inner: InMemoryKv,
    state: Mutex<RendezvousState>,
    released: Condvar,
}

#[derive(Default)]
struct RendezvousState {
    /// Readers that must arrive before any is released. `None` = unarmed.
    expected: Option<usize>,
    arrived: usize,
}

impl RendezvousKv {
    /// Hold the next reads until `readers` of them have taken their snapshot.
    pub(crate) fn arm(&self, readers: usize) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.expected = Some(readers);
        state.arrived = 0;
    }

    /// Block until every armed reader has snapshotted, or the wait times out.
    fn rendezvous(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(expected) = state.expected else {
            return;
        };
        state.arrived += 1;
        if state.arrived >= expected {
            self.released.notify_all();
            return;
        }
        let _ = self
            .released
            .wait_timeout_while(state, RENDEZVOUS_TIMEOUT, |state| {
                state
                    .expected
                    .is_some_and(|expected| state.arrived < expected)
            });
    }
}

impl KvStore for RendezvousKv {
    fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
        let value = self.inner.get(scope, key);
        self.rendezvous();
        value
    }

    fn put(&self, scope: &ObjectId, key: &str, value: &[u8]) -> Result<(), KvError> {
        self.inner.put(scope, key, value)
    }

    fn delete(&self, scope: &ObjectId, key: &str) -> Result<(), KvError> {
        self.inner.delete(scope, key)
    }

    fn list_keys(&self, scope: &ObjectId, prefix: Option<&str>) -> Result<Vec<String>, KvError> {
        self.inner.list_keys(scope, prefix)
    }
}
