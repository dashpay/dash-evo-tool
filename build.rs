use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Fetch the version from CARGO_PKG_VERSION
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "??".to_string());

    // Generate a Rust file with the version constant
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("version.rs");
    fs::write(
        dest_path,
        format!(r#"pub const VERSION: &str = "{}";"#, version),
    )
    .expect("Failed to write version.rs");
}
