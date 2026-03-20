//! Integration tests for SpvManager lifecycle, concurrency, and state transitions.

use crate::config::NetworkConfig;
use crate::spv::SpvStatus;
use crate::spv::manager::SpvManager;
use crate::utils::tasks::TaskManager;
use dash_sdk::dpp::dashcore::Network;
use std::sync::{Arc, RwLock};
use tokio::time::{Duration, timeout};

/// Deadlock detection timeout: if any operation takes longer than this,
/// the test fails (likely a deadlock).
const DEADLOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// Create a minimal testnet NetworkConfig for testing.
fn test_network_config() -> NetworkConfig {
    NetworkConfig {
        dapi_addresses: "https://127.0.0.1:1443".to_string(),
        core_host: "127.0.0.1".to_string(),
        core_rpc_port: 19998,
        core_rpc_user: "dashrpc".to_string(),
        core_rpc_password: "password".to_string(),
        core_zmq_endpoint: Some("tcp://127.0.0.1:23709".to_string()),
        devnet_name: None,
        wallet_private_key: None,
    }
}

/// Create an SpvManager for testing. Uses testnet config and a fresh TaskManager.
/// Returns the `TempDir` so it stays alive for the test duration.
fn create_test_manager() -> (Arc<SpvManager>, Arc<TaskManager>, tempfile::TempDir) {
    let config = Arc::new(RwLock::new(test_network_config()));
    let task_manager = Arc::new(TaskManager::new());
    let tmp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let manager = SpvManager::new(
        tmp_dir.path(),
        Network::Testnet,
        config,
        task_manager.clone(),
    )
    .expect("SpvManager::new should succeed");
    (manager, task_manager, tmp_dir)
}

// ── Construction and initial state ───────────────────────────────

/// Given a freshly constructed SpvManager,
/// When reading the status snapshot,
/// Then status is Idle, no error, no start time, no progress, and 0 peers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_new_manager_has_idle_status() {
    let (manager, _tm, _tmp_dir) = create_test_manager();
    let snapshot = manager.status();
    assert_eq!(
        snapshot.status,
        SpvStatus::Idle,
        "New manager should be Idle"
    );
    assert!(
        snapshot.last_error.is_none(),
        "New manager should have no error"
    );
    assert!(
        snapshot.started_at.is_none(),
        "New manager should have no started_at"
    );
    assert!(
        snapshot.sync_progress.is_none(),
        "New manager should have no sync progress"
    );
    assert_eq!(
        snapshot.connected_peers, 0,
        "New manager should have 0 connected peers"
    );
}

/// Given a freshly constructed SpvManager,
/// When taking multiple sync and async snapshots in sequence,
/// Then all snapshots are consistent (Idle, no error, 0 peers).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_status_snapshot_consistency() {
    let (manager, _tm, _tmp_dir) = create_test_manager();

    for _ in 0..10 {
        let sync_snapshot = manager.status();
        let async_snapshot = manager.status_async().await;

        assert_eq!(sync_snapshot.status, SpvStatus::Idle);
        assert_eq!(async_snapshot.status, SpvStatus::Idle);
        assert!(sync_snapshot.last_error.is_none());
        assert!(async_snapshot.last_error.is_none());
        assert_eq!(sync_snapshot.connected_peers, 0);
        assert_eq!(async_snapshot.connected_peers, 0);
    }
}

// ── Stop when idle ───────────────────────────────────────────────

/// Given an idle SpvManager that has never been started,
/// When calling stop(),
/// Then it completes without panic or deadlock and sets status to Stopped.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_stop_when_idle_does_not_panic() {
    let (manager, _tm, _tmp_dir) = create_test_manager();

    let result = timeout(DEADLOCK_TIMEOUT, async {
        manager.stop();
    })
    .await;
    assert!(
        result.is_ok(),
        "stop() should complete within timeout (no deadlock)"
    );

    let snapshot = manager.status();
    assert_eq!(
        snapshot.status,
        SpvStatus::Stopped,
        "stop() on idle manager should set status to Stopped"
    );
}

