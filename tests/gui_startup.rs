//! Start-up of the GUI binary before any window opens.

/// A failed environment setup panics before the file logger exists. The
/// redacting panic hook must already be in place, so stderr carries the crash
/// marker without the formatted panic message.
#[test]
fn environment_setup_panic_reaches_stderr_redacted() {
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
        stderr.contains("<redacted: formatted panic message"),
        "{stderr}"
    );
    assert!(!stderr.contains("InvalidInput"), "{stderr}");
}
