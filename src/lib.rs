pub mod app;
pub mod app_dir;
pub mod backend_task;
pub mod bundled;
pub mod components;
pub mod config;
pub mod context;
pub mod context_provider;
pub mod cpu_compatibility;
pub mod database;
pub mod logging;
pub mod model;
pub mod sdk_wrapper;
pub mod ui;
pub mod utils;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
