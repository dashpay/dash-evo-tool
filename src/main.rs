#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use dash_evo_tool::context::SDK_THREAD_STACK_SIZE;
use dash_evo_tool::*;
use std::process::ExitCode;

use crate::boot::prepare_environment;
use crate::cpu_compatibility::check_cpu_compatibility;
use crate::logging::{
    capture_stderr_to_file, install_alt_signal_stack, install_fatal_signal_handler,
    report_startup_failure_to_terminal,
};

fn main() -> ExitCode {
    // Single owner of dir/env/logger setup; runs again inside `BootApp` boot,
    // where the logger's `Once` guard makes the repeat a no-op. A failure here
    // is shown in the startup failure window below.
    let environment = prepare_environment();
    // Redirect stderr into det.log so native crashes (SIGSEGV, abort, OOM)
    // that bypass the tracing panic hook still leave evidence on disk, then
    // mark which fatal signal fired. Both must run before the eframe/tokio
    // runtime starts.
    capture_stderr_to_file();
    install_fatal_signal_handler();
    tracing::info!(
        version = VERSION,
        data_dir = ?environment.as_ref().ok(),
        "Starting dash-evo-tool"
    );
    check_cpu_compatibility();
    // Initialize the Tokio runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(12)
        .thread_stack_size(SDK_THREAD_STACK_SIZE)
        // Each worker/blocking thread needs its own alternate signal stack.
        .on_thread_start(install_alt_signal_stack)
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(?error, "Could not start the async runtime");
            report_startup_failure_to_terminal(&[&error]);
            return ExitCode::FAILURE;
        }
    };

    // Run the native application. `environment` stays here so its error is
    // still reported if eframe fails before the app is created.
    let outcome = crate::boot::StartupOutcome::default();
    let result = runtime.block_on(start(&environment, outcome.clone()));
    if let Err(e) = &result {
        tracing::error!(error = ?e, "Dash Evo Tool failed to start");
    }
    if result.is_err() || outcome.failed() {
        // Notice to the real terminal, independent of tracing (fd 2 is
        // redirected into det.log, so this writes to the preserved stderr).
        let mut causes: Vec<&dyn std::error::Error> = Vec::new();
        if let Err(error) = &environment {
            causes.push(error);
        }
        if let Err(error) = &result {
            causes.push(error);
        }
        report_startup_failure_to_terminal(&causes);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
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

async fn start(
    environment: &Result<std::path::PathBuf, std::io::Error>,
    outcome: crate::boot::StartupOutcome,
) -> Result<(), eframe::Error> {
    // Load icon for the window
    let icon_data = load_icon();
    // Boot steps that need no window run first: the window saves its state
    // into the profile only when they succeed, never for a failed launch
    // (e.g. a second instance) that would overwrite another's `app.ron`.
    let preparation = environment
        .as_ref()
        .map(|_| crate::boot::BootPreparation::run());

    let native_options = eframe::NativeOptions {
        // Persist window size and position
        persist_window: preparation
            .as_ref()
            .is_ok_and(crate::boot::BootPreparation::may_persist),
        centered: true, // Center window on startup if not maximized
        // Without a data folder this is None, and eframe 0.37 falls back to its
        // default storage folder: it creates that folder (empty) at start-up.
        // Nothing is written there, because the failure dialog persists no
        // egui memory and window geometry is off (`BootApp::persist_egui_memory`).
        persistence_path: environment.as_ref().ok().map(|dir| dir.join("app.ron")),
        viewport: egui::ViewportBuilder::default()
            .with_icon(icon_data)
            .with_app_id("org.dash.DashEvoTool")
            // Only takes effect on a genuine first launch: once `persist_window`
            // has written window geometry to `app.ron`, the persisted state
            // (size/maximized/fullscreen) unconditionally overrides this on
            // every later run. Without it, winit falls back to its own small
            // platform-default size, which doesn't fit the onboarding page.
            .with_maximized(true),
        // Use wgpu instead of glow (OpenGL) to avoid platform-specific rendering
        // issues, e.g. NSOpenGLContext idle/sleep crashes on macOS (#629)
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        &format!("Dash Evo Tool v{}", VERSION),
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(match preparation {
                Ok(preparation) => {
                    crate::boot::BootApp::start(cc.egui_ctx.clone(), preparation, &outcome)
                }
                Err(error) => crate::boot::BootApp::failed(error, &outcome),
            }))
        }),
    )
}
