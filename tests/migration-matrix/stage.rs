//! Unpacks a fixture into a throwaway data directory the harness may mutate.
//!
//! A fixture is either an already-unpacked directory or an archive; both are
//! copied into a temp dir so the source stays pristine across the two boots.
//! The staged tree is then held to the same rules `app_dir::ensure_data_dir_exists`
//! enforces on a real data directory — `0700`, no symlinks — so a fixture that
//! smuggles in a symlink fails loudly here instead of silently redirecting a
//! boot write outside the sandbox.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::manifest::{Fixture, IN_REPO_ROOT};

/// Archive extensions tried when the manifest carries no explicit pointer.
const ARCHIVE_SUFFIXES: &[&str] = &[".tar.zst", ".tar.gz", ".tgz", ".tar.xz", ".tar"];

/// Files that identify an unpacked tree as a DET data directory. `data.db` is
/// the v0.9.3-era marker, `det-app.sqlite` the modern one, `.env` common to both.
const DATA_DIR_MARKERS: &[&str] = &["data.db", "det-app.sqlite", ".env"];

/// A fixture unpacked into a temp sandbox.
///
/// The sandbox holds the data directory plus the XDG dirs the spawned
/// `det-cli` is pointed at, so nothing the subprocess writes escapes into the
/// real user home. Dropping it deletes the whole tree.
#[derive(Debug)]
pub struct StagedFixture {
    sandbox: TempDir,
    data_dir: PathBuf,
}

impl StagedFixture {
    /// The staged data directory, i.e. what `DASH_EVO_DATA_DIR` points at.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Root of the sandbox — parent of the data dir and of the redirected
    /// XDG dirs.
    pub fn sandbox(&self) -> &Path {
        self.sandbox.path()
    }

    /// Writes `password` as a one-line, owner-only (`0600`) file in the
    /// sandbox — the only kind of file det-cli's `--password-file` accepts.
    pub fn write_password_file(&self, password: &str) -> Result<PathBuf, String> {
        use std::io::Write as _;

        let path = self.sandbox.path().join("wallet-password");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|e| format!("could not create {}: {e}", path.display()))?;
        writeln!(file, "{password}")
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        Ok(path)
    }

    /// A scratch directory for harness-side copies (SQLite snapshots), kept
    /// outside the data dir so reading never perturbs what is under test.
    pub fn scratch(&self) -> Result<PathBuf, String> {
        let scratch = self.sandbox.path().join("scratch");
        fs::create_dir_all(&scratch)
            .map_err(|e| format!("could not create scratch dir {}: {e}", scratch.display()))?;
        Ok(scratch)
    }
}

/// Stages `fixture` from the repository or `fixtures_dir` into a fresh sandbox.
pub fn stage(fixtures_dir: &Path, fixture: &Fixture) -> Result<StagedFixture, String> {
    stage_from(fixtures_dir, Path::new(IN_REPO_ROOT), fixture)
}

/// [`stage`] with in-repo archive paths resolved below `in_repo_root`.
fn stage_from(
    fixtures_dir: &Path,
    in_repo_root: &Path,
    fixture: &Fixture,
) -> Result<StagedFixture, String> {
    let source = resolve_source(fixtures_dir, in_repo_root, fixture)?;
    let sandbox = tempfile::Builder::new()
        .prefix("det-migration-matrix-")
        .tempdir()
        .map_err(|e| format!("could not create sandbox for fixture '{}': {e}", fixture.id))?;
    let unpacked = sandbox.path().join("unpacked");
    fs::create_dir_all(&unpacked)
        .map_err(|e| format!("could not create {}: {e}", unpacked.display()))?;

    match source {
        Source::Directory(dir) => copy_tree(&dir, &unpacked)?,
        Source::Archive(archive) => {
            verify_checksum(&archive, fixture)?;
            extract(&archive, &unpacked)?;
        }
    }

    let data_dir = locate_data_dir(&unpacked, &fixture.id)?;
    harden(&data_dir)?;
    restrict_ancestors(sandbox.path(), &data_dir)?;

    Ok(StagedFixture { sandbox, data_dir })
}

