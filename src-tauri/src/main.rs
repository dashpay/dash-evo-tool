// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to Dash Evo Tool.", name)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running Dash Evo Tool");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greet_returns_expected_message() {
        assert_eq!(greet("Alice"), "Hello, Alice! Welcome to Dash Evo Tool.");
    }

    #[test]
    fn greet_handles_empty_name() {
        assert_eq!(greet(""), "Hello, ! Welcome to Dash Evo Tool.");
    }
}
