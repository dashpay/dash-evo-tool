use crate::{VERSION, app_dir::app_user_data_file_path};
use chrono::{Duration, Local};
use std::backtrace::Backtrace;
use std::fs;
use std::panic;
use std::path::Path;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

static INIT_LOGGER: Once = Once::new();

/// Whether the tracing subscriber writes to the on-disk log file (`det.log`).
///
/// `false` means the file could not be created and the logger fell back to
/// stderr. In that case stderr must stay attached to the terminal so the
/// fallback logs remain visible, and [`capture_stderr_to_file`] must not
/// redirect it.
static LOGGER_USES_FILE: AtomicBool = AtomicBool::new(false);

/// Number of days a rotated log file is kept before cleanup removes it.
const LOG_RETENTION_DAYS: i64 = 7;

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
            if set {
                LOGGER_USES_FILE.store(true, Ordering::SeqCst);
            }
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

/// Redirects the process's stderr (fd 2) to a persistent sidecar file so
/// abnormal terminations leave evidence on disk.
///
/// The tracing panic hook already records catchable Rust panics to `det.log`,
/// but native crashes — `SIGSEGV`/`SIGABRT` from FFI, `abort()` (including a
/// `panic = abort` double-panic), allocation-failure aborts ("memory allocation
/// of N bytes failed") and OOM — write to stderr or nowhere. In a GUI launch
/// there is no terminal, so that output is lost. Pointing stderr at
/// `det-stderr.log` captures all of it.
///
/// Call once, early in GUI startup, after [`initialize_logger`] and before the
/// eframe/tokio runtime starts. CLI/stdio entry points should not call this:
/// their stderr is used for interactive diagnostics or carries protocol output.
///
/// No-op (with a warning) when the logger fell back to stderr — redirecting
/// then would swallow the fallback logs. On non-Unix targets this is currently
/// a best-effort no-op; see the inline note.
pub fn capture_stderr_to_file() {
    if !LOGGER_USES_FILE.load(Ordering::SeqCst) {
        tracing::warn!(
            "Logging is using the stderr fallback; skipping crash-stderr capture to keep logs visible"
        );
        return;
    }

    let path = match app_user_data_file_path("det-stderr.log") {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!(error = %e, "Could not resolve crash-stderr log path; capture disabled");
            return;
        }
    };

    if let Some(parent) = path.parent() {
        rotate_log_in_dir(parent, "det-stderr");
    }

    redirect_stderr_to(&path);
}

#[cfg(unix)]
fn redirect_stderr_to(path: &Path) {
    use std::os::fd::AsRawFd;

    let file = match fs::File::create(path) {
        Ok(file) => file,
        Err(e) => {
            tracing::warn!(error = %e, "Could not open crash-stderr log file; capture disabled");
            return;
        }
    };

    // SAFETY: `dup2` only manipulates the file-descriptor table. `file`'s fd is
    // valid for the duration of the call, and `STDERR_FILENO` (2) is the
    // standard error descriptor. On success fd 2 is rebound to the sidecar
    // file; on failure (-1) stderr is left unchanged and we report the error.
    let dup_ok = unsafe { nix::libc::dup2(file.as_raw_fd(), nix::libc::STDERR_FILENO) != -1 };

    if dup_ok {
        // Keep the file alive so fd 2 stays backed by it for the whole run.
        std::mem::forget(file);
        info!(stderr_log = ?path, "Crash stderr capture enabled");
    } else {
        let err = std::io::Error::last_os_error();
        tracing::warn!(error = %err, "Could not redirect stderr to crash-stderr log file");
    }
}

#[cfg(not(unix))]
fn redirect_stderr_to(_path: &Path) {
    // TODO: Capture stderr on Windows (e.g. SetStdHandle on a CreateFileW
    // handle, or freopen). Until then native crashes that write to stderr are
    // not captured on this target.
    tracing::warn!(
        "Crash stderr capture is only implemented on Unix; native crash output may be lost on this platform"
    );
}

