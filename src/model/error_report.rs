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
/// Shortest base64 run removed: 32 bytes encoded.
const MIN_REDACTED_BASE64_LEN: usize = 44;
/// Fewest numbers 0..=255 in a list removed: a `Debug` byte array.
const MIN_REDACTED_BYTE_LIST_LEN: usize = 16;
/// Fewest consecutive BIP39 words removed: the shortest recovery phrase.
const MIN_REDACTED_PHRASE_WORDS: usize = 12;
/// Shortest home folder path replaced, so `/` or `C:\` is left alone.
const MIN_REDACTED_HOME_LEN: usize = 3;
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
             Please explain in simple terms what is wrong and give me step-by-step instructions to fix it on my system. \
             The app data folder holds my wallet keys. \
             Do not suggest deleting, resetting or moving the app data folder or any file in it. \
             Never ask me for my recovery phrase, private keys or passwords.\n\n\
             Background you can read:\n\
             - Dash Evo Tool user docs: https://docs.dash.org/en/stable/docs/user/network/dash-evo-tool/\n\
             - Source and issues: https://github.com/dashpay/dash-evo-tool\n\
             - Dash Platform knowledge base: https://github.com/lklimek/claudash/blob/0e3a0ecc77aeb126f3ee9fe89a041b3cc6808cfa/skills/dash-dev/SKILL.md\n",
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

/// `text` with the home folder shown as `~` and the login and profile folder
/// names replaced. Also matches the home folder as `Debug` prints it on
/// Windows (doubled backslashes) and with forward slashes.
pub fn redact_identity(text: &str, env: &QuestionEnvironment) -> String {
    let mut text = text.to_owned();
    let home = env
        .home_dir
        .as_deref()
        .and_then(Path::to_str)
        .map(|home| home.trim_end_matches(['/', '\\']))
        .filter(|home| home.len() >= MIN_REDACTED_HOME_LEN);
    if let Some(home) = home {
        let ignore_case = matches!(env.os, "windows" | "macos");
        if home.contains('\\') {
            text = replace_path(&text, &home.replace('\\', "\\\\"), ignore_case);
            text = replace_path(&text, &home.replace('\\', "/"), ignore_case);
        }
        text = replace_path(&text, home, ignore_case);
    }
    let profile_folder = home.and_then(|home| home.rsplit(['/', '\\']).next());
    for name in [env.user_name.as_deref(), profile_folder]
        .into_iter()
        .flatten()
    {
        if name.len() >= MIN_REDACTED_USER_NAME_LEN {
            text = replace_word(&text, name, "<user>");
        }
    }
    text
}

/// `text` with every occurrence of the home folder `home` shown as `~`, in any
/// letter case when `ignore_case`, as Windows and macOS compare paths.
fn replace_path(text: &str, home: &str, ignore_case: bool) -> String {
    if !ignore_case {
        return text.replace(home, "~");
    }
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        let mut candidate = rest.chars();
        let mut matched_len = 0;
        let matched = home.chars().all(|expected| {
            candidate.next().is_some_and(|found| {
                matched_len += found.len_utf8();
                same(found, expected)
            })
        });
        if matched {
            out.push('~');
            rest = &rest[matched_len..];
        } else {
            out.push(first);
            rest = &rest[first.len_utf8()..];
        }
    }
    out
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

/// `text` with anything that could be a key or seed removed: recovery
/// phrases, `Debug` byte lists, base64 runs, and long hex (also `0x`-prefixed)
/// or Base58 runs.
fn redact_key_material(text: &str) -> String {
    let text = redact_recovery_phrases(text);
    let text = redact_byte_lists(&text);
    let text = redact_runs(&text, is_base64_char, |run| {
        run.len() >= MIN_REDACTED_BASE64_LEN
            && run.chars().any(|c| c.is_ascii_uppercase())
            && run.chars().any(|c| c.is_ascii_lowercase())
    });
    redact_runs(
        &text,
        |c| c.is_ascii_alphanumeric(),
        |run| {
            let digits = run
                .strip_prefix("0x")
                .or_else(|| run.strip_prefix("0X"))
                .unwrap_or(run);
            let hex = digits.len() >= MIN_REDACTED_HEX_LEN
                && digits.chars().all(|c| c.is_ascii_hexdigit());
            let base58 = run.len() >= MIN_REDACTED_BASE58_LEN
                && run.chars().all(|c| !matches!(c, '0' | 'O' | 'I' | 'l'));
            hex || base58
        },
    )
}

fn is_base64_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')
}

