//! The one policy for which storage failures are environmental (and which of
//! those retrying can fix), shared by the upgrade engine and `TaskError`.

use std::error::Error;

/// An environmental storage failure, as opposed to a defect in the data itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StorageFailure {
    /// Another connection or process holds the database.
    InUse,
    /// The disk is full.
    Full,
    /// The OS reported an I/O failure or a transient interruption.
    Unavailable,
    /// Memory ran out.
    OutOfMemory,
    /// Permissions or a read-only medium prevent writing.
    AccessDenied,
}

impl StorageFailure {
    /// Classify one error, ignoring its sources.
    pub(crate) fn of(error: &(dyn Error + 'static)) -> Option<Self> {
        if let Some(sqlite) = error.downcast_ref::<rusqlite::Error>() {
            return Self::of_sqlite(sqlite);
        }
        error.downcast_ref::<std::io::Error>().and_then(Self::of_io)
    }

    /// Classify the first error in `error`'s source chain that has a category.
    pub(crate) fn in_chain(error: &(dyn Error + 'static)) -> Option<Self> {
        let mut cause = Some(error);
        while let Some(current) = cause {
            if let Some(failure) = Self::of(current) {
                return Some(failure);
            }
            cause = current.source();
        }
        None
    }

    pub(crate) fn of_sqlite(error: &rusqlite::Error) -> Option<Self> {
        use rusqlite::ErrorCode;
        match error.sqlite_error_code()? {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => Some(Self::InUse),
            ErrorCode::DiskFull => Some(Self::Full),
            ErrorCode::SystemIoFailure => Some(Self::Unavailable),
            ErrorCode::OutOfMemory => Some(Self::OutOfMemory),
            ErrorCode::PermissionDenied | ErrorCode::ReadOnly => Some(Self::AccessDenied),
            _ => None,
        }
    }

    pub(crate) fn of_io(error: &std::io::Error) -> Option<Self> {
        use std::io::ErrorKind;
        match error.kind() {
            ErrorKind::StorageFull => Some(Self::Full),
            ErrorKind::OutOfMemory => Some(Self::OutOfMemory),
            ErrorKind::Interrupted
            | ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::ResourceBusy => Some(Self::Unavailable),
            ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => Some(Self::AccessDenied),
            _ => None,
        }
    }

    /// Whether freeing the resource and retrying can recover. Access failures
    /// never fix themselves.
    pub(crate) fn is_retryable(self) -> bool {
        !matches!(self, Self::AccessDenied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_failure_walks_the_source_chain() {
        let sqlite = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            None,
        );
        let wrapped = platform_wallet_storage::WalletStorageError::Sqlite(sqlite);
        assert_eq!(
            StorageFailure::in_chain(&wrapped),
            Some(StorageFailure::Full)
        );
        let io = std::io::Error::from(std::io::ErrorKind::ReadOnlyFilesystem);
        assert_eq!(
            StorageFailure::in_chain(&io),
            Some(StorageFailure::AccessDenied)
        );
        assert!(!StorageFailure::AccessDenied.is_retryable());
        assert_eq!(
            StorageFailure::in_chain(&std::io::Error::other("plain")),
            None
        );
    }
}
