use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

const QUALIFIER: &str = ""; // Typically empty on macOS and Linux
const ORGANIZATION: &str = "DashCoreGroup";
const APPLICATION: &str = "DashEvoTool";

pub fn app_user_data_dir_path() -> PathBuf {
    let proj_dirs = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .expect("Failed to get project directories");
    proj_dirs.config_dir().to_path_buf()
}
pub fn create_app_user_data_directory_if_not_exists() {
    let app_data_dir = app_user_data_dir_path();
    fs::create_dir_all(app_data_dir).expect("Failed to create config directory");
}

pub fn app_user_data_file_path(filename: &str) -> Result<PathBuf, std::io::Error> {
    if filename.is_empty() || filename.contains(std::path::is_separator) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid filename",
        ));
    }
    let app_data_dir = app_user_data_dir_path();
    Ok(app_data_dir.join(filename))
}

pub fn copy_mainnet_env_file_if_not_exists() {
    let app_data_dir = app_user_data_dir_path();
    let env_mainnet_file = app_data_dir.join(".env".to_string());
    if env_mainnet_file.exists() && env_mainnet_file.is_file() {
    } else {
        let env_example_file = PathBuf::from(".env.example");
        let target_env_file_path = app_user_data_file_path(".env").expect("should create target env file path");
        fs::copy(
            &env_example_file,
            target_env_file_path,
        )
        .expect("Failed to copy main net env file");
    }
}
