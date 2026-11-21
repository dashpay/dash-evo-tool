//! What the user is shown about an error: a plain-language message and the
//! technical details behind it.

use std::error::Error;
use std::path::{Path, PathBuf};

use crate::backend_task::error::TaskError;

/// Message shown when a startup error carries no user-facing message of its own.
pub const GENERIC_STARTUP_FAILURE: &str = "Dash Evo Tool could not start because of an unexpected problem. Restart the application to try again.";

/// A user-facing message and the technical details behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReport {
    message: String,
    details: String,
}

impl ErrorReport {
    /// A report from an already prepared message and details.
    pub fn new(message: impl Into<String>, details: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            details: details.into(),
        }
    }

    /// The message is the `Display` of the first [`TaskError`] in `error`'s
    /// source chain, so every typed error brings its own wording; any other
    /// error gets `fallback`. The details are `error`'s `Debug` form.
    pub fn from_error(error: &(dyn Error + 'static), fallback: &str) -> Self {
        let message = std::iter::successors(Some(error), |&e| e.source())
            .find_map(|e| e.downcast_ref::<TaskError>())
            .map_or_else(|| fallback.to_owned(), ToString::to_string);
        Self::new(message, format!("{error:?}"))
    }

    /// Plain-language description of what happened and what to try.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Technical details (the error chain) for a support request.
    pub fn details(&self) -> &str {
        &self.details
    }
}

/// A fatal startup failure: its report and the log file of this run.
#[derive(Debug, Clone)]
pub struct StartupFailure {
    report: ErrorReport,
    log_file: Option<PathBuf>,
}

impl StartupFailure {
    /// A failure reported through [`ErrorReport::from_error`] with
    /// [`GENERIC_STARTUP_FAILURE`] as the fallback message.
    pub fn from_error(error: &(dyn Error + 'static), log_file: Option<PathBuf>) -> Self {
        Self::with_fallback(error, GENERIC_STARTUP_FAILURE, log_file)
    }

    /// A failure reported through [`ErrorReport::from_error`] with `fallback`
    /// as the message when the error carries none of its own.
    pub fn with_fallback(
        error: &(dyn Error + 'static),
        fallback: &str,
        log_file: Option<PathBuf>,
    ) -> Self {
        Self {
            report: ErrorReport::from_error(error, fallback),
            log_file,
        }
    }

    /// What to show the user.
    pub fn report(&self) -> &ErrorReport {
        &self.report
    }

    /// The log file holding the full record of this run, when one is written.
    pub fn log_file(&self) -> Option<&Path> {
        self.log_file.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wraps an error as its `source`, like a caller adding context.
    #[derive(Debug, thiserror::Error)]
    #[error("wrapped")]
    struct Wrapped(#[source] TaskError);

    /// A typed error boxed by `?` keeps its own user-facing message.
    #[test]
    fn boxed_task_error_keeps_its_message() {
        let error: Box<dyn Error + Send + Sync> = Box::new(TaskError::WalletStorageNotReady);
        let report = ErrorReport::from_error(&*error, "fallback");
        assert_eq!(
            report.message(),
            TaskError::WalletStorageNotReady.to_string()
        );
        assert_eq!(report.details(), "WalletStorageNotReady");
    }

    /// A typed error deeper in the source chain is still found.
    #[test]
    fn task_error_in_source_chain_is_found() {
        let report =
            ErrorReport::from_error(&Wrapped(TaskError::WalletStorageNotReady), "fallback");
        assert_eq!(
            report.message(),
            TaskError::WalletStorageNotReady.to_string()
        );
        assert!(report.details().starts_with("Wrapped("));
    }

    /// An untyped error never leaks its raw text as the message.
    #[test]
    fn untyped_error_uses_the_fallback() {
        let error = std::io::Error::other("disk exploded");
        let report = ErrorReport::from_error(&error, "fallback");
        assert_eq!(report.message(), "fallback");
        assert!(report.details().contains("disk exploded"));
    }

    /// Startup failures fall back to the generic startup message.
    #[test]
    fn startup_failure_uses_the_generic_startup_fallback() {
        let failure =
            StartupFailure::from_error(&std::io::Error::other("x"), Some(PathBuf::from("det.log")));
        assert_eq!(failure.report().message(), GENERIC_STARTUP_FAILURE);
        assert_eq!(failure.log_file(), Some(Path::new("det.log")));
    }
}
