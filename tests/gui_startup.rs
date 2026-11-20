//! Start-up of the GUI binary before any window opens.

/// A failed environment setup happens before the file logger exists, so the
/// terminal is the only place the user can learn why the app did not start:
/// stderr must name the cause instead of a panic or a redacted crash marker.
#[test]
fn environment_setup_failure_names_its_cause_on_stderr() {
    let parent = tempfile::tempdir().expect("temp dir");
    let data_dir = parent.path().join("data");
    std::fs::write(&data_dir, b"not a directory").expect("create regular file");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_dash-evo-tool"))
        .env("DASH_EVO_DATA_DIR", &data_dir)
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
