#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use dash_evo_tool::*;

use crate::app_dir::{app_user_data_dir_path, create_app_user_data_directory_if_not_exists};
use crate::cpu_compatibility::check_cpu_compatibility;
use crate::logging::initialize_logger;

fn main() -> eframe::Result<()> {
    create_app_user_data_directory_if_not_exists()
        .expect("Failed to create app user_data directory");
    let app_data_dir =
        app_user_data_dir_path().expect("Failed to get app user_data directory path");
    initialize_logger();
    tracing::info!(
        version = VERSION,
        data_dir = %app_data_dir.display(),
        "Starting dash-evo-tool"
    );
    check_cpu_compatibility();
    // Initialize the Tokio runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(12)
        .enable_all()
        .build()
        .expect("multi-threading runtime cannot be initialized");

    // Run the native application
    runtime.block_on(start(&app_data_dir))
}

fn load_icon() -> egui::IconData {
    let icon_bytes = include_bytes!("../assets/DET_LOGO.png");
    let image = image::load_from_memory(icon_bytes)
        .expect("Failed to load icon")
        .to_rgba8();
    // Windows can ignore overly large icons; keep a reasonable size.
    let image = image::imageops::resize(&image, 64, 64, image::imageops::FilterType::Lanczos3);
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.to_vec(),
        width,
        height,
    }
}

async fn start(app_data_dir: &std::path::Path) -> Result<(), eframe::Error> {
    // Load icon for the window
    let icon_data = load_icon();

    let native_options = eframe::NativeOptions {
        persist_window: true, // Persist window size and position
        centered: true,       // Center window on startup if not maximized
        persistence_path: Some(app_data_dir.join("app.ron")),
        viewport: egui::ViewportBuilder::default().with_icon(icon_data),
        ..Default::default()
    };

    eframe::run_native(
        &format!("Dash Evo Tool v{}", VERSION),
        native_options,
        Box::new(|cc| Ok(Box::new(crate::app::AppState::new(cc.egui_ctx.clone())))),
    )
}
