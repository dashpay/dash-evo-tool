//! Non-interactive recovery phrase input: `--mnemonic-stdin`.
//!
//! A recovery phrase given as `mnemonic=...` is visible to every local user
//! through `ps` and is saved in shell history. This flag reads it from a pipe
//! instead and delivers it as the tool's `mnemonic` parameter; the tool itself
//! (`src/mcp/tools/`) never learns where it came from.

use std::io::IsTerminal as _;

use platform_wallet_storage::secrets::SecretString;
use serde_json::{Map, Value};

use super::password;

/// Tool parameter the recovery phrase is delivered as.
pub(super) const MNEMONIC_PARAM: &str = "mnemonic";

const STDIN_FLAG: &str = "--mnemonic-stdin";

/// Longest accepted input: far above any BIP-39 phrase, low enough to bound
/// the read.
const MAX_PHRASE_LEN: usize = 1024;

/// Bytes read before giving up: the longest accepted phrase, a CRLF line
/// ending, and one more byte so an over-long input is refused rather than
/// silently truncated.
const READ_LIMIT: usize = MAX_PHRASE_LEN + 3;

/// Shown when the phrase is passed inline; the command still runs.
pub(super) const INLINE_MNEMONIC_WARNING: &str = "Warning: a recovery phrase given as mnemonic=... is visible to other users of this computer and is saved in shell history. Pass it with --mnemonic-stdin instead.";

/// `--mnemonic-stdin` used wrongly on the command line.
#[derive(Debug, thiserror::Error)]
pub(super) enum MnemonicArgError {
    #[error("Give --mnemonic-stdin once.")]
    RepeatedFlag,
    #[error("Choose one recovery phrase source: mnemonic=... or --mnemonic-stdin.")]
    InlineAndStdin,
    #[error(
        "--mnemonic-stdin and --password-stdin cannot both read standard input. Pass the password with --password-file instead."
    )]
    BothFromStdin,
    #[error(
        "The command {tool} does not take a recovery phrase, so --mnemonic-stdin does not apply to it."
    )]
    ToolTakesNoMnemonic { tool: String },
    #[error(
        "The recovery phrase would travel unencrypted to {addr}. Send it only to this computer (127.0.0.1, ::1 or localhost) or to an https address."
    )]
    InsecureDestination { addr: String },
}

/// The recovery phrase could not be read, or was refused. No variant carries
/// any part of the phrase.
#[derive(Debug, thiserror::Error)]
pub(super) enum MnemonicReadError {
    #[error(
        "--mnemonic-stdin reads the recovery phrase from a pipe or a redirected file. On a terminal the phrase would be shown as you type it, so pipe it in."
    )]
    StdinIsTerminal,
    #[error("Could not read the recovery phrase from standard input")]
    ReadStdin {
        #[source]
        source: std::io::Error,
    },
    #[error("The recovery phrase is empty.")]
    Empty,
    #[error("The recovery phrase must be a single line.")]
    MultipleLines,
    #[error("The recovery phrase is longer than {max} bytes.")]
    TooLong { max: usize },
    #[error("The recovery phrase is not valid UTF-8 text.")]
    NotUtf8,
}

/// Removes `--mnemonic-stdin` from `params` — a tool's arguments after its
/// name — and returns whether it was given.
///
/// # Errors
///
/// [`MnemonicArgError::RepeatedFlag`] when it is given more than once.
pub(super) fn take_stdin_flag(params: &mut Vec<String>) -> Result<bool, MnemonicArgError> {
    let before = params.len();
    params.retain(|arg| arg != STDIN_FLAG);
    match before - params.len() {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(MnemonicArgError::RepeatedFlag),
    }
}

/// Checks a `--mnemonic-stdin` call before standard input is read, so a
/// mistyped command never consumes it.
///
/// `takes_mnemonic` is whether the cached schema of the tool declares the
/// parameter, `None` when unknown: the server is then the authority.
/// `http_addr` is the server address in HTTP mode, `None` in-process.
///
/// # Errors
///
/// [`MnemonicArgError`] when the phrase is also inline, the password reads
/// standard input too, the tool takes no phrase, or the phrase would leave
/// this computer unencrypted.
pub(super) fn check_stdin_request(
    tool: &str,
    arguments: &Map<String, Value>,
    password_from_stdin: bool,
    takes_mnemonic: Option<bool>,
    http_addr: Option<&str>,
) -> Result<(), MnemonicArgError> {
    if arguments.contains_key(MNEMONIC_PARAM) {
        return Err(MnemonicArgError::InlineAndStdin);
    }
    if password_from_stdin {
        return Err(MnemonicArgError::BothFromStdin);
    }
    if takes_mnemonic == Some(false) {
        return Err(MnemonicArgError::ToolTakesNoMnemonic {
            tool: tool.to_owned(),
        });
    }
    if let Some(addr) = http_addr
        && !password::is_private_destination(addr)
    {
        return Err(MnemonicArgError::InsecureDestination {
            addr: addr.to_owned(),
        });
    }
    Ok(())
}

