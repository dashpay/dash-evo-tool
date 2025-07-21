use crate::{VERSION, app_dir::app_user_data_file_path};
use std::panic;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

pub fn initialize_logger() {
    // Initialize log file, with improved error handling
    let log_file_path = app_user_data_file_path("det.log").expect("should create log file path");
    let log_file = match std::fs::File::create(&log_file_path) {
        Ok(file) => file,
        Err(e) => panic!("Failed to create log file: {:?}", e),
    };

    let filter = EnvFilter::try_new(
        "info,dash_evo_tool=trace,dash_sdk=debug,dash_spv=trace,tenderdash_abci=debug,drive=debug,drive_proof_verifier=debug,rs_dapi_client=debug,h2=warn",
    )
        .unwrap_or_else(|e| panic!("Failed to create EnvFilter: {:?}", e));

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(log_file)
        .with_ansi(false)
        .finish();

    // Set global subscriber with proper error handling
    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        panic!("Unable to set global default subscriber: {:?}", e);
    }

    // Log panic events
    let default_panic_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        let message = panic_info
            .payload()
            .downcast_ref::<&str>()
            .unwrap_or(&"unknown panic");

        let location = panic_info
            .location()
            .unwrap_or_else(|| panic::Location::caller());

        error!(
            location = tracing::field::display(location),
            "Panic occurred: {}", message
        );

        default_panic_hook(panic_info);
    }));

    info!(
        version = VERSION,
        log_file = ?log_file_path,
        "Dash-Evo-Tool logging initialized successfully"
    );
}
