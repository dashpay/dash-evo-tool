/// Oldest released DET `data.db` version the direct storage update supports.
pub(crate) const MIN_DIRECT_MIGRATION_VERSION: i64 = 11;

/// Newest known pre-unwire DET data version the compatibility readers support.
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
