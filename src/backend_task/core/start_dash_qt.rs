use crate::app_dir::{app_user_data_file_path, create_dash_core_config_if_not_exists};
use crate::context::AppContext;
use crate::utils::path::format_path_for_display;
use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;
use tokio::process::{Child, Command};

impl AppContext {
    /// Function to start Dash QT based on the selected network
    pub(super) fn start_dash_qt(
        &self,
        network: Network,
        dash_qt_path: PathBuf,
        overwrite_dash_conf: bool,
    ) -> std::io::Result<()> {
        // Ensure the Dash-Qt binary path exists
        if !dash_qt_path.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "Dash-Qt binary file not found at: {}",
                    format_path_for_display(&dash_qt_path)
                ),
            ));
        }

        let mut command = Command::new(&dash_qt_path);

        // we need two separate file handles that will write to the same log file
        let outlog = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(app_user_data_file_path("core.log")?)?;

        let errlog = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(app_user_data_file_path("core-err.log")?)?;

        command.stdout(outlog).stderr(errlog); // Suppress output

        if overwrite_dash_conf {
            let config_path = create_dash_core_config_if_not_exists(network)?;
            command.arg(format!("-conf={}", config_path.display()));
        } else if network == Network::Testnet {
            command.arg("-testnet");
        } else if network == Network::Devnet {
            command.arg("-devnet");
        } else if network == Network::Regtest {
            command.arg("-local");
        }
        // Spawn the Dash-Qt process

        // Spawn a task to wait for the Dash-Qt process to exit
        let cancel = self.subtasks.cancellation_token.clone();
        self.subtasks.spawn_sync(async move {
            let mut dash_qt = command
                .spawn()
                .inspect_err(
                    |e| tracing::error!(error=?e, ?command, "failed to start dash-qt binary"),
                )
                .expect("Failed to spawn dash-qt process");

            tracing::debug!(?command, pid = dash_qt.id(), "dash-qt started");

            // Wait for the process to exit or current task to be cancelled
            tokio::select! {
                exited = dash_qt.wait() => {
                    match exited {
                        Err(e) => {
                            tracing::error!(error=?e, "dash-qt process failed");
                        },
                        Ok(status) => {
                            tracing::debug!(%status, "dash-qt process exited");
                        }
                    };
                },
                _ = cancel.cancelled() => {
                    tracing::debug!("dash-qt process was cancelled, sending SIGTERM");
                    signal_term(&dash_qt)
                        .unwrap_or_else(|e| tracing::error!(error=?e, "Failed to send SIGTERM to dash-qt"));
                    let status = dash_qt.wait().await
                        .inspect_err(|e| tracing::error!(error=?e, "Failed to wait for dash-qt process to exit"));
                    tracing::debug!(?status, "dash-qt process stopped gracefully");

                }
            }
        });
        Ok(())
    }
}

/// Send a SIGTERM signal to the Dash-Qt process to gracefully terminate it.
/// Only on UNIX-like systems.
#[cfg(unix)]
fn signal_term(child: &Child) -> Result<(), String> {
    let Some(raw_pid) = child.id() else {
        // No-op, most likely the child process has already exited
        tracing::trace!("Child process ID is not available, cannot send SIGTERM.");
        return Ok(());
    };

    let pid = nix::unistd::Pid::from_raw(raw_pid as i32);
    match nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM) {
        Ok(_) => {
            tracing::debug!(
                "SIGTERM signal sent to Dash-Qt process with PID: {}",
                raw_pid
            );
            Ok(())
        }
        Err(e) => Err(format!(
            "Failed to send SIGTERM signal to dash-qt({}): {}",
            raw_pid, e
        )),
    }
}

#[cfg(windows)]
fn signal_term(child: &Child) -> Result<(), String> {
    // TODO: Implement graceful termination for Dash-Qt on Windows.
    tracing::warn!("SIGTERM signal is not supported on Windows. Dash-Qt process will not be gracefully terminated.");
}
