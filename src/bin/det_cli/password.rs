//! Non-interactive password input: `--password-stdin` and `--password-file`.
//!
//! A password never travels as a `password=...` argument — `ps` shows every
//! process's arguments to every local user, and shells save them in history —
//! and never through an environment variable, which is readable through
//! `/proc/<pid>/environ`, inherited by every child process and swept into
//! crash reports and CI logs. These flags read it from a pipe or from an
//! owner-only file instead, then deliver it as the tool's `password`
//! parameter. The tool itself (`src/mcp/tools/`) never learns where it came
//! from.

use std::io::{IsTerminal as _, Read};
use std::path::{Path, PathBuf};

use platform_wallet_storage::secrets::{MAX_PASSPHRASE_LEN, SecretString};
use serde_json::{Map, Value};
use zeroize::Zeroizing;

/// Tool parameter the password is delivered as.
pub(super) const PASSWORD_PARAM: &str = "password";

const STDIN_FLAG: &str = "--password-stdin";
const FILE_FLAG: &str = "--password-file";

/// Bytes read before giving up: the longest accepted password, a CRLF line
/// ending, and one more byte so an over-long input is refused rather than
/// silently truncated.
const READ_LIMIT: usize = MAX_PASSPHRASE_LEN + 3;

/// Permission bits that let anyone but the owner read or change the file.
/// OpenSSH refuses a private key with any of them set.
#[cfg(unix)]
const GROUP_OR_OTHER_ACCESS: u32 = 0o077;

/// Where the password comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PasswordSource {
    Stdin,
    File(PathBuf),
}

/// A password flag used wrongly on the command line.
#[derive(Debug, thiserror::Error)]
pub(super) enum PasswordArgError {
    #[error("Choose one password source: --password-stdin or --password-file, given once.")]
    ConflictingSources,
    #[error("--password-file needs the path of the file that holds the password.")]
    MissingFilePath,
    #[error(
        "A password given as password=... is visible to other users of this computer and is saved in shell history. Pass it with --password-stdin or --password-file instead."
    )]
    InlinePassword,
    #[error(
        "The command {tool} does not take a password, so --password-stdin and --password-file do not apply to it."
    )]
    ToolTakesNoPassword { tool: String },
    #[error(
        "The password would travel unencrypted to {addr}. Send it only to this computer (127.0.0.1, ::1 or localhost) or to an https address."
    )]
    InsecureDestination { addr: String },
}

/// The password could not be read, or was refused. No variant carries any
/// part of the password.
#[derive(Debug, thiserror::Error)]
pub(super) enum PasswordReadError {
    #[error(
        "--password-stdin reads the password from a pipe or a redirected file. On a terminal the password would be shown as you type it, so pipe it in or use --password-file."
    )]
    StdinIsTerminal,
    #[error("Could not read the password from standard input")]
    ReadStdin {
        #[source]
        source: std::io::Error,
    },
    #[error("Could not read the password file {}", .path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("The password file {} is a directory, not a file.", .path.display())]
    NotAFile { path: PathBuf },
    #[cfg(unix)]
    #[error(
        "The password file {} can be read or changed by other users (permissions {mode:04o}). Restrict it to your user, for example with: chmod 600 {}",
        .path.display(),
        .path.display()
    )]
    InsecurePermissions { path: PathBuf, mode: u32 },
    #[cfg(not(unix))]
    #[error(
        "--password-file is not available on this platform, because the file's permissions cannot be checked. Use --password-stdin instead."
    )]
    FileUnsupportedOnPlatform,
    #[error("The password is empty.")]
    Empty,
    #[error("The password must be a single line.")]
    MultipleLines,
    #[error("The password is longer than {max} bytes.")]
    TooLong { max: usize },
    #[error("The password is not valid UTF-8 text.")]
    NotUtf8,
}

/// Renders an error with its whole `source` chain, for the one-line
/// `Error: ...` det-cli prints.
pub(super) fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(inner) = cause {
        text.push_str(": ");
        text.push_str(&inner.to_string());
        cause = inner.source();
    }
    text
}

