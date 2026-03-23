use crate::{VERSION, app_dir::app_user_data_file_path};
use chrono::{Duration, Local};
use std::backtrace::Backtrace;
use std::fs;
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
    rotate_log_file();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::try_new(
            "info,dash_evo_tool=trace,dash_sdk=debug,dash_sdk::platform::transition=trace,tenderdash_abci=debug,drive=debug,drive_proof_verifier=debug,rs_dapi_client=debug,h2=warn,dash_spv=debug,key_wallet=debug,mempool_filter=debug",
        )
        .unwrap_or_else(|_| EnvFilter::new("info"))
    });

    // Try to create a log file; fall back to stderr if it fails
    let log_file_result = app_user_data_file_path("det.log").and_then(std::fs::File::create);

    let (subscriber_set, log_file_path_for_msg) = match log_file_result {
        Ok(log_file) => {
            let subscriber = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(log_file)
                .with_ansi(false)
                .finish();
            let set = tracing::subscriber::set_global_default(subscriber).is_ok();
            (set, Some(app_user_data_file_path("det.log").ok()))
        }
        Err(e) => {
            // Fall back to stderr logging
            let subscriber = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .with_ansi(true)
                .finish();
            let set = tracing::subscriber::set_global_default(subscriber).is_ok();
            if set {
                tracing::warn!(
                    error = %e,
                    "Could not create log file, logging to stderr"
                );
            }
            (set, None)
        }
    };

    if !subscriber_set {
        // Logger already initialized, this is fine
        return;
    }

    // Log panic events
    let default_panic_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        let payload = panic_info.payload();
        let message = if let Some(s) = payload.downcast_ref::<&str>() {
            *s
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.as_str()
        } else {
            "unknown panic"
        };

        let location = panic_info
            .location()
            .unwrap_or_else(|| panic::Location::caller());

        let backtrace = Backtrace::force_capture();

        error!(
            location = tracing::field::display(location),
            "Panic occurred: {}\n{}", message, backtrace
        );

        default_panic_hook(panic_info);
    }));

    if let Some(Some(path)) = log_file_path_for_msg {
        info!(
            version = VERSION,
            log_file = ?path,
            "Dash-Evo-Tool logging initialized successfully"
        );
    } else {
        info!(
            version = VERSION,
            "Dash-Evo-Tool logging initialized (stderr fallback)"
        );
    }
}

fn rotate_log_file() {
    let Ok(log_path) = app_user_data_file_path("det.log") else {
        return;
    };
    if log_path.exists() {
        let ts = fs::metadata(&log_path)
            .and_then(|m| m.modified())
            .map(chrono::DateTime::<Local>::from)
            .unwrap_or_else(|_| Local::now())
            .timestamp();
        let rotated = log_path.with_file_name(format!("det.{ts:010}.log"));
        let _ = fs::rename(&log_path, rotated);
    }

    let Some(parent) = log_path.parent() else {
        return;
    };
    let cutoff = (Local::now() - Duration::days(7)).timestamp();
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(ts_str) = name
            .strip_prefix("det.")
            .and_then(|s| s.strip_suffix(".log"))
        else {
            continue;
        };
        if let Ok(ts) = ts_str.parse::<i64>()
            && ts < cutoff
        {
            let _ = fs::remove_file(path);
        }
    }
}
