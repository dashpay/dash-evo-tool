use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

const QUALIFIER: &str = ""; // Typically empty on macOS and Linux
const ORGANIZATION: &str = "DashCoreGroup";
const APPLICATION: &str = "DashEvoTool";

pub fn app_user_data_dir_path() -> Result<PathBuf, std::io::Error> {
    let proj_dirs = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .ok_or_else(|| std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Failed to determine project directories",
        ))?;
    Ok(proj_dirs.config_dir().to_path_buf())
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
    let app_data_dir = app_user_data_dir_path().expect("Failed to determine application data directory");
    let env_file = app_data_dir.join(".env".to_string());
    if env_file.exists() && env_file.is_file() {
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