/// Removes the password flags from `params` — a tool's arguments after its
/// name — and returns the source they name, if any.
///
/// # Errors
///
/// [`PasswordArgError::ConflictingSources`] when more than one flag is given,
/// [`PasswordArgError::MissingFilePath`] when `--password-file` has no path.
pub(super) fn take_password_source(
    params: &mut Vec<String>,
) -> Result<Option<PasswordSource>, PasswordArgError> {
    let mut source = None;
    let mut kept = Vec::with_capacity(params.len());
    let mut args = std::mem::take(params).into_iter();
    while let Some(arg) = args.next() {
        let named = if arg == STDIN_FLAG {
            PasswordSource::Stdin
        } else if arg == FILE_FLAG {
            match args.next() {
                Some(path) if !path.is_empty() && !path.starts_with("--") => {
                    PasswordSource::File(PathBuf::from(path))
                }
                _ => return Err(PasswordArgError::MissingFilePath),
            }
        } else if let Some(path) = arg
            .strip_prefix(FILE_FLAG)
            .and_then(|rest| rest.strip_prefix('='))
        {
            if path.is_empty() {
                return Err(PasswordArgError::MissingFilePath);
            }
            PasswordSource::File(PathBuf::from(path))
        } else {
            kept.push(arg);
            continue;
        };
        if source.replace(named).is_some() {
            return Err(PasswordArgError::ConflictingSources);
        }
    }
    *params = kept;
    Ok(source)
}

/// Refuses a password passed as an ordinary `password=...` argument.
///
/// # Errors
///
/// [`PasswordArgError::InlinePassword`] when `arguments` holds a password.
pub(super) fn reject_inline_password(
    arguments: &Map<String, Value>,
) -> Result<(), PasswordArgError> {
    match arguments.contains_key(PASSWORD_PARAM) {
        true => Err(PasswordArgError::InlinePassword),
        false => Ok(()),
    }
}

/// Reads the password from `source`.
///
/// # Errors
///
/// [`PasswordReadError`] when the source cannot be read safely, or holds
/// anything but exactly one non-empty line.
pub(super) fn read_password(source: &PasswordSource) -> Result<SecretString, PasswordReadError> {
    match source {
        PasswordSource::Stdin => {
            if std::io::stdin().is_terminal() {
                return Err(PasswordReadError::StdinIsTerminal);
            }
            let raw = raw_stdin()
                .and_then(read_bounded)
                .map_err(|source| PasswordReadError::ReadStdin { source })?;
            parse_password(&raw)
        }
        PasswordSource::File(path) => read_password_file(path),
    }
}

/// Standard input as an unbuffered `File` over a duplicate of its descriptor.
///
/// `std::io::stdin()` reads through a process-wide 8 KiB buffer that is never
/// wiped, so a password read through it would outlive the zeroizing buffer of
/// [`read_bounded`]. Reading the duplicated descriptor directly leaves that
/// buffer untouched: the only user-space copy is the zeroizing one.
#[cfg(unix)]
fn raw_stdin() -> std::io::Result<std::fs::File> {
    use std::os::fd::AsFd as _;
    Ok(std::fs::File::from(
        std::io::stdin().as_fd().try_clone_to_owned()?,
    ))
}

/// See the Unix variant: the same bypass of std's stdin buffer, over a
/// duplicated handle.
#[cfg(windows)]
fn raw_stdin() -> std::io::Result<std::fs::File> {
    use std::os::windows::io::AsHandle as _;
    Ok(std::fs::File::from(
        std::io::stdin().as_handle().try_clone_to_owned()?,
    ))
}

/// Refuses to send a password over plain HTTP anywhere but this computer.
///
/// Over the HTTP transport the password travels in the request body. `https`
/// protects it on any network; plain `http` only on loopback, where it never
/// leaves the machine.
///
/// # Errors
///
/// [`PasswordArgError::InsecureDestination`] for plain HTTP to a non-loopback
/// host, and for an address that does not parse — an unknown destination is
/// not a safe one.
pub(super) fn ensure_safe_destination(addr: &str) -> Result<(), PasswordArgError> {
    let safe = match reqwest::Url::parse(addr) {
        Ok(url) if url.scheme() == "https" => true,
        Ok(url) if url.scheme() == "http" => url.host_str().is_some_and(is_loopback_host),
        _ => false,
    };
    if safe {
        Ok(())
    } else {
        Err(PasswordArgError::InsecureDestination {
            addr: addr.to_owned(),
        })
    }
}

