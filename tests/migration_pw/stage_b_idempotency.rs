//! Stage-B idempotency lane (network-free boundary).
//!
//! The full Stage-B engine needs a live network (wallet re-registration,
//! identity sync, contact derivation), which is out of scope for this
//! harness — those are covered by the backend-e2e suite. The
//! deterministic destructive-ordering invariants are pinned by the in-crate
//! `p3d` unit module. Here we pin the cross-launch, network-free guarantees
//! the marker provides: the pending marker is the single source of truth for
//! "Stage-B still owes work", it survives crashes at the DB boundary, and
//! clearing it is idempotent.

use crate::common::{finalize_stage_b, pending_db_with_floor};
use dash_evo_tool::database::Database;

/// The marker persists across an abrupt relaunch (crash before Stage-B
/// finalised) so the next launch re-runs Stage-B.
#[test]
fn marker_persists_across_crash_relaunch_until_finalised() {
    let tmp = tempfile::tempdir().unwrap();
    let db_file;
    {
        let (db, f) = pending_db_with_floor(tmp.path());
        db_file = f;
        assert!(db.get_platform_wallet_migration_pending().unwrap());
        // Simulated crash: drop without finalising.
    }
    // Relaunch #1: still pending → Stage-B would re-run.
    {
        let db = Database::new(&db_file).unwrap();
        db.initialize(&db_file).unwrap();
        assert!(
            db.get_platform_wallet_migration_pending().unwrap(),
            "marker must survive crash-relaunch until Stage-B finalises"
        );
        // Crash again before finalising.
    }
    // Relaunch #2: finalise this time.
    {
        let db = Database::new(&db_file).unwrap();
        db.initialize(&db_file).unwrap();
        assert!(db.get_platform_wallet_migration_pending().unwrap());
        finalize_stage_b(&db);
        assert!(!db.get_platform_wallet_migration_pending().unwrap());
    }
    // Relaunch #3: finalised → Stage-B does not run again.
    {
        let db = Database::new(&db_file).unwrap();
        db.initialize(&db_file).unwrap();
        assert!(
            !db.get_platform_wallet_migration_pending().unwrap(),
            "once finalised the marker stays cleared across relaunch"
        );
    }
}

/// Clearing the marker twice (crash after the drop but before clearing, then
/// a re-run that clears again) is harmless and idempotent.
#[test]
fn clearing_marker_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, _f) = pending_db_with_floor(tmp.path());

    finalize_stage_b(&db);
    finalize_stage_b(&db); // idempotent re-run of the finalize tail
    assert!(
        !db.get_platform_wallet_migration_pending().unwrap(),
        "marker stays cleared across idempotent re-finalise"
    );
}

/// Reentrancy at the cross-launch level: independent of how many times the
/// launch path observes the marker, exactly one transition pending→cleared
/// is meaningful. We model concurrent observers reading the same durable
/// marker and assert they converge.
#[test]
fn marker_is_single_source_of_truth_for_reentrant_launch() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, db_file) = pending_db_with_floor(tmp.path());

    // Two "observers" (e.g. a reentrant ensure_wallet_backend) read the
    // pending marker; both must see the same authoritative value.
    let a = db.get_platform_wallet_migration_pending().unwrap();
    let b_db = Database::new(&db_file).unwrap();
    let b = b_db.get_platform_wallet_migration_pending().unwrap();
    assert_eq!(a, b, "marker is the single cross-instance source of truth");
    assert!(a, "still pending");

    // Exactly one finalisation flips it; further observers see cleared.
    finalize_stage_b(&db);
    assert!(!b_db.get_platform_wallet_migration_pending().unwrap());
}
