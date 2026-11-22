//! Start-up of the GUI binary before any window opens.

/// A failed environment setup happens before the file logger exists, so the
/// terminal is the only place the user can learn why the app did not start:
/// stderr must name the cause instead of a panic or a redacted crash marker.
/// The child gets no display, so no failure window can wait for a user; the
/// kittests cover the window itself.
#[cfg(target_os = "linux")]
#[test]
fn environment_setup_failure_names_its_cause_on_stderr() {
    let parent = tempfile::tempdir().expect("temp dir");
    let data_dir = parent.path().join("data");
    std::fs::write(&data_dir, b"not a directory").expect("create regular file");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_dash-evo-tool"))
        .env("DASH_EVO_DATA_DIR", &data_dir)
        .env("XDG_DATA_HOME", parent.path())
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .output()
        .expect("run the GUI binary");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "start-up must fail:\n{stderr}");
    assert!(
        stderr.contains("Data directory path is not a directory"),
        "{stderr}"
    );
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(!stderr.contains("<redacted"), "{stderr}");
}

/// When no window can open, a launch without a terminal (stderr to
/// /dev/null) and with logging filtered off must still leave the cause in
/// the log file.
#[cfg(target_os = "linux")]
#[test]
fn window_failure_is_written_to_the_log_file() {
    let data_dir = tempfile::tempdir().expect("temp dir");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_dash-evo-tool"))
        .env("DASH_EVO_DATA_DIR", data_dir.path())
        .env("XDG_DATA_HOME", data_dir.path())
        .env("RUST_LOG", "off")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .stderr(std::process::Stdio::null())
        .output()
        .expect("run the GUI binary");
    let log = std::fs::read_to_string(data_dir.path().join("det.log")).expect("read det.log");

    assert!(!output.status.success(), "start-up must fail:\n{log}");
    assert!(log.contains("failed to start"), "{log}");
    assert!(log.contains("Cause:"), "{log}");
}
