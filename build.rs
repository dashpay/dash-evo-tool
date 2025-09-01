use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Generate version info
    let out_dir = env::var("OUT_DIR").unwrap();
    let version_file = Path::new(&out_dir).join("version.rs");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    fs::write(
        version_file,
        format!("pub const VERSION: &str = \"{}\";", version),
    )
    .unwrap();

    // Windows-specific icon embedding
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/DET_LOGO.ico");
        res.compile().unwrap();
    }
}