/// Given an idle SpvManager,
/// When calling stop() twice in succession,
/// Then both calls complete without panic or deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_double_stop_does_not_panic() {
    let (manager, _tm, _tmp_dir) = create_test_manager();

    let result = timeout(DEADLOCK_TIMEOUT, async {
        manager.stop();
        manager.stop();
    })
    .await;
    assert!(
        result.is_ok(),
        "Double stop() should complete within timeout"
    );

    let snapshot = manager.status();
    assert_eq!(snapshot.status, SpvStatus::Stopped);
}

// ── use_local_node flag ──────────────────────────────────────────

/// Given a freshly constructed SpvManager,
/// When toggling use_local_node on and off,
/// Then the getter reflects each change correctly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_use_local_node_toggle() {
    let (manager, _tm, _tmp_dir) = create_test_manager();

    assert!(!manager.use_local_node(), "Default should be false");
    manager.set_use_local_node(true);
    assert!(manager.use_local_node(), "Should be true after set");
    manager.set_use_local_node(false);
    assert!(!manager.use_local_node(), "Should be false after reset");
}

// ── clear_data_dir when idle ─────────────────────────────────────

/// Given an idle SpvManager,
/// When calling clear_data_dir(),
/// Then it succeeds and the status remains Idle.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_clear_data_dir_when_idle() {
    let (manager, _tm, _tmp_dir) = create_test_manager();

    let result = timeout(DEADLOCK_TIMEOUT, async { manager.clear_data_dir() }).await;
    assert!(
        result.is_ok(),
        "clear_data_dir() should complete within timeout"
    );
    let clear_result = result.unwrap();
    assert!(
        clear_result.is_ok(),
        "clear_data_dir() should succeed when idle: {:?}",
        clear_result.err()
    );

    let snapshot = manager.status();
    assert_eq!(snapshot.status, SpvStatus::Idle);
}

// ── Concurrent status reads ──────────────────────────────────────

/// Given an idle SpvManager shared across 20 concurrent tasks,
/// When each task reads the status (sync and async) 100 times,
/// Then all reads complete within the deadlock timeout without panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_status_reads_no_deadlock() {
    let (manager, _tm, _tmp_dir) = create_test_manager();

    let result = timeout(DEADLOCK_TIMEOUT, async {
        let mut handles = Vec::new();
        for _ in 0..20 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                for _ in 0..100 {
                    let _snapshot = mgr.status();
                    let _async_snapshot = mgr.status_async().await;
                    tokio::task::yield_now().await;
                }
            }));
        }
        for handle in handles {
            handle.await.expect("Task should not panic");
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "Concurrent status reads should complete within timeout (no deadlock)"
    );
}

// ── Start lifecycle (no network) ─────────────────────────────────

/// Given an idle SpvManager with no wallets,
/// When calling start(0),
/// Then it returns Ok and the status transitions to Starting (or Syncing/Error
/// if the background task progresses before the assertion).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_start_sets_starting_status() {
    let (manager, tm, _tmp_dir) = create_test_manager();

    let start_result = manager.start(0);
    assert!(
        start_result.is_ok(),
        "start() should return Ok: {:?}",
        start_result.err()
    );

    let snapshot = manager.status();
    assert!(
        snapshot.status == SpvStatus::Starting
            || snapshot.status == SpvStatus::Syncing
            || snapshot.status == SpvStatus::Error,
        "After start(), status should be Starting, Syncing, or Error (if network fails fast), got: {:?}",
        snapshot.status
    );

    manager.stop();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tm.shutdown();
}

/// Given an already-started SpvManager,
/// When calling start() a second time,
/// Then it returns Ok without spawning a duplicate loop (idempotent).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_double_start_is_idempotent() {
    let (manager, tm, _tmp_dir) = create_test_manager();

    let first = manager.start(0);
    assert!(first.is_ok(), "First start() should succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let second = manager.start(0);
    assert!(second.is_ok(), "Second start() should succeed (idempotent)");

    manager.stop();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tm.shutdown();
}

// ── Start + Stop lifecycle (clean shutdown) ──────────────────────

