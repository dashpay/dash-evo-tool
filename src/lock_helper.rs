//! Lock poisoning recovery helpers.
//!
//! Provides extension traits for `Mutex` and `RwLock` that recover from
//! poison errors instead of panicking. When a lock is poisoned (because
//! another thread panicked while holding it), these helpers log a warning
//! and return the inner data, which is still valid for use.
//!
//! # Usage
//!
//! ```ignore
//! use crate::lock_helper::{MutexExt, RwLockExt};
//!
//! let mutex = std::sync::Mutex::new(42);
//! let guard = mutex.lock_or_recover(); // never panics
//!
//! let rwlock = std::sync::RwLock::new(42);
//! let read = rwlock.read_or_recover();   // never panics
//! let write = rwlock.write_or_recover(); // never panics
//! ```

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Extension trait for `Mutex` that recovers from poisoning.
pub trait MutexExt<T> {
    /// Lock the mutex, recovering from poisoning if necessary.
    ///
    /// If the lock is poisoned, logs a warning and returns the inner data.
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("Mutex poisoned, recovering: {}", std::any::type_name::<T>());
            poisoned.into_inner()
        })
    }
}

/// Extension trait for `RwLock` that recovers from poisoning.
pub trait RwLockExt<T> {
    /// Acquire a read lock, recovering from poisoning if necessary.
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T>;

    /// Acquire a write lock, recovering from poisoning if necessary.
    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> RwLockExt<T> for RwLock<T> {
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|poisoned| {
            tracing::warn!(
                "RwLock poisoned (read), recovering: {}",
                std::any::type_name::<T>()
            );
            poisoned.into_inner()
        })
    }

    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|poisoned| {
            tracing::warn!(
                "RwLock poisoned (write), recovering: {}",
                std::any::type_name::<T>()
            );
            poisoned.into_inner()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Helper: poison a Mutex by panicking inside a thread that holds the lock.
    fn poison_mutex<T: Send + 'static>(mutex: &Arc<Mutex<T>>) {
        let m = Arc::clone(mutex);
        let handle = std::thread::spawn(move || {
            let _guard = m.lock().unwrap();
            panic!("intentional panic to poison mutex");
        });
        let _ = handle.join(); // join the panicked thread
    }

    /// Helper: poison a RwLock by panicking inside a thread that holds a write lock.
    fn poison_rwlock<T: Send + Sync + 'static>(rwlock: &Arc<RwLock<T>>) {
        let rw = Arc::clone(rwlock);
        let handle = std::thread::spawn(move || {
            let _guard = rw.write().unwrap();
            panic!("intentional panic to poison rwlock");
        });
        let _ = handle.join();
    }

    // --- Mutex tests ---

    #[test]
    fn mutex_lock_or_recover_normal() {
        let mutex = Mutex::new(42);
        let guard = mutex.lock_or_recover();
        assert_eq!(*guard, 42);
    }

    #[test]
    fn mutex_lock_or_recover_after_poison() {
        let mutex = Arc::new(Mutex::new(100));
        poison_mutex(&mutex);

        // Standard lock should fail with poison error
        assert!(mutex.lock().is_err());

        // lock_or_recover should succeed and return the inner value
        let guard = mutex.lock_or_recover();
        assert_eq!(*guard, 100);
    }

    #[test]
    fn mutex_lock_or_recover_mutation_after_poison() {
        let mutex = Arc::new(Mutex::new(String::from("hello")));
        poison_mutex(&mutex);

        {
            let mut guard = mutex.lock_or_recover();
            guard.push_str(" world");
        }

        // Subsequent recovery should see the mutation
        let guard = mutex.lock_or_recover();
        assert_eq!(*guard, "hello world");
    }

    // --- RwLock read tests ---

    #[test]
    fn rwlock_read_or_recover_normal() {
        let rwlock = RwLock::new(42);
        let guard = rwlock.read_or_recover();
        assert_eq!(*guard, 42);
    }

    #[test]
    fn rwlock_read_or_recover_after_poison() {
        let rwlock = Arc::new(RwLock::new(200));
        poison_rwlock(&rwlock);

        // Standard read should fail with poison error
        assert!(rwlock.read().is_err());

        // read_or_recover should succeed
        let guard = rwlock.read_or_recover();
        assert_eq!(*guard, 200);
    }

    // --- RwLock write tests ---

    #[test]
    fn rwlock_write_or_recover_normal() {
        let rwlock = RwLock::new(42);
        let mut guard = rwlock.write_or_recover();
        *guard = 99;
        drop(guard);

        let guard = rwlock.read_or_recover();
        assert_eq!(*guard, 99);
    }

    #[test]
    fn rwlock_write_or_recover_after_poison() {
        let rwlock = Arc::new(RwLock::new(300));
        poison_rwlock(&rwlock);

        // Standard write should fail with poison error
        assert!(rwlock.write().is_err());

        // write_or_recover should succeed and allow mutation
        let mut guard = rwlock.write_or_recover();
        assert_eq!(*guard, 300);
        *guard = 400;
        drop(guard);

        // Verify the mutation persists through subsequent recovery
        let guard = rwlock.read_or_recover();
        assert_eq!(*guard, 400);
    }

    #[test]
    fn rwlock_multiple_concurrent_reads_normal() {
        let rwlock = RwLock::new(55);
        let r1 = rwlock.read_or_recover();
        let r2 = rwlock.read_or_recover();
        assert_eq!(*r1, 55);
        assert_eq!(*r2, 55);
    }
}
