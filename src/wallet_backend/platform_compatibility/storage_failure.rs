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
    fn storage_failure_classifies_every_error_source() {
        use rusqlite::ffi;
        use std::io::ErrorKind;
        let sqlite = |code| rusqlite::Error::SqliteFailure(ffi::Error::new(code), None);
        let sqlite_cases = [
            (ffi::SQLITE_BUSY, Some(StorageFailure::InUse)),
            (ffi::SQLITE_LOCKED, Some(StorageFailure::InUse)),
            (ffi::SQLITE_FULL, Some(StorageFailure::Full)),
            (ffi::SQLITE_IOERR, Some(StorageFailure::Unavailable)),
            (ffi::SQLITE_NOMEM, Some(StorageFailure::OutOfMemory)),
            (ffi::SQLITE_PERM, Some(StorageFailure::AccessDenied)),
            (ffi::SQLITE_READONLY, Some(StorageFailure::AccessDenied)),
            (ffi::SQLITE_CORRUPT, None),
        ];
        for (code, expected) in sqlite_cases {
            // Wrapped, so the source chain is walked too.
            let wrapped = platform_wallet_storage::WalletStorageError::Sqlite(sqlite(code));
            assert_eq!(StorageFailure::in_chain(&wrapped), expected, "{code}");
        }
        let io_cases = [
            (ErrorKind::StorageFull, Some(StorageFailure::Full)),
            (ErrorKind::OutOfMemory, Some(StorageFailure::OutOfMemory)),
            (ErrorKind::Interrupted, Some(StorageFailure::Unavailable)),
            (ErrorKind::WouldBlock, Some(StorageFailure::Unavailable)),
            (ErrorKind::TimedOut, Some(StorageFailure::Unavailable)),
            (ErrorKind::ResourceBusy, Some(StorageFailure::Unavailable)),
            (
                ErrorKind::PermissionDenied,
                Some(StorageFailure::AccessDenied),
            ),
            (
                ErrorKind::ReadOnlyFilesystem,
                Some(StorageFailure::AccessDenied),
            ),
            (ErrorKind::InvalidInput, None),
            (ErrorKind::Other, None),
        ];
        for (kind, expected) in io_cases {
            assert_eq!(
                StorageFailure::in_chain(&std::io::Error::from(kind)),
                expected,
                "{kind:?}"
            );
        }
        assert!(!StorageFailure::AccessDenied.is_retryable());
        assert!(StorageFailure::Full.is_retryable());
    }
}
