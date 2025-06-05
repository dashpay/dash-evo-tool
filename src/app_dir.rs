use dash_sdk::dpp::dashcore::Network;
use directories::ProjectDirs;
#[cfg(target_os = "linux")]
use directories::UserDirs;
use std::path::PathBuf;
use std::{fs, io};

use crate::bundled::BundledResource;

const QUALIFIER: &str = ""; // Typically empty on macOS and Linux
const ORGANIZATION: &str = "";
const APPLICATION: &str = "Dash-Evo-Tool";

#[allow(dead_code)]
const CORE_APPLICATION: &str = "DashCore";

fn user_data_dir_path(app: &str) -> Result<PathBuf, std::io::Error> {
    let proj_dirs = ProjectDirs::from(QUALIFIER, ORGANIZATION, app).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Failed to determine project directories",
        )
    })?;
    Ok(proj_dirs.config_dir().to_path_buf())
}

pub fn app_user_data_dir_path() -> Result<PathBuf, std::io::Error> {
    user_data_dir_path(APPLICATION)
}

pub fn core_user_data_dir_path() -> Result<PathBuf, std::io::Error> {
    #[cfg(target_os = "linux")]
    {
        UserDirs::new()
            .and_then(|dirs| dirs.home_dir().to_owned().into())
            .map(|home_dir| home_dir.join(".dashcore"))
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Failed to determine user home directory",
                )
            })
    }

    #[cfg(not(target_os = "linux"))]
    {
        user_data_dir_path(CORE_APPLICATION)
    }
}

pub fn core_cookie_path(
    network: Network,
    devnet_name: &Option<String>,
) -> Result<PathBuf, std::io::Error> {
    core_user_data_dir_path().map(|path| {
        let network_dir = match network {
            Network::Dash => "",
            Network::Testnet => "testnet3",
            Network::Devnet => devnet_name.as_deref().unwrap_or(""),
            Network::Regtest => "regtest",
            _ => unimplemented!(),
        };
        path.join(network_dir).join(".cookie")
    })
}

pub fn create_app_user_data_directory_if_not_exists() -> Result<(), std::io::Error> {
    let app_data_dir = app_user_data_dir_path()?;
    fs::create_dir_all(&app_data_dir)?;

    // Verify directory permissions
    let metadata = fs::metadata(&app_data_dir)?;
    if !metadata.is_dir() {
        return Err(std::io::Error::other("Created path is not a directory"));
    }
    Ok(())
}

pub fn app_user_data_file_path(filename: &str) -> Result<PathBuf, std::io::Error> {
    if filename.is_empty() || filename.contains(std::path::is_separator) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid filename",
        ));
    }
    let app_data_dir = app_user_data_dir_path()?;
    Ok(app_data_dir.join(filename))
}

/// If .env file does not exist in the application data directory,
/// copy the bundled `.env.example` file to that directory.
pub fn copy_env_file_if_not_exists() {
    let app_data_dir =
        app_user_data_dir_path().expect("Failed to determine application data directory");
    let env_file_in_app_dir = app_data_dir.join(".env");
    // try to write bundled .env.example file to `env_file_in_app_dir`; it will return false if the file already exists
    // what we can safely ignore
    BundledResource::DotEnvExample
        .write_to_file(&env_file_in_app_dir, false)
        .expect("Failed to write bundled .env.example file");
}

/// For a given network, create dash core config file in the application data directory if it does not exist.
///
/// Returns the path to the config file or an error if it fails.
pub fn create_dash_core_config_if_not_exists(network: Network) -> Result<PathBuf, io::Error> {
    let (resource, filename) = match network {
        Network::Dash => (BundledResource::CoreConfigMainnet, "mainnet.conf"),
        Network::Testnet => (BundledResource::CoreConfigTestnet, "testnet.conf"),
        Network::Devnet => (BundledResource::CoreConfigDevnet, "devnet.conf"),
        Network::Regtest => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Local network does not support overwriting dash.conf",
            ));
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unsupported network",
            ))
        }
    };
    // Construct the full path to the config file
    let dir = app_user_data_dir_path().expect("Failed to get app user data directory path");
    let config_path = dir.join("dash_core_configs").join(filename);
    resource.write_to_file(&config_path, false)?;

    Ok(config_path)
}
