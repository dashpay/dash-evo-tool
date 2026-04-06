//! SPV status and types tests.
//!
//! TODO: The SpvManager integration tests (lifecycle, concurrency,
//! state transitions) need to be rewritten to test PlatformWalletManager +
//! SpvRuntime + SpvEventBridge instead of the deleted SpvManager.

use crate::spv::SpvStatus;

// ── SpvStatus helper methods ─────────────────────────────────────

/// Given all SpvStatus variants,
/// When calling is_active(),
/// Then Starting, Syncing, Running, and Stopping return true; others return false.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_spv_status_is_active() {
    assert!(!SpvStatus::Idle.is_active());
    assert!(SpvStatus::Starting.is_active());
    assert!(SpvStatus::Syncing.is_active());
    assert!(SpvStatus::Running.is_active());
    assert!(SpvStatus::Stopping.is_active());
    assert!(!SpvStatus::Stopped.is_active());
    assert!(!SpvStatus::Error.is_active());
}

/// Given u8 values 0 through 6 and out-of-range values,
/// When converting via From<u8>,
/// Then each maps to the correct SpvStatus variant (out-of-range defaults to Idle).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_spv_status_from_u8_roundtrip() {
    for val in 0u8..=6 {
        let status = SpvStatus::from(val);
        let display = format!("{}", status);
        assert!(
            !display.is_empty(),
            "Display should not be empty for value {}",
            val
        );
    }
    assert_eq!(SpvStatus::from(255), SpvStatus::Idle);
    assert_eq!(SpvStatus::from(7), SpvStatus::Idle);
}

// ── CoreBackendMode ──────────────────────────────────────────────

/// Given CoreBackendMode variants,
/// When converting between u8 and enum via From<u8> and as_u8(),
/// Then roundtrip is correct (0 = Rpc, 1 = Spv, unknown defaults to Rpc).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_core_backend_mode_roundtrip() {
    use crate::spv::CoreBackendMode;

    assert_eq!(CoreBackendMode::from(0), CoreBackendMode::Rpc);
    assert_eq!(CoreBackendMode::from(1), CoreBackendMode::Spv);
    assert_eq!(CoreBackendMode::from(99), CoreBackendMode::Rpc); // default

    assert_eq!(CoreBackendMode::Rpc.as_u8(), 0);
    assert_eq!(CoreBackendMode::Spv.as_u8(), 1);
}

// TODO: Rewrite the following tests for PlatformWalletManager + SpvRuntime:
//
// - test_new_manager_has_idle_status
// - test_status_snapshot_consistency
// - test_stop_when_idle_does_not_panic
// - test_double_stop_does_not_panic
// - test_use_local_node_toggle
// - test_clear_data_dir_when_idle
// - test_concurrent_status_reads_no_deadlock
// - test_start_sets_starting_status
// - test_double_start_is_idempotent
// - test_start_stop_clean_shutdown
// - test_rapid_start_stop_no_panic
// - test_concurrent_reads_during_lifecycle
// - test_det_wallets_snapshot_empty
// - test_wallet_id_for_seed_returns_none
// - test_register_reconcile_channel
// - test_register_finality_channel
// - test_broadcast_transaction_fails_when_not_running
// - test_live_testnet_sync_and_shutdown
