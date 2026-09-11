//! Drives a staged fixture through the real `det-cli` binary.
//!
//! The whole point of the matrix is that migration is exercised by a genuine
//! production boot: an in-process `AppState` under the `testing` feature
//! substitutes an in-memory database, which would make every assertion below
//! vacuous. So the harness spawns the compiled binary as a subprocess with
//! `DASH_EVO_DATA_DIR` pointed at the staged directory and reads its stdout.
//!
//! `core-wallets-list` is the boot of record: it runs
//! `resolve::ensure_wallets_hydrated`, which prepares this network's storage,
//! runs the legacy drain, and waits for a terminal migration state — the same
//! path the GUI takes on cold start, minus the display.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::stage::StagedFixture;

/// Overrides the binary under test (a release build, a downloaded artifact).
pub const BINARY_ENV: &str = "DET_CLI_BIN";

/// Path Cargo hands us when `det-cli` is built alongside this test — i.e.
/// whenever the `cli` feature is on, as it is under `--all-features`.
const CARGO_BIN: Option<&str> = option_env!("CARGO_BIN_EXE_det-cli");

/// One subprocess invocation, captured whole. Both streams are kept: the tool
/// result lands on stdout, panics and migration diagnostics on stderr.
pub struct CliRun {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl CliRun {
    pub fn succeeded(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }

    /// The tool's JSON output. `det-cli` prints the MCP text content verbatim,
    /// so the whole of stdout is the document.
    pub fn json(&self) -> Result<Value, String> {
        serde_json::from_str(self.stdout.trim()).map_err(|e| {
            format!(
                "`{}` did not print JSON: {e}\n{}",
                self.command,
                self.report()
            )
        })
    }

    /// Multi-line dump for assertion failures — the command, how it ended, and
    /// both streams.
    pub fn report(&self) -> String {
        let outcome = if self.timed_out {
            "timed out".to_string()
        } else {
            match self.exit_code {
                Some(code) => format!("exit code {code}"),
                None => "killed by a signal".to_string(),
            }
        };
        format!(
            "command: {}\noutcome: {outcome}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.command,
            self.stdout.trim_end(),
            self.stderr.trim_end()
        )
    }
}

/// A `det-cli` bound to one staged data directory.
pub struct DetCli {
    binary: PathBuf,
    data_dir: PathBuf,
    home: PathBuf,
    xdg: PathBuf,
}

impl DetCli {
    pub fn new(staged: &StagedFixture) -> Result<Self, String> {
        let binary = locate_binary()?;
        let home = staged.sandbox().join("home");
        let xdg = staged.sandbox().join("xdg");
        for dir in [&home, &xdg] {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        Ok(Self {
            binary,
            data_dir: staged.data_dir().to_path_buf(),
            home,
            xdg,
        })
    }

    /// Runs one subcommand, killing the child if it outlives `timeout`.
    pub fn run(&self, args: &[&str], timeout: Duration) -> Result<CliRun, String> {
        let command = format!("det-cli --standalone {}", args.join(" "));
        let mut child = Command::new(&self.binary)
            .arg("--standalone")
            .args(args)
            .env("DASH_EVO_DATA_DIR", &self.data_dir)
            // Keep every incidental write inside the sandbox: `det-cli` caches
            // the tool list under the user's cache dir and installs a bash
            // completion script under the user's data dir.
            .env("HOME", &self.home)
            .env("XDG_CACHE_HOME", self.xdg.join("cache"))
            .env("XDG_DATA_HOME", self.xdg.join("data"))
            .env("XDG_CONFIG_HOME", self.xdg.join("config"))
            .env("XDG_STATE_HOME", self.xdg.join("state"))
            // An inherited key would flip the binary into HTTP mode and talk to
            // whatever DET instance happens to be running on this machine.
            .env_remove("MCP_API_KEY")
            .env_remove("MCP_LISTEN")
            .env("RUST_LOG", log_filter())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not spawn {}: {e}", self.binary.display()))?;

        let mut stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        // Drained on threads: a child that fills a pipe buffer would deadlock
        // against a parent that only polls for exit.
        let stdout_reader = std::thread::spawn(move || read_stream(&mut stdout));
        let stderr_reader = std::thread::spawn(move || read_stream(&mut stderr));

        let deadline = Instant::now() + timeout;
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(e) => return Err(format!("could not wait for `{command}`: {e}")),
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = child.kill();
                break child.wait().ok();
            }
            std::thread::sleep(Duration::from_millis(50));
        };

