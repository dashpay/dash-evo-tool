//! Compile-time embedded Fluent localization.

use std::borrow::Cow;
use std::collections::HashMap;

use fluent_templates::Loader;
use fluent_templates::fluent_bundle::FluentValue;
use fluent_templates::static_loader;
use unic_langid::LanguageIdentifier;

static_loader! {
    pub static LOCALES = {
        locales: "locales",
        fallback_language: "en",
    };
}

/// Detect the system locale, falling back to English.
pub fn system_locale() -> LanguageIdentifier {
    sys_locale::get_locale()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| "en".parse().expect("valid langid"))
}

/// Initialize localization — call once at startup.
pub fn init() {
    let locale = system_locale();
    tracing::info!(%locale, "Locale initialized");
}

/// Look up a Fluent message by ID for the current locale.
pub fn lookup(msg_id: &str) -> Option<String> {
    let locale = system_locale();
    LOCALES.try_lookup(&locale, msg_id)
}

/// Look up a Fluent message with arguments.
pub fn lookup_with_args(
    msg_id: &str,
    args: &HashMap<Cow<'static, str>, FluentValue<'_>>,
) -> Option<String> {
    let locale = system_locale();
    LOCALES.try_lookup_with_args(&locale, msg_id, args)
}
