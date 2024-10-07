use tokio::runtime::Runtime;

mod app;
mod config;
mod database;
mod logging;
mod sdk_wrapper;
mod ui;

mod context;
mod model;
mod platform;

fn main() -> eframe::Result<()> {
    // Initialize the Tokio runtime
    let runtime = Runtime::new().unwrap();

    runtime.block_on(async {
        let native_options = eframe::NativeOptions {
            persist_window: true, // Persist window size and position
            centered: true,       // Center window on startup if not maximized
            ..Default::default()
        };
        eframe::run_native(
            "Identity Manager",
            native_options,
            Box::new(|cc| Ok(Box::new(app::AppState::new()))),
        )
    })
}
