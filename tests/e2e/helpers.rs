//! E2E Test Helpers
//!
//! This module provides shared utilities for E2E testing, including:
//! - Test harness setup
//! - Per-test data-directory isolation
//!
//! Tests that build a full `AppState` (via `AppState::new`) resolve their data
//! directory through `app_user_data_dir_path()`, which honors the
//! `DASH_EVO_DATA_DIR` env var. Without isolation those tests open the real
//! `~/.config/Dash-Evo-Tool` data dir, crash on a pre-existing
//! `det-app.sqlite` whose migration checksum diverges from the current schema,
//! and pollute the user's wallet data. [`with_isolated_data_dir`] redirects the
//! data dir to a throwaway temp dir for the duration of the closure. This
//! mirrors the identical helper in `tests/kittest/support.rs` (the two test
//! binaries cannot share a module).

use std::sync::{Mutex, MutexGuard, OnceLock};

/// Create a minimal test harness for E2E tests
#[allow(dead_code)]
pub struct TestHarness {
    pub runtime: tokio::runtime::Runtime,
}

impl TestHarness {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        Self { runtime }
    }
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}

const DATA_DIR_ENV: &str = "DASH_EVO_DATA_DIR";

/// Serializes data-dir isolation across tests. `DASH_EVO_DATA_DIR` is
/// process-global, so AppState-constructing tests must not run in parallel.
fn data_dir_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// RAII guard restoring the prior `DASH_EVO_DATA_DIR` value on drop and
/// holding the serialization lock for the lifetime of the override.
struct DataDirGuard {
    prior: Option<String>,
    _tempdir: tempfile::TempDir,
    _lock: MutexGuard<'static, ()>,
}

impl Drop for DataDirGuard {
    fn drop(&mut self) {
        // Safety: the lock held by `_lock` serializes all env mutation; no other
        // test touches DASH_EVO_DATA_DIR while this guard is alive.
        unsafe {
            match &self.prior {
                Some(value) => std::env::set_var(DATA_DIR_ENV, value),
                None => std::env::remove_var(DATA_DIR_ENV),
            }
        }
    }
}

/// Runs `f` with `DASH_EVO_DATA_DIR` pointed at a fresh temp directory, then
/// restores the prior value and removes the temp dir. Acquires a process-global
/// lock so concurrent AppState-constructing tests serialize rather than race on
/// the shared env var.
pub fn with_isolated_data_dir<R>(f: impl FnOnce() -> R) -> R {
    let lock = data_dir_lock();
    let tempdir = tempfile::tempdir().expect("create temp data dir");
    let prior = std::env::var(DATA_DIR_ENV).ok();

    // Safety: serialized by `lock`; restored by `DataDirGuard::drop`.
    unsafe {
        std::env::set_var(DATA_DIR_ENV, tempdir.path());
    }

    let _guard = DataDirGuard {
        prior,
        _tempdir: tempdir,
        _lock: lock,
    };

    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harness_creation() {
        let harness = TestHarness::new();
        // Just verify we can create the harness without panicking
        drop(harness);
    }
}