/// `text` with every maximal run of `in_run` characters that `secret` accepts
/// replaced by [`REDACTED`].
fn redact_runs(text: &str, in_run: impl Fn(char) -> bool, secret: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = None;
    for (index, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (start, index < text.len() && in_run(c)) {
            (None, true) => start = Some(index),
            (Some(from), false) => {
                let run = &text[from..index];
                out.push_str(if secret(run) { REDACTED } else { run });
                start = None;
            }
            _ => {}
        }
        if start.is_none() && index < text.len() {
            out.push(c);
        }
    }
    out
}

/// `text` with lists of at least [`MIN_REDACTED_BYTE_LIST_LEN`] numbers
/// 0..=255, as `Debug` prints byte arrays, replaced by [`REDACTED`].
fn redact_byte_lists(text: &str) -> String {
    let bytes = text.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let skip_space = |mut at: usize| {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        at
    };
    let mut out = String::with_capacity(text.len());
    let (mut copied, mut at) = (0, 0);
    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() || (at > 0 && is_word(bytes[at - 1])) {
            at += 1;
            continue;
        }
        let (mut count, mut end, mut next) = (0, at, at);
        loop {
            let digits_end = (next..bytes.len())
                .find(|&i| !bytes[i].is_ascii_digit())
                .unwrap_or(bytes.len());
            let is_byte = text[next..digits_end]
                .parse::<u16>()
                .is_ok_and(|value| value <= 255);
            if !is_byte || bytes.get(digits_end).is_some_and(|&b| is_word(b)) {
                break;
            }
            count += 1;
            end = digits_end;
            let comma = skip_space(digits_end);
            if bytes.get(comma) != Some(&b',') {
                break;
            }
            next = skip_space(comma + 1);
        }
        if count >= MIN_REDACTED_BYTE_LIST_LEN {
            out.push_str(&text[copied..at]);
            out.push_str(REDACTED);
            copied = end;
        }
        at = end.max(at + 1);
    }
    out.push_str(&text[copied..]);
    out
}

