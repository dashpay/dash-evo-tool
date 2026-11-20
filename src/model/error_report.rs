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
        Self {
            report: ErrorReport::from_error(error, GENERIC_STARTUP_FAILURE),
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

/// Longest assistant link [`ai_question`] produces; longer URLs are cut by
/// some browsers and servers.
pub const MAX_QUESTION_URL_LEN: usize = 4000;

/// How [`ai_question`] describes a failure at startup.
pub const STARTUP_SITUATION: &str = "It fails to start and shows this message:";

/// Shortest user name redacted on its own; shorter ones would mangle words.
const MIN_REDACTED_USER_NAME_LEN: usize = 3;
/// Shortest hex run treated as possible key material.
const MIN_REDACTED_HEX_LEN: usize = 32;
/// Shortest Base58 run treated as possible key material (a WIF key is 51+).
const MIN_REDACTED_BASE58_LEN: usize = 50;
const REDACTED: &str = "<redacted>";
const TRUNCATED: &str = "… (truncated)";

/// An AI assistant that opens with a question prefilled from its URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiAssistant {
    /// ChatGPT, run by OpenAI.
    ChatGpt,
}

impl AiAssistant {
    /// Every assistant offered, in display order.
    pub const ALL: [Self; 1] = [Self::ChatGpt];

    /// The assistant's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::ChatGpt => "ChatGPT",
        }
    }

    /// The company that receives the question.
    pub fn operator(self) -> &'static str {
        match self {
            Self::ChatGpt => "OpenAI",
        }
    }

    fn url_prefix(self) -> &'static str {
        match self {
            Self::ChatGpt => "https://chatgpt.com/?q=",
        }
    }

    /// The link that opens this assistant with `question` prefilled.
    pub fn question_url(self, question: &str) -> String {
        format!("{}{}", self.url_prefix(), urlencoding::encode(question))
    }
}

/// Where a question is asked from: shown in it, and removed from it.
#[derive(Debug, Clone)]
pub struct QuestionEnvironment {
    /// App version.
    pub version: String,
    /// `std::env::consts::OS` value.
    pub os: &'static str,
    /// `std::env::consts::ARCH` value.
    pub arch: &'static str,
    /// Home folder, shown as `~`.
    pub home_dir: Option<PathBuf>,
    /// Login name, replaced wherever it stands as a separate word.
    pub user_name: Option<String>,
}

impl QuestionEnvironment {
    /// This process's version, platform, home folder and login name.
    pub fn current() -> Self {
        Self {
            version: crate::VERSION.to_owned(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            home_dir: directories::UserDirs::new().map(|dirs| dirs.home_dir().to_owned()),
            user_name: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok(),
        }
    }

    fn os_name(&self) -> &'static str {
        match self.os {
            "linux" => "Linux",
            "macos" => "macOS",
            "windows" => "Windows",
            other => other,
        }
    }
}

/// A question for an AI assistant about `report`, introduced by `situation`.
///
/// The home folder becomes `~`, the login name and anything that looks like
/// key material are removed, and the details are cut so that every
/// [`AiAssistant::question_url`] stays within [`MAX_QUESTION_URL_LEN`].
pub fn ai_question(report: &ErrorReport, situation: &str, env: &QuestionEnvironment) -> String {
    let message = redact_identity(report.message(), env);
    let details = redact_key_material(&redact_identity(report.details(), env));
    let compose = |details: &str| {
        format!(
            "I use Dash Evo Tool (desktop wallet for the Dash network) version {version} on {os} {arch}. {situation}\n\n\
             \"{message}\"\n\n\
             Technical details (paths redacted):\n{details}\n\n\
             Please explain in simple terms what is wrong and give me step-by-step instructions to fix it on my system.\n\n\
             Background you can read:\n\
             - Dash Evo Tool user docs: https://docs.dash.org/en/stable/docs/user/network/dash-evo-tool/\n\
             - Source and issues: https://github.com/dashpay/dash-evo-tool\n\
             - Dash Platform knowledge base: https://github.com/lklimek/claudash/blob/main/skills/dash-dev/SKILL.md\n",
            version = env.version,
            os = env.os_name(),
            arch = env.arch,
        )
    };
    let prefix = AiAssistant::ALL
        .iter()
        .map(|assistant| assistant.url_prefix().len())
        .max()
        .unwrap_or(0);
    let fits =
        |question: &str| prefix + urlencoding::encode(question).len() <= MAX_QUESTION_URL_LEN;
    let full = compose(&details);
    if fits(&full) {
        return full;
    }
    // Keep the longest prefix of the details that still fits.
    let boundaries: Vec<usize> = details.char_indices().map(|(index, _)| index).collect();
    if boundaries.is_empty() {
        return full;
    }
    let cut = |kept: usize| format!("{}{TRUNCATED}", &details[..boundaries[kept]]);
    let (mut low, mut high) = (0, boundaries.len() - 1);
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(&compose(&cut(middle))) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    compose(&cut(low))
}

