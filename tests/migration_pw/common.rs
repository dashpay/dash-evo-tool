//! Shared fixtures for the migration QA harness. PUBLIC-API only.

use dash_evo_tool::database::Database;
use std::path::{Path, PathBuf};

/// The retained recovery-floor path, mirroring
/// `Database::premigration_backup_path` (which is crate-private). Keep in
/// sync with `database::initialization`.
pub fn floor_path(db_file: &Path) -> PathBuf {
    let mut p = db_file.as_os_str().to_os_string();
    p.push(".premigration");
    PathBuf::from(p)
}

/// A freshly-initialised DB (v-current, no migration pending). This models a
/// brand-new install: nothing to migrate.
pub fn fresh_db(dir: &Path) -> (Database, PathBuf) {
    let db_file = dir.join("data.db");
    let db = Database::new(&db_file).unwrap();
    db.initialize(&db_file).unwrap();
    (db, db_file)
}

/// A DB that has gone through Stage A (the one-time migration marker is set)
/// with the retained `.premigration` recovery floor created — i.e. a user
/// upgrading into the platform-wallet version, post-unlock Stage-B not yet
/// finalised.
///
/// Built through the public surface: initialise, arm the pending marker,
/// then re-initialise so `initialize()` lays down the floor exactly as the
/// real upgrade path does.
pub fn pending_db_with_floor(dir: &Path) -> (Database, PathBuf) {
    let db_file = dir.join("data.db");
    {
        let db = Database::new(&db_file).unwrap();
        db.initialize(&db_file).unwrap();
        db.set_platform_wallet_migration_pending(true).unwrap();
    }
    // Second open + initialize: marker is pending, so initialize() creates
    // the retained pre-migration floor (idempotent, best-effort).
    let db = Database::new(&db_file).unwrap();
    db.initialize(&db_file).unwrap();
    let floor = floor_path(&db_file);
    assert!(
        floor.exists(),
        "fixture precondition: initialize() must create the recovery floor \
         while the migration marker is pending"
    );
    (db, db_file)
}

/// Simulate Stage-B finalising successfully: the engine records completion
/// then clears the pending marker as its strictly-last writes. (The
/// destructive table-drop ordering is pinned by the in-crate `p3d` unit
/// suite; here we only need the durable post-conditions that gate the
/// notice.)
pub fn finalize_stage_b(db: &Database) {
    db.set_platform_wallet_migration_completed(true).unwrap();
    db.set_platform_wallet_migration_pending(false).unwrap();
}
