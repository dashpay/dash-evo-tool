use crate::context::AppContext;
use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::{env, io};

impl AppContext {
    /// Function to start Dash QT based on the selected network
    pub(super) fn start_dash_qt(
        &self,
        network: Network,
        custom_dash_qt: Option<String>,
        overwrite_dash_conf: bool,
    ) -> io::Result<()> {
        let dash_qt_path = match custom_dash_qt {
            Some(ref custom_path) => PathBuf::from(custom_path),
            None => {
                if cfg!(target_os = "macos") {
                    PathBuf::from("/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt")
                } else if cfg!(target_os = "windows") {
                    // Retrieve the PROGRAMFILES environment variable or default to "C:\\Program Files"
                    let program_files = env::var("PROGRAMFILES")
                        .unwrap_or_else(|_| "C:\\Program Files".to_string());
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
            Network::Devnet => "dash_core_configs/devnet.conf",
            Network::Regtest => "dash_core_configs/local.conf",
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unsupported network",
                ))
            }
        };

        let mut command = Command::new(&dash_qt_path);
        command.stdout(Stdio::null()).stderr(Stdio::null()); // Suppress output

        if overwrite_dash_conf {
            // Construct the full path to the config file
            // Try to get the directory where the executable is located first
            let config_path = if let Ok(exe_path) = env::current_exe() {
                if let Some(exe_dir) = exe_path.parent() {
                    exe_dir.join(config_file)
                } else {
                    // Fallback to current working directory
                    env::current_dir()?.join(config_file)
                }
            } else {
                // Fallback to current working directory
                env::current_dir()?.join(config_file)
            };
            command.arg(format!("-conf={}", config_path.display()));
        } else if network == Network::Testnet {
            command.arg("-testnet");
        } else if network == Network::Devnet {
            command.arg("-devnet");
        } else if network == Network::Regtest {
            command.arg("-local");
        }

        // Spawn the Dash-Qt process
        command.spawn()?;

        Ok(())
    }
}
