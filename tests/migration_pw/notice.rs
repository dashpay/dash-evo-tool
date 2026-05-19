//! RELEASE-BLOCKING: the mandatory one-time post-migration notice.
//!
//! The notice is the sole compensating control for the accepted DashPay
//! fund-visibility trade-off. These tests pin its non-negotiable contract:
//! shows exactly once, gated by the one-shot
//! `platform_wallet_migration_notice_shown` flag, unconditional for every
//! migrated user, jargon-free, verbatim. A regression here MUST block the
//! release.

use crate::common::{finalize_stage_b, pending_db_with_floor};
use dash_evo_tool::migration_notice::{MIGRATION_NOTICE_TEXT, should_show};

/// The flag defaults to "not shown" on a migrated DB, so the notice is
/// guaranteed to reach the user once Stage-B finalises.
#[test]
fn notice_flag_defaults_to_not_shown_for_migrated_user() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, _f) = pending_db_with_floor(tmp.path());
    assert!(
        !db.get_platform_wallet_migration_notice_shown().unwrap(),
        "a migrated user must start with notice_shown == false"
    );
}

/// While Stage-B is still pending the notice is NOT shown (its text is past
/// tense); once finalised it IS shown.
#[test]
fn notice_gated_until_stage_b_finalises_then_shows_once() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, _f) = pending_db_with_floor(tmp.path());

    // Pending, not yet completed → not shown yet.
    assert!(db.get_platform_wallet_migration_pending().unwrap());
    assert!(
        !should_show(
            db.get_platform_wallet_migration_completed().unwrap(),
            db.get_platform_wallet_migration_notice_shown().unwrap(),
        ),
        "must not show while migration still pending"
    );

    // Stage-B finalises (records completion, clears the marker).
    finalize_stage_b(&db);

    // Now the gate opens — show exactly once.
    assert!(
        should_show(
            db.get_platform_wallet_migration_completed().unwrap(),
            db.get_platform_wallet_migration_notice_shown().unwrap(),
        ),
        "must show after Stage-B finalises and before it has been shown"
    );

    // App shows it and persists the one-shot guard.
    db.set_platform_wallet_migration_notice_shown(true).unwrap();

    // Never again — even across simulated relaunches.
    for _ in 0..3 {
        assert!(
            !should_show(
                db.get_platform_wallet_migration_completed().unwrap(),
                db.get_platform_wallet_migration_notice_shown().unwrap(),
            ),
            "notice must show exactly once (one-shot flag is durable)"
        );
    }
}

/// The one-shot flag is durable across DB reopen — a relaunch right after
/// the notice was shown must not show it again.
#[test]
fn notice_flag_is_durable_across_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let db_file;
    {
        let (db, f) = pending_db_with_floor(tmp.path());
        db_file = f;
        finalize_stage_b(&db);
        db.set_platform_wallet_migration_notice_shown(true).unwrap();
    }
    // Relaunch.
    let db = dash_evo_tool::database::Database::new(&db_file).unwrap();
    db.initialize(&db_file).unwrap();
    assert!(
        db.get_platform_wallet_migration_notice_shown().unwrap(),
        "one-shot flag must survive a reopen"
    );
    assert!(
        !should_show(
            db.get_platform_wallet_migration_completed().unwrap(),
            db.get_platform_wallet_migration_notice_shown().unwrap(),
        ),
        "relaunch after shown must not re-show"
    );
}

/// Unconditional: the gate depends ONLY on the two durable bits
/// (migration_completed, notice_shown), never on network/account/contact
/// state. The decision is invariant under any DashPay contents by
/// construction (no other inputs exist).
#[test]
fn notice_decision_is_unconditional() {
    // (migration_completed, notice_shown)
    assert!(should_show(true, false), "migrated + unseen → show");
    assert!(!should_show(true, true), "migrated + shown → never again");
    assert!(!should_show(false, false), "fresh install → never");
    assert!(!should_show(false, true), "not migrated + shown → never");
}

/// A fresh install (never migrated) MUST NOT see the notice even though it
/// also has the pending marker cleared.
#[test]
fn fresh_install_never_sees_notice() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, _f) = crate::common::fresh_db(tmp.path());
    assert!(
        !db.get_platform_wallet_migration_pending().unwrap(),
        "fresh install has no pending marker"
    );
    assert!(
        !db.get_platform_wallet_migration_completed().unwrap(),
        "fresh install never completed a migration"
    );
    assert!(
        !should_show(
            db.get_platform_wallet_migration_completed().unwrap(),
            db.get_platform_wallet_migration_notice_shown().unwrap(),
        ),
        "fresh install must never see the post-migration notice"
    );
}

/// Verbatim + jargon-free. Any drift in the mandated wording fails the
/// release.
#[test]
fn notice_text_is_exact_and_jargon_free() {
    assert_eq!(
        MIGRATION_NOTICE_TEXT,
        "This update changes how DashPay contact payment addresses are \
         calculated. Payments you received from DashPay contacts on Testnet \
         or Devnet, or on a secondary account, may not appear in this \
         version. Your funds are not lost — your previous data has been \
         saved as a backup, and you can still access these payments using \
         the previous version of the app. Mainnet payments on your main \
         account are unaffected. Funding an identity directly from a \
         scanned external payment is no longer supported; receive the \
         payment into your wallet first, then fund the identity from your \
         wallet balance."
    );
    for jargon in [
        "DIP-14",
        "DIP-15",
        "xpub",
        "SDK",
        "RPC",
        "consensus",
        "nonce",
        "state transition",
        "derivation",
    ] {
        assert!(
            !MIGRATION_NOTICE_TEXT.contains(jargon),
            "notice must stay jargon-free; found {jargon:?}"
        );
    }
    // Reassures the user (not alarming) and points to a concrete recourse.
    assert!(MIGRATION_NOTICE_TEXT.contains("Your funds are not lost"));
    assert!(
        MIGRATION_NOTICE_TEXT.contains("previous version of the app"),
        "must give the user a concrete self-service recourse"
    );
}