/// The warning for a phrase passed inline as `mnemonic=...`, if it was.
pub(super) fn inline_mnemonic_warning(arguments: &Map<String, Value>) -> Option<&'static str> {
    arguments
        .contains_key(MNEMONIC_PARAM)
        .then_some(INLINE_MNEMONIC_WARNING)
}

/// Reads the recovery phrase from standard input, which must not be a
/// terminal.
///
/// # Errors
///
/// [`MnemonicReadError`] when standard input is a terminal or cannot be read,
/// or holds anything but exactly one non-empty line.
pub(super) fn read_mnemonic_stdin() -> Result<SecretString, MnemonicReadError> {
    if std::io::stdin().is_terminal() {
        return Err(MnemonicReadError::StdinIsTerminal);
    }
    let raw = password::raw_stdin()
        .and_then(|stdin| password::read_bounded(stdin, READ_LIMIT))
        .map_err(|source| MnemonicReadError::ReadStdin { source })?;
    parse_phrase(&raw)
}

/// Accepts exactly one line and strips a single trailing `\n` or `\r\n`. The
/// tool normalizes the words.
fn parse_phrase(raw: &[u8]) -> Result<SecretString, MnemonicReadError> {
    let line = match raw.strip_suffix(b"\n") {
        Some(line) => line.strip_suffix(b"\r").unwrap_or(line),
        None => raw,
    };
    if line.len() > MAX_PHRASE_LEN {
        return Err(MnemonicReadError::TooLong {
            max: MAX_PHRASE_LEN,
        });
    }
    if line.iter().any(|byte| matches!(byte, b'\n' | b'\r')) {
        return Err(MnemonicReadError::MultipleLines);
    }
    let text = std::str::from_utf8(line).map_err(|_| MnemonicReadError::NotUtf8)?;
    if text.trim().is_empty() {
        return Err(MnemonicReadError::Empty);
    }
    Ok(SecretString::from(text))
}

