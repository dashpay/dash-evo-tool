//! Shared identity label resolution.

/// Resolve a profile name, username, or shortened identity identifier.
pub fn display_label(display_name: Option<&str>, username: Option<&str>, id: &str) -> String {
    [display_name, username]
        .into_iter()
        .flatten()
        .map(clean_display_text)
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| {
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

/// Keep visible text and normal Unicode shaping while excluding controls and bidi directives.
pub(super) fn is_safe_display_character(character: char) -> bool {
    !character.is_control()
        && !matches!(character,
            '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        )
}

/// Shorten a Base58 identifier while preserving both ends.
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
}