/// Makes every directory from the sandbox root down to (excluding) the data
/// dir owner-only. [`harden`] owns the data dir and below; the directories
/// above it were created with the process umask or an archive's modes, and
/// the wallet store refuses a data dir under a group- or other-writable
/// ancestor (`insecure_parent_dir`).
fn restrict_ancestors(sandbox: &Path, data_dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for dir in data_dir
            .ancestors()
            .skip(1)
            .take_while(|dir| dir.starts_with(sandbox))
        {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("could not restrict {}: {e}", dir.display()))?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (sandbox, data_dir);
    }
    Ok(())
}

enum Source {
    Directory(PathBuf),
    Archive(PathBuf),
}

/// Resolves the fixture bytes: the in-repo archive, else the downloaded one
/// the manifest names, else a directory or archive named after the fixture id.
fn resolve_source(
    fixtures_dir: &Path,
    in_repo_root: &Path,
    fixture: &Fixture,
) -> Result<Source, String> {
    if let Some(archive) = &fixture.archive {
        let path = archive
            .locate(in_repo_root)
            .map_err(|e| format!("fixture '{}': {e}", fixture.id))?;
        return match path.is_file() {
            true => Ok(Source::Archive(path)),
            false => Err(format!(
                "fixture '{}' names in-repo archive {}, which is not a file",
                fixture.id,
                path.display()
            )),
        };
    }
    if let Some(name) = fixture.artifact.archive.as_deref() {
        let path = fixtures_dir.join(name);
        return match path.metadata() {
            Ok(meta) if meta.is_dir() => Ok(Source::Directory(path)),
            Ok(_) => Ok(Source::Archive(path)),
            Err(e) => Err(format!(
                "fixture '{}' points at {} which cannot be read: {e}{}",
                fixture.id,
                path.display(),
                describe_dir(fixtures_dir)
            )),
        };
    }

    let unpacked = fixtures_dir.join(&fixture.id);
    if unpacked.is_dir() {
        return Ok(Source::Directory(unpacked));
    }
    for suffix in ARCHIVE_SUFFIXES {
        let candidate = fixtures_dir.join(format!("{}{suffix}", fixture.id));
        if candidate.is_file() {
            return Ok(Source::Archive(candidate));
        }
    }
    Err(format!(
        "fixture '{}' has no archive pointer and neither {}/ nor {}[{}] exists{}",
        fixture.id,
        fixture.id,
        fixture.id,
        ARCHIVE_SUFFIXES.join("|"),
        describe_dir(fixtures_dir)
    ))
}

/// Lists what the fixtures directory actually holds, so a pointer typo or an
/// expired artifact download names the mismatch instead of just "not found".
fn describe_dir(dir: &Path) -> String {
    match fs::read_dir(dir) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            format!(" — {} contains: {}", dir.display(), names.join(", "))
        }
        Err(e) => format!(" — {} is unreadable: {e}", dir.display()),
    }
}

fn verify_checksum(archive: &Path, fixture: &Fixture) -> Result<(), String> {
    let (expected, size) = match &fixture.archive {
        Some(in_repo) => (Some(in_repo.sha256.as_str()), Some(in_repo.bytes)),
        None => (fixture.artifact.sha256.as_deref(), None),
    };
    let Some(expected) = expected else {
        return Ok(());
    };
    let bytes = fs::read(archive)
        .map_err(|e| format!("could not read fixture archive {}: {e}", archive.display()))?;
    if let Some(size) = size
        && bytes.len() as u64 != size
    {
        return Err(format!(
            "fixture '{}' archive {} has {} bytes, manifest expects {size}",
            fixture.id,
            archive.display(),
            bytes.len()
        ));
    }
    let actual = hex::encode(Sha256::digest(&bytes));
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(format!(
            "fixture '{}' archive {} has sha256 {actual}, manifest expects {expected}",
            fixture.id,
            archive.display()
        ));
    }
    Ok(())
}

