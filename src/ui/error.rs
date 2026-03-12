//! UI-layer error types.
use thiserror::Error;

/// A user-facing validation or UI operation error.
#[derive(Debug, Error)]
pub enum UiError {
    /// A user input validation or UI operation error.
    #[error("{0}")]
    Validation(String),
}

impl From<String> for UiError {
    fn from(s: String) -> Self {
        UiError::Validation(s)
    }
}

impl From<&str> for UiError {
    fn from(s: &str) -> Self {
        UiError::Validation(s.to_string())
    }
}
