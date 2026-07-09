//! Poison-recovery for locks guarding **derived, rebuildable** in-memory state.
//!
//! The free functions ([`read_recover`], [`write_recover`], [`lock_recover`])
//! and their extension-trait forms ([`RwLockRecover`], [`MutexRecover`]) recover
//! a poisoned guard (`PoisonError::into_inner`) instead of propagating the
//! poison. They apply across the app wherever a lock protects state that is not
//! a source of truth and can always be re-derived: the wallet backend's
//! single-key in-memory index (rebuilt from the k/v sidecar plus the secret
//! vault) and secret session cache (a TTL'd convenience over the vault); the
//! loaded-wallet map and per-screen view state (`RwLock<Wallet>`,
//! `RwLock<WalletFundedScreenStep>`, the settings cache); the Identities / DPNS /
//! token-search screen caches (`Mutex`, re-fetched on demand); the coordinator
//! gate's action slot; and the SQLite connection mutex (a `rusqlite::Connection`
//! carries no cross-call invariant a panic can break).
//!
//! A thread that panics mid-update poisons the lock, but the guarded value is at
//! worst missing or holding a single stale entry — never a corruption that risks
//! funds — and the next read re-derives it. Failing every subsequent operation
//! because an unrelated thread panicked (the default poison behavior) would turn
//! a recoverable, self-healing subsystem into a dead one, so recovery is the
//! correct policy. Use these helpers for rebuildable state only — a lock
//! guarding a non-reconstructable invariant should keep failing (via the blanket
//! `From<PoisonError<T>>` for `TaskError::LockPoisoned`), and a lock whose
//! poison must fail loud for fund safety (the DashPay address-index mutex) keeps
//! its explicit `.expect(...)`.

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Acquire a read guard, recovering it if the lock is poisoned.
///
/// Recovery is safe for the rebuildable in-memory state these locks guard (see
/// the module docs); a poisoned lock yields the same guard the happy path would.
pub(crate) fn read_recover<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

/// Acquire a write guard, recovering it if the lock is poisoned.
///
/// Recovery is safe for the rebuildable in-memory state these locks guard (see
/// the module docs); a poisoned lock yields the same guard the happy path would.
pub(crate) fn write_recover<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

/// Ergonomic `.read_recover()` / `.write_recover()` on `RwLock`, delegating to
/// [`read_recover`] / [`write_recover`].
///
/// Lets call sites read as naturally as `.read().unwrap()` while routing every
/// lock acquisition through the single recovery policy — no panic on a poisoned
/// lock. Same contract as the free functions: rebuildable in-memory state only.
/// A lock guarding a non-reconstructable invariant must keep failing via the
/// blanket `From<PoisonError<T>>` for `TaskError::LockPoisoned` instead.
pub(crate) trait RwLockRecover<T> {
    /// Acquire a read guard, recovering a poisoned lock instead of panicking.
    fn read_recover(&self) -> RwLockReadGuard<'_, T>;
    /// Acquire a write guard, recovering a poisoned lock instead of panicking.
    fn write_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> RwLockRecover<T> for RwLock<T> {
    fn read_recover(&self) -> RwLockReadGuard<'_, T> {
        read_recover(self)
    }
    fn write_recover(&self) -> RwLockWriteGuard<'_, T> {
        write_recover(self)
    }
}

/// Acquire a `Mutex` guard, recovering it if the lock is poisoned.
///
/// Recovery is safe for the rebuildable in-memory state these locks guard (see
/// the module docs); a poisoned lock yields the same guard the happy path would.
pub(crate) fn lock_recover<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Ergonomic `.lock_recover()` on `Mutex`, delegating to [`lock_recover`].
///
/// The `Mutex` analog of [`RwLockRecover`]: routes lock acquisition through the
/// single recovery policy so a thread panicking mid-update never wedges the
/// subsystem. Same contract — rebuildable in-memory state only.
pub(crate) trait MutexRecover<T> {
    /// Acquire the guard, recovering a poisoned lock instead of panicking.
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexRecover<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        lock_recover(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A thread that panics while holding the write lock poisons it; both
    /// helpers must still hand back a usable guard exposing the value written
    /// before the panic — proving the subsystem self-heals instead of wedging.
    #[test]
    fn recovers_a_poisoned_lock() {
        let lock = Arc::new(RwLock::new(41u32));

        let poisoner = Arc::clone(&lock);
        let handle = std::thread::spawn(move || {
            let mut guard = poisoner.write().expect("first writer");
            *guard = 42;
            panic!("poison the lock while holding the write guard");
        });
        assert!(handle.join().is_err(), "the poisoning thread must panic");
        assert!(
            lock.is_poisoned(),
            "the lock must be poisoned after the panic"
        );

        // Read recovery sees the value written before the panic.
        assert_eq!(*read_recover(&lock), 42, "read_recover yields the value");
        // Write recovery yields a usable guard the operation can proceed with.
        *write_recover(&lock) = 7;
        assert_eq!(
            *read_recover(&lock),
            7,
            "write_recover yields a usable guard"
        );
    }
}
