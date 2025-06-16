use crate::app_dir::{app_user_data_file_path, create_dash_core_config_if_not_exists};

use crate::context::AppContext;
use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;
use tokio::process::Command;

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
                format!("Dash-Qt binary file not found at: {:?}", dash_qt_path),
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
        tokio::spawn(async move {
            let mut dash_qt = command
                .spawn()
                .inspect_err(
                    |e| tracing::error!(error=?e, ?command, "failed to start dash-qt binary"),
                )
                .expect("Failed to spawn dash-qt process");

            tracing::debug!(?command, pid = dash_qt.id(), "dash-qt started");
            match dash_qt.wait().await {
                Ok(status) => {
                    if status.success() {
                        tracing::debug!("dash-qt process exited successfully");
                    } else {
                        tracing::warn!("dash-qt process exited with status: {}", status);
                    }
                }
                Err(e) => tracing::error!(error=?e, "dash-qt process failed to wait"),
            }
        });

        Ok(())
    }
}
