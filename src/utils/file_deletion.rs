//! The single chokepoint for deleting files from disk.
//!
//! Every production file deletion goes through [`delete_file`] (or
//! [`delete_tree`], which unlinks each file through [`delete_file`]). A deletion
//! states its purpose as a [`DeletionIntent`], and the chokepoint enforces, in
//! this order, before anything is unlinked:
//!
//! 1. the target is a regular file — never a symlink or directory;
//! 2. a hard deny-list of live application files by **name** — `det-app.sqlite`,
//!    `data.db`, every network's wallet and shielded database, `*.pwsvault`,
//!    `.env`, `*.premigration`, each with its `-wal`/`-shm`/`-journal` sidecar.
//!    It depends on no caller-supplied path, so a caller's matcher bug or a
//!    wrong data directory can never reach these files;
//! 3. the same deny-list by **identity** — the canonical location, and on Unix
//!    the device/inode, of every protected file in the intent's data directory,
//!    which catches `..`, symlinked-directory and case aliases;
//! 4. on Unix, a single hard link, so no alias of another file is removed;
//! 5. the intent's scope — the directories that purpose may delete from.
//!
//! The chokepoint needs no `AppContext`: each intent carries the paths it is
//! confined to, so it also serves pre-context callers such as log rotation.
//!
//! A check and the unlink that follows it are not atomic; a concurrent rename in
//! the window is out of scope. The data directory is owner-only.

use dash_sdk::dpp::dashcore::Network;
use std::path::{Path, PathBuf};

/// SQLite sidecars that belong to a database file and share its protection.
const SQLITE_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Why a deletion needs to happen, and therefore where it may reach.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DeletionIntent<'a> {
    /// A retained backup of `database`, inside the database's directory, its
    /// `backups/` directory, or the upstream auto-backup directory.
    Backup {
        /// The live database the backup was taken from.
        database: &'a Path,
    },
    /// Resyncable per-network data removed by a network clear: anything under
    /// `<data_dir>/spv/<network>/`, plus the `<data_dir>/spv/<network>.lock`
    /// storage lock.
    NetworkClear {
        /// The application data directory.
        data_dir: &'a Path,
        /// The network whose data is being cleared.
        network: Network,
    },
    /// A rotated log file named `<stem>.<timestamp>.log` directly in `dir`.
    Log {
        /// The directory holding the logs; also the application data directory.
        dir: &'a Path,
        /// The log's base name, e.g. `det`.
        stem: &'a str,
    },
}

/// A deletion the chokepoint refused. Wrapped in an [`std::io::Error`], so
/// callers can downcast it from the error's inner value.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeletionRefused {
    /// The target is a live application file (database, sidecar, vault, config).
    #[error("A protected application file was not deleted.")]
    ProtectedFile {
        /// The refused target.
        path: PathBuf,
    },
    /// The target is a symlink, directory, or other non-regular file.
    #[error("Only regular files can be deleted.")]
    NotRegularFile {
        /// The refused target.
        path: PathBuf,
    },
    /// The target has more than one hard link.
    #[error("A file with more than one hard link was not deleted.")]
    HardLinked {
        /// The refused target.
        path: PathBuf,
    },
    /// The target lies outside what the deletion's intent may reach.
    #[error("A file outside the permitted location was not deleted.")]
    OutsideScope {
        /// The refused target.
        path: PathBuf,
    },
}

impl From<DeletionRefused> for std::io::Error {
    fn from(refused: DeletionRefused) -> Self {
        std::io::Error::other(refused)
    }
}

/// Delete the regular file at `path` if every chokepoint check passes.
///
/// # Errors
///
/// A refused deletion is an [`std::io::Error`] wrapping [`DeletionRefused`]. A
/// missing target is [`std::io::ErrorKind::NotFound`], so callers that tolerate
/// an already-gone file keep doing so. Other I/O failures pass through.
pub(crate) fn delete_file(path: &Path, intent: DeletionIntent<'_>) -> std::io::Result<()> {
    check(path, intent)?;
    std::fs::remove_file(path)
}

/// Delete the directory tree at `dir`, unlinking every file through
/// [`delete_file`] and then removing the emptied directories.
///
/// Symlinks are never followed; one inside the tree is refused like any other
/// non-regular file. Stops at the first failure.
///
/// # Errors
///
/// As [`delete_file`]; a missing `dir` is [`std::io::ErrorKind::NotFound`], and
/// a `dir` that is not a real directory is refused as [`DeletionRefused::NotRegularFile`].
pub(crate) fn delete_tree(dir: &Path, intent: DeletionIntent<'_>) -> std::io::Result<()> {
    if !std::fs::symlink_metadata(dir)?.is_dir() {
        return Err(DeletionRefused::NotRegularFile {
            path: dir.to_owned(),
        }
        .into());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if std::fs::symlink_metadata(&path)?.is_dir() {
            delete_tree(&path, intent)?;
        } else {
            delete_file(&path, intent)?;
        }
    }
    // Only an empty directory can be removed here; every file went through the checks.
    std::fs::remove_dir(dir)
}

