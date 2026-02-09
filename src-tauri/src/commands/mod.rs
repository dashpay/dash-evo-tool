//! Tauri IPC command modules.
//!
//! Each module corresponds to a domain in the backend task system.
//! Commands are thin wrappers around the existing `BackendTask` dispatch
//! mechanism or direct `AppContext`/`Database` reads.

pub mod identity;
