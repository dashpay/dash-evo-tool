pub mod app;
pub mod app_dir;
pub mod backend_task;
pub mod boot;
pub mod bundled;
pub mod config;
pub mod context;
pub mod context_provider;
pub mod cpu_compatibility;
pub mod database;
pub mod logging;
#[cfg(any(feature = "mcp", feature = "cli"))]
pub mod mcp;
pub mod model;
pub mod platform;
pub mod sdk_wrapper;
#[cfg(test)]
pub(crate) mod test_support;
pub mod ui;
pub mod utils;
pub mod wallet_backend;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
