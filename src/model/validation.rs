/// A text field is outside its permitted character-count range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("text contains {actual} characters; expected between {min} and {max}")]
pub struct TextLengthError {
    /// Actual character count.
    pub actual: usize,
    /// Minimum permitted character count.
    pub min: usize,
    /// Maximum permitted character count.
    pub max: usize,
}

pub(crate) fn validate_char_count(
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), TextLengthError> {
    let actual = value.chars().count();
    if (min..=max).contains(&actual) {
        Ok(())
    } else {
        Err(TextLengthError { actual, min, max })
    }
}
