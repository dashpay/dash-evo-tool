use std::panic;
use tracing::error;
use tracing_subscriber::EnvFilter;

pub fn initialize_logger() {
    // Initialize logger
    let log_file = std::fs::File::create("explorer.log").expect("Failed to create log file");

    let filter = EnvFilter::try_new("info")
        .unwrap()
        .add_directive("rs_dapi_client=off".parse().unwrap());

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(log_file)
        .with_ansi(false)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("Unable to set global default subscriber");

    // Log panics
    let default_panic_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        let message = panic_info
            .payload()
            .downcast_ref::<&str>()
            .unwrap_or(&"unknown");

        let location = panic_info
            .location()
            .unwrap_or_else(|| panic::Location::caller());

        error!(
            location = tracing::field::display(location),
            "Panic occurred: {}", message
        );

        default_panic_hook(panic_info);
    }));

    tracing::info!("Logger initialized successfully");
}
