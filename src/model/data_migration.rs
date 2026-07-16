/// Oldest released DET `data.db` version the direct storage update supports.
pub(crate) const MIN_DIRECT_MIGRATION_VERSION: i64 = 11;

/// Newest known pre-unwire DET data version the compatibility readers support.
///
/// Deliberately kept a few versions above `DEFAULT_DB_VERSION` (38) as headroom,
/// so data written by a slightly newer build (39, 40) still migrates rather than
/// failing closed; only a version above this ceiling is rejected as too new.
pub(crate) const MAX_DIRECT_MIGRATION_VERSION: i64 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectMigrationVersion {
    TooOld,
    Supported,
    TooNew,
}

/// Classify every SQLite integer before the direct storage update starts.
pub(crate) fn classify_direct_migration_version(version: i64) -> DirectMigrationVersion {
    if version < MIN_DIRECT_MIGRATION_VERSION {
        DirectMigrationVersion::TooOld
    } else if version > MAX_DIRECT_MIGRATION_VERSION {
        DirectMigrationVersion::TooNew
    } else {
        DirectMigrationVersion::Supported
    }
}
