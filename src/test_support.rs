//! Shared support for unit tests across the library crate.

use std::sync::Mutex;

/// Serializes every unit test that mutates the process-global data directory.
pub(crate) static DASH_EVO_DATA_DIR_LOCK: Mutex<()> = Mutex::new(());
