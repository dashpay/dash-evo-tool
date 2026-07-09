#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use dash_evo_tool::*;

use crate::boot::prepare_environment;
use crate::cpu_compatibility::check_cpu_compatibility;
use crate::logging::{
    capture_stderr_to_file, install_fatal_signal_handler, report_startup_failure_to_terminal,
};

fn main() -> eframe::Result<()> {
    // Single owner of dir/env/logger setup; runs again inside `BootApp` boot,
    // where the logger's `Once` guard makes the repeat a no-op.
    let app_data_dir = prepare_environment().expect("Failed to prepare app environment");
    // Redirect stderr to a sidecar file so native crashes (SIGSEGV, abort, OOM)
    // that bypass the tracing panic hook still leave evidence on disk, then
    // mark which fatal signal fired. Both must run before the eframe/tokio
    // runtime starts.
    capture_stderr_to_file();
    install_fatal_signal_handler();
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
    let result = runtime.block_on(start(&app_data_dir));
    if let Err(e) = &result {
        // Full technical detail to det.log; the returned Err's Debug repr still
        // reaches the redirected det-stderr.log via default termination.
        tracing::error!(error = ?e, "Dash Evo Tool failed to start");
        // Generic, actionable notice to the real terminal (fd 2 is redirected
        // to the sidecar log, so this writes to the preserved original stderr).
        report_startup_failure_to_terminal();
    }
    result
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
        viewport: egui::ViewportBuilder::default()
            .with_icon(icon_data)
            .with_app_id("org.dash.DashEvoTool"),
        // Use wgpu instead of glow (OpenGL) to avoid platform-specific rendering
        // issues, e.g. NSOpenGLContext idle/sleep crashes on macOS (#629)
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        &format!("Dash Evo Tool v{}", VERSION),
        native_options,
        Box::new(|cc| Ok(Box::new(crate::boot::BootApp::new(cc.egui_ctx.clone())?))),
    )
}
