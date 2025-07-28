use std::path::Path;

/// Formats a file path for user-friendly display.
///
/// On macOS, this function handles .app bundles specially:
/// - If the path ends with `.app/Contents/MacOS/Dash-Qt`, it displays as `Dash-Qt.app`
/// - Otherwise, it displays the full path
///
/// # Examples
/// ```no_run
/// "/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt" -> "Dash-Qt.app"
/// "/usr/local/bin/dash-qt" -> "/usr/local/bin/dash-qt"
/// ```
pub fn format_path_for_display(path: &Path) -> String {
    let path_str = path.to_string_lossy();

    // Check if this is a macOS app bundle executable path
    if cfg!(target_os = "macos") {
        // Check if the path matches the pattern for an app bundle executable
        if let Some(app_start) = path_str.rfind(".app/Contents/MacOS/") {
            // Find the start of the app name by looking backwards for a path separator
            let before_app = &path_str[..app_start];
            let app_name_start = before_app.rfind('/').map(|i| i + 1).unwrap_or(0);
            let app_name = &path_str[app_name_start..app_start + 4]; // Include ".app"
            return app_name.to_string();
        }
    }

    // For all other cases, return the full path
    path_str.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_format_macos_app_bundle() {
        let path = PathBuf::from("/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt");
        assert_eq!(format_path_for_display(&path), "Dash-Qt.app");
    }

    #[test]
    fn test_format_macos_app_bundle_with_spaces() {
        let path = PathBuf::from("/Applications/My Apps/Dash Qt.app/Contents/MacOS/Dash-Qt");
        assert_eq!(format_path_for_display(&path), "Dash Qt.app");
    }

    #[test]
    fn test_format_regular_path() {
        let path = PathBuf::from("/usr/local/bin/dash-qt");
        assert_eq!(format_path_for_display(&path), "/usr/local/bin/dash-qt");
    }

    #[test]
    fn test_format_windows_path() {
        let path = PathBuf::from("C:\\Program Files\\Dash\\dash-qt.exe");
        assert_eq!(
            format_path_for_display(&path),
            "C:\\Program Files\\Dash\\dash-qt.exe"
        );
    }
}