/// `text` with the home folder shown as `~` and the login name replaced.
fn redact_identity(text: &str, env: &QuestionEnvironment) -> String {
    let mut text = text.to_owned();
    if let Some(home) = env.home_dir.as_deref().and_then(Path::to_str)
        && !home.is_empty()
    {
        text = text.replace(home, "~");
    }
    match env.user_name.as_deref() {
        Some(name) if name.len() >= MIN_REDACTED_USER_NAME_LEN => {
            replace_word(&text, name, "<user>")
        }
        _ => text,
    }
}

/// `text` with every standalone occurrence of `word` replaced by `with`.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let is_word_char = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(found) = rest.find(word) {
        let before = rest[..found].chars().next_back();
        let after = rest[found + word.len()..].chars().next();
        out.push_str(&rest[..found]);
        if before.is_some_and(is_word_char) || after.is_some_and(is_word_char) {
            out.push_str(word);
        } else {
            out.push_str(with);
        }
        rest = &rest[found + word.len()..];
    }
    out.push_str(rest);
    out
}

/// `text` with long hex or Base58 runs, which could be keys or seeds, removed.
fn redact_key_material(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let hex = run.len() >= MIN_REDACTED_HEX_LEN && run.chars().all(|c| c.is_ascii_hexdigit());
        let base58 = run.len() >= MIN_REDACTED_BASE58_LEN
            && run.chars().all(|c| !matches!(c, '0' | 'O' | 'I' | 'l'));
        out.push_str(if hex || base58 { REDACTED } else { run });
        run.clear();
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
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

    fn environment() -> QuestionEnvironment {
        QuestionEnvironment {
            version: "1.2.3".to_owned(),
            os: "linux",
            arch: "x86_64",
            home_dir: Some(PathBuf::from("/home/alice")),
            user_name: Some("alice".to_owned()),
        }
    }

    /// The question names the app, version, platform, message, details and the
    /// background links, and asks for plain step-by-step help.
    #[test]
    fn question_contains_message_details_and_links() {
        let report = ErrorReport::new("The folder is unsafe.", "InsecureParentDir { mode: 0o775 }");
        let question = ai_question(&report, STARTUP_SITUATION, &environment());
        for part in [
            "Dash Evo Tool",
            "version 1.2.3 on Linux x86_64",
            STARTUP_SITUATION,
            "\"The folder is unsafe.\"",
            "InsecureParentDir { mode: 0o775 }",
            "step-by-step",
            "https://docs.dash.org/en/stable/docs/user/network/dash-evo-tool/",
            "https://github.com/dashpay/dash-evo-tool",
            "https://github.com/lklimek/claudash/blob/main/skills/dash-dev/SKILL.md",
        ] {
            assert!(question.contains(part), "{part} missing:\n{question}");
        }
    }

    /// The home folder becomes `~`, the user name is replaced, and long hex or
    /// Base58 runs that could be keys are removed, in message and details.
    #[test]
    fn question_redacts_home_user_and_key_material() {
        let key = "a".repeat(64);
        // A Base58 run as long as a WIF key; not a real key.
        let wif = format!("5Hue{}", "K".repeat(47));
        let report = ErrorReport::new(
            "Folder /home/alice/.config is unsafe.",
            format!("Path(\"/home/alice/.config\") owner alice key {key} wif {wif}"),
        );
        let question = ai_question(&report, STARTUP_SITUATION, &environment());
        assert!(
            question.contains("Folder ~/.config is unsafe."),
            "{question}"
        );
        assert!(question.contains("Path(\"~/.config\")"), "{question}");
        for secret in ["/home/alice", "alice", key.as_str(), wif.as_str()] {
            assert!(!question.contains(secret), "{secret} leaked:\n{question}");
        }
        assert!(question.contains("<redacted>"), "{question}");
    }

    /// Overlong details are cut so every assistant URL stays within the limit,
    /// while the message and the links are kept.
    #[test]
    fn question_url_stays_within_the_limit() {
        let report = ErrorReport::new("Short message.", "word ".repeat(4_000));
        let question = ai_question(&report, STARTUP_SITUATION, &environment());
        for assistant in AiAssistant::ALL {
            assert!(assistant.question_url(&question).len() <= MAX_QUESTION_URL_LEN);
        }
        assert!(question.contains("Short message."));
        assert!(question.contains("(truncated)"));
        assert!(question.contains("SKILL.md"));
    }

    /// The URL carries the question percent-encoded after the assistant's prefix.
    #[test]
    fn question_url_round_trips() {
        let question = "Why? & what: 100% \"quoted\"\nnext line";
        let url = AiAssistant::ChatGpt.question_url(question);
        let encoded = url
            .strip_prefix("https://chatgpt.com/?q=")
            .expect("ChatGPT prefix");
        assert_eq!(urlencoding::decode(encoded).unwrap(), question);
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
