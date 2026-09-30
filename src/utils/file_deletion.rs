//! The single chokepoint for deleting files from disk: every production deletion goes
//! through [`delete_file`] (or [`delete_tree`]) with a [`DeletionIntent`]. Before any
//! unlink it refuses, in order:
//!
//! 1. anything but a regular file (a symlink is unlinked, never followed, only under
//!    [`DeletionIntent::NetworkClear`]);
//! 2. live application files by name (app/legacy/wallet/shielded databases, vaults,
//!    `.env`, `*.premigration`, with SQLite sidecars), independent of caller paths;
//! 3. the same files by canonical location and, on Unix, device/inode;
//! 4. on Unix, files with more than one hard link (on Windows, alias-prone names);
//! 5. anything outside the intent's directories and file-name grammar.
//!
//! Check and unlink are not atomic; a concurrent rename in the window is out of scope.

use dash_sdk::dpp::dashcore::Network;
use std::path::{Path, PathBuf};

/// SQLite sidecars that belong to a database file and share its protection.
const SQLITE_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Why a deletion needs to happen, and therefore where it may reach.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DeletionIntent<'a> {
    /// A retained backup of `database`: a bridge snapshot of it next to it, an
    /// upstream `pre-migration-*` snapshot of it in the auto-backup directory, or,
    /// for the legacy `data.db`, a `backups/data_backup_*.db` copy.
    Backup {
        /// The live database the backup was taken from.
        database: &'a Path,
    },
    /// Resyncable per-network data removed by a network clear: anything under
    /// `<data_dir>/spv/<network>/` (including emptied directories and symlinks,
    /// which are unlinked, never followed), plus the `<data_dir>/spv/<network>.lock`
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
    /// The target is a directory, a symlink outside a network clear, or another
    /// non-regular file.
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
    let metadata = check(path, intent)?;
    unlink(path, &metadata)
}

/// Delete the directory tree at `dir`, unlinking every file through the same
/// checks as [`delete_file`] and then removing the emptied directories.
///
/// Only [`DeletionIntent::NetworkClear`] may remove directories, and only strictly
/// inside its network's SPV directory; each directory is scope-checked before it is
/// read and again before it is removed. Symlinks are unlinked, never followed. The
/// walk uses an explicit stack, so depth cannot overflow the call stack. Stops at
/// the first failure.
///
/// # Errors
///
/// As [`delete_file`]; a missing `dir` is [`std::io::ErrorKind::NotFound`], a `dir`
/// that is not a real directory is refused as [`DeletionRefused::NotRegularFile`],
/// and one outside the intent's scope as [`DeletionRefused::OutsideScope`].
pub(crate) fn delete_tree(dir: &Path, intent: DeletionIntent<'_>) -> std::io::Result<()> {
    if !std::fs::symlink_metadata(dir)?.is_dir() {
        return Err(DeletionRefused::NotRegularFile {
            path: dir.to_owned(),
        }
        .into());
    }
    check_dir_scope(dir, intent)?;
    // Post-order walk: a directory is removed once all its entries are gone.
    let mut stack = vec![(dir.to_owned(), false)];
    while let Some((directory, emptied)) = stack.pop() {
        if emptied {
            check_dir_scope(&directory, intent)?;
            remove_empty_dir(&directory)?;
            continue;
        }
        stack.push((directory.clone(), true));
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if std::fs::symlink_metadata(&path)?.is_dir() {
                check_dir_scope(&path, intent)?;
                stack.push((path, false));
            } else {
                delete_file(&path, intent)?;
            }
        }
    }
    Ok(())
}

/// Refuse a directory outside what `intent` may remove.
fn check_dir_scope(dir: &Path, intent: DeletionIntent<'_>) -> std::io::Result<()> {
    let located = canonical_location(dir)?;
    let allowed = match intent {
        DeletionIntent::NetworkClear { data_dir, network } => {
            let root = canonical_dir(&crate::wallet_backend::spv_storage_dir(data_dir, network));
            located.starts_with(&root) && located != root
        }
        DeletionIntent::Backup { .. } | DeletionIntent::Log { .. } => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(DeletionRefused::OutsideScope {
            path: dir.to_owned(),
        }
        .into())
    }
}

/// The only raw directory removal; its caller has emptied and scope-checked `dir`.
#[expect(
    clippy::disallowed_methods,
    reason = "the deletion chokepoint itself; `dir` is empty and scope-checked"
)]
fn remove_empty_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::remove_dir(dir)
}