/// Whether a parsed URL host names this computer.
///
/// The URL parser has already normalized the host: IPv4 forms such as
/// `0x7f.1` read as dotted quads, domains are lowercase, and IPv6 literals
/// keep their brackets.
fn is_loopback_host(host: &str) -> bool {
    let ip_literal = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    match ip_literal.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => host == "localhost",
    }
}

/// Adds the password to a tool call's arguments.
///
/// The JSON value is an ordinary `String`: the MCP transport serializes it,
/// so that copy lives outside `SecretString`'s guarded memory — the residual
/// every secret tool parameter shares.
pub(super) fn insert_password(arguments: &mut Map<String, Value>, password: &SecretString) {
    arguments.insert(
        PASSWORD_PARAM.to_owned(),
        Value::String(password.expose_secret().to_owned()),
    );
}

#[cfg(unix)]
fn read_password_file(path: &Path) -> Result<SecretString, PasswordReadError> {
    use std::os::unix::fs::PermissionsExt as _;

    let read_error = |source: std::io::Error| PasswordReadError::ReadFile {
        path: path.to_path_buf(),
        source,
    };
    let file = std::fs::File::open(path).map_err(read_error)?;
    // Checked on the opened descriptor rather than the path, so the file
    // cannot be swapped between the check and the read.
    let metadata = file.metadata().map_err(read_error)?;
    if metadata.is_dir() {
        return Err(PasswordReadError::NotAFile {
            path: path.to_path_buf(),
        });
    }
    let mode = metadata.permissions().mode();
    if mode & GROUP_OR_OTHER_ACCESS != 0 {
        return Err(PasswordReadError::InsecurePermissions {
            path: path.to_path_buf(),
            mode: mode & 0o7777,
        });
    }
    let raw = read_bounded(file).map_err(read_error)?;
    parse_password(&raw)
}

#[cfg(not(unix))]
fn read_password_file(_path: &Path) -> Result<SecretString, PasswordReadError> {
    // TODO(windows): accept the file once its ACL is verified to grant access
    // to the current user only, as OpenSSH for Windows does for private keys.
    // Until then a file other users can read cannot be told apart, so refuse.
    Err(PasswordReadError::FileUnsupportedOnPlatform)
}

/// Reads at most [`READ_LIMIT`] bytes into a zeroizing buffer allocated once
/// up front, so no reallocation leaves an unwiped copy of the password behind.
fn read_bounded(mut reader: impl Read) -> std::io::Result<Zeroizing<Vec<u8>>> {
    let mut buffer = Zeroizing::new(vec![0u8; READ_LIMIT]);
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    buffer.truncate(filled);
    Ok(buffer)
}

