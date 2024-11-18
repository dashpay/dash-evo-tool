use crate::context::AppContext;
use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, io};

impl AppContext {
    /// Function to start Dash QT based on the selected network
    pub(super) fn start_dash_qt(&self, network: Network, custom_dash_qt:Option<String>, overwrite_dash_conf: bool) -> io::Result<()> {
        let dash_qt_path = match custom_dash_qt {
            Some(ref custom_path) => PathBuf::from(custom_path),
            None => {
                if cfg!(target_os = "macos") {
                    PathBuf::from("/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt")
                } else if cfg!(target_os = "windows") {
                    // Retrieve the PROGRAMFILES environment variable or default to "C:\\Program Files"
                    let program_files = env::var("PROGRAMFILES").unwrap_or_else(|_| "C:\\Program Files".to_string());
                    PathBuf::from(program_files).join("DashCore\\dash-qt.exe")
                } else {
                    PathBuf::from("/usr/local/bin/dash-qt") // Default Linux path
                }
            }
        };

        // Ensure the Dash-Qt binary path exists
        if !dash_qt_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Dash-Qt not found at: {:?}", dash_qt_path),
            ));
        }

        // Determine the config file based on the network
        let config_file: &str = match network {
            Network::Dash => "dash_core_configs/mainnet.conf",
            Network::Testnet => "dash_core_configs/testnet.conf",
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unsupported network",
                ))
            }
        };

        if (overwrite_dash_conf) {
            // Construct the full path to the config file
            let current_dir = env::current_dir()?;
            let config_path = current_dir.join(config_file);

            // Start Dash-Qt with the appropriate config
            Command::new(&dash_qt_path)
                .arg(format!("-conf={}", config_path.display()))
                .stdout(Stdio::null()) // Optional: Suppress output
                .stderr(Stdio::null())
                .spawn()?; // Spawn the Dash-Qt process
        }
        else {
            // Start Dash-Qt
            Command::new(&dash_qt_path)
                .stdout(Stdio::null()) // Optional: Suppress output
                .stderr(Stdio::null())
                .spawn()?; // Spawn the Dash-Qt process
        }

        Ok(())
    }
}
