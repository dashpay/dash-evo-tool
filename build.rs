use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/DET_LOGO.ico");
    println!("cargo:rerun-if-changed=assets/DET_LOGO.png");
    println!("cargo:rerun-if-env-changed=WINDRES");

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

    let windres_cmd = resolve_windres();
    if let Some(cmd) = windres_cmd {
        res.set_windres_path(&cmd);
        let ar_cmd = resolve_ar(&target).unwrap_or_else(|| {
            panic!(
                "Required tool not found: ar. Set AR_{} or AR, or install x86_64-w64-mingw32-ar.",
                target.replace('-', "_")
            )
        });
        res.set_ar_path(&ar_cmd);
    } else {
        panic!(
            "Required tool not found: windres. Set WINDRES or install x86_64-w64-mingw32-windres."
        );
    }

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

fn resolve_windres() -> Option<String> {
    resolve_tool(
        &[String::from("WINDRES")],
        &["x86_64-w64-mingw32-windres", "windres"],
    )
}

fn resolve_ar(target: &str) -> Option<String> {
    let ar_target_key = format!("AR_{}", target.replace('-', "_"));
    resolve_tool(
        &[ar_target_key, String::from("AR")],
        &["x86_64-w64-mingw32-ar", "ar"],
    )
}

fn resolve_tool(env_keys: &[String], candidates: &[&str]) -> Option<String> {
    for key in env_keys {
        if let Ok(cmd) = env::var(key)
            && is_available(&cmd)
        {
            return Some(cmd);
        }
    }

    for &cmd in candidates {
        if is_available(cmd) {
            return Some(cmd.to_string());
        }
    }

    None
}

fn is_available(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