/// Extracts with the system `tar`, which auto-detects the compression. Keeps
/// the harness free of archive crates — adding them is a dependency decision
/// the fixture-capture workstream owns, not this test target.
fn extract(archive: &Path, dest: &Path) -> Result<(), String> {
    let output = Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .output()
        .map_err(|e| format!("could not run tar to unpack {}: {e}", archive.display()))?;
    if !output.status.success() {
        return Err(format!(
            "tar failed to unpack {} ({}): {}",
            archive.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Copies an unpacked fixture, rejecting symlinks at the source so a staged
/// tree can never contain one.
fn copy_tree(source: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("could not create {}: {e}", dest.display()))?;
    let entries = fs::read_dir(source)
        .map_err(|e| format!("could not read fixture dir {}: {e}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not walk {}: {e}", source.display()))?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let meta = fs::symlink_metadata(&from)
            .map_err(|e| format!("could not stat {}: {e}", from.display()))?;
        if meta.file_type().is_symlink() {
            return Err(symlink_rejection(&from));
        }
        if meta.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|e| {
                format!("could not copy {} to {}: {e}", from.display(), to.display())
            })?;
        }
    }
    Ok(())
}

/// Finds the data directory inside an unpacked tree, descending through
/// single-child wrapper directories the archive may have kept.
fn locate_data_dir(root: &Path, fixture_id: &str) -> Result<PathBuf, String> {
    let mut candidate = root.to_path_buf();
    for _ in 0..4 {
        if DATA_DIR_MARKERS
            .iter()
            .any(|marker| candidate.join(marker).exists())
        {
            return Ok(candidate);
        }
        let mut children: Vec<PathBuf> = fs::read_dir(&candidate)
            .map_err(|e| format!("could not read {}: {e}", candidate.display()))?
            .flatten()
            .map(|entry| entry.path())
            .collect();
        children.retain(|path| path.is_dir());
        match children.len() {
            1 => candidate = children.remove(0),
            _ => break,
        }
    }
    Err(format!(
        "fixture '{fixture_id}' does not look like a DET data directory: none of {} found under {}",
        DATA_DIR_MARKERS.join(", "),
        root.display()
    ))
}

/// Applies the data-directory rules from `app_dir::ensure_data_dir_exists`:
/// owner-only permissions and no symlinks anywhere in the tree.
fn harden(data_dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let meta = fs::symlink_metadata(data_dir)
            .map_err(|e| format!("could not stat {}: {e}", data_dir.display()))?;
        if meta.file_type().is_symlink() {
            return Err(symlink_rejection(data_dir));
        }
        if !meta.is_dir() {
            return Err(format!("{} is not a directory", data_dir.display()));
        }
        fs::set_permissions(data_dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("could not restrict {}: {e}", data_dir.display()))?;

        let entries = fs::read_dir(data_dir)
            .map_err(|e| format!("could not read {}: {e}", data_dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("could not walk {}: {e}", data_dir.display()))?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| format!("could not stat {}: {e}", path.display()))?;
            if meta.file_type().is_symlink() {
                return Err(symlink_rejection(&path));
            }
            if meta.is_dir() {
                harden(&path)?;
            } else {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                    .map_err(|e| format!("could not restrict {}: {e}", path.display()))?;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = data_dir;
    }
    Ok(())
}

