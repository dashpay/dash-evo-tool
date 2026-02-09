//! Data Transfer Objects (DTOs) for the Tauri IPC boundary.
//!
//! These types replace non-serializable Rust types (Arc, RwLock, Identity,
//! DataContract, etc.) with owned, JSON-serializable equivalents suitable
//! for passing between the Rust backend and the TypeScript frontend via
//! Tauri's IPC mechanism.
//!
//! All DTOs derive `Serialize`, `Deserialize`, `specta::Type`, and `Clone`.

pub mod common;
pub mod contract;
pub mod document;
pub mod fee;
pub mod identity;
pub mod token;
pub mod wallet;

pub use common::*;
pub use contract::*;
pub use document::*;
pub use fee::*;
pub use identity::*;
pub use token::*;
pub use wallet::*;

#[cfg(test)]
mod tests;