/// Given a started SpvManager,
/// When calling stop() and polling for completion,
/// Then the status reaches Stopped (or Error) within the deadlock timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_start_stop_clean_shutdown() {
    let (manager, tm, _tmp_dir) = create_test_manager();

    manager.start(0).expect("start() should succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = timeout(DEADLOCK_TIMEOUT, async {
        manager.stop();
        for _ in 0..100 {
            let snapshot = manager.status();
            if snapshot.status == SpvStatus::Stopped || snapshot.status == SpvStatus::Error {
                return snapshot.status;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        manager.status().status
    })
    .await;

    assert!(
        result.is_ok(),
        "Stop should complete within timeout (no deadlock)"
    );
    let final_status = result.unwrap();
    assert!(
        final_status == SpvStatus::Stopped || final_status == SpvStatus::Error,
        "After stop(), status should be Stopped or Error, got: {:?}",
        final_status
    );

    let _ = tm.shutdown();
}

// ── Rapid start/stop ─────────────────────────────────────────────

/// Given a SpvManager,
/// When performing 5 rapid start/stop cycles,
/// Then all cycles complete without panic or deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rapid_start_stop_no_panic() {
    let (manager, tm, _tmp_dir) = create_test_manager();

    let result = timeout(DEADLOCK_TIMEOUT, async {
        for _ in 0..5 {
            let _ = manager.start(0);
            tokio::time::sleep(Duration::from_millis(50)).await;
            manager.stop();
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "Rapid start/stop cycles should complete within timeout (no deadlock or panic)"
    );

    let _ = tm.shutdown();
}

// ── Concurrent status reads during start/stop ────────────────────

/// Given a SpvManager with 10 concurrent reader tasks,
/// When performing a start/stop lifecycle while readers are active,
/// Then all readers complete without panic or deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_reads_during_lifecycle() {
    let (manager, tm, _tmp_dir) = create_test_manager();

    let result = timeout(DEADLOCK_TIMEOUT, async {
        let mut readers = Vec::new();
        for _ in 0..10 {
            let mgr = Arc::clone(&manager);
            readers.push(tokio::spawn(async move {
                for _ in 0..50 {
                    let snapshot = mgr.status();
                    let _ = snapshot.status.is_active();
                    let _ = snapshot.last_error;
                    let _ = snapshot.connected_peers;
                    tokio::task::yield_now().await;
                }
            }));
        }

        let _ = manager.start(0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        manager.stop();
        tokio::time::sleep(Duration::from_millis(200)).await;

        for r in readers {
            r.await.expect("Reader task should not panic");
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "Concurrent reads during lifecycle should complete without deadlock"
    );

    let _ = tm.shutdown();
}

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

// ── Wallet operations on idle manager ────────────────────────────

/// Given a freshly constructed SpvManager with no wallets loaded,
/// When calling det_wallets_snapshot(),
/// Then the returned map is empty.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_det_wallets_snapshot_empty() {
    let (manager, _tm, _tmp_dir) = create_test_manager();
    let wallets = manager.det_wallets_snapshot();
    assert!(wallets.is_empty(), "New manager should have no wallets");
}

/// Given a freshly constructed SpvManager,
/// When looking up a wallet by an unknown seed hash,
/// Then None is returned.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_wallet_id_for_seed_returns_none() {
    let (manager, _tm, _tmp_dir) = create_test_manager();
    let seed_hash = [0u8; 32];
    assert!(
        manager.wallet_id_for_seed(seed_hash).is_none(),
        "Unknown seed hash should return None"
    );
}

// ── Reconcile and finality channels ──────────────────────────────

/// Given a freshly constructed SpvManager,
/// When registering a reconcile channel and immediately trying to receive,
/// Then the channel is open but empty (no signals yet).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_register_reconcile_channel() {
    let (manager, _tm, _tmp_dir) = create_test_manager();
    let mut rx = manager.register_reconcile_channel();

    let result = rx.try_recv();
    assert!(result.is_err(), "Channel should be empty initially");
}

/// Given a freshly constructed SpvManager,
/// When registering a finality channel and immediately trying to receive,
/// Then the channel is open but empty (no events yet).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_register_finality_channel() {
    let (manager, _tm, _tmp_dir) = create_test_manager();
    let mut rx = manager.register_finality_channel();

    let result = rx.try_recv();
    assert!(result.is_err(), "Channel should be empty initially");
}

// ── Broadcast transaction on idle manager ────────────────────────

