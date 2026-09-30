//! Shared identity label resolution.

/// Resolve a profile name, username, or shortened identity identifier.
pub fn display_label(display_name: Option<&str>, username: Option<&str>, id: &str) -> String {
    [display_name, username]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if id.trim().is_empty() {
                "Unknown identity".into()
            } else {
                shorten_id(id.trim())
            }
        })
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