fn check(path: &Path, intent: DeletionIntent<'_>) -> std::io::Result<()> {
    let refused =
        |make: fn(PathBuf) -> DeletionRefused| -> std::io::Error { make(path.to_owned()).into() };
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(refused(|path| DeletionRefused::NotRegularFile { path }));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| refused(|path| DeletionRefused::OutsideScope { path }))?;
    if is_protected_name(name) {
        return Err(refused(|path| DeletionRefused::ProtectedFile { path }));
    }
    let located = canonical_location(path)?;
    let protected = protected_paths(intent);
    if protected
        .iter()
        .any(|file| same_location(&located, file) || same_inode(&metadata, file))
    {
        return Err(refused(|path| DeletionRefused::ProtectedFile { path }));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match metadata.nlink() {
            1 => {}
            // Unlinked by a concurrent deletion between lookup and stat.
            0 => return Err(std::io::ErrorKind::NotFound.into()),
            _ => return Err(refused(|path| DeletionRefused::HardLinked { path })),
        }
    }
    if !in_scope(&located, name, intent) {
        return Err(refused(|path| DeletionRefused::OutsideScope { path }));
    }
    Ok(())
}

/// Every network, via an exhaustive match: a new [`Network`] variant fails to
/// compile here until its databases are added to the deny-list.
fn all_networks() -> [Network; 4] {
    let _exhaustive = |network: Network| match network {
        Network::Mainnet | Network::Testnet | Network::Devnet | Network::Regtest => (),
    };
    [
        Network::Mainnet,
        Network::Testnet,
        Network::Devnet,
        Network::Regtest,
    ]
}

/// Whether `name` is a live application file, regardless of its directory.
fn is_protected_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let base = SQLITE_SIDECAR_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or(&name);
    if matches!(base, "det-app.sqlite" | "data.db" | ".env")
        || base.ends_with(".pwsvault")
        || base.ends_with(".premigration")
    {
        return true;
    }
    all_networks().into_iter().any(|network| {
        let prefix = crate::wallet_backend::network_prefix(network);
        base == format!("det-{prefix}.sqlite") || base == format!("det-{prefix}-shielded.sqlite")
    })
}

/// The live files in the intent's data directory, with their SQLite sidecars.
fn protected_paths(intent: DeletionIntent<'_>) -> Vec<PathBuf> {
    let (data_dir, database) = match intent {
        DeletionIntent::Backup { database } => (parent_of(database), Some(database)),
        DeletionIntent::NetworkClear { data_dir, .. } => (data_dir, None),
        DeletionIntent::Log { dir, .. } => (dir, None),
    };
    let mut roots = vec![
        data_dir.join("det-app.sqlite"),
        data_dir.join("data.db"),
        data_dir.join(".env"),
        crate::context::AppContext::secret_store_path(data_dir),
    ];
    for network in all_networks() {
        roots.push(crate::wallet_backend::wallet_database_path(
            data_dir, network,
        ));
        roots.push(crate::wallet_backend::shielded_database_path(
            data_dir, network,
        ));
    }
    roots.extend(database.map(Path::to_owned));
    let mut protected = Vec::with_capacity(roots.len() * (SQLITE_SIDECAR_SUFFIXES.len() + 1));
    for root in roots {
        for suffix in SQLITE_SIDECAR_SUFFIXES {
            let mut name = root.as_os_str().to_owned();
            name.push(suffix);
            protected.push(PathBuf::from(name));
        }
        protected.push(root);
    }
    protected
}

/// `path` with its parent directory resolved, keeping the final component as is
/// so a symlink target is never followed.
fn canonical_location(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let name = absolute
        .file_name()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    Ok(parent_of(&absolute).canonicalize()?.join(name))
}

fn parent_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// `dir` resolved when it exists; otherwise as given (so it matches nothing real).
fn canonical_dir(dir: &Path) -> PathBuf {
    dir.canonicalize().unwrap_or_else(|_| dir.to_owned())
}

fn same_location(located: &Path, protected: &Path) -> bool {
    canonical_location(protected).is_ok_and(|protected| {
        protected
            .as_os_str()
            .eq_ignore_ascii_case(located.as_os_str())
    })
}

#[cfg(unix)]
fn same_inode(target: &std::fs::Metadata, protected: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(protected)
        .is_ok_and(|protected| protected.dev() == target.dev() && protected.ino() == target.ino())
}

#[cfg(not(unix))]
fn same_inode(_target: &std::fs::Metadata, _protected: &Path) -> bool {
    false
}

fn in_scope(located: &Path, name: &str, intent: DeletionIntent<'_>) -> bool {
    let Some(parent) = located.parent() else {
        return false;
    };
    match intent {
        DeletionIntent::Backup { database } => {
            let directory = parent_of(database);
            [
                directory.to_owned(),
                directory.join(crate::database::legacy_backups::BACKUP_DIR),
                platform_wallet_storage::default_auto_backup_dir(database),
            ]
            .iter()
            .any(|allowed| canonical_dir(allowed) == parent)
        }
        DeletionIntent::NetworkClear { data_dir, network } => {
            let spv = data_dir.join("spv");
            let prefix = crate::wallet_backend::network_prefix(network);
            let lock = format!("{prefix}.lock");
            parent.starts_with(canonical_dir(&spv.join(prefix)))
                || (parent == canonical_dir(&spv) && name == lock)
        }
        DeletionIntent::Log { dir, stem } => {
            parent == canonical_dir(dir) && is_rotated_log_name(name, stem)
        }
    }
}

/// Whether `name` is `<stem>.<digits>.log`.
fn is_rotated_log_name(name: &str, stem: &str) -> bool {
    !stem.is_empty()
        && name
            .strip_prefix(stem)
            .and_then(|rest| rest.strip_prefix('.'))
            .and_then(|rest| rest.strip_suffix(".log"))
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests;