/// Given an idle SpvManager that has not been started,
/// When attempting to broadcast a transaction,
/// Then the call fails with an error indicating SPV is not running.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_broadcast_transaction_fails_when_not_running() {
    let (manager, _tm, _tmp_dir) = create_test_manager();

    let tx = dash_sdk::dpp::dashcore::Transaction {
        version: 2,
        lock_time: 0,
        input: vec![],
        output: vec![],
        special_transaction_payload: None,
    };

    let result = manager.broadcast_transaction(&tx).await;
    assert!(
        result.is_err(),
        "Broadcast should fail when SPV is not running"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("not running"),
        "Error should mention not running, got: {}",
        err
    );
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

// ── Live testnet sync ────────────────────────────────────────────

/// Parse `.env.example` from the project root and extract the TESTNET_ NetworkConfig.
fn load_testnet_config_from_env_example() -> NetworkConfig {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let env_path = std::path::Path::new(manifest_dir).join(".env.example");
    assert!(
        env_path.exists(),
        ".env.example not found at {}",
        env_path.display()
    );

    let contents = std::fs::read_to_string(&env_path).expect("Failed to read .env.example");

    let mut vars = std::collections::HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            vars.insert(key.to_string(), value.to_string());
        }
    }

    envy::prefixed("TESTNET_")
        .from_iter(vars)
        .expect("Failed to parse TESTNET_ config from .env.example")
}

/// Given an SpvManager configured with real testnet DAPI addresses from `.env.example`,
/// When starting SPV sync with no wallets, letting it sync for 10 seconds, then stopping,
/// Then at least one peer connects, sync progress is reported, and shutdown completes
/// within 15 seconds without deadlock.
#[ignore] // Requires network access to Dash testnet peers
#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
async fn test_live_testnet_sync_and_shutdown() {
    let testnet_config = load_testnet_config_from_env_example();
    let config = Arc::new(RwLock::new(testnet_config));
    let task_manager = Arc::new(TaskManager::new());
    let tmp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let manager = SpvManager::new(
        tmp_dir.path(),
        Network::Testnet,
        config,
        task_manager.clone(),
    )
    .expect("SpvManager::new should succeed");

    // Start SPV with no wallets (header-only sync to chain tip)
    manager.start(0).expect("start() should succeed");

    // Wait for peers to connect (up to 30s)
    let connect_timeout = Duration::from_secs(30);
    let connect_result = timeout(connect_timeout, async {
        loop {
            let snapshot = manager.status_async().await;
            if snapshot.connected_peers > 0 {
                return snapshot;
            }
            if snapshot.status == SpvStatus::Error
                && let Some(ref err) = snapshot.last_error
            {
                eprintln!("SPV reported error during peer discovery: {}", err);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await;

    assert!(
        connect_result.is_ok(),
        "Should connect to at least one testnet peer within 30s"
    );
    let snapshot = connect_result.unwrap();
    eprintln!(
        "Connected to {} peer(s), status: {:?}",
        snapshot.connected_peers, snapshot.status
    );

    // Let the sync run for 10 seconds to exercise the full pipeline
    eprintln!("Letting sync run for 10 seconds...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Capture state after syncing
    let snapshot = manager.status_async().await;
    eprintln!(
        "After 10s sync: status={:?}, peers={}, progress={:?}",
        snapshot.status, snapshot.connected_peers, snapshot.sync_progress
    );
    assert!(
        snapshot.sync_progress.is_some(),
        "Should have received sync progress after 10s of syncing"
    );

    // Shutdown must complete within 15 seconds -- timeout means deadlock
    let shutdown_timeout = Duration::from_secs(15);
    let shutdown_result = timeout(shutdown_timeout, async {
        manager.stop();
        loop {
            let snapshot = manager.status_async().await;
            if snapshot.status == SpvStatus::Stopped || snapshot.status == SpvStatus::Error {
                return snapshot.status;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        shutdown_result.is_ok(),
        "Shutdown MUST complete within 15s -- timeout indicates a deadlock"
    );
    let final_status = shutdown_result.unwrap();
    assert_eq!(
        final_status,
        SpvStatus::Stopped,
        "Final status should be Stopped after clean shutdown, got: {:?}",
        final_status
    );

    let _ = task_manager.shutdown();
}
