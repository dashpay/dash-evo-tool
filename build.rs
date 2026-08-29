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

    // `winres` picks its resource compiler from the target environment: GNU
    // (mingw) targets go through `windres` + `ar`, while MSVC targets go through
    // the Windows SDK's `rc.exe`, which `winres` locates on its own. Only resolve
    // the mingw tools when the target actually needs them, so a native MSVC build
    // on Windows does not require a mingw toolchain.
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env != "msvc" {
        let windres_cmd = resolve_windres().unwrap_or_else(|| {
            panic!(
                "Required tool not found: windres. Set WINDRES or install x86_64-w64-mingw32-windres."
            )
        });
        res.set_windres_path(&windres_cmd);
        let ar_cmd = resolve_ar(&target).unwrap_or_else(|| {
            panic!(
                "Required tool not found: ar. Set AR_{} or AR, or install x86_64-w64-mingw32-ar.",
                target.replace('-', "_")
            )
        });
        res.set_ar_path(&ar_cmd);
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
