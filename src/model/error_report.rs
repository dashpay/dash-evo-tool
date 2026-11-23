//! What the user is shown about an error: a plain-language message and the
//! technical details behind it.

use std::error::Error;
use std::path::{Path, PathBuf};

use crate::backend_task::error::TaskError;

/// Message shown when a startup error carries no user-facing message of its own.
pub const GENERIC_STARTUP_FAILURE: &str = "Dash Evo Tool could not start because of an unexpected problem. Restart the application to try again.";

/// Why the app window could not be opened, as far as the user can act on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowFailureKind {
    /// The graphics system (driver or graphics card) could not be started.
    Graphics,
    /// The app could not connect to the desktop to create its window.
    DesktopSession,
    /// Any other cause.
    Other,
}

impl WindowFailureKind {
    /// What happened and what the user can do about it.
    pub fn message(self) -> &'static str {
        match self {
            Self::Graphics => {
                "Dash Evo Tool could not open its window because the graphics system could not be started. Update your graphics drivers, then restart the application."
            }
            Self::DesktopSession => {
                "Dash Evo Tool could not open its window because it could not connect to your desktop. Sign out and sign back in, or restart your computer, then start the application again."
            }
            Self::Other => {
                "Dash Evo Tool could not open its window because of an unexpected problem. Restart the application to try again."
            }
        }
    }
}

/// The system dialog text for a window failure: the message for `kind`, the
/// log file location when known, and where to report the problem, which is
/// public.
pub fn window_failure_text(kind: WindowFailureKind, log_file: Option<&Path>) -> String {
    let mut text = kind.message().to_owned();
    match log_file {
        Some(log_file) => {
            text.push_str(&format!(
                "\n\nThe log file is at {path}.",
                path = log_file.display()
            ));
            text.push_str(&format!(
                "\n\nIf you cannot solve this yourself, report the problem at {url} and include the log file. The issue page is public, and the log file can include folder names from your computer, so read it before posting and remove anything you do not want to share.",
                url = crate::SUPPORT_URL
            ));
        }
        None => text.push_str(&format!(
            "\n\nIf you cannot solve this yourself, report the problem at {url}. The issue page is public, so do not post anything you do not want to share.",
            url = crate::SUPPORT_URL
        )),
    }
    text
}

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
        let error: Box<dyn Error + Send + Sync> = Box::new(TaskError::NoNetworkAvailable);
        let report = ErrorReport::from_error(&*error, "fallback");
        assert_eq!(report.message(), TaskError::NoNetworkAvailable.to_string());
        assert_eq!(report.details(), "NoNetworkAvailable");
    }

    /// A typed error deeper in the source chain is still found.
    #[test]
    fn task_error_in_source_chain_is_found() {
        let report = ErrorReport::from_error(&Wrapped(TaskError::NoNetworkAvailable), "fallback");
        assert_eq!(report.message(), TaskError::NoNetworkAvailable.to_string());
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

    /// The window failure text says what happened, what to do, where the log
    /// is and where to report the problem.
    #[test]
    fn window_failure_text_names_the_log_and_support() {
        let kind = WindowFailureKind::Graphics;
        let text = window_failure_text(kind, Some(Path::new("/logs/det.log")));
        assert!(text.starts_with(kind.message()), "{text}");
        assert!(text.contains("The log file is at /logs/det.log."), "{text}");
        assert!(text.contains(crate::SUPPORT_URL), "{text}");

        let text = window_failure_text(kind, None);
        assert!(!text.contains("log file is at"), "{text}");
    }

    /// The text asks for the log file only when there is one, and warns in
    /// both cases that the issue page is public.
    #[test]
    fn window_failure_text_asks_for_the_log_only_when_it_exists() {
        let kind = WindowFailureKind::Other;
        let text = window_failure_text(kind, Some(Path::new("/logs/det.log")));
        assert!(text.contains("include the log file"), "{text}");
        assert!(text.contains("public"), "{text}");

        let text = window_failure_text(kind, None);
        assert!(!text.contains("log file"), "{text}");
        assert!(text.contains(crate::SUPPORT_URL), "{text}");
        assert!(text.contains("public"), "{text}");
    }

    /// Only a graphics failure sends the user to their graphics drivers; a
    /// desktop connection failure or an unknown cause gets its own advice.
    #[test]
    fn window_failure_advice_follows_the_kind() {
        let graphics = WindowFailureKind::Graphics.message();
        let desktop = WindowFailureKind::DesktopSession.message();
        let other = WindowFailureKind::Other.message();
        assert!(graphics.contains("graphics drivers"), "{graphics}");
        assert!(!desktop.contains("graphics"), "{desktop}");
        assert!(desktop.contains("desktop"), "{desktop}");
        assert!(!other.contains("graphics"), "{other}");
        assert_ne!(desktop, other);

        let text = window_failure_text(WindowFailureKind::DesktopSession, None);
        assert!(text.starts_with(desktop), "{text}");
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
