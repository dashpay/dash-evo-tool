use crate::{VERSION, app_dir::app_user_data_file_path};
use chrono::{Duration, Local};
use std::backtrace::Backtrace;
use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

static INIT_LOGGER: Once = Once::new();

/// Duplicate of the real terminal stderr (fd 2), saved before it is redirected
/// to the crash sidecar. Lets [`report_startup_failure_to_terminal`] surface a
/// user-facing notice on the terminal even while fd 2 points at the log file.
/// `-1` means no handle was preserved (capture skipped or `dup` failed).
static ORIGINAL_STDERR_FD: AtomicI32 = AtomicI32::new(-1);

/// Whether the tracing subscriber writes to the on-disk log file (`det.log`).
///
/// `false` means the file could not be created and the logger fell back to
/// stderr. In that case stderr must stay attached to the terminal so the
/// fallback logs remain visible, and [`capture_stderr_to_file`] must not
/// redirect it.
static LOGGER_USES_FILE: AtomicBool = AtomicBool::new(false);

/// Number of days a rotated log file is kept before cleanup removes it.
const LOG_RETENTION_DAYS: i64 = 7;

/// Whether stderr may be redirected to the crash sidecar file.
///
/// Returns `false` when the logger fell back to stderr, so redirecting would
/// hide the only visible logs. This guards the no-op-on-fallback contract of
/// [`capture_stderr_to_file`].
fn should_capture_stderr() -> bool {
    LOGGER_USES_FILE.load(Ordering::SeqCst)
}

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
    if !should_capture_stderr() {
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

    // fd 2 is about to point at the sidecar; keep a handle to the real terminal
    // so a startup-failure notice can still reach the user (see ORIGINAL_STDERR_FD).
    // SAFETY: `dup` only reads the fd table; STDERR_FILENO (2) is valid here.
    let saved_fd = unsafe { nix::libc::dup(nix::libc::STDERR_FILENO) };
    if saved_fd >= 0 {
        ORIGINAL_STDERR_FD.store(saved_fd, Ordering::SeqCst);
    }

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

/// Size of the alternate signal stack used by [`install_alt_signal_stack`].
///
/// The fatal-signal handler itself does a single `write(2)` call with
/// nothing beneath it, but the kernel/libc signal-delivery machinery needs
/// some stack space of its own to build the signal frame before the handler
/// runs. 64 KiB is a generous multiple of `libc::SIGSTKSZ` (8 KiB on this
/// target) for what is a one-time, process-lifetime allocation per thread.
#[cfg(unix)]
const ALT_SIGNAL_STACK_SIZE: usize = 64 * 1024;

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
/// sidecar file rather than a lost terminal. This also installs an alternate
/// signal stack for the calling thread. Other threads need to call
/// [`install_alt_signal_stack`] separately; `src/main.rs` does so for every
/// Tokio worker/blocking thread through `Builder::on_thread_start`.
/// Best-effort no-op on non-Unix.
pub fn install_fatal_signal_handler() {
    install_fatal_signal_handler_impl();
}

/// Installs an alternate signal stack for the *calling* thread, so a
/// `SA_ONSTACK` handler installed by [`install_fatal_signal_handler`] can
/// still run when this thread's normal stack is exhausted (a guard-page
/// stack-overflow SIGSEGV).
///
/// `sigaltstack(2)` is per-thread: registering it on one thread does
/// nothing for a fault on any other thread. [`install_fatal_signal_handler`]
/// calls this once for the thread that calls it (normally `main`), but any
/// other thread that can run deep, stack-hungry code must call this too —
/// in this project, every Tokio worker/blocking thread, via
/// `Builder::on_thread_start` in `src/main.rs`.
///
/// The backing buffer is intentionally leaked: the alternate stack must
/// stay valid for the entire lifetime of the thread that registered it,
/// since a fatal signal can arrive at any time. Best-effort no-op (with a
/// warning on failure) on Unix; a true no-op on non-Unix targets, since
/// `sigaltstack` and the fatal-signal handler itself are Unix-only.
pub fn install_alt_signal_stack() {
    install_alt_signal_stack_impl();
}

#[cfg(unix)]
fn install_alt_signal_stack_impl() {
    // Leaked intentionally -- see doc comment on `install_alt_signal_stack`.
    let stack = vec![0u8; ALT_SIGNAL_STACK_SIZE].into_boxed_slice();
    let stack_ptr = Box::leak(stack).as_mut_ptr();

    let alt_stack = nix::libc::stack_t {
        ss_sp: stack_ptr as *mut nix::libc::c_void,
        ss_flags: 0,
        ss_size: ALT_SIGNAL_STACK_SIZE,
    };

    // SAFETY: `stack_ptr` points at a leaked buffer of exactly
    // `ALT_SIGNAL_STACK_SIZE` bytes. `sigaltstack` copies `alt_stack`, and the
    // retained `ss_sp` remains valid for the rest of the process.
    let ret = unsafe { nix::libc::sigaltstack(&alt_stack, std::ptr::null_mut()) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        tracing::warn!(
            error = %err,
            "Could not install alternate signal stack; a stack-overflow SIGSEGV on this thread may leave no crash marker"
        );
    }
}

#[cfg(not(unix))]
fn install_alt_signal_stack_impl() {
    // No-op: sigaltstack is a POSIX/Unix mechanism, and the fatal-signal
    // handler itself is already Unix-only.
}

/// Maps a fatal signal number to a fixed `'static` marker line written to
/// stderr from the signal handler. Pure match over constants — no allocation,
/// no formatting — so it stays async-signal-safe when called from the handler.
#[cfg(unix)]
fn marker_for_signal(sig: nix::libc::c_int) -> &'static [u8] {
    use nix::sys::signal::Signal;
    match Signal::try_from(sig) {
        Ok(Signal::SIGSEGV) => b"\nFATAL: SIGSEGV (segmentation fault)\n",
        Ok(Signal::SIGABRT) => b"\nFATAL: SIGABRT (abort)\n",
        Ok(Signal::SIGBUS) => b"\nFATAL: SIGBUS (bus error)\n",
        Ok(Signal::SIGILL) => b"\nFATAL: SIGILL (illegal instruction)\n",
        Ok(Signal::SIGFPE) => b"\nFATAL: SIGFPE (arithmetic error)\n",
        _ => b"\nFATAL: unexpected signal\n",
    }
}

