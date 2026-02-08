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