fn symlink_rejection(path: &Path) -> String {
    format!(
        "fixture contains a symbolic link at {} — a staged data directory must not contain symlinks",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(id: &str, archive: Option<&str>) -> Fixture {
        let archive = match archive {
            Some(name) => format!(r#", "artifact": {{ "archive": "{name}" }}"#),
            None => String::new(),
        };
        serde_json::from_str(&format!(
            r#"{{ "id": "{id}", "network": "testnet"{archive} }}"#
        ))
        .expect("test fixture json")
    }

    #[test]
    fn an_unpacked_directory_is_staged_and_locked_down() {
        use std::os::unix::fs::PermissionsExt;

        let fixtures = tempfile::tempdir().expect("fixtures dir");
        let source = fixtures.path().join("v0.9.3-testnet");
        fs::create_dir_all(source.join("dash_core_configs")).expect("create fixture");
        fs::write(source.join("data.db"), b"not really sqlite").expect("write data.db");
        fs::set_permissions(source.join("data.db"), fs::Permissions::from_mode(0o644))
            .expect("loosen fixture permissions");

        let staged = stage(fixtures.path(), &fixture("v0.9.3-testnet", None)).expect("stage");

        assert!(staged.data_dir().join("data.db").is_file());
        assert!(
            staged.data_dir().starts_with(staged.sandbox()),
            "the staged copy must live inside the sandbox, not in the fixtures dir"
        );
        let dir_mode = fs::metadata(staged.data_dir())
            .expect("stat staged dir")
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o077, 0, "staged data dir must be owner-only");
        let file_mode = fs::metadata(staged.data_dir().join("data.db"))
            .expect("stat staged file")
            .permissions()
            .mode();
        assert_eq!(file_mode & 0o077, 0, "staged files must be owner-only");
    }

    /// The wallet store refuses a data dir under a group- or other-writable
    /// ancestor (`insecure_parent_dir`), so every directory the stager creates
    /// above the data dir must be owner-only too, whatever the umask.
    #[test]
    fn the_password_file_is_owner_only_and_holds_one_line() {
        let staged = StagedFixture {
            sandbox: tempfile::tempdir().expect("sandbox"),
            data_dir: PathBuf::new(),
        };
        let path = staged
            .write_password_file("one line")
            .expect("password file");
        assert_eq!(fs::read_to_string(&path).expect("read"), "one line\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "det-cli refuses anything wider");
        }
    }

    #[test]
    fn every_directory_above_the_data_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let fixtures = tempfile::tempdir().expect("fixtures dir");
        let nested = fixtures.path().join("wrapped").join("Dash-Evo-Tool");
        fs::create_dir_all(&nested).expect("create fixture");
        fs::write(nested.join("det-app.sqlite"), b"x").expect("write marker");

        let staged = stage(fixtures.path(), &fixture("wrapped", None)).expect("stage");

        let ancestors: Vec<&Path> = staged
            .data_dir()
            .ancestors()
            .skip(1)
            .take_while(|dir| dir.starts_with(staged.sandbox()))
            .collect();
        assert!(
            ancestors.len() >= 2,
            "expected the sandbox root and at least one staging dir above the data dir, got {ancestors:?}"
        );
        for dir in ancestors {
            let mode = fs::metadata(dir)
                .expect("stat ancestor")
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o077,
                0,
                "{} is mode {:o}; the data dir's ancestors must be owner-only",
                dir.display(),
                mode & 0o777
            );
        }
    }

    #[test]
    fn a_wrapper_directory_is_descended_into() {
        let fixtures = tempfile::tempdir().expect("fixtures dir");
        let nested = fixtures.path().join("wrapped").join("Dash-Evo-Tool");
        fs::create_dir_all(&nested).expect("create fixture");
        fs::write(nested.join("det-app.sqlite"), b"x").expect("write marker");

        let staged = stage(fixtures.path(), &fixture("wrapped", None)).expect("stage");

        assert!(staged.data_dir().join("det-app.sqlite").is_file());
        assert_eq!(
            staged.data_dir().file_name().and_then(|n| n.to_str()),
            Some("Dash-Evo-Tool")
        );
    }

    #[test]
    fn a_symlinked_fixture_entry_is_rejected() {
        use std::os::unix::fs::symlink;

        let fixtures = tempfile::tempdir().expect("fixtures dir");
        let source = fixtures.path().join("linky");
        fs::create_dir_all(&source).expect("create fixture");
        fs::write(source.join("data.db"), b"x").expect("write marker");
        symlink("/etc/passwd", source.join("escape")).expect("create symlink");

        let error = stage(fixtures.path(), &fixture("linky", None))
            .expect_err("a symlinked entry must be rejected");

        assert!(error.contains("symbolic link"), "{error}");
    }

    #[test]
    fn a_tree_without_markers_is_rejected() {
        let fixtures = tempfile::tempdir().expect("fixtures dir");
        fs::create_dir_all(fixtures.path().join("empty")).expect("create fixture");

        let error = stage(fixtures.path(), &fixture("empty", None))
            .expect_err("a non-DET tree must be rejected");

        assert!(
            error.contains("does not look like a DET data directory"),
            "{error}"
        );
    }

    #[test]
    fn a_missing_archive_lists_what_the_fixtures_dir_holds() {
        let fixtures = tempfile::tempdir().expect("fixtures dir");
        fs::write(fixtures.path().join("something-else.tar"), b"x").expect("write decoy");

        let error = stage(fixtures.path(), &fixture("gone", Some("gone.tar.zst")))
            .expect_err("a missing archive must fail loudly");

        assert!(
            error.contains("gone.tar.zst") && error.contains("something-else.tar"),
            "{error}"
        );
    }

    #[test]
    fn a_checksum_mismatch_is_rejected() {
        let fixtures = tempfile::tempdir().expect("fixtures dir");
        fs::write(fixtures.path().join("tampered.tar"), b"x").expect("write archive");
        let fixture: Fixture = serde_json::from_str(
            r#"{ "id": "tampered", "network": "testnet",
                 "artifact": { "archive": "tampered.tar", "sha256": "00" } }"#,
        )
        .expect("fixture json");

        let error = stage(fixtures.path(), &fixture).expect_err("a bad checksum must be rejected");

        assert!(error.contains("sha256"), "{error}");
    }

    /// Packs a minimal data directory below `root` and returns the manifest
    /// entry naming it as an in-repo archive with `bytes` and `sha256` as
    /// given, or the archive's own when `None`.
    fn in_repo_fixture(root: &Path, bytes: Option<u64>, sha256: Option<&str>) -> Fixture {
        let profile = tempfile::tempdir().expect("profile dir");
        fs::write(profile.path().join(".env"), b"MCP_API_KEY=\n").expect("write .env");
        fs::create_dir_all(root.join("archives")).expect("archives dir");
        let archive = root.join("archives/profile.tar");
        let status = Command::new("tar")
            .arg("-cf")
            .arg(&archive)
            .arg("-C")
            .arg(profile.path())
            .arg(".")
            .status()
            .expect("tar runs");
        assert!(status.success(), "tar packs the profile");
        let content = fs::read(&archive).expect("read archive");
        let digest = hex::encode(Sha256::digest(&content));
        serde_json::from_value(serde_json::json!({
            "id": "in-repo", "network": "testnet",
            "archive": {
                "path": "archives/profile.tar",
                "sha256": sha256.unwrap_or(&digest),
                "bytes": bytes.unwrap_or(content.len() as u64),
            },
            "artifact": { "archive": "downloaded.tar" },
        }))
        .expect("fixture json")
    }

    #[test]
    fn an_in_repo_archive_is_staged_from_the_repository_not_the_fixtures_dir() {
        let repo = tempfile::tempdir().expect("repo dir");
        let fixtures = tempfile::tempdir().expect("fixtures dir");
        let fixture = in_repo_fixture(repo.path(), None, None);

        let staged = stage_from(fixtures.path(), repo.path(), &fixture).expect("stage");

        assert!(staged.data_dir().join(".env").is_file());
    }

    #[test]
    fn an_in_repo_archive_of_another_size_or_digest_is_rejected() {
        let repo = tempfile::tempdir().expect("repo dir");
        let fixtures = tempfile::tempdir().expect("fixtures dir");
        for (fixture, field) in [
            (in_repo_fixture(repo.path(), Some(1), None), "bytes"),
            (in_repo_fixture(repo.path(), None, Some("00")), "sha256"),
        ] {
            let error = stage_from(fixtures.path(), repo.path(), &fixture)
                .expect_err("a mismatched archive must be rejected");
            assert!(error.contains(field), "{error}");
        }
    }

    #[test]
    fn a_missing_in_repo_archive_fails_without_falling_back() {
        let packed_in = tempfile::tempdir().expect("repo the archive is packed in");
        let repo = tempfile::tempdir().expect("repo without the archive");
        let fixtures = tempfile::tempdir().expect("fixtures dir");
        let fixture = in_repo_fixture(packed_in.path(), None, None);
        fs::write(fixtures.path().join("downloaded.tar"), b"x").expect("write decoy");

        let error = stage_from(fixtures.path(), repo.path(), &fixture)
            .expect_err("a missing in-repo archive must fail");

        assert!(error.contains("archives/profile.tar"), "{error}");
    }
}