/// Installs a handler for the synchronous fatal signals (`SIGSEGV`, `SIGABRT`,
/// `SIGBUS`, `SIGILL`, `SIGFPE`) that writes a one-line marker to stderr before
/// the process dies, then lets the default handler take over.
///
/// A bare native fault from FFI (grovedb, secp, prover, zmq…) terminates the
/// process with nothing on stderr, so [`capture_stderr_to_file`] alone cannot
/// see it. This handler closes that gap: it records *which* signal fired so the
/// sidecar log shows a crash happened even when there is no other output.
///
/// The handler is async-signal-safe — it only calls `write(2)` on a fixed
/// byte string, no allocation, no locks, no `tracing`. It is registered with
/// `SA_RESETHAND`, so after it returns the default disposition is restored and
/// the faulting instruction (or `abort()`) re-raises the signal to produce the
/// normal crash / core dump.
///
/// Call once, after [`capture_stderr_to_file`], so the marker lands in the
/// sidecar file rather than a lost terminal. Best-effort no-op on non-Unix.
pub fn install_fatal_signal_handler() {
    install_fatal_signal_handler_impl();
}

#[cfg(unix)]
fn install_fatal_signal_handler_impl() {
    use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

    extern "C" fn handle_fatal(sig: nix::libc::c_int) {
        // Async-signal-safe: a single `write` of a constant string. Picking the
        // marker by signal number avoids any formatting/allocation.
        let marker: &[u8] = match Signal::try_from(sig) {
            Ok(Signal::SIGSEGV) => b"\nFATAL: SIGSEGV (segmentation fault)\n",
            Ok(Signal::SIGABRT) => b"\nFATAL: SIGABRT (abort)\n",
            Ok(Signal::SIGBUS) => b"\nFATAL: SIGBUS (bus error)\n",
            Ok(Signal::SIGILL) => b"\nFATAL: SIGILL (illegal instruction)\n",
            Ok(Signal::SIGFPE) => b"\nFATAL: SIGFPE (arithmetic error)\n",
            _ => b"\nFATAL: unexpected signal\n",
        };
        // SAFETY: `write` is async-signal-safe. `marker` is a 'static byte
        // string, valid for the call. The result is intentionally ignored —
        // there is nothing safe to do on failure inside a signal handler.
        unsafe {
            nix::libc::write(
                nix::libc::STDERR_FILENO,
                marker.as_ptr() as *const nix::libc::c_void,
                marker.len(),
            );
        }
        // SA_RESETHAND restored the default handler; returning re-raises the
        // fault (or lets abort() proceed) for the normal crash / core dump.
    }

    // SA_RESETHAND: one-shot, then default disposition re-raises the crash.
    // SA_NODEFER: don't block the signal during the handler, so a fault while
    // the default handler is being restored still terminates cleanly.
    let action = SigAction::new(
        SigHandler::Handler(handle_fatal),
        SaFlags::SA_RESETHAND | SaFlags::SA_NODEFER,
        SigSet::empty(),
    );

    for signal in [
        Signal::SIGSEGV,
        Signal::SIGABRT,
        Signal::SIGBUS,
        Signal::SIGILL,
        Signal::SIGFPE,
    ] {
        // SAFETY: the handler is async-signal-safe (single `write`), so it is
        // sound to install for these synchronous fatal signals.
        if let Err(e) = unsafe { sigaction(signal, &action) } {
            tracing::warn!(error = %e, ?signal, "Could not install fatal-signal handler");
        }
    }
}

#[cfg(not(unix))]
fn install_fatal_signal_handler_impl() {
    // TODO: Install a fatal-exception filter on Windows
    // (AddVectoredExceptionHandler / SetUnhandledExceptionFilter) to record
    // access violations. Not yet implemented on this target.
    tracing::warn!(
        "Fatal-signal capture is only implemented on Unix; native faults may leave no marker on this platform"
    );
}

fn rotate_log_file() {
    let Ok(log_path) = app_user_data_file_path("det.log") else {
        return;
    };
    let Some(parent) = log_path.parent() else {
        return;
    };
    rotate_log_in_dir(parent, "det");
}

