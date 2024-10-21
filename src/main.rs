mod app;
mod config;
mod database;
mod logging;
mod sdk_wrapper;
mod ui;

mod components;
mod context;
mod context_provider;
mod model;
mod platform;

fn main() -> eframe::Result<()> {
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
            "Identity Manager",
            native_options,
            Box::new(|_cc| Ok(Box::new(app::AppState::new()))),
        )
    })
}