/// Accepts exactly one line: strips a single trailing `\n` or `\r\n`, keeps
/// every other byte (spaces included), and refuses anything that would make
/// the password ambiguous.
fn parse_password(raw: &[u8]) -> Result<SecretString, PasswordReadError> {
    let line = match raw.strip_suffix(b"\n") {
        Some(line) => line.strip_suffix(b"\r").unwrap_or(line),
        None => raw,
    };
    if line.is_empty() {
        return Err(PasswordReadError::Empty);
    }
    if line.len() > MAX_PASSPHRASE_LEN {
        return Err(PasswordReadError::TooLong {
            max: MAX_PASSPHRASE_LEN,
        });
    }
    if line.iter().any(|byte| matches!(byte, b'\n' | b'\r')) {
        return Err(PasswordReadError::MultipleLines);
    }
    let text = std::str::from_utf8(line).map_err(|_| PasswordReadError::NotUtf8)?;
    Ok(SecretString::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct horse canary staple";

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    /// Asserts that neither rendering of `error` carries the password.
    fn assert_no_password(error: &dyn std::error::Error) {
        let rendered = format!("{} {error:?} {}", error, describe(error));
        assert!(
            !rendered.contains(PASSWORD) && !rendered.contains("canary"),
            "an error must never carry the password: {rendered}"
        );
    }

    #[test]
    fn password_flags_are_taken_out_of_the_tool_arguments() {
        let mut params = args(&["network=testnet", "--password-stdin", "alias=x"]);
        let source = take_password_source(&mut params).expect("valid flags");
        assert_eq!(source, Some(PasswordSource::Stdin));
        assert_eq!(params, args(&["network=testnet", "alias=x"]));

        let mut params = args(&["--password-file", "/run/pw", "network=testnet"]);
        let source = take_password_source(&mut params).expect("valid flags");
        assert_eq!(source, Some(PasswordSource::File("/run/pw".into())));
        assert_eq!(params, args(&["network=testnet"]));

        let mut params = args(&["--password-file=/run/pw"]);
        let source = take_password_source(&mut params).expect("valid flags");
        assert_eq!(source, Some(PasswordSource::File("/run/pw".into())));
        assert!(params.is_empty());

        let mut params = args(&["network=testnet"]);
        assert_eq!(take_password_source(&mut params).expect("no flags"), None);
        assert_eq!(params, args(&["network=testnet"]));
    }

    #[test]
    fn more_than_one_password_source_is_refused() {
        for list in [
            &["--password-stdin", "--password-file", "/run/pw"][..],
            &["--password-file=/a", "--password-file=/b"][..],
            &["--password-stdin", "--password-stdin"][..],
        ] {
            let mut params = args(list);
            assert!(
                matches!(
                    take_password_source(&mut params),
                    Err(PasswordArgError::ConflictingSources)
                ),
                "{list:?} must be refused"
            );
        }
    }

    #[test]
    fn a_password_file_flag_without_a_path_is_refused() {
        for list in [
            &["--password-file"][..],
            &["--password-file", "--password-stdin"][..],
            &["--password-file="][..],
            &["--password-file", ""][..],
        ] {
            let mut params = args(list);
            assert!(
                matches!(
                    take_password_source(&mut params),
                    Err(PasswordArgError::MissingFilePath)
                ),
                "{list:?} must be refused"
            );
        }
    }

    #[test]
    fn a_password_on_the_command_line_is_refused() {
        let mut arguments = Map::new();
        arguments.insert("network".to_owned(), Value::String("testnet".to_owned()));
        assert!(reject_inline_password(&arguments).is_ok());

        arguments.insert(
            PASSWORD_PARAM.to_owned(),
            Value::String(PASSWORD.to_owned()),
        );
        let error = reject_inline_password(&arguments).expect_err("inline password");
        assert!(matches!(error, PasswordArgError::InlinePassword));
        assert_no_password(&error);
    }

    #[test]
    fn exactly_one_line_is_accepted_and_only_its_line_ending_is_stripped() {
        for raw in [
            PASSWORD.to_owned(),
            format!("{PASSWORD}\n"),
            format!("{PASSWORD}\r\n"),
        ] {
            let secret = parse_password(raw.as_bytes()).expect("one line");
            assert_eq!(secret.expose_secret(), PASSWORD, "from {raw:?}");
        }
        let padded = parse_password(b" padded \n").expect("spaces are password bytes");
        assert_eq!(padded.expose_secret(), " padded ");
    }

    #[test]
    fn anything_but_one_non_empty_line_is_refused() {
        assert!(matches!(parse_password(b""), Err(PasswordReadError::Empty)));
        assert!(matches!(
            parse_password(b"\n"),
            Err(PasswordReadError::Empty)
        ));
        assert!(matches!(
            parse_password(b"\r\n"),
            Err(PasswordReadError::Empty)
        ));
        for raw in [
            format!("{PASSWORD}\nsecond line"),
            format!("{PASSWORD}\n\n"),
            format!("{PASSWORD}\rrest"),
        ] {
            let error = parse_password(raw.as_bytes()).expect_err("more than one line");
            assert!(matches!(error, PasswordReadError::MultipleLines), "{raw:?}");
            assert_no_password(&error);
        }
        assert!(matches!(
            parse_password(&[0xff, 0xfe]),
            Err(PasswordReadError::NotUtf8)
        ));
    }

    #[test]
    fn an_over_long_password_is_refused_not_truncated() {
        let longest = "a".repeat(MAX_PASSPHRASE_LEN);
        assert!(parse_password(longest.as_bytes()).is_ok());

        let too_long = "a".repeat(MAX_PASSPHRASE_LEN + 1);
        assert!(matches!(
            parse_password(too_long.as_bytes()),
            Err(PasswordReadError::TooLong { .. })
        ));

        // A reader stops at the limit, and what it returns is still refused.
        let endless = std::io::repeat(b'a');
        let raw = read_bounded(endless.take(10 * READ_LIMIT as u64)).expect("bounded read");
        assert_eq!(raw.len(), READ_LIMIT);
        assert!(matches!(
            parse_password(&raw),
            Err(PasswordReadError::TooLong { .. })
        ));
    }

    #[test]
    fn a_pipe_is_read_to_its_single_line() {
        let raw = read_bounded(std::io::Cursor::new(format!("{PASSWORD}\n"))).expect("read");
        let secret = parse_password(&raw).expect("one line");
        assert_eq!(secret.expose_secret(), PASSWORD);
    }

    #[test]
    fn the_password_is_never_in_debug_output() {
        let secret = parse_password(PASSWORD.as_bytes()).expect("one line");
        let debug = format!("{secret:?}");
        assert!(!debug.contains(PASSWORD), "Debug must redact: {debug}");

        let mut arguments = Map::new();
        insert_password(&mut arguments, &secret);
        assert_eq!(
            arguments.get(PASSWORD_PARAM).and_then(Value::as_str),
            Some(PASSWORD),
            "the tool receives the password under its parameter name"
        );
    }

    #[cfg(unix)]
    fn password_file(mode: u32) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("wallet-password");
        std::fs::write(&path, format!("{PASSWORD}\n")).expect("write password file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .expect("set permissions");
        (dir, path)
    }

    #[cfg(unix)]
    #[test]
    fn an_owner_only_password_file_is_read() {
        for mode in [0o600, 0o400] {
            let (_dir, path) = password_file(mode);
            let secret = read_password(&PasswordSource::File(path)).expect("owner-only file");
            assert_eq!(secret.expose_secret(), PASSWORD, "mode {mode:o}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_password_file_other_users_can_reach_is_refused() {
        for mode in [0o644, 0o640, 0o604, 0o620, 0o602, 0o660, 0o666] {
            let (_dir, path) = password_file(mode);
            let error = read_password(&PasswordSource::File(path.clone()))
                .expect_err("group- or world-accessible file");
            match &error {
                PasswordReadError::InsecurePermissions {
                    path: reported,
                    mode: reported_mode,
                } => {
                    assert_eq!(reported, &path);
                    assert_eq!(*reported_mode, mode);
                }
                other => panic!("mode {mode:o}: expected InsecurePermissions, got {other:?}"),
            }
            assert!(
                error.to_string().contains("chmod 600"),
                "the refusal names the fix: {error}"
            );
            assert_no_password(&error);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_file_or_a_directory_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("absent");
        let error = read_password(&PasswordSource::File(missing)).expect_err("missing file");
        assert!(matches!(error, PasswordReadError::ReadFile { .. }));
        assert!(
            describe(&error).contains("absent:"),
            "the cause is rendered after the path: {}",
            describe(&error)
        );

        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only directory");
        let error = read_password(&PasswordSource::File(dir.path().to_path_buf()))
            .expect_err("a directory");
        assert!(
            matches!(error, PasswordReadError::NotAFile { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_password_goes_over_http_only_to_this_computer() {
        for addr in [
            "http://127.0.0.1:9527/mcp",
            "http://127.8.0.1:9527/mcp",
            "http://localhost:9527/mcp",
            "http://LocalHost:9527/mcp",
            "http://[::1]:9527/mcp",
            "http://0x7f.1:9527/mcp",
        ] {
            assert!(
                ensure_safe_destination(addr).is_ok(),
                "{addr} is this computer"
            );
        }
    }

    #[test]
    fn a_password_goes_anywhere_over_https() {
        for addr in [
            "https://det.example.com/mcp",
            "https://192.168.1.5:9527/mcp",
        ] {
            assert!(ensure_safe_destination(addr).is_ok(), "{addr} is encrypted");
        }
    }

    #[test]
    fn a_password_never_goes_unencrypted_off_this_computer() {
        for addr in [
            "http://192.168.1.5:9527/mcp",
            "http://det.example.com/mcp",
            "http://localhost.example.com/mcp",
            "http://127.0.0.1.nip.io/mcp",
            "http://0.0.0.0:9527/mcp",
            "http://[::]:9527/mcp",
            "http://[::ffff:127.0.0.1]:9527/mcp",
            "ws://127.0.0.1:9527/mcp",
            "127.0.0.1:9527/mcp",
            "not a url",
        ] {
            match ensure_safe_destination(addr) {
                Err(PasswordArgError::InsecureDestination { addr: reported }) => {
                    assert_eq!(reported, addr);
                }
                other => panic!("{addr}: expected InsecureDestination, got {other:?}"),
            }
        }
    }
}