/// The only raw file unlink; its caller has passed every chokepoint check.
#[expect(
    clippy::disallowed_methods,
    reason = "the deletion chokepoint itself; every check has passed"
)]
fn unlink(path: &Path, metadata: &std::fs::Metadata) -> std::io::Result<()> {
    // Windows removes a symlink to a directory as a directory; the link is still
    // unlinked without following it.
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if metadata.file_type().is_symlink_dir() {
            return std::fs::remove_dir(path);
        }
    }
    #[cfg(not(windows))]
    let _ = metadata;
    std::fs::remove_file(path)
}

/// Every check before an unlink; returns the target's own (unfollowed) metadata.
fn check(path: &Path, intent: DeletionIntent<'_>) -> std::io::Result<std::fs::Metadata> {
    let refused =
        |make: fn(PathBuf) -> DeletionRefused| -> std::io::Error { make(path.to_owned()).into() };
    let metadata = std::fs::symlink_metadata(path)?;
    let unlinkable_link =
        metadata.is_symlink() && matches!(intent, DeletionIntent::NetworkClear { .. });
    if !metadata.is_file() && !unlinkable_link {
        return Err(refused(|path| DeletionRefused::NotRegularFile { path }));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| refused(|path| DeletionRefused::OutsideScope { path }))?;
    if is_protected_name(name) || (cfg!(windows) && is_windows_alias_name(name)) {
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
    Ok(metadata)
}

/// Whether `name` is a live application file, regardless of its directory.
fn is_protected_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let base = SQLITE_SIDECAR_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or(&name);
    if [
        crate::wallet_backend::APP_DATABASE_FILE,
        crate::database::legacy_backups::LEGACY_DATABASE,
        crate::app_dir::ENV_FILE,
    ]
    .contains(&base)
        || base.ends_with(".pwsvault")
        || base.ends_with(".premigration")
    {
        return true;
    }
    crate::wallet_backend::all_networks()
        .into_iter()
        .any(|network| {
            let prefix = crate::wallet_backend::network_prefix(network);
            base == format!("det-{prefix}.sqlite")
                || base == format!("det-{prefix}-shielded.sqlite")
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
        crate::wallet_backend::app_database_path(data_dir),
        data_dir.join(crate::database::legacy_backups::LEGACY_DATABASE),
        data_dir.join(crate::app_dir::ENV_FILE),
        crate::context::AppContext::secret_store_path(data_dir),
    ];
    for network in crate::wallet_backend::all_networks() {
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

/// Whether `name` could alias another file on Windows: an 8.3 short name (`~`), an
/// alternate data stream (`:`), or a trailing `.` or space that Win32 strips.
fn is_windows_alias_name(name: &str) -> bool {
    name.contains(['~', ':']) || name.ends_with(['.', ' '])
}

fn in_scope(located: &Path, name: &str, intent: DeletionIntent<'_>) -> bool {
    let Some(parent) = located.parent() else {
        return false;
    };
    let is = |dir: &Path| canonical_dir(dir) == parent;
    match intent {
        DeletionIntent::Backup { database } => {
            let directory = parent_of(database);
            let legacy = database.file_name().is_some_and(|file| {
                file.eq_ignore_ascii_case(crate::database::legacy_backups::LEGACY_DATABASE)
            });
            (is(directory)
                && crate::wallet_backend::platform_compatibility::is_bridge_backup_name(
                    database, name,
                ))
                || (is(&platform_wallet_storage::default_auto_backup_dir(database))
                    && crate::wallet_backend::platform_compatibility::upstream_backup_timestamp(
                        database, name,
                    )
                    .is_some())
                || (legacy
                    && is(&directory.join(crate::database::legacy_backups::BACKUP_DIR))
                    && crate::database::legacy_backups::backup_timestamp(name).is_some())
        }
        DeletionIntent::NetworkClear { data_dir, network } => {
            let spv = crate::wallet_backend::spv_root_dir(data_dir);
            let lock = format!("{}.lock", crate::wallet_backend::network_prefix(network));
            parent.starts_with(canonical_dir(&crate::wallet_backend::spv_storage_dir(
                data_dir, network,
            ))) || (is(&spv) && name == lock)
        }
        DeletionIntent::Log { dir, stem } => {
            is(dir) && crate::logging::parse_rotated_ts(name, stem).is_some()
        }
    }
}

#[cfg(test)]
mod tests;
