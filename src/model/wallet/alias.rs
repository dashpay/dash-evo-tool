use std::fmt;

pub const MAX_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasError {
    EmptyOrWhitespace,
    TooLong {
        max_chars: usize,
        actual_chars: usize,
    },
}

impl fmt::Display for AliasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AliasError::EmptyOrWhitespace => {
                write!(
                    f,
                    "Enter a name with at least one non-whitespace character."
                )
            }
            AliasError::TooLong {
                max_chars,
                actual_chars,
            } => {
                write!(
                    f,
                    "Name is {actual_chars} characters; the limit is {max_chars}."
                )
            }
        }
    }
}

pub fn validate_optional_alias(alias_input: &str) -> Result<Option<&str>, AliasError> {
    let trimmed_alias = alias_input.trim();
    if trimmed_alias.is_empty() {
        return Ok(None);
    }

    validate_alias_length(trimmed_alias).map(Some)
}

pub fn validate_required_alias(alias_input: &str) -> Result<&str, AliasError> {
    let trimmed_alias = alias_input.trim();
    if trimmed_alias.is_empty() {
        return Err(AliasError::EmptyOrWhitespace);
    }

    validate_alias_length(trimmed_alias)
}

fn validate_alias_length(alias: &str) -> Result<&str, AliasError> {
    let actual_chars = alias.chars().count();
    if actual_chars > MAX_CHARS {
        return Err(AliasError::TooLong {
            max_chars: MAX_CHARS,
            actual_chars,
        });
    }

    Ok(alias)
}

#[cfg(test)]
mod tests {
    use super::{AliasError, MAX_CHARS, validate_optional_alias, validate_required_alias};

    #[test]
    fn optional_alias_allows_empty_input() {
        assert_eq!(validate_optional_alias(""), Ok(None));
        assert_eq!(validate_optional_alias("   "), Ok(None));
    }

    #[test]
    fn required_alias_rejects_empty_input() {
        assert_eq!(
            validate_required_alias(""),
            Err(AliasError::EmptyOrWhitespace)
        );
        assert_eq!(
            validate_required_alias("\t  \n"),
            Err(AliasError::EmptyOrWhitespace)
        );
    }

    #[test]
    fn accepts_single_ascii_character() {
        assert_eq!(validate_optional_alias("a"), Ok(Some("a")));
        assert_eq!(validate_required_alias("a"), Ok("a"));
    }

    #[test]
    fn accepts_exactly_sixty_four_ascii_characters() {
        let alias = "a".repeat(MAX_CHARS);
        assert_eq!(validate_optional_alias(&alias), Ok(Some(alias.as_str())));
        assert_eq!(validate_required_alias(&alias), Ok(alias.as_str()));
    }

    #[test]
    fn rejects_sixty_five_ascii_characters() {
        let alias = "a".repeat(MAX_CHARS + 1);
        assert_eq!(
            validate_optional_alias(&alias),
            Err(AliasError::TooLong {
                max_chars: MAX_CHARS,
                actual_chars: MAX_CHARS + 1,
            })
        );
        assert_eq!(
            validate_required_alias(&alias),
            Err(AliasError::TooLong {
                max_chars: MAX_CHARS,
                actual_chars: MAX_CHARS + 1,
            })
        );
    }

    #[test]
    fn accepts_exactly_sixty_four_multibyte_codepoints() {
        let alias = "界".repeat(MAX_CHARS);
        assert_eq!(alias.chars().count(), MAX_CHARS);
        assert!(alias.len() > MAX_CHARS);
        assert_eq!(validate_optional_alias(&alias), Ok(Some(alias.as_str())));
        assert_eq!(validate_required_alias(&alias), Ok(alias.as_str()));
    }

    #[test]
    fn rejects_sixty_five_multibyte_codepoints() {
        let alias = "界".repeat(MAX_CHARS + 1);
        assert_eq!(alias.chars().count(), MAX_CHARS + 1);
        assert!(alias.len() > MAX_CHARS + 1);
        assert_eq!(
            validate_optional_alias(&alias),
            Err(AliasError::TooLong {
                max_chars: MAX_CHARS,
                actual_chars: MAX_CHARS + 1,
            })
        );
        assert_eq!(
            validate_required_alias(&alias),
            Err(AliasError::TooLong {
                max_chars: MAX_CHARS,
                actual_chars: MAX_CHARS + 1,
            })
        );
    }

    #[test]
    fn display_messages_are_actionable() {
        assert_eq!(
            AliasError::EmptyOrWhitespace.to_string(),
            "Enter a name with at least one non-whitespace character."
        );
        assert_eq!(
            AliasError::TooLong {
                max_chars: MAX_CHARS,
                actual_chars: MAX_CHARS + 1,
            }
            .to_string(),
            "Name is 65 characters; the limit is 64."
        );
    }
}
