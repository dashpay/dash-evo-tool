//! Test helper utilities for database testing.
//!
//! This module provides utilities for creating test databases that can be used
//! in unit and integration tests throughout the codebase.

use crate::database::Database;
use std::path::PathBuf;
use tempfile::TempDir;

/// Creates an in-memory SQLite database for testing.
///
/// This is the fastest option for tests that don't need to persist data
/// or test file-based functionality.
///
/// # Example
/// ```
/// use dash_evo_tool::database::test_helpers::create_test_database;
///
/// let db = create_test_database().unwrap();
/// // Use db for testing...
/// ```
pub fn create_test_database() -> rusqlite::Result<Database> {
    let db = Database::new(":memory:")?;
    // Initialize tables using the standard initialization path
    // Note: We use a dummy path since :memory: doesn't use the file system
    let dummy_path = PathBuf::from(":memory:");
    db.initialize(&dummy_path)?;
    Ok(db)
}

/// Creates a file-based temporary database for testing.
///
/// Use this when you need to test file-based operations like:
/// - Database migrations
/// - Backup functionality
/// - Persistence across connections
///
/// The returned `TempDir` must be kept alive for the duration of the test,
/// as dropping it will delete the temporary directory.
///
/// # Example
/// ```
/// use dash_evo_tool::database::test_helpers::create_temp_database;
///
/// let (db, _temp_dir) = create_temp_database().unwrap();
/// // Use db for testing...
/// // _temp_dir is dropped at the end, cleaning up the test files
/// ```
pub fn create_temp_database() -> rusqlite::Result<(Database, TempDir)> {
    let temp_dir = tempfile::tempdir().map_err(|e| {
        rusqlite::Error::ToSqlConversionFailure(format!("Failed to create temp dir: {}", e).into())
    })?;
    let db_path = temp_dir.path().join("test_data.db");
    let db = Database::new(&db_path)?;
    db.initialize(&db_path)?;
    Ok((db, temp_dir))
}

/// Creates a test database with a specific file path.
///
/// Useful when you need to control the exact location of the database file.
pub fn create_database_at_path(path: &std::path::Path) -> rusqlite::Result<Database> {
    let db = Database::new(path)?;
    db.initialize(path)?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_test_database() {
        let db = create_test_database();
        assert!(db.is_ok(), "Should create in-memory database successfully");
    }

    #[test]
    fn test_create_temp_database() {
        let result = create_temp_database();
        assert!(result.is_ok(), "Should create temporary database successfully");

        let (db, temp_dir) = result.unwrap();
        let db_path = temp_dir.path().join("test_data.db");
        assert!(db_path.exists(), "Database file should exist");

        // Verify database is functional
        let settings = db.get_settings();
        assert!(settings.is_ok(), "Should be able to query settings");
    }
}