        let stdout = stdout_reader.join().unwrap_or_else(|_| String::new());
        let stderr = stderr_reader.join().unwrap_or_else(|_| String::new());

        Ok(CliRun {
            command,
            exit_code: status.and_then(|s| s.code()),
            stdout,
            stderr,
            timed_out,
        })
    }

    /// Cold boot of record: hydrates storage, runs the legacy drain, waits for
    /// the migration to reach a terminal state, then lists the wallets.
    pub fn wallets_list(&self, timeout: Duration) -> Result<CliRun, String> {
        self.run(&["core-wallets-list"], timeout)
    }

    /// Active network and configured networks. Network-exempt — no SPV gate.
    pub fn network_info(&self, timeout: Duration) -> Result<CliRun, String> {
        self.run(&["network-info"], timeout)
    }

    /// Derives a fresh receive address. Waits on the SPV gate, so it needs a
    /// reachable chain.
    pub fn address_create(&self, wallet_id: &str, timeout: Duration) -> Result<CliRun, String> {
        self.run(
            &["core-address-create", &format!("wallet-id={wallet_id}")],
            timeout,
        )
    }
}

fn read_stream(stream: &mut impl Read) -> String {
    let mut buffer = Vec::new();
    let _ = stream.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

/// `info` keeps the migration's own progress logging in the captured stderr,
/// which is where a failure report gets its evidence from.
fn log_filter() -> String {
    std::env::var("MIGRATION_MATRIX_RUST_LOG").unwrap_or_else(|_| "info".to_string())
}

/// Resolves the binary under test, preferring an explicit override.
pub fn locate_binary() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var(BINARY_ENV)
        && !path.trim().is_empty()
    {
        let path = PathBuf::from(path);
        return match path.is_file() {
            true => Ok(path),
            false => Err(format!(
                "{BINARY_ENV} points at {}, which is not a file",
                path.display()
            )),
        };
    }

    let cargo_bin = CARGO_BIN.map(Path::new).filter(|path| path.is_file());
    match cargo_bin {
        Some(path) => Ok(path.to_path_buf()),
        None => Err(format!(
            "det-cli was not built alongside this test. Build the matrix with the `cli` feature \
             (`cargo test --test migration-matrix --features testing,cli,headless`) or point \
             {BINARY_ENV} at a det-cli binary."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bogus_binary_override_is_reported() {
        // Safety: single-threaded scope; the var is restored before returning
        // and no other test in this binary reads it.
        unsafe { std::env::set_var(BINARY_ENV, "/nonexistent/det-cli") };
        let error = locate_binary().expect_err("a missing override must fail");
        unsafe { std::env::remove_var(BINARY_ENV) };

        assert!(error.contains("/nonexistent/det-cli"), "{error}");
    }

    #[test]
    fn a_failed_run_reports_both_streams() {
        let run = CliRun {
            command: "det-cli --standalone core-wallets-list".to_string(),
            exit_code: Some(1),
            stdout: "partial".to_string(),
            stderr: "Error: boom".to_string(),
            timed_out: false,
        };

        assert!(!run.succeeded());
        let report = run.report();
        assert!(
            report.contains("exit code 1") && report.contains("boom") && report.contains("partial")
        );
    }

    #[test]
    fn a_timeout_is_named_in_the_report() {
        let run = CliRun {
            command: "det-cli --standalone core-address-create".to_string(),
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
        };

        assert!(!run.succeeded());
        assert!(run.report().contains("timed out"), "{}", run.report());
    }
}
