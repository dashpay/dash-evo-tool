//! Tauri IPC command modules.
//!
//! Each module corresponds to a domain in the backend task system.
//! Commands are thin wrappers around the existing `BackendTask` dispatch
//! mechanism or direct `AppContext`/`Database` reads.

pub mod contested;
pub mod contract;
pub mod core;
pub mod dashpay;
pub mod document;
pub mod identity;
pub mod platform_info;
pub mod settings;
pub mod system;
pub mod token;
pub mod wallet;
