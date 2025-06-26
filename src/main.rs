use crate::app_dir::{app_user_data_dir_path, create_app_user_data_directory_if_not_exists};
use crate::cpu_compatibility::check_cpu_compatibility;
use std::env;

mod app;
mod app_dir;
mod backend_task;
mod bundled;
mod components;
mod config;
mod context;
mod context_provider;
mod cpu_compatibility;
mod database;
mod logging;
mod model;
mod sdk_wrapper;
mod ui;
mod utils;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> eframe::Result<()> {
    create_app_user_data_directory_if_not_exists()
        .expect("Failed to create app user_data directory");
    let app_data_dir =
        app_user_data_dir_path().expect("Failed to get app user_data directory path");
    println!(
        "Starting dash-evo-tool, version: {}, data dir: {}",
        VERSION,
        app_data_dir.display()
    );
    check_cpu_compatibility();
    // Initialize the Tokio runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(40)
        .enable_all()
        .build()
        .expect("multi-threading runtime cannot be initialized");

    // Run the native application
    runtime.block_on(async {
        let native_options = eframe::NativeOptions {
            persist_window: true, // Persist window size and position
            centered: true,       // Center window on startup if not maximized
            persistence_path: Some(app_data_dir.join("app.ron")),
            ..Default::default()
        };
        eframe::run_native(
            &format!("Dash Evo Tool v{}", VERSION),
            native_options,
            Box::new(|cc| {
                use egui::{FontData, FontDefinitions, FontFamily};

                let mut fonts = FontDefinitions::default();

                fonts.font_data.insert(
                    "noto_sans".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans/static/NotoSans-Light.ttf"
                    ))
                    .into(),
                );

                // Insert each regional font
                fonts.font_data.insert(
                    "noto_sans_sc".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans_SC/static/NotoSansSC-Light.ttf"
                    ))
                    .into(),
                );
                fonts.font_data.insert(
                    "noto_sans_tc".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans_TC/static/NotoSansTC-Light.ttf"
                    ))
                    .into(),
                );
                fonts.font_data.insert(
                    "noto_sans_jp".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans_JP/static/NotoSansJP-Light.ttf"
                    ))
                    .into(),
                );
                fonts.font_data.insert(
                    "noto_sans_kr".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans_KR/static/NotoSansKR-Light.ttf"
                    ))
                    .into(),
                );
                fonts.font_data.insert(
                    "noto_sans_thai".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans_Thai/static/NotoSansThai-Light.ttf"
                    ))
                    .into(),
                );
                fonts.font_data.insert(
                    "noto_sans_khmer".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans_Khmer/static/NotoSansKhmer-Light.ttf"
                    ))
                    .into(),
                );
                fonts.font_data.insert(
                    "noto_sans_arabic".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans_Arabic/static/NotoSansArabic-Light.ttf"
                    ))
                    .into(),
                );
                fonts.font_data.insert(
                    "noto_sans_hebrew".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans_Hebrew/static/NotoSansHebrew-Light.ttf"
                    ))
                    .into(),
                );
                fonts.font_data.insert(
                    "noto_sans_devanagari".to_owned(),
                    FontData::from_static(include_bytes!(
                        "../assets/Fonts/Noto_Sans_Devanagari/static/NotoSansDevanagari-Light.ttf"
                    ))
                    .into(),
                );

                // Define fallback chain for proportional text
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .splice(
                        ..,
                        vec![
                            "Ubuntu-Light".into(),
                            "emoji-icon-font".into(),
                            "NotoEmoji-Regular".into(),
                            "noto_sans".to_owned(),
                            "noto_sans_sc".to_owned(), // Simplified Chinese
                            "noto_sans_tc".to_owned(), // Traditional Chinese
                            "noto_sans_jp".to_owned(), // Japanese
                            "noto_sans_kr".to_owned(), // Korean
                            "noto_sans_thai".to_owned(),
                            "noto_sans_khmer".to_owned(),
                            "noto_sans_arabic".to_owned(),
                            "noto_sans_hebrew".to_owned(),
                            "noto_sans_devanagari".to_owned(),
                        ],
                    );

                fonts
                    .families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .splice(
                        ..,
                        vec![
                            "Hack".into(),
                            "Ubuntu-Light".into(),
                            "emoji-icon-font".into(),
                            "NotoEmoji-Regular".into(),
                            "noto_sans".to_owned(),
                            "noto_sans_sc".to_owned(), // Simplified Chinese
                            "noto_sans_tc".to_owned(), // Traditional Chinese
                            "noto_sans_jp".to_owned(), // Japanese
                            "noto_sans_kr".to_owned(), // Korean
                            "noto_sans_thai".to_owned(),
                            "noto_sans_khmer".to_owned(),
                            "noto_sans_arabic".to_owned(),
                            "noto_sans_hebrew".to_owned(),
                            "noto_sans_devanagari".to_owned(),
                        ],
                    );

                cc.egui_ctx.set_fonts(fonts);

                Ok(Box::new(app::AppState::new()))
            }),
        )
    })
}
