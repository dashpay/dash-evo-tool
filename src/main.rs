use crate::cpu_compatibility::check_cpu_compatibility;

mod app;
mod app_dir;
mod backend_task;
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

fn main() -> eframe::Result<()> {
    check_cpu_compatibility();
    // Initialize the Tokio runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(40)
        .enable_all()
        .build()
        .expect("multi-threading runtime cannot be initialized");

    // Run the native application
    runtime.block_on(async {
        let native_options = eframe::NativeOptions {
            persist_window: true, // Persist window size and position
            centered: true,       // Center window on startup if not maximized
            ..Default::default()
        };
        eframe::run_native(
            "Dash Evo Tool",
            native_options,
            Box::new(|_cc| Ok(Box::new(app::AppState::new()))),
        )
    })
}
