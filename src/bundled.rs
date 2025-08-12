use egui::{FontData, FontDefinitions, FontFamily};

/// Load some resource that is bundled with the application.
///
/// Supported paths:
/// - `.env.example`: Loads the bundled `.env.example` file.
pub(crate) enum BundledResource {
    DotEnvExample,
    CoreConfigMainnet,
    CoreConfigTestnet,
    CoreConfigDevnet,
}

impl BundledResource {
    /// Loads the resource as a byte slice.
    pub(crate) fn load(&self) -> &'static [u8] {
        match self {
            Self::DotEnvExample => include_bytes!("../.env.example"),
            Self::CoreConfigMainnet => include_bytes!("../dash_core_configs/mainnet.conf"),
            Self::CoreConfigTestnet => include_bytes!("../dash_core_configs/testnet.conf"),
            Self::CoreConfigDevnet => include_bytes!("../dash_core_configs/devnet.conf"),
        }
    }

    /// Writes the resource to a file. Creates directories if they do not exist.
    /// When overwriting, it will replace the file if it exists.
    ///
    /// Returns `Ok(true)` if the file was written, `Ok(false)` if it already existed and was not overwritten,
    /// or an `io::Error` if there was an issue writing the file.
    pub(crate) fn write_to_file(
        &self,
        path: &std::path::Path,
        overwrite: bool,
    ) -> std::io::Result<bool> {
        let exists = path.exists();
        if !exists || overwrite {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, self.load())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Load bundled fonts into an `egui::FontDefinitions`.
///
/// This function takes a vector of tuples where each tuple contains the font name and the path to the font file.
/// Path should be relative to the `assets/Fonts` directory.
pub fn fonts() -> Result<egui::FontDefinitions, Box<dyn std::error::Error>> {
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

    Ok(fonts)
}
