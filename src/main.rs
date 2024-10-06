use tokio::runtime::Runtime;

mod app;
mod database;
mod logging;
mod sdk_wrapper;
mod ui;
mod config;

mod context;

fn main() -> eframe::Result<()> {
    // Initialize the Tokio runtime
    let runtime = Runtime::new().unwrap();

    runtime.block_on(async {
        let native_options = eframe::NativeOptions::default();
        eframe::run_native(
            "Identity Manager",
            native_options,
            Box::new(|cc| Ok(Box::new(app::AppState::new()))),
        )
    })
}