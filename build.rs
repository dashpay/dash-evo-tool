use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets/DET_LOGO.ico");

    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }

    let icon_path = Path::new("assets/DET_LOGO.ico");
    if !icon_path.exists() {
        eprintln!(
            "cargo:warning=Windows icon asset missing at {}",
            icon_path.display()
        );
        return;
    }

    let mut res = winres::WindowsResource::new();
    let icon_str = icon_path
        .to_str()
        .expect("icon path must be valid UTF-8 for the resource compiler");
    res.set_icon(icon_str);

    if let Ok(version) = env::var("CARGO_PKG_VERSION") {
        res.set("FileVersion", &version);
        res.set("ProductVersion", &version);
    }

    if let Ok(product_name) = env::var("CARGO_PKG_NAME") {
        res.set("ProductName", &product_name);
    }

    if let Err(err) = res.compile() {
        panic!("Failed to embed Windows resources: {err}");
    }
}
