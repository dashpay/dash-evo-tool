use dash_evo_tool::*;

use crate::app_dir::{app_user_data_dir_path, create_app_user_data_directory_if_not_exists};
use crate::cpu_compatibility::check_cpu_compatibility;

mod app;
mod app_dir;
mod backend_task;
mod bundled;
mod components;
mod config;
mod context;
mod context_provider;
mod cpu_compatibility;
mod database;
mod logging;
mod model;
mod sdk_wrapper;
mod ui;
mod utils;

fn main() -> eframe::Result<()> {
    create_app_user_data_directory_if_not_exists()
        .expect("Failed to create app user_data directory");
    let app_data_dir =
        app_user_data_dir_path().expect("Failed to get app user_data directory path");
    println!(
        "Starting dash-evo-tool, version: {}, data dir: {}",
        VERSION,
        app_data_dir.display()
    );
    check_cpu_compatibility();
    // Initialize the Tokio runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(40)
        .enable_all()
        .build()
        .expect("multi-threading runtime cannot be initialized");

    // Run the native application
    runtime.block_on(start(&app_data_dir))
}

async fn start(app_data_dir: &std::path::Path) -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        persist_window: true, // Persist window size and position
        centered: true,       // Center window on startup if not maximized
        persistence_path: Some(app_data_dir.join("app.ron")),
        ..Default::default()
    };

    eframe::run_native(
        &format!("Dash Evo Tool v{}", VERSION),
        native_options,
        Box::new(|cc| Ok(Box::new(crate::app::AppState::new(cc.egui_ctx.clone())))),
    )
}