#[cfg(unix)]
fn install_fatal_signal_handler_impl() {
    use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

    // Cover the calling (normally main) thread before installing the handler.
    install_alt_signal_stack_impl();

    extern "C" fn handle_fatal(sig: nix::libc::c_int) {
        let marker = marker_for_signal(sig);
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

    // SA_ONSTACK: use the registered alternate stack so a guard-page overflow
    // can run the handler; unregistered threads fall back to their normal stack.
    // SA_RESETHAND: one-shot, then default disposition re-raises the crash.
    // SA_NODEFER: don't block the signal during the handler, so a fault while
    // the default handler is being restored still terminates cleanly.
    let action = SigAction::new(
        SigHandler::Handler(handle_fatal),
        SaFlags::SA_RESETHAND | SaFlags::SA_NODEFER | SaFlags::SA_ONSTACK,
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

/// Builds the calm, generic notice shown on the terminal when startup fails.
///
/// Pure function: names what happened and lists each provided log path on its
/// own line, with no raw error text, no jargon, and no support redirect. Side
/// effect free so it can be unit-tested directly.
fn startup_failure_message(log_paths: &[PathBuf]) -> String {
    let mut message = String::from("Dash Evo Tool failed to start.\n");
    if log_paths.is_empty() {
        message.push_str("Please try again, and report the problem if it keeps happening.\n");
        return message;
    }
    message.push_str(
        "Details were written to the log files below. Please check them, or include them when reporting the problem:\n",
    );
    for path in log_paths {
        message.push_str("  ");
        message.push_str(&path.display().to_string());
        message.push('\n');
    }
    message
}

/// Writes a generic startup-failure notice to the real terminal.
///
/// Best-effort: resolves the log paths, builds the message, and writes it to the
/// preserved terminal stderr (fd 2 is redirected to the sidecar during a normal
/// run). Never panics; write errors are ignored.
pub fn report_startup_failure_to_terminal() {
    let log_paths: Vec<PathBuf> = ["det.log", "det-stderr.log"]
        .into_iter()
        .filter_map(|name| app_user_data_file_path(name).ok())
        .collect();
    let message = startup_failure_message(&log_paths);
    write_to_terminal(message.as_bytes());
}

/// Writes `bytes` to the real terminal, best-effort.
#[cfg(unix)]
fn write_to_terminal(bytes: &[u8]) {
    // Use the preserved terminal handle if we captured one; otherwise fd 2 is
    // still the terminal (capture skipped or failed), so write straight to it.
    let saved = ORIGINAL_STDERR_FD.load(Ordering::SeqCst);
    let fd = if saved >= 0 {
        saved
    } else {
        nix::libc::STDERR_FILENO
    };
    // SAFETY: `write` only touches the given fd and reads `bytes` for its length.
    // A single best-effort write is enough for a short message; failures are
    // ignored because there is nothing useful to do at terminating startup.
    unsafe {
        nix::libc::write(fd, bytes.as_ptr() as *const nix::libc::c_void, bytes.len());
    }
}

/// Writes `bytes` to the terminal, best-effort.
#[cfg(not(unix))]
fn write_to_terminal(bytes: &[u8]) {
    // stderr is not redirected on non-Unix targets, so a plain write reaches the
    // terminal. Lossy UTF-8 is fine here — the message is ASCII.
    eprint!("{}", String::from_utf8_lossy(bytes));
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

    #[cfg(unix)]
    #[test]
    fn install_alt_signal_stack_registers_stack_for_calling_thread() {
        install_alt_signal_stack();

        let mut current_stack = std::mem::MaybeUninit::<nix::libc::stack_t>::uninit();
        // SAFETY: a null first argument queries the calling thread's current
        // alternate stack into the valid `current_stack` out-pointer.
        let ret = unsafe { nix::libc::sigaltstack(std::ptr::null(), current_stack.as_mut_ptr()) };

        assert_eq!(ret, 0, "sigaltstack query should succeed");
        // SAFETY: a successful query initialized every field of `current_stack`.
        let current_stack = unsafe { current_stack.assume_init() };
        assert_eq!(current_stack.ss_flags & nix::libc::SS_DISABLE, 0);
        assert_eq!(current_stack.ss_size, ALT_SIGNAL_STACK_SIZE);
    }

    #[cfg(unix)]
    #[test]
    fn marker_for_signal_names_each_fatal_signal() {
        use nix::sys::signal::Signal;

        for (signal, needle) in [
            (Signal::SIGSEGV, "SIGSEGV"),
            (Signal::SIGABRT, "SIGABRT"),
            (Signal::SIGBUS, "SIGBUS"),
            (Signal::SIGILL, "SIGILL"),
            (Signal::SIGFPE, "SIGFPE"),
        ] {
            let marker = marker_for_signal(signal as nix::libc::c_int);
            let text = std::str::from_utf8(marker).expect("marker is utf8");
            assert!(
                text.contains(needle),
                "marker for {signal:?} should contain {needle:?}, got {text:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn marker_for_signal_falls_back_for_unknown() {
        // SIGUSR1 is not one of the handled fatal signals, so it must hit the
        // generic fallback rather than naming a specific fatal signal.
        let marker = marker_for_signal(nix::libc::SIGUSR1);
        let text = std::str::from_utf8(marker).expect("marker is utf8");
        assert!(text.contains("unexpected signal"), "got {text:?}");
    }

    #[test]
    fn startup_failure_message_states_failure_and_lists_each_path() {
        let paths = vec![
            PathBuf::from("/tmp/data/det.log"),
            PathBuf::from("/tmp/data/det-stderr.log"),
        ];
        let message = startup_failure_message(&paths);

        assert!(
            message.contains("failed to start"),
            "message should state the app failed to start, got {message:?}"
        );
        for path in &paths {
            assert!(
                message.contains(&path.display().to_string()),
                "message should list {path:?}, got {message:?}"
            );
        }
        // No raw error text and no "contact support" redirect (project rule).
        assert!(
            !message.to_lowercase().contains("contact support"),
            "message must not redirect to support, got {message:?}"
        );
    }

    #[test]
    fn startup_failure_message_omits_path_list_when_empty() {
        let message = startup_failure_message(&[]);
        assert!(
            message.contains("failed to start"),
            "message should still state the failure, got {message:?}"
        );
    }

    #[test]
    fn should_capture_stderr_tracks_logger_uses_file() {
        // LOGGER_USES_FILE is a process-global static shared with other tests
        // in this binary; save and restore it so this test is order-independent.
        let saved = LOGGER_USES_FILE.load(Ordering::SeqCst);

        LOGGER_USES_FILE.store(false, Ordering::SeqCst);
        assert!(
            !should_capture_stderr(),
            "must not capture when logging fell back to stderr"
        );

        LOGGER_USES_FILE.store(true, Ordering::SeqCst);
        assert!(
            should_capture_stderr(),
            "must capture when logging owns the det.log file"
        );

        LOGGER_USES_FILE.store(saved, Ordering::SeqCst);
    }
}
