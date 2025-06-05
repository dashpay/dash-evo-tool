use dash_sdk::dpp::dashcore::Network;
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;

const QUALIFIER: &str = ""; // Typically empty on macOS and Linux
const ORGANIZATION: &str = "";
const APPLICATION: &str = "Dash-Evo-Tool";

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
        use directories::UserDirs;

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
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Created path is not a directory",
        ));
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

pub fn copy_env_file_if_not_exists() {
    let app_data_dir =
        app_user_data_dir_path().expect("Failed to determine application data directory");
    let env_file_in_app_dir = app_data_dir.join(".env".to_string());
    if !env_file_in_app_dir.exists() || !env_file_in_app_dir.is_file() {
        // Get the directory where the executable is located
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|p| p.to_path_buf()));

        if let Some(exe_dir) = exe_dir {
            let env_example_file_in_exe_dir = exe_dir.join(".env.example");
            if env_example_file_in_exe_dir.exists() && env_example_file_in_exe_dir.is_file() {
                fs::copy(&env_example_file_in_exe_dir, env_file_in_app_dir)
                    .expect("Failed to copy env file");
                return;
            }

            let env_file_in_exe_dir = exe_dir.join(".env");
            if env_file_in_exe_dir.exists() && env_file_in_exe_dir.is_file() {
                fs::copy(&env_file_in_exe_dir, env_file_in_app_dir)
                    .expect("Failed to copy env file");
                return;
            }
        }

        // Fallback to current working directory
        let env_example_file_in_exe_dir = PathBuf::from(".env.example");
        if env_example_file_in_exe_dir.exists() && env_example_file_in_exe_dir.is_file() {
            fs::copy(&env_example_file_in_exe_dir, env_file_in_app_dir)
                .expect("Failed to copy env file");
        } else {
            let env_file_in_exe_dir = PathBuf::from(".env");
            if env_file_in_exe_dir.exists() && env_file_in_exe_dir.is_file() {
                fs::copy(&env_file_in_exe_dir, env_file_in_app_dir)
                    .expect("Failed to copy env file");
            }
        }
    }
}
