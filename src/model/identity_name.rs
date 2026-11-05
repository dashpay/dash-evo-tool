//! Shared identity label resolution.

/// Resolve a masternode alias or its shortened hexadecimal ProTxHash.
pub fn masternode_label(id: dash_sdk::platform::Identifier, alias: Option<&str>) -> String {
    alias
        .filter(|alias| !alias.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            shorten_id(&id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Hex))
        })
}

/// Resolve a profile name or username, or `None` for an identity with neither.
pub fn display_name_or_username(
    display_name: Option<&str>,
    username: Option<&str>,
) -> Option<String> {
    [display_name, username]
        .into_iter()
        .flatten()
        .map(clean_display_text)
        .find(|name| !name.is_empty())
}

/// Resolve a profile name, username, or shortened identity identifier.
pub fn display_label(display_name: Option<&str>, username: Option<&str>, id: &str) -> String {
    display_name_or_username(display_name, username).unwrap_or_else(|| {
        if id.trim().is_empty() {
            "Unknown identity".into()
        } else {
            shorten_id(id.trim())
        }
    })
}

/// Remove control and bidi characters from rendered text, then trim surrounding whitespace.
pub fn clean_display_text(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| is_safe_display_character(*c))
        .collect();
    cleaned.trim().to_owned()
}

/// Keep visible text and normal Unicode shaping while excluding controls,
/// bidi directives and every invisible format character that makes two names
/// look identical: soft hyphen, Mongolian vowel separator, ZWSP, word joiner
/// and the invisible operators, BOM, and the tag characters. ZWNJ and ZWJ
/// stay: scripts and emoji sequences need them.
pub(super) fn is_safe_display_character(character: char) -> bool {
    !character.is_control()
        && !matches!(character,
            '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}' | '\u{200e}' | '\u{200f}'
            | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}'
            | '\u{feff}' | '\u{e0000}'..='\u{e007f}'
        )
}

/// Shorten an identifier while preserving both ends.
pub fn shorten_id(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 10 {
        return id.to_owned();
    }
    let head: String = chars[..5].iter().collect();
    let tail: String = chars[chars.len() - 3..].iter().collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_filter_controls_before_profile_username_id_fallback() {
        assert_eq!(
            display_label(Some(" Al\u{202e}i\nce "), Some("username"), "abcdefghijk"),
            "Alice"
        );
        assert_eq!(
            display_label(
                Some("\u{061c}\u{2066} "),
                Some(" user\u{200f}name "),
                "abcdefghijk"
            ),
            "username"
        );
        assert_eq!(
            display_label(Some("\0"), Some("\u{202a}\u{2069}"), "abcdefghijk"),
            "abcde…ijk"
        );
        assert_eq!(
            display_label(Some("李雷 👩‍💻 العربية"), Some("username"), "abcdefghijk"),
            "李雷 👩‍💻 العربية"
        );
    }

    #[test]
    fn invisible_spaces_are_dropped_but_joiners_shape_text() {
        assert_eq!(clean_display_text("Dash\u{200b}Coffee"), "DashCoffee");
        assert_eq!(clean_display_text("\u{feff}Grit\u{2060}"), "Grit");
        assert_eq!(
            clean_display_text("\u{1F468}\u{200d}\u{1F4BB}"),
            "\u{1F468}\u{200d}\u{1F4BB}",
            "an emoji sequence keeps its joiner"
        );
    }

    /// Every invisible format character an attacker can hide in a name.
    const INVISIBLE: [&str; 9] = [
        "\u{00ad}",
        "\u{180e}",
        "\u{200b}",
        "\u{2060}",
        "\u{2061}",
        "\u{2063}",
        "\u{2064}",
        "\u{feff}",
        "\u{e0041}",
    ];

    #[test]
    fn invisible_format_characters_are_dropped_from_identity_names() {
        for hidden in INVISIBLE {
            let name = format!("Al{hidden}ice");
            assert_eq!(clean_display_text(&name), "Alice", "{hidden:?} survived");
            assert_eq!(
                display_label(Some(&name), None, "abcdefghijk"),
                "Alice",
                "{hidden:?} survived in a profile name"
            );
            assert_eq!(
                display_label(None, Some(&format!("{hidden}bob{hidden}")), "abcdefghijk"),
                "bob",
                "{hidden:?} survived in a username"
            );
        }
        let only_hidden = INVISIBLE.concat();
        assert_eq!(
            display_label(Some(&only_hidden), Some(&only_hidden), "abcdefghijk"),
            "abcde…ijk",
            "a name made only of invisible characters falls back to the ID"
        );
    }

    #[test]
    fn profile_name_precedes_username_and_blank_names_fall_back() {
        assert_eq!(
            display_label(Some(" Alex "), Some("alex.dash"), "abcdefghijk"),
            "Alex"
        );
        assert_eq!(
            display_label(Some(" "), Some("alex.dash"), "abcdefghijk"),
            "alex.dash"
        );
        assert_eq!(display_label(None, None, "abcdefghijk"), "abcde…ijk");
    }

    #[test]
    fn an_identity_without_a_profile_name_or_username_has_no_name() {
        assert_eq!(
            display_name_or_username(Some(" Alex "), Some("alex.dash")).as_deref(),
            Some("Alex")
        );
        assert_eq!(
            display_name_or_username(Some("\u{202e} "), Some("alex.dash")).as_deref(),
            Some("alex.dash")
        );
        assert_eq!(display_name_or_username(Some(" "), Some("\0")), None);
        assert_eq!(display_name_or_username(None, None), None);
    }
}