/// Adds the recovery phrase to a tool call's arguments.
///
/// The JSON value is an ordinary `String`: the MCP transport serializes it,
/// so that copy lives outside `SecretString`'s guarded memory — the residual
/// every secret tool parameter shares.
pub(super) fn insert_mnemonic(arguments: &mut Map<String, Value>, phrase: &SecretString) {
    arguments.insert(
        MNEMONIC_PARAM.to_owned(),
        Value::String(phrase.expose_secret().to_owned()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a valid BIP-39 phrase: the CLI reads it, the tool validates it.
    const PHRASE: &str = "alpha bravo canary delta echo foxtrot";

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    fn inline(phrase: bool) -> Map<String, Value> {
        let mut arguments = Map::new();
        arguments.insert("network".to_owned(), Value::String("testnet".to_owned()));
        if phrase {
            arguments.insert(MNEMONIC_PARAM.to_owned(), Value::String(PHRASE.to_owned()));
        }
        arguments
    }

    /// Asserts that no rendering of `error` carries the phrase.
    fn assert_no_phrase(error: &dyn std::error::Error) {
        let rendered = format!("{} {error:?} {}", error, password::describe(error));
        assert!(
            !rendered.contains(PHRASE) && !rendered.contains("canary"),
            "an error must never carry the recovery phrase: {rendered}"
        );
    }

    #[test]
    fn the_stdin_flag_is_taken_out_of_the_tool_arguments() {
        let mut params = args(&["network=testnet", "--mnemonic-stdin", "alias=x"]);
        assert!(take_stdin_flag(&mut params).expect("valid flag"));
        assert_eq!(params, args(&["network=testnet", "alias=x"]));

        let mut params = args(&["network=testnet"]);
        assert!(!take_stdin_flag(&mut params).expect("no flag"));
        assert_eq!(params, args(&["network=testnet"]));
    }

    #[test]
    fn the_stdin_flag_given_twice_is_refused() {
        let mut params = args(&["--mnemonic-stdin", "--mnemonic-stdin"]);
        assert!(matches!(
            take_stdin_flag(&mut params),
            Err(MnemonicArgError::RepeatedFlag)
        ));
    }

    #[test]
    fn a_stdin_request_is_checked_before_stdin_is_read() {
        let local = Some("http://127.0.0.1:9527/mcp");
        assert!(
            check_stdin_request(
                "core-wallet-import",
                &inline(false),
                false,
                Some(true),
                None
            )
            .is_ok()
        );
        assert!(
            check_stdin_request("core-wallet-import", &inline(false), false, None, local).is_ok()
        );

        let error =
            check_stdin_request("core-wallet-import", &inline(true), false, Some(true), None)
                .expect_err("inline and stdin");
        assert!(matches!(error, MnemonicArgError::InlineAndStdin));
        assert_no_phrase(&error);

        assert!(matches!(
            check_stdin_request("core-wallet-import", &inline(false), true, Some(true), None),
            Err(MnemonicArgError::BothFromStdin)
        ));
        assert!(matches!(
            check_stdin_request("core-wallets-list", &inline(false), false, Some(false), None),
            Err(MnemonicArgError::ToolTakesNoMnemonic { tool }) if tool == "core-wallets-list"
        ));
        for addr in ["http://192.168.1.5:9527/mcp", "not a url"] {
            assert!(matches!(
                check_stdin_request("core-wallet-import", &inline(false), false, Some(true), Some(addr)),
                Err(MnemonicArgError::InsecureDestination { addr: reported }) if reported == addr
            ));
        }
        assert!(
            check_stdin_request(
                "core-wallet-import",
                &inline(false),
                false,
                Some(true),
                Some("https://det.example.com/mcp")
            )
            .is_ok()
        );
    }

    #[test]
    fn only_an_inline_phrase_gets_the_warning() {
        assert_eq!(inline_mnemonic_warning(&inline(false)), None);
        assert_eq!(
            inline_mnemonic_warning(&inline(true)),
            Some(INLINE_MNEMONIC_WARNING)
        );
    }

    #[test]
    fn exactly_one_line_is_accepted_and_only_its_line_ending_is_stripped() {
        for raw in [
            PHRASE.to_owned(),
            format!("{PHRASE}\n"),
            format!("{PHRASE}\r\n"),
        ] {
            let phrase = parse_phrase(raw.as_bytes()).expect("one line");
            assert_eq!(phrase.expose_secret(), PHRASE, "from {raw:?}");
        }
    }

    #[test]
    fn anything_but_one_non_empty_line_is_refused() {
        for raw in [&b""[..], b"\n", b"\r\n", b"   \n"] {
            assert!(
                matches!(parse_phrase(raw), Err(MnemonicReadError::Empty)),
                "{raw:?}"
            );
        }
        for raw in [format!("{PHRASE}\nsecond"), format!("{PHRASE}\n\n")] {
            let error = parse_phrase(raw.as_bytes()).expect_err("more than one line");
            assert!(matches!(error, MnemonicReadError::MultipleLines));
            assert_no_phrase(&error);
        }
        assert!(matches!(
            parse_phrase(&[0xff, 0xfe]),
            Err(MnemonicReadError::NotUtf8)
        ));
    }

    #[test]
    fn an_over_long_phrase_is_refused_not_truncated() {
        let endless = std::io::Read::take(std::io::repeat(b'a'), 10 * READ_LIMIT as u64);
        let raw = password::read_bounded(endless, READ_LIMIT).expect("bounded read");
        assert_eq!(raw.len(), READ_LIMIT);
        assert!(matches!(
            parse_phrase(&raw),
            Err(MnemonicReadError::TooLong { .. })
        ));
    }

    #[test]
    fn the_phrase_reaches_the_tool_but_never_debug_output() {
        let phrase = parse_phrase(PHRASE.as_bytes()).expect("one line");
        assert!(!format!("{phrase:?}").contains("canary"));

        let mut arguments = inline(false);
        insert_mnemonic(&mut arguments, &phrase);
        assert_eq!(
            arguments.get(MNEMONIC_PARAM).and_then(Value::as_str),
            Some(PHRASE)
        );
    }
}
