use crate::{VERSION, app_dir::app_user_data_file_path};
use std::panic;
use std::sync::Once;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

static INIT_LOGGER: Once = Once::new();

pub fn initialize_logger() {
    INIT_LOGGER.call_once(|| {
        initialize_logger_internal();
    });
}

fn initialize_logger_internal() {
    // Initialize log file, with improved error handling
    let log_file_path = app_user_data_file_path("det.log").expect("should create log file path");
    let log_file = match std::fs::File::create(&log_file_path) {
        Ok(file) => file,
        Err(e) => panic!("Failed to create log file: {:?}", e),
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::try_new(
            "info,dash_evo_tool=trace,dash_sdk=debug,dash_sdk::platform::transition=trace,tenderdash_abci=debug,drive=debug,drive_proof_verifier=debug,rs_dapi_client=debug,h2=warn,dash_spv=debug",
        )
        .unwrap_or_else(|e| panic!("Failed to create EnvFilter: {:?}", e))
    });

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(log_file)
        .with_ansi(false)
        .finish();

    // Set global subscriber - ignore error if already set (can happen in tests)
    if let Err(_e) = tracing::subscriber::set_global_default(subscriber) {
        // Logger already initialized, this is fine
        return;
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