/// Rotates `{stem}.log` in `dir` to a timestamped name and removes rotated
/// copies of the same stem older than [`LOG_RETENTION_DAYS`].
fn rotate_log_in_dir(dir: &Path, stem: &str) {
    let log_path = dir.join(format!("{stem}.log"));
    if log_path.exists() {
        let ts = fs::metadata(&log_path)
            .and_then(|m| m.modified())
            .map(chrono::DateTime::<Local>::from)
            .unwrap_or_else(|_| Local::now())
            .timestamp();
        let rotated = dir.join(rotated_name(stem, ts));
        let _ = fs::rename(&log_path, rotated);
    }

    let cutoff = (Local::now() - Duration::days(LOG_RETENTION_DAYS)).timestamp();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(ts) = parse_rotated_ts(name, stem)
            && ts < cutoff
        {
            let _ = fs::remove_file(path);
        }
    }
}

/// File name for a rotated log: `{stem}.{ts:010}.log`.
fn rotated_name(stem: &str, ts: i64) -> String {
    format!("{stem}.{ts:010}.log")
}

/// Parses the timestamp out of a rotated log file name produced by
/// [`rotated_name`], returning `None` for names that don't match the stem.
fn parse_rotated_ts(name: &str, stem: &str) -> Option<i64> {
    name.strip_prefix(&format!("{stem}."))
        .and_then(|s| s.strip_suffix(".log"))
        .and_then(|s| s.parse::<i64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rotated_name_is_zero_padded() {
        assert_eq!(rotated_name("det", 42), "det.0000000042.log");
        assert_eq!(
            rotated_name("det-stderr", 1_700_000_000),
            "det-stderr.1700000000.log"
        );
    }

    #[test]
    fn parse_rotated_ts_round_trips() {
        let name = rotated_name("det", 1_234_567_890);
        assert_eq!(parse_rotated_ts(&name, "det"), Some(1_234_567_890));
    }

    #[test]
    fn parse_rotated_ts_rejects_wrong_stem() {
        // A det-stderr rotation must not be parsed under the "det" stem,
        // otherwise the two logs would clean up each other's files.
        let name = rotated_name("det-stderr", 1_700_000_000);
        assert_eq!(parse_rotated_ts(&name, "det"), None);
    }

    #[test]
    fn parse_rotated_ts_rejects_non_matching_names() {
        assert_eq!(parse_rotated_ts("det.log", "det"), None);
        assert_eq!(parse_rotated_ts("det.notanumber.log", "det"), None);
        assert_eq!(parse_rotated_ts("unrelated.txt", "det"), None);
    }

    #[test]
    fn rotate_log_in_dir_moves_current_log_aside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = dir.path().join("det-stderr.log");
        fs::write(&current, b"old contents").expect("write");

        rotate_log_in_dir(dir.path(), "det-stderr");

        assert!(!current.exists(), "current log should be renamed away");
        let rotated: Vec<_> = fs::read_dir(dir.path())
            .expect("read_dir")
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| parse_rotated_ts(n, "det-stderr").is_some())
            .collect();
        assert_eq!(rotated.len(), 1, "exactly one rotated file expected");
    }

    #[test]
    fn rotate_log_in_dir_removes_only_old_same_stem_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old_ts = (Local::now() - Duration::days(LOG_RETENTION_DAYS + 1)).timestamp();
        let fresh_ts = Local::now().timestamp();

        let old = dir.path().join(rotated_name("det-stderr", old_ts));
        let fresh = dir.path().join(rotated_name("det-stderr", fresh_ts));
        // A different stem with an old timestamp must survive.
        let other_stem = dir.path().join(rotated_name("det", old_ts));
        fs::write(&old, b"x").expect("write old");
        fs::write(&fresh, b"x").expect("write fresh");
        fs::write(&other_stem, b"x").expect("write other");

        rotate_log_in_dir(dir.path(), "det-stderr");

        assert!(!old.exists(), "old same-stem rotation should be deleted");
        assert!(fresh.exists(), "fresh same-stem rotation should be kept");
        assert!(
            other_stem.exists(),
            "other-stem rotation must not be touched"
        );
    }

    #[test]
    fn rotate_log_in_dir_noop_without_current_log() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No det-stderr.log present; rotation should not create anything.
        rotate_log_in_dir(dir.path(), "det-stderr");
        let count = fs::read_dir(dir.path()).expect("read_dir").count();
        assert_eq!(count, 0);
    }
}
