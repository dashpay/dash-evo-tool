//! Launch-time recovery lane: restore-from-premigration ONLY on injected
//! corruption; user-never-unlocks; no restore without a floor. Driven
//! through the public `Database::recover_from_premigration_if_corrupt`.

use crate::common::{floor_path, fresh_db, pending_db_with_floor};
use dash_evo_tool::database::Database;

/// User upgrades but never unlocks: Stage-B never runs, yet the marker
/// persists, the recovery floor exists, and the app DB stays usable.
#[test]
fn user_never_unlocks_marker_and_floor_persist_app_usable() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, db_file) = pending_db_with_floor(tmp.path());

    // Marker persists (Stage-B gated behind unlock, never ran).
    assert!(
        db.get_platform_wallet_migration_pending().unwrap(),
        "marker must persist when the user never unlocks"
    );
    // Floor exists.
    assert!(floor_path(&db_file).exists(), "recovery floor must exist");
    // App DB is still usable for ordinary reads (settings present).
    assert!(
        !db.get_platform_wallet_migration_notice_shown().unwrap(),
        "DB remains usable; notice not yet shown"
    );
    drop(db);

    // A healthy-but-pending DB is NOT restored — that is the Stage-B retry
    // path, not the exceptional path.
    let restored = Database::recover_from_premigration_if_corrupt(&db_file).unwrap();
    assert!(
        !restored,
        "healthy pending DB must not be restored (Stage-B retry instead)"
    );
}

/// Injected corruption of the post-migration DB while a floor exists →
/// restored to the consistent pre-migration state; marker still pending so
/// Stage-B will retry on the next unlock.
#[test]
fn restore_on_injected_corruption_then_retry() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, db_file) = pending_db_with_floor(tmp.path());
    drop(db);

    std::fs::write(&db_file, b"this is not a sqlite database").unwrap();

    let restored = Database::recover_from_premigration_if_corrupt(&db_file).unwrap();
    assert!(restored, "corrupt DB with a floor must be restored");

    // Restored DB opens cleanly and the marker is still set.
    let db = Database::new(&db_file).unwrap();
    db.initialize(&db_file).unwrap();
    assert!(
        db.get_platform_wallet_migration_pending().unwrap(),
        "marker still pending after restore — Stage-B retries"
    );
}

/// No floor (fresh install / migration already finalised) → never restore,
/// even if the live DB is missing.
#[test]
fn no_restore_without_floor() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, db_file) = fresh_db(tmp.path());
    assert!(
        !floor_path(&db_file).exists(),
        "fresh install has no recovery floor"
    );
    drop(db);
    std::fs::remove_file(&db_file).unwrap();

    let restored = Database::recover_from_premigration_if_corrupt(&db_file).unwrap();
    assert!(!restored, "no floor → never restore");
}

/// Restore is re-runnable: the floor is never consumed, so repeated
/// corruption cycles each recover cleanly (models a crash mid-restore).
#[test]
fn restore_is_rerunnable() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, db_file) = pending_db_with_floor(tmp.path());
    drop(db);

    std::fs::write(&db_file, b"corrupt-1").unwrap();
    assert!(Database::recover_from_premigration_if_corrupt(&db_file).unwrap());
    // Now healthy → no-op.
    assert!(
        !Database::recover_from_premigration_if_corrupt(&db_file).unwrap(),
        "second call on a healthy DB is a no-op"
    );
    // Corrupt again → still recoverable (floor survived).
    std::fs::write(&db_file, b"corrupt-2").unwrap();
    assert!(
        Database::recover_from_premigration_if_corrupt(&db_file).unwrap(),
        "floor must remain usable for a subsequent corruption"
    );
}