/// `text` with runs of at least [`MIN_REDACTED_PHRASE_WORDS`] English BIP39
/// words, separated only by spaces, quotes or commas, replaced by [`REDACTED`].
fn redact_recovery_phrases(text: &str) -> String {
    let mut words = Vec::new();
    let mut start = None;
    for (index, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (start, index < text.len() && c.is_ascii_lowercase()) {
            (None, true) => start = Some(index),
            (Some(from), false) => {
                words.push(from..index);
                start = None;
            }
            _ => {}
        }
    }
    let is_bip39 = |word: &std::ops::Range<usize>| {
        bip39::Language::English
            .find_word(&text[word.clone()])
            .is_some()
    };
    let joined = |left: &std::ops::Range<usize>, right: &std::ops::Range<usize>| {
        text[left.end..right.start]
            .chars()
            .all(|c| c.is_whitespace() || matches!(c, '"' | '\'' | ','))
    };
    let mut out = String::with_capacity(text.len());
    let (mut copied, mut first) = (0, 0);
    while first < words.len() {
        let mut last = first;
        if is_bip39(&words[first]) {
            while last + 1 < words.len()
                && is_bip39(&words[last + 1])
                && joined(&words[last], &words[last + 1])
            {
                last += 1;
            }
            if last - first + 1 >= MIN_REDACTED_PHRASE_WORDS {
                out.push_str(&text[copied..words[first].start]);
                out.push_str(REDACTED);
                copied = words[last].end;
            }
        }
        first = last + 1;
    }
    out.push_str(&text[copied..]);
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
            "https://github.com/lklimek/claudash/blob/0e3a0ecc77aeb126f3ee9fe89a041b3cc6808cfa/skills/dash-dev/SKILL.md",
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

    /// The question tells the assistant never to ask for wallet secrets and never
    /// to suggest deleting or resetting the data folder that holds the keys.
    #[test]
    fn question_forbids_asking_for_secrets_and_deleting_wallet_data() {
        let report = ErrorReport::new("Message.", "Details");
        let question = ai_question(&report, STARTUP_SITUATION, &environment());
        for part in [
            "Never ask me for my recovery phrase, private keys or passwords.",
            "Do not suggest deleting, resetting or moving the app data folder or any file in it",
        ] {
            assert!(question.contains(part), "{part} missing:\n{question}");
        }
    }

    /// Key material in other common renderings is removed too: 0x-prefixed hex,
    /// `Debug` byte lists, base64 and BIP39 recovery phrases.
    #[test]
    fn question_redacts_other_key_renderings() {
        use crate::test_support::keys;
        use base64::Engine as _;

        let secret = keys::secret_bytes();
        let prefixed_hex = format!("0x{}", hex::encode(secret));
        let byte_list = format!("{secret:?}");
        let base64 = base64::engine::general_purpose::STANDARD.encode(secret);
        let phrase = keys::mnemonic(12).to_string();
        let report = ErrorReport::new(
            "Message.",
            format!(
                "Err {{ key: {prefixed_hex}, bytes: {byte_list}, b64: \"{base64}\", words: \"{phrase}\" }}"
            ),
        );
        let question = ai_question(&report, STARTUP_SITUATION, &environment());
        for leaked in [
            prefixed_hex.as_str(),
            &hex::encode(secret),
            &byte_list,
            &keys::rendered_forms(&secret)[1],
            &base64,
            &phrase,
        ] {
            assert!(!question.contains(leaked), "{leaked} leaked:\n{question}");
        }
        let last_words = phrase.split(' ').skip(6).collect::<Vec<_>>().join(" ");
        assert!(!question.contains(&last_words), "{question}");
        assert!(question.contains("Err { key: <redacted>"), "{question}");
    }

    /// A short run of ordinary numbers or words is not mistaken for key material.
    #[test]
    fn question_keeps_ordinary_numbers_and_words() {
        let details =
            "SqliteFailure(Error { code: 5, extended: 517 }, [1, 2, 3]) able to open the file";
        let report = ErrorReport::new("Message.", details);
        let question = ai_question(&report, STARTUP_SITUATION, &environment());
        assert!(question.contains(details), "{question}");
    }

    /// A Windows home folder is removed also in its `Debug` form, where each
    /// backslash is doubled, and the profile folder name is removed even when it
    /// differs from the login name.
    #[test]
    fn question_redacts_windows_home_and_profile_folder() {
        let env = QuestionEnvironment {
            home_dir: Some(PathBuf::from(r"C:\Users\alice_000")),
            user_name: Some("alice".to_owned()),
            ..environment()
        };
        let report = ErrorReport::new(
            r"Folder C:\Users\alice_000\AppData is unsafe.",
            r#"Path("C:\\Users\\alice_000\\AppData") C:/Users/alice_000/x profile alice_000"#,
        );
        let question = ai_question(&report, STARTUP_SITUATION, &env);
        assert!(
            question.contains(r"Folder ~\AppData is unsafe."),
            "{question}"
        );
        assert!(question.contains(r#"Path("~\\AppData")"#), "{question}");
        assert!(question.contains("~/x"), "{question}");
        assert!(!question.contains("alice_000"), "{question}");
    }

    /// On Windows and macOS, where paths ignore case, the home folder is found
    /// in any letter case; on Linux only as written.
    #[test]
    fn question_matches_home_folder_case_as_the_system_does() {
        for os in ["windows", "macos"] {
            let env = QuestionEnvironment {
                os,
                home_dir: Some(PathBuf::from(r"C:\Users\Alice_000")),
                user_name: None,
                ..environment()
            };
            let report = ErrorReport::new(
                r"Folder c:\users\alice_000\AppData is unsafe.",
                r#"Path("C:\\USERS\\ALICE_000\\AppData")"#,
            );
            let question = ai_question(&report, STARTUP_SITUATION, &env);
            assert!(
                question.contains(r"Folder ~\AppData is unsafe."),
                "{os}: {question}"
            );
            assert!(
                question.contains(r#"Path("~\\AppData")"#),
                "{os}: {question}"
            );
        }

        let env = QuestionEnvironment {
            user_name: None,
            ..environment()
        };
        let report = ErrorReport::new("Message.", "/HOME/ALICE/x");
        let question = ai_question(&report, STARTUP_SITUATION, &env);
        assert!(question.contains("/HOME/ALICE/x"), "{question}");
    }

    /// A root or very short home folder is left alone instead of turning every
    /// slash into `~`.
    #[test]
    fn question_ignores_a_root_home_folder() {
        for home in ["/", r"C:\"] {
            let env = QuestionEnvironment {
                home_dir: Some(PathBuf::from(home)),
                user_name: None,
                ..environment()
            };
            let report = ErrorReport::new("Message.", "/var/lib/data.db");
            let question = ai_question(&report, STARTUP_SITUATION, &env);
            assert!(question.contains("/var/lib/data.db"), "{home}: {question}");
        }
    }

    /// Text copied for a bug report shows the home folder as `~` and hides the
    /// login name.
    #[test]
    fn redact_identity_hides_home_and_user() {
        let text = redact_identity("/home/alice/.config owner alice", &environment());
        assert_eq!(text, "~/.config owner <user>");
    }

    /// Overlong details are cut so every assistant URL stays within the limit,
    /// while the message and the links are kept.
    #[test]
    fn question_url_stays_within_the_limit() {
        let report = ErrorReport::new("Short message.", "qwx ".repeat(5_000));
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
