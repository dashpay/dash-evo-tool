use super::*;
use crate::app::TaskResult;
use crate::app_dir::{ensure_data_dir_exists, ensure_env_file};
use crate::context::AppContext;
use crate::context::connection_status::ConnectionStatus;
use crate::context::migration_status::MigrationState;
use crate::database::test_helpers::create_database_at_path;
use crate::model::secret::Secret;
use crate::utils::egui_mpsc::SenderAsync;
use crate::utils::tasks::TaskManager;

/// Build an offline `AppContext` for testnet in an isolated temp dir. No
/// network I/O happens at construction: the SDK and Core client are built
/// from bundled `.env` addresses but connect lazily. The `TempDir` must
/// outlive the context — its drop deletes the data dir.
fn offline_testnet_context() -> (Arc<AppContext>, SenderAsync<TaskResult>, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let (ctx, sender) = offline_testnet_context_at(temp_dir.path());
    (ctx, sender, temp_dir)
}

/// Build an offline testnet `AppContext` rooted at an existing `data_dir`.
/// Splitting this out lets a test build a second, independent context over
/// the *same* on-disk sidecars to simulate a process restart (cold boot).
fn offline_testnet_context_at(
    data_dir: &std::path::Path,
) -> (Arc<AppContext>, SenderAsync<TaskResult>) {
    let db =
        Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("create test database"));
    offline_testnet_context_with_db(data_dir, db)
}

/// Build an offline testnet `AppContext` whose `data.db` went through the
/// **real** `Database::initialize` fresh-install path (the path production
/// uses at `app.rs:322`), which gates the legacy `wallet`/`wallet_addresses`
/// tables OUT. Use this for fresh-install regression tests; the default
/// helper force-creates those tables via `create_tables(true)`.
fn offline_testnet_context_fresh_init(
    data_dir: &std::path::Path,
) -> (Arc<AppContext>, SenderAsync<TaskResult>) {
    let db_file = data_dir.join("data.db");
    let db = crate::database::Database::new(&db_file).expect("create fresh test database");
    db.initialize(&db_file)
        .expect("fresh Database::initialize should succeed");
    offline_testnet_context_with_db(data_dir, Arc::new(db))
}

fn offline_testnet_context_with_db(
    data_dir: &std::path::Path,
    db: Arc<crate::database::Database>,
) -> (Arc<AppContext>, SenderAsync<TaskResult>) {
    let data_dir = data_dir.to_path_buf();
    ensure_env_file(&data_dir);

    let subtasks = Arc::new(TaskManager::new());
    let connection_status = Arc::new(ConnectionStatus::new());
    let egui_ctx = egui::Context::default();
    let app_kv = AppContext::open_app_kv(&data_dir).expect("open app k/v");
    let secret_store = AppContext::open_secret_store(&data_dir).expect("open secret store");

    let ctx = AppContext::new(
        data_dir,
        Network::Testnet,
        db,
        subtasks,
        connection_status,
        egui_ctx,
        app_kv,
        secret_store,
        crate::model::user_role::UserRoleCell::default(),
    )
    .expect("AppContext::new should succeed offline with bundled testnet config");

    let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
    let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
    (ctx, sender)
}

/// Recursively copy a directory tree. Cold-boot tests reopen wallet state
/// over a fresh path (identical on-disk bytes) to sidestep the persister's
/// single-open advisory lock a lingering subtask may still hold.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    ensure_data_dir_exists(dst).expect("create secure destination directory");
    for entry in std::fs::read_dir(src).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            std::fs::copy(&from, &to).expect("copy file");
        }
    }
}

/// Process-global serialization lock for tests that tear a wallet backend
/// down and immediately rebuild it over the *same* on-disk path. The
/// upstream persister enforces a single open per `platform-wallet.sqlite`
/// (`WalletStorageError::AlreadyOpen`); a bootstrap subtask spawned by
/// `ensure_wallet_backend` may keep its `Arc<WalletBackend>` — and that
/// open's advisory lock — alive a beat past `stop_spv`, so under parallel
/// scheduling the reopen can lose the race. Serializing these reopen tests
/// removes the scheduler pressure so the lingering subtask drops the old
/// handle before the reopen. Mirrors `support::data_dir_lock` in the
/// kittest suite. Held across awaits, hence a `tokio::sync::Mutex`.
async fn backend_reopen_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

/// Before the wallet seam is wired, the `wallet_backend()` gate must fail
/// fast with the typed `WalletBackendNotYetWired` rather than handing back a
/// half-built backend. This is the gate the speculative pre-wire callers
/// were tripping.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wallet_backend_gate_errors_when_not_wired() {
    let (ctx, _sender, _tmp) = offline_testnet_context();

    let err = ctx
        .wallet_backend()
        .expect_err("wallet_backend() must fail before the backend is wired");
    assert!(
        matches!(err, TaskError::WalletBackendNotYetWired),
        "expected WalletBackendNotYetWired, got: {err:?}"
    );
}

/// Wiring the backend must not start chain sync: `ensure_wallet_backend`
/// builds the seam but leaves the upstream run loop unstarted, so the start
/// latch stays low until the chokepoint (or a manual Connect) starts it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wiring_does_not_start_chain_sync() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx
        .wallet_backend()
        .expect("backend must be wired after ensure_wallet_backend");
    assert!(
        !backend.is_started(),
        "wiring alone must not start chain sync"
    );

    backend.shutdown().await;
}

/// The async chokepoint wires the backend and starts chain sync in one call,
/// so a caller need not have wired the backend beforehand. Pins the
/// "ensure-then-start" sequencing the GUI/MCP/network-switch paths share.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ensure_wallet_backend_and_start_spv_wires_then_starts() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    assert!(
        ctx.wallet_backend().is_err(),
        "precondition: backend unwired before the chokepoint"
    );

    ctx.ensure_wallet_backend_and_start_spv(sender)
        .await
        .expect("chokepoint should wire then start offline");

    let backend = ctx
        .wallet_backend()
        .expect("backend must be wired after the chokepoint");
    assert!(
        backend.is_started(),
        "chokepoint must have started chain sync"
    );

    backend.shutdown().await;
}

/// The Disconnect chokepoint must produce a *visible* state change: after a
/// successful start, `stop_spv` stops chain sync IN PLACE — keeping the
/// backend wired for a restart — and settles the indicator on `Stopped` /
/// `Disconnected`. Regression guard ensuring the Disconnect button drives
/// the overall state out of its active value while preserving the backend.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_spv_in_place_keeps_backend_and_disconnects_indicator() {
    use crate::context::connection_status::OverallConnectionState;

    let (ctx, sender, _tmp) = offline_testnet_context();

    ctx.ensure_wallet_backend_and_start_spv(sender)
        .await
        .expect("chokepoint should wire then start offline");
    assert!(
        ctx.wallet_backend().is_ok(),
        "precondition: backend wired after start"
    );
    // Simulate a session that reached quorum readiness, so the disconnect
    // has a flag to re-arm.
    ctx.connection_status().set_masternodes_ready(true);

    ctx.stop_spv().await;

    let backend = ctx
        .wallet_backend()
        .expect("stop_spv must KEEP the backend wired for restart-in-place (NOT unwire it)");
    assert!(
        !backend.is_started(),
        "stop_spv must re-arm the start latch so the next Connect can restart"
    );
    assert!(
        !ctx.connection_status().masternodes_ready(),
        "stop_spv must re-arm the quorum gate so the next reconnect waits for masternode re-sync"
    );
    assert_eq!(
        ctx.connection_status().spv_status(),
        SpvStatus::Stopped,
        "stop_spv must leave the SPV indicator Stopped"
    );
    assert_eq!(
        ctx.connection_status().overall_state(),
        OverallConnectionState::Disconnected,
        "stop_spv must leave the overall state Disconnected"
    );
    assert_eq!(
        ctx.connection_status().spv_connected_peers(),
        0,
        "stop_spv must clear the live peer count"
    );
}

/// `stop_spv` is idempotent: calling it with no wired backend must not panic
/// and must still settle the indicator on `Stopped` / `Disconnected`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_spv_is_idempotent_without_a_wired_backend() {
    use crate::context::connection_status::OverallConnectionState;

    let (ctx, _sender, _tmp) = offline_testnet_context();
    assert!(
        ctx.wallet_backend().is_err(),
        "precondition: backend unwired"
    );

    ctx.stop_spv().await;

    assert_eq!(ctx.connection_status().spv_status(), SpvStatus::Stopped);
    assert_eq!(
        ctx.connection_status().overall_state(),
        OverallConnectionState::Disconnected
    );
}

/// Restart-in-place reconnect: a same-network Disconnect → Connect keeps the
/// SAME `WalletBackend` (and its `Arc<SqlitePersister>`) wired, so the
/// persister DB is never closed/reopened and `AlreadyOpen` is impossible by
/// construction — the retry below only absorbs a storage-lock timing window,
/// not a release barrier. Drives the real production path: `stop_spv()`
/// (in-place) then `ensure_wallet_backend_and_start_spv()`.
///
/// Validated offline (passes now): the backend pointer is identical across
/// disconnect→connect (reuse, not rebuild); `is_started()` is cleared by
/// `stop_spv` and re-set by the reconnect (latch + gate re-armed); the
/// reconnect returns `Ok` with no `AlreadyOpen`.
///
/// Upstream Q3 race protection now lives in the pinned platform rev: all
/// three coordinators (incl. `platform_address_sync` since b4506492) gate
/// their cancel-slot clear on `background_generation`, so a rapid restart of
/// the SAME instance cannot leak an uncancellable / duplicate loop. This
/// offline test asserts the DET-level reuse/restart contract; it does not
/// itself force the timing race — full live behavior is covered by the
/// network-gated (`#[ignore]`d) backend-e2e B-reconnect test.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_restart_in_place_reuses_backend() {
    use crate::context::connection_status::OverallConnectionState;

    let _reopen_guard = backend_reopen_lock().await;

    let (ctx, sender, _tmp) = offline_testnet_context();

    ctx.ensure_wallet_backend_and_start_spv(sender.clone())
        .await
        .expect("initial start should wire then start offline");
    let first = ctx.wallet_backend().expect("backend wired after start");
    assert!(first.is_started(), "initial start must latch the backend");
    let first_ptr = Arc::as_ptr(&first);
    drop(first);

    // Disconnect IN PLACE via the production chokepoint: the backend stays
    // wired (slot not taken), the start latch is re-armed, the indicator
    // settles on Disconnected.
    ctx.stop_spv().await;
    let after_stop = ctx
        .wallet_backend()
        .expect("stop_spv must KEEP the backend wired for restart-in-place");
    assert!(
        !after_stop.is_started(),
        "stop_spv must re-arm the start latch (is_started == false)"
    );
    assert_eq!(
        ctx.connection_status().overall_state(),
        OverallConnectionState::Disconnected,
        "stop_spv must settle the indicator on Disconnected"
    );
    assert!(
        !ctx.connection_status().masternodes_ready(),
        "stop_spv must re-arm the quorum gate (masternodes_ready == false)"
    );
    drop(after_stop);

    // Reconnect: `ensure_wallet_backend` fast-paths on the populated slot
    // (no `WalletBackend::new`, no `SqlitePersister::open`), so the SAME
    // instance restarts — structurally immune to `AlreadyOpen`.
    // dash-spv's storage flock can remain observable briefly after stop returns.
    // This test-only retry is bounded so genuine restart regressions still fail.
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match ctx
            .ensure_wallet_backend_and_start_spv(sender.clone())
            .await
        {
            Ok(()) => break,
            Err(_) if attempt < 6 => {
                let backoff_ms = 25u64 * (1u64 << (attempt - 1));
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            }
            Err(e) => panic!(
                "reconnect should restart the SAME backend in place after {attempt} attempts: {e}"
            ),
        }
    }
    let second = ctx
        .wallet_backend()
        .expect("backend still wired after reconnect");
    assert_eq!(
        first_ptr,
        Arc::as_ptr(&second),
        "restart-in-place must REUSE the same backend, not rebuild it"
    );
    assert!(
        second.is_started(),
        "reconnect must restart chain sync on the reused backend's re-armed latch"
    );

    second.shutdown().await;
}

/// Two genuinely-parallel first-open attempts on the SAME never-wired context
/// must NOT race into a double `WalletBackend::new` / `SqlitePersister::open`.
/// The upstream persister is single-open-per-path, so a concurrent double-open
/// errors — `WalletStorageError::AlreadyOpen` against a live persister (the
/// reported production symptom) or a DB-init race on a fresh file.
///
/// This guards the GUI's `finalize_network_switch` fast path, which spawns a
/// `wallet-backend-eager-init` subtask on every switch with no re-entrancy
/// guard: a rapid switch-away-and-back to the same (already-cached) network
/// fires a second eager-init for the same context before the first finishes
/// wiring. `ensure_wallet_backend` serializes them behind the per-context
/// `wallet_backend_build` mutex with a double-checked slot — the first builds
/// and stores, the second re-checks under the guard, sees the populated slot,
/// and no-ops. One open, one shared backend, no error. The eager-init entry
/// `ensure_wallet_backend_and_start_spv` delegates its open to exactly this
/// function, so guarding the open here covers that path too.
///
/// Deleting the guard (fast-path recheck + build mutex + post-guard recheck)
/// makes both racers reach `WalletBackend::new` and the second's open fails —
/// verified: the test then panics on the `must succeed` expectation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_ensure_wallet_backend_does_not_double_open() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    assert!(
        ctx.wallet_backend().is_err(),
        "precondition: backend must be unwired before the concurrent race"
    );

    let ctx_a = Arc::clone(&ctx);
    let ctx_b = Arc::clone(&ctx);
    let sender_a = sender.clone();
    let sender_b = sender.clone();
    let a = tokio::spawn(async move { ctx_a.ensure_wallet_backend(sender_a).await });
    let b = tokio::spawn(async move { ctx_b.ensure_wallet_backend(sender_b).await });
    let (ra, rb) = tokio::join!(a, b);

    ra.expect("first-open task A must not panic")
        .expect("concurrent first-open A must succeed — a double-open would error");
    rb.expect("first-open task B must not panic")
        .expect("concurrent first-open B must succeed — a double-open would error");

    // Exactly one backend was built and both racers converged on it (first
    // writer wins; the second no-ops on the populated slot).
    let backend = ctx
        .wallet_backend()
        .expect("backend must be wired after the concurrent open");

    backend.shutdown().await;
}

/// A failure at the (fallible) wiring step must surface — the
/// chokepoint returns `Err` AND flips the SPV indicator to `Error`, so the
/// user does not silently fall back to `Disconnected` with no feedback.
///
/// Induces the wiring failure offline by planting a regular file where the
/// per-network SPV storage directory would be created: `WalletBackend::new`
/// calls `create_dir_all(data_dir/spv/testnet)`, which cannot succeed when a
/// path component (`spv`) is a file rather than a directory.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chokepoint_wiring_failure_flips_indicator_to_error() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    // Block the SPV storage dir creation: a file at `data_dir/spv` makes
    // `create_dir_all(.../spv/testnet)` fail deterministically (no reliance
    // on filesystem permissions, which root can bypass in CI).
    std::fs::write(ctx.data_dir().join("spv"), b"not a directory")
        .expect("plant blocking file at the spv path");

    assert_ne!(
        ctx.connection_status.spv_status(),
        SpvStatus::Error,
        "precondition: indicator must not already be in the Error state"
    );

    let err = ctx
        .ensure_wallet_backend_and_start_spv(sender)
        .await
        .expect_err("wiring must fail when the spv path is blocked by a file");
    assert!(
        matches!(err, TaskError::FileSystem { .. }),
        "expected a FileSystem wiring error, got: {err:?}"
    );

    assert_eq!(
        ctx.connection_status.spv_status(),
        SpvStatus::Error,
        "wiring failure must flip the SPV indicator to Error"
    );
}

/// A failure inside `WalletBackend::start` must be returned, mark the
/// indicator as `Error`, and re-arm the start latch for a retry.
///
/// Wiring succeeds first. A directory is then planted at dash-spv's lock-file
/// path, so `DiskStorageManager::new` fails during `SpvRuntime::start`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chokepoint_spv_start_failure_is_returned_and_retryable() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    ctx.ensure_wallet_backend(sender.clone())
        .await
        .expect("backend wiring should succeed before the start failure");
    let backend = ctx
        .wallet_backend()
        .expect("backend must be wired before start");
    let spv_lock_path = backend.spv_storage_dir().with_extension("lock");
    std::fs::create_dir(&spv_lock_path)
        .expect("plant a directory where dash-spv expects its lock file");

    let err = ctx
        .ensure_wallet_backend_and_start_spv(sender.clone())
        .await
        .expect_err("SPV initialization failure must be returned");
    assert!(
        matches!(err, TaskError::WalletBackend { .. }),
        "expected a WalletBackend start error, got: {err:?}"
    );
    assert_eq!(
        ctx.connection_status.spv_status(),
        SpvStatus::Error,
        "SPV initialization failure must flip the indicator to Error"
    );
    assert!(
        !backend.is_started(),
        "failed SPV initialization must re-arm the start latch"
    );

    ctx.ensure_wallet_backend_and_start_spv(sender)
        .await
        .expect_err("a retry must reach the still-failing SPV initialization");
    assert!(
        ctx.connection_status.begin_spv_stop(),
        "Disconnect must claim teardown from the Error state"
    );
    ctx.stop_spv().await;

    backend.shutdown().await;
}

/// Concurrent callers that join the same failing SPV start must both receive
/// its typed error rather than letting the start-latch loser report success.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_spv_start_failure_is_returned_to_every_caller() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    ctx.ensure_wallet_backend(sender)
        .await
        .expect("backend wiring should succeed before the start failure");
    let backend = ctx
        .wallet_backend()
        .expect("backend must be wired before start");
    let spv_lock_path = backend.spv_storage_dir().with_extension("lock");
    std::fs::create_dir(&spv_lock_path)
        .expect("plant a directory where dash-spv expects its lock file");

    let backend_a = Arc::clone(&backend);
    let backend_b = Arc::clone(&backend);
    let (result_a, result_b) = tokio::join!(backend_a.start(), backend_b.start());

    let error_a = result_a.expect_err("first caller must receive the SPV start failure");
    let error_b = result_b.expect_err("second caller must receive the SPV start failure");
    let (source_a, source_b) = match (&error_a, &error_b) {
        (
            TaskError::WalletBackend { source: source_a },
            TaskError::WalletBackend { source: source_b },
        ) => (source_a, source_b),
        _ => panic!("expected WalletBackend start errors, got: {error_a:?}, {error_b:?}"),
    };
    assert!(
        Arc::ptr_eq(source_a, source_b),
        "both callers must receive the same shared SPV start error"
    );
    assert!(
        !backend.is_started(),
        "failed shared SPV start must re-arm the start latch"
    );

    backend.shutdown().await;
}

/// Cold-boot signability regression, adapted to the JIT secret model: a
/// no-password wallet must remain signable after a cold-boot hydration
/// without any seed ever being parked in a long-lived cache.
///
/// Under the JIT chokepoint there is no `inner.seeds` cache to fill or
/// clear; signing decrypts the seed just-in-time from the encrypted vault
/// envelope. For a no-password wallet (`uses_password = false`) the
/// chokepoint's unprotected fast-path decrypts with **no passphrase and no
/// prompt** — so the wallet signs whether or not the session cache holds
/// it. This test proves that:
///   1. a freshly-registered no-password wallet signs in-process; and
///   2. after `forget_all_secrets()` wipes the session cache (the exact
///      state a real cold-boot leaves: watch-only, nothing remembered) the
///      wallet STILL signs — the seed is pulled from the vault on demand.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_password_wallet_resignable_via_unlock_chokepoint() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let seed = [0x24u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("cold-boot".to_string()),
        None, // no password
    )
    .expect("build no-password wallet");
    assert!(wallet.is_open(), "a no-password wallet is open on creation");

    let (seed_hash, wallet_arc) = ctx
        .register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");
    let backend = ctx.wallet_backend().expect("backend wired");

    // Live (same-process) state: registration wrote the seed envelope to
    // the vault, so the chokepoint can decrypt the no-password seed.
    backend
        .assert_can_sign(&seed_hash)
        .await
        .expect("freshly-registered no-password wallet must sign in-process");

    // Simulate the seedless cold-boot state: wipe the session cache so
    // nothing is remembered (what hydration leaves behind). The wallet is
    // still `Open` for display, but no plaintext seed is cached anywhere.
    backend.forget_all_secrets();
    assert!(
        wallet_arc.read_recover().is_open(),
        "the wallet is still Open after the session cache is dropped"
    );

    // The JIT guarantee: a no-password wallet signs from the vault with no
    // prompt and no cache — the unprotected fast-path covers it.
    backend
        .assert_can_sign(&seed_hash)
        .await
        .expect("no-password wallet must sign after cold-boot via the JIT fast-path");

    backend.shutdown().await;
}

/// Leaving a network must not strand session-cached secrets on the
/// outgoing context. `finalize_network_switch` funnels through
/// [`WalletBackend::forget_all_secrets`]; this exercises that exact call
/// against a populated session cache and asserts it is emptied — the JIT
/// design's eager "no secrets linger across a network change" guarantee.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn network_switch_path_clears_outgoing_session_cache() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let seed = [0x31u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("switching".to_string()),
        None,
    )
    .expect("build wallet");
    let (seed_hash, _wallet_arc) = ctx
        .register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    let backend = ctx.wallet_backend().expect("backend wired");
    let scope = crate::wallet_backend::SecretScope::HdSeed { seed_hash };

    // Promote the seed into the session cache (what the unlock gesture or a
    // remembered op leaves behind).
    let held = zeroize::Zeroizing::new(seed);
    backend.secret_access().remember_session(
        &scope,
        crate::wallet_backend::SecretPlaintext::HdSeed(&held),
        crate::wallet_backend::RememberPolicy::UntilAppClose,
    );
    assert!(
        backend.secret_access().is_session_cached(&scope),
        "precondition: the seed is session-cached before the switch"
    );

    // The exact call `finalize_network_switch` makes on the outgoing
    // context before leaving it.
    backend.forget_all_secrets();

    assert!(
        !backend.secret_access().is_session_cached(&scope),
        "the outgoing context's session cache must be empty after the switch path runs"
    );

    backend.shutdown().await;
}

/// W1 idempotency: registering the same wallet twice with the
/// upstream backend is a no-op the second time — the wallet is watched once,
/// never double-watched. The pre-fix bug was the *opposite* (a never-watched
/// wallet); this pins that the new writer is also safe to call repeatedly,
/// as both W1 (create/import) and W2 (cold-boot) may fire for one wallet in
/// a single session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_wallet_from_seed_is_idempotent() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    let seed = [0x5Au8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();

    assert!(
        !backend.is_wallet_registered(&seed_hash),
        "precondition: wallet must not be registered before the first call"
    );

    backend
        .register_wallet_from_seed(&seed_hash, &seed, Some(0))
        .await
        .expect("first registration must succeed");
    assert!(
        backend.is_wallet_registered(&seed_hash),
        "the wallet must be registered after the first call"
    );
    assert_eq!(
        backend.wallet_count().await,
        1,
        "exactly one wallet is watched after the first registration"
    );

    // Second call: idempotent no-op, no double-watch.
    backend
        .register_wallet_from_seed(&seed_hash, &seed, Some(0))
        .await
        .expect("second registration must be a no-op, not an error");
    assert_eq!(
        backend.wallet_count().await,
        1,
        "a repeat registration must not double-watch the wallet"
    );

    backend.shutdown().await;
}

/// Regression guard for issue #7 (now PASSES — was the bug reproducer).
///
/// Before the upstream fix (platform PR #3828), `WalletAccountCreationOptions::Default`
/// created BOTH a BIP32 account-0 (`m/0'`, depth-1) and a BIP44 account-0
/// (`m/44'/coin'/0'`, depth-3), but the persistor collapsed both
/// `StandardAccountType` variants to the single `account_type` label
/// `"standard"`. They shared the `account_registrations` primary key
/// `(wallet_id, account_type, account_index)`, so the BIP32 row overwrote the
/// BIP44 row via `ON CONFLICT DO UPDATE`. The seedless cold-boot reload then
/// read back the depth-1 xpub, it matched no DET sidecar bridge entry, and the
/// fund-routing gate rejected every wallet -> systematic WalletNotLoaded.
///
/// The fix distinguishes the two standard accounts in the persistor key:
/// the label is now `"standard_bip44"` vs `"standard_bip32"`, so both rows
/// coexist and the BIP44 depth-3 xpub survives alongside the BIP32 one.
/// This guard asserts the post-fix invariant: a current-binary wallet
/// survives create -> persist -> real `load_from_persistor_seedless` -> gate,
/// BOTH standard rows persist, and the stored BIP44 xpub matches the bridge.
///
/// It inspects the persistor `account_registrations` directly (a read-only
/// rusqlite connection) rather than reopening an AppContext, because the
/// offline harness can't release the shared `app_kv` advisory lock to reopen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue7_fresh_persistor_bip44_xpub_matches_det_bridge() {
    let _serialize = backend_reopen_lock().await;
    let temp_dir = tempfile::tempdir().expect("tempdir");

    let seed = [0x71u8; 64];
    let (seed_hash, meta_xpub) = {
        // ---- First boot: create + register through the full W1 path ----
        let (ctx, sender) = offline_testnet_context_at(temp_dir.path());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend (first boot)");
        let backend = ctx.wallet_backend().expect("backend wired (first boot)");

        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        let det_master_bip44 = wallet.master_bip44_ecdsa_extended_public_key;

        // Write the wallet-meta sidecar (the seedless bridge key) DIRECTLY —
        // avoid `register_wallet`, which spawns an upstream-registration
        // subtask that keeps an `Arc<WalletBackend>` (and the shared app_kv
        // handle) alive and blocks the cold-boot reopen below.
        backend
            .wallet_meta()
            .set(
                Network::Testnet,
                &seed_hash,
                &crate::model::wallet::meta::WalletMeta {
                    alias: String::new(),
                    is_main: false,
                    core_wallet_name: None,
                    xpub_encoded: det_master_bip44.encode().to_vec(),
                    uses_password: false,
                    password_hint: None,
                },
            )
            .expect("write wallet-meta sidecar");

        // W1 upstream registration via the REAL create_wallet_from_seed_bytes
        // writer (awaited, no spawn). Confirms the FRESH in-memory create
        // resolves through the gate.
        backend
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("W1 upstream registration must succeed on first boot");
        assert!(
            backend.is_wallet_registered(&seed_hash),
            "precondition: a fresh in-memory create must resolve through the gate"
        );
        let meta_xpub = det_master_bip44.encode().to_vec();

        backend.shutdown().await;
        // Drain ctx1's subtasks + drop everything so the persistor + app_kv
        // advisory locks release before the cold-boot reopen.
        let _ = ctx.subtasks.shutdown_async().await;
        drop(backend);
        drop(ctx);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        (seed_hash, meta_xpub)
    };

    // Cold boot over a COPY of the on-disk state: the first context's
    // app_kv/persistor advisory locks can linger in-process, so cold-booting
    // over an identical-bytes copy drives the genuine
    // `load_from_persistor_seedless` inside `WalletBackend::new` without a
    // lock conflict.
    let cold_dir = tempfile::tempdir().expect("cold tempdir");
    copy_dir_recursive(temp_dir.path(), cold_dir.path());

    let cold_boot_registered = {
        let data_dir = cold_dir.path().to_path_buf();
        let app_kv = AppContext::open_app_kv(&data_dir).expect("cold-boot open app k/v");
        let secret_store =
            AppContext::open_secret_store(&data_dir).expect("cold-boot open secret store");
        let db = Arc::new(
            create_database_at_path(&data_dir.join("data.db")).expect("reopen test database"),
        );
        let ctx2 = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("cold-boot AppContext::new");
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender2 = SenderAsync::new(tx, ctx2.egui_ctx().clone());
        // ensure_wallet_backend -> WalletBackend::new runs the real
        // load_from_persistor_seedless pass (builds the bridge from the
        // sidecar, loads the persistor, resolves via the fund-routing gate).
        ctx2.ensure_wallet_backend(sender2)
            .await
            .expect("ensure_wallet_backend (cold boot)");
        let backend2 = ctx2.wallet_backend().expect("backend wired (cold boot)");
        let registered = backend2.is_wallet_registered(&seed_hash);
        backend2.shutdown().await;
        let _ = ctx2.subtasks.shutdown_async().await;
        registered
    };
    let _ = seed_hash;

    // Inspect the persistor on disk directly (a fresh read-only rusqlite
    // connection; SQLite allows concurrent readers, so the lingering app_kv
    // handle on the *other* file does not block this). This shows exactly
    // what the seedless reload would read back for the BIP44 account-0 row —
    // the gate's "loaded" side — without needing a second AppContext.
    let persistor_path = temp_dir
        .path()
        .join("spv")
        .join("testnet")
        .join("platform-wallet.sqlite");
    let conn = rusqlite::Connection::open_with_flags(
        &persistor_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open persistor read-only");
    let rows: Vec<(String, i64, Vec<u8>)> = conn
        .prepare(
            "SELECT account_type, account_index, account_xpub_bytes FROM account_registrations",
        )
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();

    // The seedless reload needs a BIP44 account-0 ("standard_bip44", 0) row
    // to rebuild the watch-only account the gate reads. If it's absent or
    // under a different key, the gate rejects every wallet on a fresh DB.
    // The label is "standard_bip44" (not the pre-fix "standard"): the fix
    // distinguishes the two StandardAccountType variants so the BIP44 row no
    // longer shares a primary key with — and is no longer overwritten by —
    // the BIP32 account-0 row.
    let bip44_0_blob = rows
        .iter()
        .find(|(at, idx, _)| at == "standard_bip44" && *idx == 0)
        .map(|(_, _, blob)| blob.clone());
    assert!(
        bip44_0_blob.is_some(),
        "persistor has no BIP44 account-0 (standard_bip44,0) row after W1. rows={rows:?}"
    );

    // Coexistence guarantee (the heart of the fix): the BIP32 account-0 row
    // must ALSO survive — the collision used to drop one of the two. People
    // hold funds on the BIP32 m/0' account, so it must never be clobbered.
    let bip32_0_present = rows
        .iter()
        .any(|(at, idx, _)| at == "standard_bip32" && *idx == 0);
    assert!(
        bip32_0_present,
        "persistor lost the BIP32 account-0 (standard_bip32,0) row — the collision fix must keep BOTH standard accounts. rows={rows:?}"
    );

    // The gate invariant: the persisted BIP44 account-0 xpub, decoded exactly
    // as the seedless reload does, must equal DET's sidecar bridge xpub —
    // that equality is what the fund-routing gate checks on a cold boot.
    // Before the fix the stored row was the depth-1 BIP32 xpub, which
    // differed and rejected every wallet.
    {
        use platform_wallet::changeset::AccountRegistrationEntry;
        let blob = bip44_0_blob.unwrap();
        let cfg = bincode::config::standard();
        let (entry, _): (AccountRegistrationEntry, usize) =
            bincode::serde::decode_from_slice(&blob, cfg).expect("decode stored entry");
        let stored_xpub_encoded = entry.account_xpub.encode().to_vec();
        assert_eq!(
            stored_xpub_encoded, meta_xpub,
            "stored BIP44 account-0 xpub must match the DET bridge xpub — the fund-routing gate rejects the wallet otherwise"
        );
    }

    // Primary invariant: a current-binary wallet must survive
    // create -> persist -> real load_from_persistor_seedless -> gate. A
    // failure here means the persistor regressed to storing the depth-1
    // BIP32 row, which would resurrect the systematic WalletNotLoaded.
    assert!(
        cold_boot_registered,
        "a current-binary wallet must resolve after cold-boot seedless reload; \
         a failure here resurrects the systematic WalletNotLoaded on a fresh DB"
    );
}

/// `WalletTask::ListTrackedAssetLocks` reads tracked locks off the UI thread
/// through the App Task System. This drives the production dispatch path
/// (`run_backend_task`) for a registered wallet and asserts it returns the
/// typed `TrackedAssetLocks` result — the route the egui frame loop now uses
/// instead of the deleted in-runtime blocking read. A freshly-registered
/// wallet has no locks, so an empty list is the expected, panic-free result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_tracked_asset_locks_task_returns_typed_result() {
    use crate::backend_task::BackendTask;
    use crate::backend_task::BackendTaskSuccessResult;
    use crate::backend_task::wallet::WalletTask;

    let (ctx, sender, _tmp) = offline_testnet_context();

    let seed = [0x9Eu8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    // `run_backend_task` wires the backend on first wallet task and
    // registers the wallet with the upstream manager.
    let result = ctx
        .run_backend_task(
            BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks { seed_hash }),
            sender,
        )
        .await
        .expect("listing tracked asset locks must succeed");

    match result {
        BackendTaskSuccessResult::TrackedAssetLocks {
            seed_hash: got_hash,
            locks,
        } => {
            assert_eq!(
                got_hash, seed_hash,
                "result must carry the requested wallet"
            );
            assert!(
                locks.is_empty(),
                "a freshly-registered wallet has no tracked asset locks"
            );
        }
        other => panic!("expected TrackedAssetLocks, got: {other:?}"),
    }

    ctx.wallet_backend()
        .expect("backend wired")
        .shutdown()
        .await;
}

/// W2 reconciliation (idempotency across the two writers): once a
/// wallet is registered, the W2 `ensure_upstream_registered` path is a
/// no-op — it never re-registers or double-watches. This is the cold-boot
/// bridge's safety property: an already-watched wallet is left untouched
/// while a missing one is filled exactly once.
///
/// The full cross-process cold-boot reload (a fresh `AppContext` over the
/// same persistor re-watching the wallet) and the live below-tip funding
/// repro both require process isolation — DET's `SpvProvider` holds a
/// strong `Arc<AppContext>`, so a second in-process context cannot open the
/// same secret-store vault. Those assertions live in the `#[ignore]`
/// backend-e2e lane (`tests/backend-e2e/wallet_reregistration.rs`), which
/// runs each context in its own workdir slot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ensure_upstream_registered_is_noop_when_already_registered() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    let seed = [0x6Bu8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();

    // W1 registers it once.
    backend
        .register_wallet_from_seed(&seed_hash, &seed, None)
        .await
        .expect("initial registration must succeed");
    assert_eq!(backend.wallet_count().await, 1);

    // W2 over the same, already-registered wallet is a no-op.
    backend
        .ensure_upstream_registered(&seed_hash, &seed)
        .await
        .expect("W2 must be a no-op, not an error, for a registered wallet");
    assert_eq!(
        backend.wallet_count().await,
        1,
        "W2 must not double-watch an already-registered wallet"
    );

    backend.shutdown().await;
}

/// Two subsystems can discover the same unregistered wallet at the same time
/// (the unlock bridge and cold-start bootstrap). They must join one keyed
/// registration flight: exactly one upstream create attempt, with success
/// observed by both callers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_registration_of_one_wallet_is_single_flight() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    let seed = [0x6Cu8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    backend.set_registration_test_barrier(2);

    let first = backend.ensure_upstream_registered(&seed_hash, &seed);
    let second = backend.ensure_upstream_registered(&seed_hash, &seed);
    let (first, second) = tokio::join!(first, second);

    first.expect("first caller must observe registration success");
    second.expect("second caller must observe the same registration success");
    assert_eq!(
        backend.registration_attempt_count(),
        1,
        "only the single-flight leader may call the upstream registration path",
    );
    assert_eq!(backend.wallet_count().await, 1);

    backend.shutdown().await;
}

/// A failed leader result is part of the flight too: followers must not turn
/// the same concurrent discovery into a second upstream attempt or a different
/// result.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_registration_failure_is_shared_by_the_flight() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    let seed = [0x6Eu8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    backend.set_registration_test_barrier(2);
    backend.set_registration_test_failure(true);

    let first = backend.ensure_upstream_registered(&seed_hash, &seed);
    let second = backend.ensure_upstream_registered(&seed_hash, &seed);
    let (first, second) = tokio::join!(first, second);
    let first = first.expect_err("the injected leader failure must reach the first caller");
    let second = second.expect_err("the injected leader failure must reach the follower");

    match (&first, &second) {
        (
            TaskError::WalletRegistrationFlightFailed { source: first },
            TaskError::WalletRegistrationFlightFailed { source: second },
        ) => assert!(
            Arc::ptr_eq(first, second),
            "both callers must observe the exact shared typed failure",
        ),
        other => panic!("expected shared registration-flight errors, got {other:?}"),
    }
    assert_eq!(
        backend.registration_attempt_count(),
        1,
        "a failed flight still permits only one upstream registration attempt",
    );

    backend.shutdown().await;
}

#[test]
fn poisoned_wallet_lock_is_recovered_consistently() {
    let (ctx, _sender, _tmp) = offline_testnet_context();
    let password = Secret::new("poison-test-password");
    let mut wallet = crate::model::wallet::Wallet::new_from_seed(
        [0x6Du8; 64],
        Network::Testnet,
        Some("Poisoned wallet".to_string()),
        Some(&password),
    )
    .expect("build protected wallet");
    wallet.wallet_seed.close();
    let seed_hash = wallet.seed_hash();
    let wallet = Arc::new(RwLock::new(wallet));
    ctx.wallets
        .write_recover()
        .insert(seed_hash, Arc::clone(&wallet));

    let poison_target = Arc::clone(&wallet);
    assert!(
        std::thread::spawn(move || {
            let _guard = poison_target.write().expect("take wallet write lock");
            panic!("poison the wallet lock");
        })
        .join()
        .is_err(),
    );

    assert_eq!(ctx.locked_wallet_hashes(), vec![seed_hash]);
    assert!(ctx.open_wallets().is_empty());
    assert_eq!(
        ctx.unregistered_open_wallet_count(),
        0,
        "the recovered closed wallet is handled by the password prompt, not registration",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unlock_seed_promotion_failure_is_returned_and_wallet_is_relocked() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    let seed = [0x6Eu8; 64];
    let password = Secret::new("correct-wallet-password");
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("Promotion failure".to_string()),
        Some(&password),
    )
    .expect("build protected wallet");
    let (seed_hash, wallet) = ctx
        .register_wallet(wallet, &seed, WalletOrigin::Imported)
        .expect("register protected wallet");
    wallet.write_recover().wallet_seed.close();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("wire wallet backend");

    let incompatible_password =
        platform_wallet_storage::secrets::SecretString::new("different-vault-password");
    WalletSeedView::new(&ctx.secret_store())
        .set_protected(&seed_hash, &seed, &incompatible_password)
        .expect("replace current vault entry with a different password");
    wallet
        .write_recover()
        .wallet_seed
        .open("correct-wallet-password")
        .expect("legacy wallet password opens the in-memory envelope");

    let error = ctx
        .handle_wallet_unlocked(
            &wallet,
            "correct-wallet-password",
            WalletUnlockRetention::UntilAppClose,
        )
        .expect_err("failed vault promotion must be surfaced");
    assert!(
        !wallet.read_recover().is_open(),
        "a wallet whose seed did not land must return to the locked state",
    );
    assert!(
        !error.to_string().is_empty(),
        "the typed error must retain actionable Display text",
    );

    ctx.wallet_backend()
        .expect("backend wired")
        .shutdown()
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tier2_wallet_cold_boot_unlock_uses_the_real_vault_envelope() {
    use platform_wallet_storage::secrets::{
        SecretBytes, SecretStoreError, SecretString, WalletId as SecretWalletId,
    };

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let seed = [0x8du8; 64];
    let password_text = "correct-wallet-password";
    let password_secret = Secret::new(password_text);
    let (first_ctx, _first_sender) = offline_testnet_context_at(temp_dir.path());
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("Cold boot Tier-2".to_string()),
        Some(&password_secret),
    )
    .expect("build protected wallet");
    let (seed_hash, _) = first_ctx
        .register_wallet(wallet, &seed, WalletOrigin::Imported)
        .expect("register protected wallet");
    let password = SecretString::new(password_text);
    WalletSeedView::new(&first_ctx.secret_store())
        .set_protected(&seed_hash, &seed, &password)
        .expect("write Tier-2 envelope");
    drop(first_ctx);

    let cold_boot_dir = tempfile::tempdir().expect("cold boot tempdir");
    copy_dir_recursive(temp_dir.path(), cold_boot_dir.path());
    let (ctx, sender) = offline_testnet_context_at(cold_boot_dir.path());
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("hydrate cold-boot wallet");
    let wallet = ctx.wallet_arc(&seed_hash).expect("hydrated wallet");
    assert!(
        !wallet.read_recover().is_open(),
        "a password-protected Tier-2 wallet must cold-boot locked",
    );

    let wrong = ctx
        .handle_wallet_unlocked(
            &wallet,
            "wrong-wallet-password",
            WalletUnlockRetention::UntilAppClose,
        )
        .expect_err("wrong password must fail");
    assert!(
        matches!(
            &wrong,
            TaskError::SecretSeam { source }
                if matches!(source.as_ref(), SecretStoreError::WrongPassword)
        ),
        "wrong password must retain the WrongPassword taxonomy, got {wrong:?}",
    );
    assert!(!wallet.read_recover().is_open());

    crate::wallet_backend::SecretSeam::new(&ctx.secret_store())
        .put_secret_protected(
            &SecretWalletId::from(seed_hash),
            crate::wallet_backend::secret_access::SEED_RAW_LABEL,
            &SecretBytes::from_slice(&[0x44; 8]),
            &password,
        )
        .expect("write truncated Tier-2 plaintext fixture");
    let malformed = ctx
        .handle_wallet_unlocked(&wallet, password_text, WalletUnlockRetention::UntilAppClose)
        .expect_err("a truncated Tier-2 seed must fail");
    assert!(
        matches!(
            &malformed,
            TaskError::WalletSeedStorage { source }
                if matches!(source.as_ref(), SecretStoreError::MalformedVault)
        ),
        "a genuinely truncated Tier-2 seed must retain the Malformed taxonomy, got {malformed:?}",
    );
    assert!(!wallet.read_recover().is_open());

    WalletSeedView::new(&ctx.secret_store())
        .set_protected(&seed_hash, &seed, &password)
        .expect("restore valid Tier-2 envelope");
    ctx.handle_wallet_unlocked(&wallet, password_text, WalletUnlockRetention::UntilAppClose)
        .expect("correct password must unlock the cold-booted Tier-2 wallet");
    assert!(wallet.read_recover().is_open());

    ctx.wallet_backend()
        .expect("backend wired")
        .shutdown()
        .await;
}

/// A seed unlocked for the storage update must outlive the unlock's own
/// reconciliation subtask, because the update itself is a second consumer of it.
///
/// The unlock gesture spawns `wallet_unlock_registration` (bootstrap + identity
/// discovery) and the storage update independently re-drives
/// `bootstrap_loaded_wallets()` for the same just-unlocked wallet. Both enter the
/// seed scope; neither ordering is guaranteed. When the subtask owned the seed's
/// lifetime outright, finishing first evicted the seed, and the update's pass
/// cache-missed into a background passphrase prompt for a wallet the user had
/// just unlocked — which, if the user ticked "keep unlocked" on that second
/// prompt, also silently restored the session-long retention the migration
/// prompt deliberately withholds.
///
/// Here the unlock subtask is driven to completion *first* (the losing
/// interleaving), and only then does the update's pass run: it must resolve the
/// seed from the session cache, prompting nobody. Releasing the run's lease
/// afterwards must still forget the seed — the unlock does not outlive the update.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_update_seed_outlives_the_unlock_subtask_that_promoted_it() {
    use crate::wallet_backend::SecretScope;
    use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
    use platform_wallet_storage::secrets::SecretString;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let seed = [0x9cu8; 64];
    let password_text = "storage-update-password";
    let password_secret = Secret::new(password_text);
    let (first_ctx, _first_sender) = offline_testnet_context_at(temp_dir.path());
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("Storage update".to_string()),
        Some(&password_secret),
    )
    .expect("build protected wallet");
    let (seed_hash, _) = first_ctx
        .register_wallet(wallet, &seed, WalletOrigin::Imported)
        .expect("register protected wallet");
    let password = SecretString::new(password_text);
    WalletSeedView::new(&first_ctx.secret_store())
        .set_protected(&seed_hash, &seed, &password)
        .expect("write Tier-2 envelope");
    drop(first_ctx);

    // Cold boot: the protected wallet hydrates locked, exactly as it does on the
    // launch that runs the storage update.
    let cold_boot_dir = tempfile::tempdir().expect("cold boot tempdir");
    copy_dir_recursive(temp_dir.path(), cold_boot_dir.path());
    let (ctx, sender) = offline_testnet_context_at(cold_boot_dir.path());

    // A prompt scripted to cancel: any background re-prompt is recorded and then
    // declined, so the test fails on the `ask_count` assertion rather than
    // deadlocking or panicking deep inside the chokepoint.
    let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::Cancel]));
    ctx.install_secret_prompt(Arc::clone(&prompt) as Arc<dyn crate::wallet_backend::SecretPrompt>);

    ctx.ensure_wallet_backend(sender)
        .await
        .expect("hydrate cold-boot wallet");
    let backend = ctx.wallet_backend().expect("backend wired");
    let wallet = ctx.wallet_arc(&seed_hash).expect("hydrated wallet");
    assert!(
        !wallet.read_recover().is_open(),
        "precondition: a password-protected wallet cold-boots locked",
    );

    // The storage update's password prompt, as the popup submits it.
    ctx.handle_wallet_unlocked(
        &wallet,
        password_text,
        WalletUnlockRetention::UntilStorageUpdateComplete,
    )
    .expect("the correct password must unlock the wallet");

    // Let the unlock's reconciliation subtask reach its own seed scope (it
    // registers the wallet upstream from inside it), then join it to completion:
    // the point at which it drops its claim on the seed.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while !backend.is_wallet_registered(&seed_hash) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the unlock subtask must register the wallet upstream from the promoted seed",
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let _ = ctx.subtasks.shutdown_async().await;

    let scope = SecretScope::HdSeed { seed_hash };
    assert!(
        backend.secret_access().can_resolve_without_prompt(&scope),
        "the storage update still needs this seed: the unlock subtask must not have forgotten it",
    );

    // The storage update's own pass over the just-unlocked wallet.
    ctx.bootstrap_loaded_wallets().await;
    assert_eq!(
        prompt.ask_count(),
        0,
        "the storage update must resolve the seed it just prompted for from the session cache",
    );

    // The run ends: its claim on the seed goes with it.
    ctx.migration_status().release_seed_leases();
    assert!(
        !backend.secret_access().can_resolve_without_prompt(&scope),
        "a storage-update unlock must not outlive the storage update",
    );

    backend.shutdown().await;
}

/// Cold-boot lockout regression, at the gesture the owner actually performs:
/// submitting the correct password to the unlock popup.
///
/// A Tier-2 wallet hydrates carrying a secret-free placeholder envelope, so a
/// popup that pre-checks the password against the in-memory wallet model reads
/// that placeholder, reports the wallet as damaged, and locks the owner out of
/// their funds with the CORRECT password. The popup must verify only through
/// the secret chokepoint, which reads the real stored envelope.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tier2_cold_boot_unlock_popup_accepts_the_correct_password() {
    use crate::ui::components::wallet_unlock_popup::{
        UnlockInteraction, UnlockMode, WalletUnlockPopup,
    };
    use platform_wallet_storage::secrets::SecretString;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let seed = [0x3cu8; 64];
    let password_text = "correct-wallet-password";
    let password_secret = Secret::new(password_text);
    let (first_ctx, _first_sender) = offline_testnet_context_at(temp_dir.path());
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("Cold boot popup".to_string()),
        Some(&password_secret),
    )
    .expect("build protected wallet");
    let (seed_hash, _) = first_ctx
        .register_wallet(wallet, &seed, WalletOrigin::Imported)
        .expect("register protected wallet");
    WalletSeedView::new(&first_ctx.secret_store())
        .set_protected(&seed_hash, &seed, &SecretString::new(password_text))
        .expect("write Tier-2 envelope");
    drop(first_ctx);

    let cold_boot_dir = tempfile::tempdir().expect("cold boot tempdir");
    copy_dir_recursive(temp_dir.path(), cold_boot_dir.path());
    let (ctx, sender) = offline_testnet_context_at(cold_boot_dir.path());
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("hydrate cold-boot wallet");
    let wallet = ctx.wallet_arc(&seed_hash).expect("hydrated wallet");
    assert!(
        !wallet.read_recover().is_open(),
        "a password-protected Tier-2 wallet must cold-boot locked",
    );

    let mut popup = WalletUnlockPopup::new();
    popup.open();

    // Acceptance is earned, not blanket: a wrong password still keeps it shut.
    assert_eq!(
        UnlockInteraction::Pending,
        popup.submit_passphrase(&ctx, &wallet, "wrong-wallet-password", UnlockMode::Standard),
        "a wrong password must not unlock a cold-booted Tier-2 wallet",
    );
    assert!(!wallet.read_recover().is_open());

    assert_eq!(
        UnlockInteraction::Unlocked,
        popup.submit_passphrase(&ctx, &wallet, password_text, UnlockMode::Standard),
        "the correct password must unlock a cold-booted Tier-2 wallet through the popup",
    );
    assert!(wallet.read_recover().is_open());

    ctx.wallet_backend()
        .expect("backend wired")
        .shutdown()
        .await;
}

/// Root-cause regression: `register_wallet` persists the
/// seed-envelope sidecar **before** the wallet backend is wired.
///
/// This is the exact ordering the backend-e2e harness uses — register the
/// framework wallet first, wire the backend second. The pre-fix bug was that
/// `write_wallet_sidecars` required `self.wallet_backend()`, so the envelope
/// was never written and the W2 cold-boot bridge could not find a seed to
/// register from. With the vault handle owned by `AppContext`, the write
/// succeeds regardless of wiring order. Reading the envelope back through the
/// shared handle is the assertion that would have failed before the fix.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_wallet_persists_seed_envelope_before_backend_wired() {
    let (ctx, _sender, _tmp) = offline_testnet_context();

    assert!(
        ctx.wallet_backend().is_err(),
        "precondition: the backend must be unwired so we exercise the pre-wire path"
    );

    let seed = [0x7Cu8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("pre-wire".to_string()),
        None,
    )
    .expect("build no-password wallet");
    let (seed_hash, _wallet_arc) = ctx
        .register_wallet(wallet, &seed, WalletOrigin::Imported)
        .expect("register wallet before the backend is wired");

    // A no-password wallet persists the RAW seed via the seam (no legacy
    // envelope), and the xpub rides in the WalletMeta sidecar.
    let raw = WalletSeedView::new(&ctx.secret_store())
        .get_raw(&seed_hash)
        .expect("vault read must not error")
        .expect("the raw seed must be persisted at register time, even unwired");
    assert_eq!(
        &*raw, &seed,
        "persisted raw seed must equal the wallet seed"
    );
    assert!(
        WalletSeedView::new(&ctx.secret_store())
            .legacy_envelope_get(&seed_hash)
            .unwrap()
            .is_none(),
        "no legacy envelope is written for a no-password wallet"
    );
    let meta = WalletMetaView::new(&ctx.app_kv())
        .get(Network::Testnet, &seed_hash)
        .expect("wallet-meta sidecar persisted at register time");
    assert!(!meta.uses_password, "no-password wallet meta flag");
    assert_eq!(
        meta.xpub_encoded,
        ctx.wallets
            .read()
            .unwrap()
            .get(&seed_hash)
            .unwrap()
            .read()
            .unwrap()
            .master_bip44_ecdsa_extended_public_key
            .encode()
            .to_vec(),
        "the persisted xpub must match the registered wallet's BIP44 account xpub"
    );
}

/// End-to-end on the harness ordering: a wallet registered
/// **before** the backend is wired is registered with the upstream SPV
/// manager once the backend comes up — the W2 cold-boot bridge fires from
/// the seed envelope persisted at register time.
///
/// This is the in-process half of the live repro: it proves the chain from
/// the persisted envelope through `bootstrap_loaded_wallets` →
/// `bootstrap_wallet_addresses_jit` → `ensure_upstream_registered` without a
/// launch-time prompt (the wallet is unprotected, so the chokepoint's
/// no-passphrase fast-path resolves the seed). The funded below-tip balance
/// assertion needs a live testnet and lives in the `#[ignore]` backend-e2e
/// lane.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wallet_registered_before_wiring_is_upstream_registered_on_cold_boot() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    let seed = [0x8Du8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("cold-boot-bridge".to_string()),
        None,
    )
    .expect("build no-password wallet");
    let (seed_hash, _wallet_arc) = ctx
        .register_wallet(wallet, &seed, WalletOrigin::Imported)
        .expect("register wallet before wiring");

    // Wiring runs hydration + the cold-boot bootstrap, which drives the W2
    // bridge from the now-persisted seed envelope.
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    assert!(
        backend.is_wallet_registered(&seed_hash),
        "the wallet must be upstream-registered by the W2 bridge after wiring"
    );
    assert_eq!(
        backend.wallet_count().await,
        1,
        "exactly one wallet must be watched after the cold-boot bridge runs"
    );

    backend.shutdown().await;
}

/// `unregistered_open_wallet_count` must count a wallet whose
/// `RwLock` is poisoned, so a prior panic can never let a premature
/// "completed" sentinel through. The previous implementation counted over
/// the `open_wallets()` snapshot, which drops a poisoned-lock wallet
/// (`read().ok()...unwrap_or(false)`) before the fail-safe could see it —
/// that version returns 0 here and fails this test.
#[tokio::test]
async fn unregistered_count_fails_safe_on_poisoned_wallet_lock() {
    let (ctx, _sender, _tmp) = offline_testnet_context();

    // One wallet, inserted straight into the map (no backend wired).
    let wallet =
        crate::model::wallet::Wallet::new_from_seed([0x42u8; 64], Network::Testnet, None, None)
            .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    let arc = Arc::new(std::sync::RwLock::new(wallet));
    ctx.wallets
        .write()
        .expect("wallets map lock")
        .insert(seed_hash, Arc::clone(&arc));

    // Poison the wallet's lock by panicking while holding its write guard.
    let poisoner = Arc::clone(&arc);
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.write().expect("acquire write lock");
        panic!("intentional poison for the fail-safe test");
    })
    .join();
    assert!(
        arc.read().is_err(),
        "precondition: the wallet lock must be poisoned",
    );

    assert_eq!(
        ctx.unregistered_open_wallet_count(),
        1,
        "a poisoned wallet lock must fail safe (counted), not be silently dropped",
    );
}

/// Fresh-install regression: on a truly-fresh install the real
/// `Database::initialize` path gates the legacy `wallet`/`wallet_addresses`
/// tables OUT, so `register_wallet` must not depend on them. The pre-fix
/// `store_wallet_with_addresses` ran an unguarded `INSERT INTO wallet` that
/// failed with `no such table: wallet`, so `register_wallet` returned `Err`
/// before any in-memory registration — fresh installs could never create or
/// import a wallet. This drives the exact production path and asserts success
/// plus in-memory registration.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_wallet_succeeds_on_fresh_install_without_legacy_tables() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let (ctx, _sender) = offline_testnet_context_fresh_init(temp_dir.path());

    // Precondition: the fresh-install schema must NOT carry the legacy
    // wallet table — this is the state that exposed the bug. Querying it
    // surfaces sqlite's "no such table: wallet" error.
    let probe = ctx.db.get_wallets(&Network::Testnet);
    assert!(
        probe.is_err(),
        "precondition: fresh install must not create the legacy `wallet` table"
    );

    let seed = [0x9Eu8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("fresh-install".to_string()),
        None,
    )
    .expect("build no-password wallet");
    let seed_hash = wallet.seed_hash();

    let (returned_hash, _wallet_arc) = ctx
        .register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register_wallet must succeed on a fresh install");
    assert_eq!(returned_hash, seed_hash);

    assert!(
        ctx.wallets.read_recover().contains_key(&seed_hash),
        "the wallet must be registered in-memory after register_wallet"
    );
    assert!(
        ctx.has_wallet.load(Ordering::Relaxed),
        "the has_wallet flag must flip true after a successful registration"
    );
}

/// Removing a wallet wipes its secret-bearing state: the encrypted
/// seed-envelope vault entry. Orchard state lives in the upstream
/// coordinator and is detached on removal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_wallet_wipes_seed_envelope() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let seed = [0xA1u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    let backend = ctx.wallet_backend().expect("backend wired");

    // Precondition: the raw seed is present (no-password wallet stores raw).
    assert!(
        WalletSeedView::new(&ctx.secret_store())
            .get_raw(&seed_hash)
            .expect("vault read")
            .is_some(),
        "precondition: the raw seed must exist before removal"
    );

    ctx.remove_wallet(&seed_hash).expect("remove wallet");

    // The seed (the JIT decrypt source) is gone in BOTH forms.
    let store = ctx.secret_store();
    let view = WalletSeedView::new(&store);
    assert!(
        view.get_raw(&seed_hash)
            .expect("raw read after removal")
            .is_none(),
        "the raw seed must be deleted from the vault on removal"
    );
    assert!(
        view.legacy_envelope_get(&seed_hash)
            .expect("legacy read after removal")
            .is_none(),
        "any legacy envelope must also be gone on removal"
    );

    backend.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_wallet_warns_when_local_secret_wipe_fails() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let seed = [0xA4u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    let backend = ctx.wallet_backend().expect("backend wired");
    backend.set_forget_wallet_local_state_test_failure(true);
    crate::ui::components::MessageBanner::clear_all_global(ctx.egui_ctx());

    ctx.remove_wallet(&seed_hash).expect("remove wallet");

    assert!(
        crate::ui::components::MessageBanner::has_global(ctx.egui_ctx()),
        "a local secret-wipe failure must raise a user-visible warning"
    );

    let _ = ctx.subtasks.shutdown_async().await;
    backend.shutdown().await;
}

/// Removing a wallet drives upstream's native viewing-key cascade.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_wallet_reaps_persisted_shielded_viewing_keys() {
    let _guard = backend_reopen_lock().await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let seed = [0xA2u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    let (ctx, sender) = offline_testnet_context_at(source_dir.path());

    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("wire wallet backend");
    let backend = ctx.wallet_backend().expect("backend wired");
    backend
        .register_wallet_from_seed(&seed_hash, &seed, Some(0))
        .await
        .expect("persist upstream wallet");
    backend
        .ensure_shielded_bound(&seed_hash, &seed)
        .await
        .expect("persist wallet viewing key");
    let wallet_id = backend
        .registered_wallet_id(&seed_hash)
        .expect("registered upstream wallet id");
    let persister_path = source_dir
        .path()
        .join("spv")
        .join("testnet")
        .join("platform-wallet.sqlite");
    let count_viewing_keys = || {
        rusqlite::Connection::open_with_flags(
            &persister_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("open persister inspection connection")
        .query_row(
            "SELECT COUNT(*) FROM shielded_viewing_keys WHERE wallet_id = ?1",
            [wallet_id.as_slice()],
            |row| row.get::<_, i64>(0),
        )
        .expect("count persisted viewing keys")
    };
    assert_eq!(count_viewing_keys(), 1, "precondition: native FVK row");

    ctx.remove_wallet(&seed_hash).expect("remove wallet");
    let _ = ctx.subtasks.shutdown_async().await;

    assert_eq!(
        count_viewing_keys(),
        0,
        "upstream deletion must cascade to the native FVK row"
    );
    assert!(
        !source_dir
            .path()
            .join("spv")
            .join("testnet")
            .join("backups")
            .join("auto")
            .exists(),
        "explicit wallet removal must not create an automatic backup"
    );
    backend.shutdown().await;
}

/// A failed upstream delete must keep the native FVK row and warn the user
/// that wallet data may remain on disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_wallet_warns_when_persisted_shielded_viewing_key_delete_fails() {
    let _guard = backend_reopen_lock().await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let seed = [0xA3u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    let (ctx, sender) = offline_testnet_context_at(source_dir.path());

    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("wire wallet backend");
    let backend = ctx.wallet_backend().expect("backend wired");
    backend
        .register_wallet_from_seed(&seed_hash, &seed, Some(0))
        .await
        .expect("persist upstream wallet");
    backend
        .ensure_shielded_bound(&seed_hash, &seed)
        .await
        .expect("persist wallet viewing key");
    let wallet_id = backend
        .registered_wallet_id(&seed_hash)
        .expect("registered upstream wallet id");
    let persister_path = source_dir
        .path()
        .join("spv")
        .join("testnet")
        .join("platform-wallet.sqlite");
    let connection =
        rusqlite::Connection::open(&persister_path).expect("open persister fault injector");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_wallet_delete \
             BEFORE DELETE ON wallets \
             BEGIN \
                 SELECT RAISE(ABORT, 'forced wallet delete failure'); \
             END;",
        )
        .expect("install wallet-delete failure trigger");
    crate::ui::components::MessageBanner::clear_all_global(ctx.egui_ctx());

    ctx.remove_wallet(&seed_hash).expect("remove DET wallet");
    let _ = ctx.subtasks.shutdown_async().await;

    let viewing_key_count = connection
        .query_row(
            "SELECT COUNT(*) FROM shielded_viewing_keys WHERE wallet_id = ?1",
            [wallet_id.as_slice()],
            |row| row.get::<_, i64>(0),
        )
        .expect("count retained viewing keys");
    assert_eq!(
        viewing_key_count, 1,
        "the failed wallet delete must leave its cascaded FVK row in place"
    );
    assert!(
        crate::ui::components::MessageBanner::has_global(ctx.egui_ctx()),
        "the asynchronous deletion failure must raise a user-visible warning"
    );

    backend.shutdown().await;
}

/// The receive-address snapshot is empty until a bind publishes into it, so a
/// wallet that is locked or not yet bound reports `None` and the Shielded tab
/// falls back to its "not ready yet" copy instead of rendering a wrong address.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shielded_receive_address_is_none_before_bind() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let seed = [0xC4u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    let backend = ctx.wallet_backend().expect("backend wired");

    assert_eq!(
        ctx.shielded_receive_address(&seed_hash),
        None,
        "an unregistered, unbound wallet must not surface any receive address"
    );

    backend.shutdown().await;
}

/// The published receive address is the Orchard **account-0 external** address
/// of the wallet's own seed — the exact key material `bind_shielded` hands the
/// coordinator to scan with, and the only account DET's spend path can spend
/// from.
///
/// This is the funds-safety contract of the whole bridge: we assert the cached
/// string against an independently ZIP-32-derived expectation, so a future
/// change that publishes some *other* account, scope, or diversifier — an
/// address the wallet could be paid at but never detect or spend — fails here
/// rather than silently costing a user their money.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_shielded_receive_address_publishes_bound_account_zero_address() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    let seed = [0xD7u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    // Register BEFORE wiring the backend: with no backend yet,
    // `register_wallet_upstream` finds none and skips the fire-and-forget
    // `wallet_upstream_registration` subtask. Wiring first would spawn that
    // subtask, which then races the explicit `ensure_upstream_registered`
    // below — both call `create_wallet_from_seed_bytes`, the loser sees
    // `WalletAlreadyExists` then `get_wallet` returns `None` in the insert gap
    // → `WalletNotFound` (reliably under CI load). This ordering makes
    // `ensure_upstream_registered` the single upstream writer.
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let backend = ctx.wallet_backend().expect("backend wired");
    // Mirror `bootstrap_wallet_addresses_jit`'s ordering: a wallet must be
    // registered upstream before its Orchard keys can bind.
    backend
        .ensure_upstream_registered(&seed_hash, &seed)
        .await
        .expect("register wallet upstream");
    backend
        .ensure_shielded_bound(&seed_hash, &seed)
        .await
        .expect("bind Orchard keys offline");
    assert!(
        backend
            .bind_shielded_from_persisted_for_test(&seed_hash)
            .await
            .expect("restore the viewing key through upstream persistence"),
        "the seed-backed bind must persist a restorable viewing key"
    );

    ctx.cache_shielded_receive_address(&backend, &seed_hash)
        .await;

    let expected_raw =
        platform_wallet::wallet::shielded::OrchardKeySet::from_seed(&seed, Network::Testnet, 0)
            .expect("ZIP-32 derivation")
            .address_at(0)
            .to_raw_address_bytes();
    let expected = crate::model::address::encode_shielded_address(&expected_raw, Network::Testnet)
        .expect("encode expected address");

    assert_eq!(
        ctx.shielded_receive_address(&seed_hash),
        Some(expected),
        "the receive address must be account 0's external address, derived from the bound keys"
    );

    backend.shutdown().await;
}

/// Removing a wallet evicts its shielded receive address. The seed hash is
/// deterministic, so without eviction a re-import of the same phrase — or any
/// later read — could surface a removed wallet's address as a live payment
/// destination.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_wallet_evicts_shielded_receive_address() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    let seed = [0xE9u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    // Register BEFORE wiring the backend so `register_wallet_upstream` skips the
    // fire-and-forget `wallet_upstream_registration` subtask; otherwise it races
    // the explicit `ensure_upstream_registered` below (both call
    // `create_wallet_from_seed_bytes`; the loser hits `WalletAlreadyExists` then
    // a `None` `get_wallet` in the insert gap → `WalletNotFound` under CI load).
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let backend = ctx.wallet_backend().expect("backend wired");
    // Mirror `bootstrap_wallet_addresses_jit`'s ordering: a wallet must be
    // registered upstream before its Orchard keys can bind.
    backend
        .ensure_upstream_registered(&seed_hash, &seed)
        .await
        .expect("register wallet upstream");
    backend
        .ensure_shielded_bound(&seed_hash, &seed)
        .await
        .expect("bind Orchard keys offline");
    ctx.cache_shielded_receive_address(&backend, &seed_hash)
        .await;
    assert!(
        ctx.shielded_receive_address(&seed_hash).is_some(),
        "precondition: the address must be published before removal"
    );

    ctx.remove_wallet(&seed_hash).expect("remove wallet");

    assert_eq!(
        ctx.shielded_receive_address(&seed_hash),
        None,
        "the receive address must be evicted on wallet removal"
    );

    backend.shutdown().await;
}

/// Removing a wallet evicts its shielded balance snapshot from
/// `AppContext::shielded_balances`. The seed hash is deterministic from the
/// seed, so without eviction a re-import of the same recovery phrase would
/// surface the removed wallet's stale shielded balance until the next sync
/// overwrote it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_wallet_evicts_shielded_balance_snapshot() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let seed = [0xB2u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    let backend = ctx.wallet_backend().expect("backend wired");

    // Seed a snapshot entry as the sync-completed push writer would.
    ctx.shielded_balances
        .lock()
        .expect("lock shielded_balances")
        .insert(seed_hash, 123_456);
    assert_eq!(
        ctx.shielded_balance_credits(&seed_hash),
        123_456,
        "precondition: the snapshot entry must exist before removal"
    );

    ctx.remove_wallet(&seed_hash).expect("remove wallet");

    assert!(
        ctx.shielded_balances
            .lock()
            .expect("lock shielded_balances")
            .get(&seed_hash)
            .is_none(),
        "the shielded balance snapshot must be evicted on removal"
    );

    backend.shutdown().await;
}

/// F17/F20 (fresh-install regression): removing a wallet must still wipe
/// its secret-bearing state on a truly-fresh install where the legacy
/// `wallet`/`wallet_addresses`/`utxos` tables are gated OUT of the schema.
///
/// The sibling `remove_wallet_wipes_seed_envelope` builds its context with
/// `create_tables(true)`, which force-creates those legacy tables and masks the
/// fresh-install shape. Removal now operates only on current stores, so it must
/// succeed without consulting or changing any pre-update table.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_wallet_wipes_secrets_on_fresh_install_without_legacy_tables() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let (ctx, sender) = offline_testnet_context_fresh_init(temp_dir.path());

    // Precondition: the fresh-install schema must NOT carry the legacy
    // `wallet_addresses` table — querying it surfaces sqlite's
    // "no such table: wallet" error from `get_wallets`. This is the state
    // that current-store removal must handle without consulting legacy state.
    assert!(
        ctx.db.get_wallets(&Network::Testnet).is_err(),
        "precondition: fresh install must not create the legacy wallet tables"
    );

    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let seed = [0xF6u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    let backend = ctx.wallet_backend().expect("backend wired");

    // Precondition: the raw seed exists.
    assert!(
        WalletSeedView::new(&ctx.secret_store())
            .get_raw(&seed_hash)
            .expect("vault read")
            .is_some(),
        "precondition: the raw seed must exist before removal"
    );

    ctx.remove_wallet(&seed_hash)
        .expect("remove_wallet must succeed on a fresh install");

    let store = ctx.secret_store();
    let view = WalletSeedView::new(&store);
    assert!(
        view.get_raw(&seed_hash)
            .expect("raw read after removal")
            .is_none(),
        "the raw seed must be deleted from the vault on a fresh install"
    );
    assert!(
        view.legacy_envelope_get(&seed_hash)
            .expect("legacy read after removal")
            .is_none(),
        "no legacy envelope must survive removal on a fresh install"
    );

    backend.shutdown().await;
}

/// F60 — "delete all local data" must leave no wallet recoverable: the
/// wallet-meta sidecar (which the cold-boot picker reads) and the
/// seed-envelope vault (which holds the encrypted seed) must both be
/// empty. Before the fix, `clear_network_database` cleared only legacy
/// data.db + the in-memory maps, so wallets rehydrated on next launch and
/// encrypted seeds persisted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_network_database_wipes_wallet_meta_and_seed_envelope() {
    use crate::backend_task::BackendTaskSuccessResult;
    use crate::backend_task::system_task::SystemTask;

    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender.clone())
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let seed = [0xB2u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    // Preconditions: both the meta sidecar and the seed envelope exist.
    assert!(
        WalletMetaView::new(&ctx.app_kv())
            .get(Network::Testnet, &seed_hash)
            .is_some(),
        "precondition: wallet-meta sidecar must exist before clear"
    );
    assert!(
        WalletSeedView::new(&ctx.secret_store())
            .get_raw(&seed_hash)
            .expect("vault read")
            .is_some(),
        "precondition: raw seed must exist before clear"
    );

    let result = ctx
        .run_system_task(SystemTask::ClearNetworkDatabase, sender)
        .await
        .expect("clear_network_database should succeed");
    assert!(
        matches!(
            result,
            BackendTaskSuccessResult::NetworkDatabaseCleared {
                network: Network::Testnet
            }
        ),
        "clear completion must identify the network that was erased"
    );

    // The wallet must not rehydrate: its meta and seed (both forms) are gone.
    assert!(
        WalletMetaView::new(&ctx.app_kv())
            .get(Network::Testnet, &seed_hash)
            .is_none(),
        "wallet-meta sidecar must be empty after clear (no rehydration)"
    );
    let store = ctx.secret_store();
    let view = WalletSeedView::new(&store);
    assert!(
        view.get_raw(&seed_hash)
            .expect("raw read after clear")
            .is_none(),
        "raw seed must be deleted from the vault after clear"
    );
    assert!(
        view.legacy_envelope_get(&seed_hash)
            .expect("legacy read after clear")
            .is_none(),
        "no legacy envelope must survive clear"
    );
    assert!(
        ctx.wallets.read_recover().is_empty(),
        "the in-memory wallet map must be empty after clear"
    );

    ctx.wallet_backend()
        .expect("backend wired")
        .shutdown()
        .await;
}

/// "Delete all local data" must also wipe every local identity's private keys.
/// Identity keys are Tier-1 keyless (plaintext-recoverable) and include
/// masternode voting/owner/payout keys, so a clear that skipped them would
/// leave fund-control keys recoverable on disk after the user asked to erase.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_network_database_wipes_local_identity_private_keys() {
    use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{
        IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
    };
    use crate::wallet_backend::IdentityKeyView;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, IdentityPublicKey};
    use std::collections::BTreeMap;

    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    // A User identity carrying one plaintext (Clear) private key.
    let pv = PlatformVersion::latest();
    let key = IdentityPublicKey::random_key(1, Some(1), pv);
    let key_id = key.id();
    let mut private_keys = KeyStorage::default();
    private_keys.private_keys.insert(
        (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id),
        (
            QualifiedIdentityPublicKey::from(key),
            PrivateKeyData::Clear([0x5Au8; 32]),
        ),
    );
    let identity_id = Identifier::from([0x33u8; 32]);
    let identity = Identity::create_basic_identity(identity_id, pv).expect("basic identity");
    let qi = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: None,
        private_keys,
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: Network::Testnet,
    };

    // Vault-first insert: the Clear key moves into the vault and the record
    // carries an InVault placeholder.
    ctx.insert_local_qualified_identity(&qi, &None)
        .expect("persist local identity");

    let store = ctx.secret_store();
    let view = IdentityKeyView::new(&store, identity_id.to_buffer());

    assert_eq!(
        ctx.local_identity_ids().expect("list ids before clear"),
        vec![identity_id],
        "precondition: the identity is stored locally before clear"
    );
    assert!(
        view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id)
            .expect("vault read before clear")
            .is_some(),
        "precondition: the identity private key is in the vault before clear"
    );

    ctx.clear_network_database()
        .await
        .expect("clear_network_database should succeed");

    assert!(
        ctx.local_identity_ids()
            .expect("list ids after clear")
            .is_empty(),
        "clear must remove every local identity record"
    );
    assert!(
        view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id)
            .expect("vault read after clear")
            .is_none(),
        "clear must wipe the identity private key from the vault"
    );

    ctx.wallet_backend()
        .expect("backend wired")
        .shutdown()
        .await;
}

/// A masternode removal must report an incomplete clear when its voting,
/// owner, or payout key cannot be deleted from the vault.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_network_database_reports_incomplete_when_masternode_key_delete_fails() {
    use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{
        IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
    };
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::Purpose;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
        IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
    };
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, IdentityPublicKey};
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    let (ctx, sender, tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let pv = PlatformVersion::latest();
    let identity_id = Identifier::from([0x73u8; 32]);
    let mut private_keys = KeyStorage::default();
    let key_specs = [
        (
            1,
            Purpose::VOTING,
            PrivateKeyTarget::PrivateKeyOnVoterIdentity,
        ),
        (
            2,
            Purpose::OWNER,
            PrivateKeyTarget::PrivateKeyOnMainIdentity,
        ),
        (
            3,
            Purpose::TRANSFER,
            PrivateKeyTarget::PrivateKeyOnMainIdentity,
        ),
    ];
    for (key_id, purpose, target) in key_specs {
        let mut key = IdentityPublicKey::random_key(key_id, Some(1), pv);
        key.set_purpose(purpose);
        private_keys.private_keys.insert(
            (target, key.id()),
            (
                QualifiedIdentityPublicKey::from(key),
                PrivateKeyData::Clear([0x70 + key_id as u8; 32]),
            ),
        );
    }
    let identity = Identity::create_basic_identity(identity_id, pv).expect("basic identity");
    let qi = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: Some(2),
        identity_type: IdentityType::Masternode,
        alias: Some("Removal failure masternode".to_string()),
        private_keys,
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: Network::Testnet,
    };
    ctx.insert_local_qualified_identity(&qi, &None)
        .expect("persist masternode identity");

    let secrets_dir = tmp.path().join("secrets");
    std::fs::set_permissions(&secrets_dir, std::fs::Permissions::from_mode(0o500))
        .expect("make vault directory read-only");
    let result = ctx.clear_network_database().await;
    std::fs::set_permissions(&secrets_dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore vault directory permissions");

    ctx.wallet_backend()
        .expect("backend wired")
        .shutdown()
        .await;

    match result {
        Err(TaskError::WalletDataClearIncomplete {
            failed,
            first_error,
        }) => {
            assert!(failed >= 1, "at least one vault-key delete must fail");
            assert!(
                matches!(*first_error, TaskError::IdentityKeyVault { .. }),
                "the first failure must preserve the identity-vault error chain"
            );
        }
        other => panic!("masternode key deletion failure must make clear incomplete: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_network_database_reports_incomplete_when_shielded_clear_fails() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");
    backend.set_clear_shielded_test_failure(true);

    let result = ctx.clear_network_database().await;

    backend.shutdown().await;
    match result {
        Err(TaskError::WalletDataClearIncomplete {
            failed,
            first_error,
        }) => {
            assert_eq!(failed, 1, "the shielded clear should be the only failure");
            assert!(
                matches!(*first_error, TaskError::WalletDataClearUnavailable),
                "the aggregate must preserve the shielded clear error"
            );
        }
        other => panic!("shielded clear failure must make clear incomplete: {other:?}"),
    }
}

/// Clear-all must fail before changing any state when the wallet backend is
/// unavailable, because persisted secrets from an earlier run may still exist.
#[tokio::test]
async fn clear_network_database_refuses_unwired_backend_without_partial_wipe() {
    let (ctx, _sender, _tmp) = offline_testnet_context();
    let seed = [0xB3u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("persist wallet before backend wiring");

    let result = ctx.clear_network_database().await;

    assert!(
        matches!(result, Err(TaskError::WalletDataClearUnavailable)),
        "clear-all must return the dedicated clear-unavailable error"
    );
    assert!(
        ctx.wallets.read_recover().contains_key(&seed_hash),
        "a refused clear must not partially remove the in-memory wallet"
    );
    assert!(
        WalletMetaView::new(&ctx.app_kv())
            .get(Network::Testnet, &seed_hash)
            .is_some(),
        "a refused clear must preserve wallet metadata for a later retry"
    );
    assert!(
        WalletSeedView::new(&ctx.secret_store())
            .get_raw(&seed_hash)
            .expect("vault read after refused clear")
            .is_some(),
        "a refused clear must preserve the seed so the caller can retry safely"
    );
}

/// F131 — locking a wallet must wipe the session-cached seed. Before the
/// fix `handle_wallet_locked` was an empty no-op, so after an
/// `UntilAppClose` unlock the plaintext seed stayed resident and the wallet
/// kept signing with no prompt despite being "locked".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lock_wipes_session_cached_seed() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let seed = [0xC3u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    let (_seed_hash, wallet_arc) = ctx
        .register_wallet(wallet, &seed, WalletOrigin::Fresh)
        .expect("register wallet");

    let backend = ctx.wallet_backend().expect("backend wired");
    let scope = crate::wallet_backend::SecretScope::HdSeed { seed_hash };

    // Promote the seed into the session cache (what an UntilAppClose unlock
    // leaves behind).
    let held = zeroize::Zeroizing::new(seed);
    backend.secret_access().remember_session(
        &scope,
        crate::wallet_backend::SecretPlaintext::HdSeed(&held),
        crate::wallet_backend::RememberPolicy::UntilAppClose,
    );
    assert!(
        backend.secret_access().is_session_cached(&scope),
        "precondition: the seed is session-cached before the lock"
    );

    ctx.handle_wallet_locked(&wallet_arc);

    assert!(
        !backend.secret_access().is_session_cached(&scope),
        "locking must wipe the session-cached seed"
    );

    backend.shutdown().await;
}

/// F62 — when the seed-envelope vault write fails, `register_wallet` must
/// FAIL CLOSED: return `Err` and NOT keep the wallet. The envelope is the
/// encrypted seed the W2 cold-boot bridge re-registers from, so silently
/// keeping an in-session wallet whose seed was never saved would lose the
/// wallet and its funds at the next launch. Before the fix the envelope
/// write was best-effort (warn + Ok), so the wallet was kept regardless.
///
/// Induces the write failure permission-free by replacing the vault file
/// with a directory: the store's atomic `persist` rename onto a directory
/// path fails deterministically (root cannot bypass this).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_wallet_fails_closed_when_seed_envelope_write_fails() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let (ctx, _sender) = offline_testnet_context_at(temp_dir.path());

    // Replace the resident vault file with a directory so the next vault
    // write (the atomic persist rename) fails.
    let vault_path = temp_dir.path().join("secrets").join("det-secrets.pwsvault");
    std::fs::remove_file(&vault_path).expect("remove vault file");
    std::fs::create_dir(&vault_path).expect("plant directory at vault path");

    let seed = [0xD4u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();

    let result = ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh);
    assert!(
        result.is_err(),
        "register_wallet must fail closed when the seed envelope cannot be saved"
    );
    assert!(
        !ctx.wallets.read_recover().contains_key(&seed_hash),
        "a wallet whose seed was not saved must not be kept in memory"
    );
    assert!(
        !ctx.has_wallet.load(Ordering::Relaxed),
        "has_wallet must not flip true when registration fails closed"
    );
}

/// When the wallet-meta sidecar write fails, `register_wallet`
/// must FAIL CLOSED: return `Err` and NOT keep the wallet. Cold-boot
/// hydration (`hydrate_wallets_for_network`) enumerates ONLY the meta
/// sidecar — `ctx.wallets` is rebuilt solely from `WalletMetaView::list`.
/// A wallet whose seed envelope was saved but whose meta row is missing is
/// never hydrated, so its funds become unreachable with no self-heal (there
/// is no upstream→meta reconstruction path). Both sidecars are required, so
/// the meta write must be fail-closed just like the seed-envelope write.
///
/// Induces the meta-write failure permission-free by dropping the
/// `meta_global` table from `det-app.sqlite` (which backs `app_kv`) through
/// a second connection: the next `WalletMetaView::set` upsert errors with
/// "no such table", deterministically, with no filesystem race.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_wallet_fails_closed_when_wallet_meta_write_fails() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let (ctx, _sender) = offline_testnet_context_at(temp_dir.path());

    // Drop the table the wallet-meta sidecar upserts into, so the next
    // `WalletMetaView::set` fails. The persister holds its own connection;
    // a second connection to the same file is enough to drop the shared
    // schema object.
    {
        let meta_db = temp_dir.path().join("det-app.sqlite");
        let conn = rusqlite::Connection::open(&meta_db).expect("open det-app.sqlite second handle");
        conn.execute("DROP TABLE meta_global", [])
            .expect("drop meta_global to force the wallet-meta write to fail");
    }

    let seed = [0x17u8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();

    let result = ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh);
    assert!(
        result.is_err(),
        "register_wallet must fail closed when the wallet-meta sidecar cannot be saved"
    );
    assert!(
        !ctx.wallets.read_recover().contains_key(&seed_hash),
        "a wallet with no meta row must not be kept in memory (it would never hydrate)"
    );
    assert!(
        !ctx.has_wallet.load(Ordering::Relaxed),
        "has_wallet must not flip true when registration fails closed"
    );
}

/// An overlong alias is pure input validation and MUST be rejected BEFORE any
/// secret-critical write. Otherwise a meta-write-time rejection lands AFTER
/// `write_seed_envelope`, orphaning the encrypted seed (no meta row → never
/// hydrated, no cleanup path). Mirrors the single-key import path, which
/// already validates before writing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_wallet_rejects_overlong_alias_before_seed_write() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let (ctx, _sender) = offline_testnet_context_at(temp_dir.path());

    let seed = [0x5Au8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(
        seed,
        Network::Testnet,
        Some("w".repeat(65)),
        None,
    )
    .expect("build wallet");
    let seed_hash = wallet.seed_hash();

    let result = ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh);
    assert!(
        matches!(result, Err(TaskError::InvalidWalletAliasLength { .. })),
        "an overlong alias must be rejected before any seed write"
    );
    assert!(
        WalletSeedView::new(&ctx.secret_store())
            .get_raw(&seed_hash)
            .expect("read raw seed")
            .is_none(),
        "no seed material must survive a rejected HD registration (orphaned secret)"
    );
    assert!(
        !ctx.wallets.read_recover().contains_key(&seed_hash),
        "a rejected wallet must not be kept in memory"
    );
    assert!(
        !ctx.has_wallet.load(Ordering::Relaxed),
        "has_wallet must not flip true when registration is rejected"
    );
}

/// Build a valid BIP44 account-0 master xpub (testnet) for a legacy wallet row.
fn legacy_master_epk_bytes(seed: &[u8; 64]) -> Vec<u8> {
    crate::database::test_helpers::legacy_master_epk_bytes(seed, Network::Testnet)
}

/// F140 — a wallet migrated from legacy `data.db` must be visible right
/// after the migration completes, NOT only after a second restart. The bug:
/// `WalletBackend::new` runs `hydrate_context_wallets` against the still-
/// empty sidecars at first boot; migration then populates the sidecars but
/// never re-hydrates `ctx.wallets`, so the in-memory map stays empty until
/// the next launch reads the now-populated sidecars. The fix re-hydrates at
/// the end of a successful migration.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migrated_wallet_is_visible_without_second_restart() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    // Seed a legacy `wallet` row with a valid xpub so the migration's
    // seed + meta passes produce a hydratable wallet.
    use crate::database::test_helpers::seed_legacy_unprotected_hd_wallet_row;
    let seed = [0xE5u8; 64];
    let seed_hash: WalletSeedHash = crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
    let epk = legacy_master_epk_bytes(&seed);
    seed_legacy_unprotected_hd_wallet_row(
        &ctx.db,
        &seed_hash,
        &seed,
        &epk,
        "migrated-wallet",
        Network::Testnet,
    )
    .expect("insert legacy wallet row");

    // Wire the backend: hydration runs now, against the EMPTY sidecars
    // (migration has not run yet), so ctx.wallets is empty.
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    assert!(
        !ctx.wallets.read_recover().contains_key(&seed_hash),
        "precondition: the migrated wallet is not yet hydrated (sidecars empty at wiring)"
    );

    // Run the migration. It populates the sidecars AND now re-hydrates.
    crate::backend_task::migration::finish_unwire::run(&ctx)
        .await
        .expect("migration should succeed");

    // The migrated wallet must be visible WITHOUT a second backend build.
    assert!(
        ctx.wallets.read_recover().contains_key(&seed_hash),
        "the migrated wallet must be in ctx.wallets right after migration (no second restart)"
    );
    assert!(
        ctx.has_wallet.load(Ordering::Relaxed),
        "has_wallet must be true after a migrated wallet is hydrated"
    );

    ctx.wallet_backend()
        .expect("backend wired")
        .shutdown()
        .await;
}

/// F140 (resolve half) — a wallet migrated from legacy `data.db` at cold
/// start must be RESOLVABLE through the wallet backend right after the
/// migration completes, NOT only after a second restart. The bug: the
/// post-migration re-hydration (`hydrate_context_wallets`) refills
/// `ctx.wallets` (so the wallet shows in the picker and addresses resolve),
/// but it never re-runs the W2 cold-boot reconciliation
/// (`bootstrap_loaded_wallets` → `ensure_upstream_registered`). So the
/// upstream `id_map` stays empty and every seed-keyed operation
/// (`resolve_wallet`) returns `WalletNotLoaded` until the next launch —
/// exactly the "wallet still loading" banner that repeats forever in the
/// field report. The companion F140 test above only proves `ctx.wallets`
/// visibility; this one proves upstream registration, which is what
/// `resolve_wallet` keys off.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migrated_wallet_is_upstream_registered_without_second_restart() {
    let (ctx, sender, _tmp) = offline_testnet_context();

    // Seed a legacy unprotected `wallet` row whose verbatim seed and
    // published xpub agree, so the migration's seed + meta passes produce a
    // wallet the W2 fund-routing gate will accept.
    use crate::database::test_helpers::seed_legacy_unprotected_hd_wallet_row;
    let seed = [0xD7u8; 64];
    let seed_hash: WalletSeedHash = crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
    let epk = legacy_master_epk_bytes(&seed);
    seed_legacy_unprotected_hd_wallet_row(
        &ctx.db,
        &seed_hash,
        &seed,
        &epk,
        "migrated-wallet",
        Network::Testnet,
    )
    .expect("insert legacy wallet row");

    // Wire the backend: hydration + the cold-boot bootstrap run NOW, against
    // the EMPTY sidecars (the migration has not run yet), so the upstream
    // persistor is empty and nothing is registered.
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");
    assert!(
        !backend.is_wallet_registered(&seed_hash),
        "precondition: the migrated wallet is not yet upstream-registered (sidecars empty at wiring)"
    );

    // Run the cold-start migration. It populates the sidecars, re-hydrates
    // `ctx.wallets`, AND must re-run the W2 cold-boot reconciliation so the
    // just-migrated wallet is registered upstream.
    crate::backend_task::migration::finish_unwire::run(&ctx)
        .await
        .expect("migration should succeed");

    // The migrated wallet must be RESOLVABLE WITHOUT a second backend build:
    // `is_wallet_registered` reads the same `id_map` that `resolve_wallet`
    // consults, so this is a deterministic proxy for "`resolve_wallet`
    // succeeds".
    assert!(
        backend.is_wallet_registered(&seed_hash),
        "the migrated wallet must be upstream-registered right after migration (no second restart)"
    );

    backend.shutdown().await;
}

/// A migrated protected wallet keeps the interactive migration pending without
/// changing the legacy database until the user unlocks or skips it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migrated_protected_wallet_waits_without_modifying_legacy_database() {
    use crate::database::test_helpers::seed_legacy_protected_hd_wallet_row;
    use crate::model::wallet::encryption::encrypt_message;

    let (ctx, sender, tmp) = offline_testnet_context();
    let seed = [0x42u8; 64];
    let passphrase = "correct-horse-battery-staple";
    let seed_hash: WalletSeedHash = crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
    let epk = legacy_master_epk_bytes(&seed);
    let crate::model::wallet::encryption::EncryptedEnvelope {
        ciphertext: encrypted_seed,
        salt,
        nonce,
    } = encrypt_message(&seed, passphrase).expect("encrypt legacy seed");
    seed_legacy_protected_hd_wallet_row(
        &ctx.db,
        &seed_hash,
        &encrypted_seed,
        &salt,
        &nonce,
        &epk,
        "protected-wallet",
        Some("the usual passphrase"),
        Network::Testnet,
    )
    .expect("insert legacy protected wallet row");
    let legacy_path = tmp.path().join("data.db");
    let legacy_before = std::fs::read(&legacy_path).expect("snapshot legacy database");

    ctx.install_secret_prompt(Arc::new(
        crate::wallet_backend::secret_prompt::test_support::TestPrompt::never(),
    ));

    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    let migration_context = Arc::clone(&ctx);
    let migration = tokio::spawn(async move {
        crate::backend_task::migration::finish_unwire::run(&migration_context).await
    });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if matches!(
            ctx.migration_status().state().as_ref(),
            MigrationState::AwaitingWalletPasswords { wallets } if wallets == &vec![seed_hash]
        ) {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert!(!migration.is_finished());
    assert_eq!(ctx.locked_wallet_hashes(), vec![seed_hash]);
    assert!(!backend.is_wallet_registered(&seed_hash));

    ctx.migration_status().skip_wallet(seed_hash);
    migration
        .await
        .expect("migration task must not panic")
        .expect("skipping the protected wallet must complete migration");
    let legacy_after = std::fs::read(&legacy_path).expect("re-read legacy database");
    assert_eq!(
        legacy_after, legacy_before,
        "waiting for and skipping a protected wallet must not modify the legacy database",
    );

    backend.shutdown().await;
}

/// Protected-unlock reconciliation: a password-protected wallet that hydrates
/// locked at cold boot, and therefore pauses the migration for password entry
/// (proven by
/// [`migrated_protected_wallet_waits_without_modifying_legacy_database`]), MUST
/// become upstream-registered on the unlock gesture — without a second app
/// restart.
///
/// The gap this guards: before the fix, the unlock path
/// ([`AppContext::handle_wallet_unlocked`]) only promoted the just-verified
/// seed into the session cache; it never re-drove
/// [`AppContext::bootstrap_wallet_addresses_jit`], so the wallet stayed out
/// of the upstream `id_map` that `resolve_wallet` keys off and every
/// seed-keyed operation kept failing with `WalletNotLoaded` for the rest of
/// the session. The fix re-drives the JIT bootstrap from
/// `handle_wallet_unlocked` once the seed is in the session cache; this test
/// asserts the post-unlock registration that fix enables.
///
/// A legacy protected `wallet` row hydrates `Closed` with an empty persistor,
/// then the wallet is opened with the real passphrase and
/// `handle_wallet_unlocked` is invoked exactly as the unlock popup does
/// (`src/ui/components/wallet_unlock_popup.rs`), passing the passphrase so the
/// seed resolves prompt-free from the session cache.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protected_wallet_registers_upstream_on_unlock_without_restart() {
    use crate::database::test_helpers::seed_legacy_protected_hd_wallet_row;
    use crate::model::wallet::encryption::encrypt_message;

    let (ctx, sender, _tmp) = offline_testnet_context();

    // Stage a legacy PROTECTED `wallet` row whose published BIP44 xpub agrees
    // with the seed, so the W2 fund-routing gate accepts it once reached. The
    // passphrase is the one the test feeds back in at unlock time.
    let seed = [0x42u8; 64];
    let passphrase = "correct-horse-battery-staple";
    let seed_hash: WalletSeedHash = crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
    let epk = legacy_master_epk_bytes(&seed);
    let crate::model::wallet::encryption::EncryptedEnvelope {
        ciphertext: encrypted_seed,
        salt,
        nonce,
    } = encrypt_message(&seed, passphrase).expect("encrypt legacy seed");
    seed_legacy_protected_hd_wallet_row(
        &ctx.db,
        &seed_hash,
        &encrypted_seed,
        &salt,
        &nonce,
        &epk,
        "protected-wallet",
        Some("the usual passphrase"),
        Network::Testnet,
    )
    .expect("insert legacy protected wallet row");

    ctx.install_secret_prompt(Arc::new(
        crate::wallet_backend::secret_prompt::test_support::TestPrompt::never(),
    ));

    // Wire the backend, then run the cold-start migration. This reproduces
    // the boot state of the acceptance flow: the protected wallet hydrates
    // into `ctx.wallets` but stays LOCKED, and the W2 bridge defers it.
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");
    let migration_context = Arc::clone(&ctx);
    let migration = tokio::spawn(async move {
        crate::backend_task::migration::finish_unwire::run(&migration_context).await
    });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while !matches!(
        ctx.migration_status().state().as_ref(),
        MigrationState::AwaitingWalletPasswords { wallets } if wallets == &vec![seed_hash]
    ) {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let wallet_arc = ctx
        .wallets
        .read()
        .unwrap()
        .get(&seed_hash)
        .cloned()
        .expect("protected wallet must be hydrated into ctx.wallets after migration");

    // Precondition: the locked protected wallet is NOT yet registered — the
    // exact `WalletNotLoaded`-producing state the unlock must clear.
    assert!(
        !wallet_arc.read_recover().is_open(),
        "precondition: the protected wallet hydrates locked"
    );
    assert!(
        !backend.is_wallet_registered(&seed_hash),
        "precondition: a still-locked protected wallet is not upstream-registered"
    );

    // The unlock gesture, exactly as the unlock popup performs it: open the
    // in-memory wallet by verifying the passphrase, then notify the context
    // with that passphrase so the seed is promoted to the session cache and
    // (with the fix) the JIT bootstrap is re-driven.
    wallet_arc
        .write()
        .unwrap()
        .wallet_seed
        .open(passphrase)
        .expect("correct passphrase opens the wallet");
    ctx.handle_wallet_unlocked(
        &wallet_arc,
        passphrase,
        crate::context::WalletUnlockRetention::UntilAppClose,
    )
    .expect("the unlocked seed must land in the current vault");
    ctx.migration_status().notify_wallet_password_submitted();

    migration
        .await
        .expect("migration task must not panic")
        .expect("migration must complete after password submission");

    // `handle_wallet_unlocked` spawns the registration on a tracked subtask,
    // so poll the `id_map` (what `resolve_wallet` consults) with a bounded
    // deadline rather than racing it. The deadline is generous because the
    // unlock reconciliation uses the genesis-floored `Imported` birth height
    // (`ensure_upstream_registered`), and the upstream
    // `create_wallet_from_seed_bytes` scan-window setup over the empty
    // offline persistor takes several seconds with no chain to read.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while !backend.is_wallet_registered(&seed_hash) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the protected wallet must be upstream-registered after unlock (no second restart)"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // The wallet is now watched exactly once — the unlock reconciliation does
    // not double-watch.
    assert_eq!(
        backend.wallet_count().await,
        1,
        "exactly one wallet must be watched after the unlock reconciliation"
    );
    assert_eq!(
        backend.registration_attempt_count(),
        1,
        "the migration and unlock paths must join one registration flight",
    );

    // Tier-2 keep-protection migration post-conditions. The
    // unlock decrypted the legacy AES-GCM envelope and RE-WRAPPED the seed
    // as a Tier-2 object-password envelope (protection KEPT, not downgraded
    // to a raw secret), then removes the redundant legacy envelope.
    let store = ctx.secret_store();
    let seed_view = WalletSeedView::new(&store);
    // Steady state is Tier-2 protected.
    assert_eq!(
        seed_view.scheme(&seed_hash).expect("scheme"),
        crate::wallet_backend::secret_seam::SecretScheme::Protected,
        "the seed must be re-wrapped to Tier-2, never downgraded to raw"
    );
    // A raw (password-free) read of a protected seed must fail — never strip.
    assert!(
        seed_view.get_raw(&seed_hash).is_err(),
        "a raw read of a Tier-2-protected seed must fail"
    );
    // It reads back only WITH the object password, byte-for-byte.
    let pw = platform_wallet_storage::secrets::SecretString::new(passphrase);
    let protected = seed_view
        .get_protected(&seed_hash, &pw)
        .expect("protected read")
        .expect("the seed must be re-stored as Tier-2 after the migrating unlock");
    assert_eq!(
        &*protected, &seed,
        "Tier-2 seed must equal the true 64-byte seed"
    );
    assert!(
        seed_view
            .legacy_envelope_get(&seed_hash)
            .expect("legacy read")
            .is_none(),
        "the protected seed must have exactly one vault copy after the storage update"
    );
    // The sidecar password flag STAYS true — protection was kept, so the
    // metadata stays accurate (no downgrade flip).
    let meta = WalletMetaView::new(&ctx.app_kv())
        .get(Network::Testnet, &seed_hash)
        .expect("wallet meta present");
    assert!(
        meta.uses_password,
        "WalletMeta.uses_password must stay true — Tier-2 keeps protection"
    );

    // A SECOND secret resolve still requires the object password (Tier-2 is
    // not prompt-free): a scripted prompt that supplies it resolves the seed.
    use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
    use crate::wallet_backend::{SecretAccess, SecretScope};
    let prompt = std::sync::Arc::new(TestPrompt::new([ScriptedAnswer::once(passphrase)]));
    let sa = SecretAccess::new(ctx.secret_store(), prompt.clone(), Network::Testnet);
    let resolved = sa
        .with_secret(&SecretScope::HdSeed { seed_hash }, |pt| {
            Ok(pt.expose_hd_seed().copied())
        })
        .await
        .expect("second resolve with the password");
    assert_eq!(resolved, Some(seed), "password resolve returns the seed");
    assert_eq!(
        prompt.ask_count(),
        1,
        "the protected seed prompts exactly once"
    );

    backend.shutdown().await;
}

/// F61 — clearing the SPV chain cache removes every `dash-spv` storage
/// folder/file (and the storage lock) under the per-network directory while
/// leaving the wallet (`platform-wallet.sqlite`) and shielded sidecars
/// intact. The pre-fix `clear_spv_data` was a no-op that still reported
/// success.
#[test]
fn clear_spv_chain_storage_removes_chain_cache_but_keeps_wallet_sidecars() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let spv_dir = spv_storage_dir(tmp.path(), Network::Testnet);
    std::fs::create_dir_all(&spv_dir).expect("create spv dir");

    // Plant one file inside each chain-storage folder, plus the loose
    // peers.dat and the sibling storage lock.
    for entry in [
        "block_headers",
        "filter_headers",
        "filters",
        "blocks",
        "metadata",
        "masternodestate",
    ] {
        let folder = spv_dir.join(entry);
        std::fs::create_dir_all(&folder).expect("create chain folder");
        std::fs::write(folder.join("segment.dat"), b"x").expect("write chain segment");
    }
    std::fs::write(spv_dir.join("peers.dat"), b"peers").expect("write peers");
    std::fs::write(spv_dir.with_extension("lock"), b"lock").expect("write lock");

    // Plant the wallet + shielded sidecars that must survive the clear.
    let wallet_sqlite = spv_dir.join("platform-wallet.sqlite");
    let shielded_tree = spv_dir.join("shielded-commitment-tree.sqlite");
    std::fs::write(&wallet_sqlite, b"wallet").expect("write wallet sqlite");
    std::fs::write(&shielded_tree, b"tree").expect("write shielded tree");

    clear_spv_chain_storage(&spv_dir).expect("clear must succeed");

    for entry in SPV_CHAIN_STORAGE_ENTRIES {
        assert!(
            !spv_dir.join(entry).exists(),
            "chain-storage entry {entry} must be deleted"
        );
    }
    assert!(
        !spv_dir.with_extension("lock").exists(),
        "the storage lock must be deleted"
    );
    assert!(
        wallet_sqlite.exists(),
        "platform-wallet.sqlite must survive an SPV-cache clear"
    );
    assert!(
        shielded_tree.exists(),
        "the shielded commitment tree must survive an SPV-cache clear"
    );
}

/// F61 — a never-synced network has no SPV directory at all; clearing it is
/// a success, not an error.
#[test]
fn clear_spv_chain_storage_is_ok_when_directory_absent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let spv_dir = spv_storage_dir(tmp.path(), Network::Testnet);
    assert!(
        !spv_dir.exists(),
        "precondition: no spv dir on a fresh install"
    );
    clear_spv_chain_storage(&spv_dir).expect("clearing an absent cache must succeed");
}

/// Seed a legacy password-protected `single_key_wallet` row into the
/// context's `data.db`, encrypted under `password`. Returns the
/// derived address. The default test DB created `single_key_wallet`
/// via `create_tables(true)`, so we only INSERT.
fn seed_legacy_protected_single_key(
    ctx: &Arc<AppContext>,
    raw_key: &[u8; 32],
    password: &str,
    alias: Option<&str>,
) -> String {
    use crate::model::wallet::single_key::ClosedSingleKey;
    use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
    use dash_sdk::dpp::dashcore::{Address, PrivateKey, PublicKey};

    let path = ctx.db.db_file_path().expect("data.db path");
    let conn = rusqlite::Connection::open(&path).expect("open data.db");

    let crate::model::wallet::encryption::EncryptedEnvelope {
        ciphertext,
        salt,
        nonce,
    } = ClosedSingleKey::encrypt_private_key(raw_key, password).expect("encrypt");
    let priv_key = PrivateKey::from_byte_array(raw_key, Network::Testnet).expect("priv");
    let secp = Secp256k1::new();
    let pub_key = PublicKey {
        compressed: priv_key.compressed,
        inner: priv_key.inner.public_key(&secp),
    };
    let address = Address::p2pkh(&pub_key, Network::Testnet).to_string();
    let key_hash = ClosedSingleKey::compute_key_hash(raw_key);
    conn.execute(
        "INSERT INTO single_key_wallet
            (key_hash, encrypted_private_key, salt, nonce, public_key,
             address, alias, uses_password, network)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)",
        rusqlite::params![
            key_hash.as_slice(),
            ciphertext,
            salt,
            nonce,
            pub_key.inner.serialize().to_vec(),
            address,
            alias,
            Network::Testnet.to_string(),
        ],
    )
    .expect("insert legacy protected row");
    address
}

/// T-SK-03 end-to-end — a legacy password-protected single-key row is
/// restored with the correct old password: the key lands in the modern
/// vault, becomes listable, and drops off the pending list. A wrong
/// password leaves the legacy row intact and surfaces the generic
/// failure (no oracle, no corruption).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restore_protected_single_key_round_trip_and_wrong_password() {
    use crate::backend_task::migration::single_key_restore::{
        list_pending_protected_restores, restore_protected_single_key,
    };
    use crate::wallet_backend::single_key::ImportPassphrase;

    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let mut raw = [0u8; 32];
    raw[31] = 0x2A;
    let address =
        seed_legacy_protected_single_key(&ctx, &raw, "old-legacy-password", Some("savings"));

    // The protected row shows up as pending (still encrypted under the
    // old password; not in the modern vault yet).
    let pending = list_pending_protected_restores(&ctx).expect("list pending");
    assert_eq!(pending.len(), 1, "exactly one protected row awaits restore");
    assert_eq!(pending[0].address, address);

    // Wrong password: generic failure, nothing restored, row intact.
    let err = restore_protected_single_key(
        &ctx,
        &address,
        "WRONG-password",
        ImportPassphrase::default(),
    )
    .expect_err("wrong password must fail");
    assert!(
        matches!(err, TaskError::SingleKeyPassphraseIncorrect),
        "wrong password must surface the generic incorrect error, got {err:?}"
    );
    let still_pending = list_pending_protected_restores(&ctx).expect("re-list pending");
    assert_eq!(
        still_pending.len(),
        1,
        "a failed restore must leave the protected row pending and uncorrupted"
    );

    // Correct password: the key is restored into the modern vault under
    // a fresh passphrase and becomes listable at the same address (S5).
    let restored_addr = restore_protected_single_key(
        &ctx,
        &address,
        "old-legacy-password",
        ImportPassphrase {
            passphrase: Some(zeroize::Zeroizing::new("a-fresh-strong-passphrase".into())),
            hint: Some("the new one".into()),
        },
    )
    .expect("correct password must restore the key");
    assert_eq!(restored_addr, address, "restored address must be stable");

    // It is now in the modern single-key index and no longer pending.
    let backend = ctx.wallet_backend().expect("backend wired");
    let listed = backend.single_key().list();
    assert!(
        listed
            .iter()
            .any(|k| k.address == address && k.has_passphrase),
        "restored key must be listable and passphrase-protected"
    );
    let after = list_pending_protected_restores(&ctx).expect("final pending");
    assert!(
        after.is_empty(),
        "the restored key must drop off the pending list"
    );
}

/// Restoring a protected key copies it into the current vault without changing
/// the legacy SQLite recovery file, even when no new passphrase is selected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restore_without_new_passphrase_leaves_legacy_database_unchanged() {
    use crate::backend_task::migration::single_key_restore::restore_protected_single_key;
    use crate::wallet_backend::single_key::ImportPassphrase;

    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let mut raw = [0u8; 32];
    raw[31] = 0x5B;
    let address =
        seed_legacy_protected_single_key(&ctx, &raw, "old-legacy-password", Some("plain"));
    let legacy_path = ctx.db.db_file_path().expect("file-backed test database");
    let before = std::fs::read(&legacy_path).expect("snapshot legacy database");

    // Restore WITHOUT a new passphrase → has_passphrase == false.
    restore_protected_single_key(
        &ctx,
        &address,
        "old-legacy-password",
        ImportPassphrase::default(),
    )
    .expect("restore without a new passphrase must succeed");
    let backend = ctx.wallet_backend().expect("backend wired");
    assert!(
        backend
            .single_key()
            .list()
            .iter()
            .any(|k| k.address == address && !k.has_passphrase),
        "the key must be restored unprotected (has_passphrase == false)"
    );

    assert_eq!(
        std::fs::read(&legacy_path).expect("read legacy database after restore"),
        before,
        "restoring must not update or drop anything in the legacy database",
    );
}

/// Build a deterministic compressed testnet WIF from `raw` so the
/// single-key import tests stay offline and reproducible.
fn testnet_wif_from_raw(raw: &[u8; 32]) -> String {
    use dash_sdk::dpp::dashcore::PrivateKey;
    PrivateKey::from_byte_array(raw, Network::Testnet)
        .expect("valid private key bytes")
        .to_wif()
}

/// Importing a **passphrase-protected** single key must NOT retain the
/// decrypted private key in the long-lived `single_key_wallets` session
/// map. The in-memory mirror must come back closed — exactly the shape
/// cold boot reconstructs — so the per-key passphrase is not silently
/// defeated by a plaintext copy lingering for the whole session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protected_single_key_import_does_not_retain_plaintext_in_session_map() {
    use crate::wallet_backend::single_key::ImportPassphrase;

    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let mut raw = [0u8; 32];
    raw[31] = 0x77;
    let wif = testnet_wif_from_raw(&raw);

    let passphrase = ImportPassphrase {
        passphrase: Some(zeroize::Zeroizing::new("a-strong-passphrase".into())),
        hint: Some("the test one".into()),
    };
    let (imported, wallet_arc) = ctx
        .import_single_key_wif(&wif, Some("protected".into()), passphrase)
        .expect("protected import must succeed");
    assert!(
        imported.has_passphrase,
        "the imported metadata must record the per-key passphrase"
    );

    // The in-memory mirror must be closed: no `is_open`, no plaintext key
    // obtainable, and the underlying data must be the encrypted variant.
    let guard = wallet_arc.read().expect("read mirror");
    assert!(
        !guard.is_open(),
        "a protected single key must be mirrored closed, not open with plaintext"
    );
    assert!(
        guard.private_key(Network::Testnet).is_none(),
        "no plaintext private key may be retrievable from the session-map mirror"
    );
    assert!(
        matches!(
            guard.private_key_data,
            crate::model::wallet::single_key::SingleKeyData::Closed(_)
        ),
        "the mirrored key data must be the Closed (encrypted) variant"
    );
    assert!(
        guard.uses_password,
        "the mirror must advertise that it needs a password"
    );

    // The same closed entry must be the one tracked in the session map.
    let key_hash = guard.key_hash();
    drop(guard);
    let map = ctx.single_key_wallets.read().expect("read map");
    let in_map = map.get(&key_hash).expect("imported key present in map");
    assert!(
        !in_map.read().expect("read map entry").is_open(),
        "the session-map entry for a protected key must stay closed"
    );
}

/// Companion to the protected-key test: an **unprotected** single key
/// has no passphrase by definition, so plaintext in the session map is
/// inherent and the mirror is expected to be open. This guards against
/// over-correcting and breaking the no-passphrase fast path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unprotected_single_key_import_mirrors_open() {
    use crate::wallet_backend::single_key::ImportPassphrase;

    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let mut raw = [0u8; 32];
    raw[31] = 0x55;
    let wif = testnet_wif_from_raw(&raw);

    let (imported, wallet_arc) = ctx
        .import_single_key_wif(&wif, Some("plain".into()), ImportPassphrase::default())
        .expect("unprotected import must succeed");
    assert!(
        !imported.has_passphrase,
        "an unprotected import must record no per-key passphrase"
    );

    let guard = wallet_arc.read().expect("read mirror");
    assert!(
        guard.is_open(),
        "an unprotected single key is mirrored open (plaintext is inherent)"
    );
    assert!(
        guard.private_key(Network::Testnet).is_some(),
        "an unprotected mirror exposes its private key for signing"
    );
    assert!(
        !guard.uses_password,
        "an unprotected mirror must not advertise a password requirement"
    );
}

/// The "Unlock" gesture for a protected single key must confirm the
/// passphrase against the vault WITHOUT re-parking the decrypted private
/// key in the long-lived `single_key_wallets` map. The map entry must stay
/// closed both before and after a successful unlock; a wrong passphrase
/// surfaces the generic incorrect error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protected_single_key_unlock_verifies_without_reparking_plaintext() {
    use crate::wallet_backend::single_key::ImportPassphrase;

    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    let mut raw = [0u8; 32];
    raw[31] = 0x91;
    let wif = testnet_wif_from_raw(&raw);
    let pass = "a-strong-passphrase";

    let passphrase = ImportPassphrase {
        passphrase: Some(zeroize::Zeroizing::new(pass.into())),
        hint: None,
    };
    let (_imported, wallet_arc) = ctx
        .import_single_key_wif(&wif, Some("protected".into()), passphrase)
        .expect("protected import must succeed");
    let address = wallet_arc.read().expect("read mirror").address.to_string();

    // Closed before the unlock gesture.
    assert!(
        !wallet_arc.read().expect("read mirror").is_open(),
        "a protected key must be closed before unlock"
    );

    // A wrong passphrase surfaces the generic incorrect error and leaves
    // the entry closed.
    let wrong = ctx
        .verify_single_key_passphrase(&address, "not-the-passphrase")
        .expect_err("a wrong passphrase must fail");
    assert!(
        matches!(wrong, TaskError::SingleKeyPassphraseIncorrect),
        "wrong passphrase must surface the generic incorrect error, got {wrong:?}"
    );
    assert!(
        !wallet_arc.read().expect("read mirror").is_open(),
        "a failed unlock must leave the key closed"
    );

    // The correct passphrase verifies successfully — and the key STILL
    // stays closed: no plaintext is re-parked in the session map.
    ctx.verify_single_key_passphrase(&address, pass)
        .expect("the correct passphrase must verify");
    let guard = wallet_arc.read().expect("read mirror");
    assert!(
        !guard.is_open(),
        "a successful unlock must NOT open the map entry (no plaintext re-parked)"
    );
    assert!(
        guard.private_key(Network::Testnet).is_none(),
        "no plaintext private key may be retrievable after unlock"
    );
    assert!(
        matches!(
            guard.private_key_data,
            crate::model::wallet::single_key::SingleKeyData::Closed(_)
        ),
        "the map entry must remain the Closed (encrypted) variant after unlock"
    );
}

// ──────────────────────────────────────────────────────────────────────
// Automatic identity-discovery trigger / latch / re-arm
// ──────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_wallets_discovery_latch_is_one_shot_until_stop_spv() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");

    assert!(
        !ctx.identity_autodiscovery_fired.load(Ordering::SeqCst),
        "latch starts unfired"
    );

    // First fire latches; a second fire is swallowed (no second sweep).
    ctx.queue_all_wallets_identity_discovery();
    assert!(
        ctx.identity_autodiscovery_fired.load(Ordering::SeqCst),
        "first call must set the one-shot latch"
    );
    ctx.queue_all_wallets_identity_discovery();
    assert!(
        ctx.identity_autodiscovery_fired.load(Ordering::SeqCst),
        "latch stays set; the second call is a no-op"
    );

    // stop_spv re-arms the latch so the next reconnect runs discovery again.
    ctx.stop_spv().await;
    assert!(
        !ctx.identity_autodiscovery_fired.load(Ordering::SeqCst),
        "stop_spv must clear the latch to re-arm discovery on reconnect"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_wallets_snapshot_excludes_locked_wallets() {
    use crate::database::test_helpers::seed_legacy_protected_hd_wallet_row;
    use crate::model::wallet::encryption::encrypt_message;

    let (ctx, sender, _tmp) = offline_testnet_context();

    // A locked, password-protected wallet staged via the legacy migration
    // row: it hydrates `WalletSeed::Closed` and must be excluded.
    let locked_seed = [0x77u8; 64];
    let locked_hash: WalletSeedHash =
        crate::model::wallet::ClosedKeyItem::compute_seed_hash(&locked_seed);
    let epk = legacy_master_epk_bytes(&locked_seed);
    let crate::model::wallet::encryption::EncryptedEnvelope {
        ciphertext: encrypted_seed,
        salt,
        nonce,
    } = encrypt_message(&locked_seed, "a-passphrase-never-fed-back").expect("encrypt seed");
    seed_legacy_protected_hd_wallet_row(
        &ctx.db,
        &locked_hash,
        &encrypted_seed,
        &salt,
        &nonce,
        &epk,
        "locked-wallet",
        None,
        Network::Testnet,
    )
    .expect("insert legacy protected wallet row");

    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    // An open, no-password wallet registered alongside it.
    let open_seed = [0x66u8; 64];
    let open_wallet =
        crate::model::wallet::Wallet::new_from_seed(open_seed, Network::Testnet, None, None)
            .expect("build open wallet");
    let open_hash = open_wallet.seed_hash();
    ctx.register_wallet(open_wallet, &open_seed, WalletOrigin::Fresh)
        .expect("register open wallet");

    let snapshot: Vec<WalletSeedHash> = ctx
        .open_wallets()
        .iter()
        .map(|w| w.read_recover().seed_hash())
        .collect();

    assert!(
        snapshot.contains(&open_hash),
        "the open wallet must be in the snapshot"
    );
    assert!(
        !snapshot.contains(&locked_hash),
        "the locked protected wallet must be excluded from the snapshot"
    );

    backend.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rediscovery_update_preserves_user_alias_and_wallet_binding() {
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::platform::Identifier;
    use std::collections::BTreeMap;

    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    let identity_id = Identifier::from([7u8; 32]);
    let make_qi = |alias: Option<&str>| {
        let identity = Identity::create_basic_identity(identity_id, ctx.platform_version())
            .expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: alias.map(str::to_string),
            private_keys: KeyStorage {
                private_keys: BTreeMap::new(),
            },
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    };

    // Initial store with a user alias and a wallet binding.
    let wallet_hash: WalletSeedHash = [0x09u8; 32];
    ctx.insert_local_qualified_identity(&make_qi(Some("my-id")), &Some((wallet_hash, 3)))
        .expect("insert identity with alias");

    // Simulate re-discovery: build a FRESH QI with no alias, carry the
    // existing alias (the carry-over under test), then update in place.
    let mut refreshed = make_qi(None);
    let existing = ctx
        .get_identity_by_id(&identity_id)
        .expect("load existing")
        .expect("identity present");
    refreshed.alias = existing.alias;
    ctx.update_local_qualified_identity(&refreshed)
        .expect("update preserving alias");

    // The alias survives, and the wallet binding is preserved by the update.
    let reloaded = ctx
        .get_identity_by_id(&identity_id)
        .expect("reload identity")
        .expect("identity present after update");
    assert_eq!(
        reloaded.alias.as_deref(),
        Some("my-id"),
        "the user alias must survive a re-discovery update (F-1 regression guard)"
    );
    assert_eq!(
        reloaded.wallet_index,
        Some(3),
        "the wallet binding index must be preserved across the update"
    );

    backend.shutdown().await;
}

/// C.7 regression guard: `ensure_identity_funding_accounts` must succeed on
/// a cold-booted (watch-only) wallet for a fresh `IdentityTopUp{index}`.
///
/// # Background
///
/// DET always reloads wallets **seedless** from the upstream persister.
/// `WalletBackend::new` → `load_from_persistor_seedless` → upstream
/// `load_from_persistor()` → `Wallet::new_external_signable(…)`.  The wallet has
/// the BIP44/BIP32 accounts it was persisted with, but **no root private
/// key**.
///
/// `WalletAccountCreationOptions::Default` (used by
/// `register_wallet_from_seed`) creates `IdentityRegistration` by default
/// and persists it in the account manifest.  `IdentityTopUp{n}` is NOT
/// created by default — it is added only after a register/top-up, so on
/// every cold boot the manifest lacks it.
///
/// Before the fix, `provision_identity_funding_account` called
/// `kw.add_account(account_type, None)`.  On a cold-boot wallet that path
/// reaches `root_extended_keys.rs:428` and fails:
///
///   `WalletBackend { source: AssetLockTransaction("Invalid parameter:
///    Watch-only wallet has no private key") }`
///
/// After the fix it builds a short-lived signable wallet from the provided
/// seed bytes, derives the account xpub, and calls
/// `kw.add_account(account_type, Some(xpub))` — succeeds regardless of
/// private-key availability.
///
/// # Why deterministic
///
/// The cold-booted wallet unconditionally has no root private key; the
/// failure path is hit every time regardless of timing or network state.
///
/// # Test structure
///
/// Two-boot scenario to match production:
///   1. **Boot 1**: wire backend, write both sidecars (wallet-meta + upstream
///      persister) from seed.
///   2. **Boot 2 (cold)**: `WalletBackend::new` over a copy of the same
///      data dir runs `load_from_persistor_seedless` — the upstream wallet is
///      loaded watch-only.  Then `ensure_identity_funding_accounts` for a
///      fresh `IdentityTopUp{3}` must return `Ok`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ensure_identity_funding_accounts_succeeds_on_cold_booted_watch_only_wallet() {
    // ── Boot 1: write wallet-meta sidecar + upstream persister from seed ──

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let seed = [0xC7u8; 64];
    let seed_hash = {
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            None,
            None, // no password
        )
        .expect("build wallet");
        let h = wallet.seed_hash();

        let (ctx, sender) = offline_testnet_context_at(temp_dir.path());

        // Register the wallet BEFORE wiring the backend.  register_wallet
        // writes the DET sidecars (seed-envelope vault + wallet-meta), but
        // register_wallet_upstream checks ctx.wallet_backend() and, finding it
        // not yet wired, returns early without spawning the background
        // "wallet_upstream_registration" subtask.  This avoids the concurrency
        // hazard: if the backend were wired first the background subtask would
        // race with the synchronous register_wallet_from_seed call below —
        // both call create_wallet_from_seed_bytes for the same wallet.  The
        // upstream register_wallet inserts into wallet_manager (step A) and into
        // self.wallets (step B) with async work in between; a concurrent caller
        // that arrives between A and B sees WalletAlreadyExists but then
        // get_wallet returns None → WalletNotFound panic.  Under CI load
        // (1000+ concurrent tests) this window is reliably hit.
        ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("boot 1: ctx.register_wallet");

        // Wire the backend now so the explicit registration below has the
        // upstream persister available.
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("boot 1: ensure_wallet_backend offline");

        // Write the upstream persister synchronously — no background subtask
        // is in flight (we didn't wire the backend when register_wallet ran),
        // so this call is race-free.
        let backend1 = ctx.wallet_backend().expect("boot 1 backend");
        backend1
            .register_wallet_from_seed(&h, &seed, Some(0))
            .await
            .expect("boot 1: upstream register");
        backend1.shutdown().await;

        h
    };
    // ctx is dropped here, releasing app_kv / secret_store file handles.

    // ── Cold-boot copy: avoid file-lock conflicts with lingering subtasks ──
    //
    // The background registration subtask may still hold an Arc<WalletBackend>
    // (and thus an open SqlitePersister handle on temp_dir).  We copy the
    // on-disk state to a fresh path so Boot 2's SqlitePersister::open does
    // not collide with the old one.  Identical on-disk bytes — the fund-
    // routing gate and the persisted manifest are preserved.
    let cold_dir = tempfile::tempdir().expect("cold tempdir");
    copy_dir_recursive(temp_dir.path(), cold_dir.path());

    // ── Boot 2 (cold): load from persister → watch-only upstream wallet ──

    let (ctx2, sender2) = offline_testnet_context_at(cold_dir.path());
    ctx2.ensure_wallet_backend(sender2)
        .await
        .expect("boot 2 (cold): ensure_wallet_backend offline");
    let backend2 = ctx2.wallet_backend().expect("boot 2 backend");

    assert!(
        backend2.is_wallet_registered(&seed_hash),
        "cold boot must load the wallet from the persisted sidecars"
    );

    // `IdentityTopUp{3}` is absent from the account manifest (it is never
    // created by WalletAccountCreationOptions::Default) — so the cold-booted
    // watch-only wallet triggers the provisioning branch.
    //
    // Before the fix: kw.add_account(IdentityTopUp{3}, None)
    //   → "Watch-only wallet has no private key" → Err
    // After the fix: builds a seed wallet, derives the account xpub,
    //   calls kw.add_account(IdentityTopUp{3}, Some(xpub)) → Ok
    let registration_index = 3u32;
    backend2
        .ensure_identity_funding_accounts(&seed_hash, &seed, registration_index)
        .await
        .expect(
            "cold-booted watch-only wallet: IdentityTopUp{3} provisioning must succeed; \
             if 'Watch-only wallet has no private key' appears the fix has been reverted",
        );

    // Idempotent: both accounts now present — second call is a no-op.
    backend2
        .ensure_identity_funding_accounts(&seed_hash, &seed, registration_index)
        .await
        .expect("second call must be idempotent (both accounts already present)");

    backend2.shutdown().await;
}

/// A malformed Orchard viewing key must be isolated to its own wallet during
/// the real seedless cold-boot load.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_boot_skips_corrupt_fvk_for_one_wallet_and_restores_healthy_wallet() {
    let _guard = backend_reopen_lock().await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let corrupt_seed = [0xD5u8; 64];
    let healthy_seed = [0xD6u8; 64];

    let (corrupt_hash, corrupt_wallet_id, healthy_hash, healthy_wallet_id) = {
        let password = Secret::new("cold-boot-fvk-password");
        let corrupt_wallet = crate::model::wallet::Wallet::new_from_seed(
            corrupt_seed,
            Network::Testnet,
            Some("corrupt-fvk".to_string()),
            Some(&password),
        )
        .expect("build corrupt-fixture wallet");
        let corrupt_hash = corrupt_wallet.seed_hash();
        let healthy_wallet = crate::model::wallet::Wallet::new_from_seed(
            healthy_seed,
            Network::Testnet,
            Some("healthy-fvk".to_string()),
            Some(&password),
        )
        .expect("build healthy wallet");
        let healthy_hash = healthy_wallet.seed_hash();
        let (ctx, sender) = offline_testnet_context_at(source_dir.path());

        ctx.register_wallet(corrupt_wallet, &corrupt_seed, WalletOrigin::Fresh)
            .expect("register corrupt-fixture DET wallet");
        ctx.register_wallet(healthy_wallet, &healthy_seed, WalletOrigin::Fresh)
            .expect("register healthy DET wallet");
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire boot-1 backend");
        let backend = ctx.wallet_backend().expect("boot-1 backend");

        backend
            .register_wallet_from_seed(&corrupt_hash, &corrupt_seed, Some(0))
            .await
            .expect("persist corrupt-fixture upstream wallet");
        backend
            .register_wallet_from_seed(&healthy_hash, &healthy_seed, Some(0))
            .await
            .expect("persist healthy upstream wallet");
        backend
            .ensure_shielded_bound(&corrupt_hash, &corrupt_seed)
            .await
            .expect("persist corrupt-fixture viewing key");
        backend
            .ensure_shielded_bound(&healthy_hash, &healthy_seed)
            .await
            .expect("persist healthy viewing key");
        assert!(
            backend
                .bind_shielded_from_persisted_for_test(&corrupt_hash)
                .await
                .expect("read corrupt-fixture viewing key before corruption"),
            "precondition: the first wallet has a restorable FVK"
        );
        assert!(
            backend
                .bind_shielded_from_persisted_for_test(&healthy_hash)
                .await
                .expect("read healthy viewing key"),
            "precondition: the second wallet has a restorable FVK"
        );
        let corrupt_wallet_id = backend
            .registered_wallet_id(&corrupt_hash)
            .expect("corrupt-fixture upstream wallet id");
        let healthy_wallet_id = backend
            .registered_wallet_id(&healthy_hash)
            .expect("healthy upstream wallet id");

        let persister_path = source_dir
            .path()
            .join("spv")
            .join("testnet")
            .join("platform-wallet.sqlite");
        let connection = rusqlite::Connection::open_with_flags(
            &persister_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("open boot-1 persister inspection connection");
        for wallet_id in [corrupt_wallet_id, healthy_wallet_id] {
            let count = connection
                .query_row(
                    "SELECT COUNT(*) FROM shielded_viewing_keys WHERE wallet_id = ?1",
                    [wallet_id.as_slice()],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count boot-1 viewing keys");
            assert_eq!(count, 1, "precondition: each wallet has one FVK row");
        }
        drop(connection);

        backend.shutdown().await;
        let _ = ctx.subtasks.shutdown_async().await;
        (
            corrupt_hash,
            corrupt_wallet_id,
            healthy_hash,
            healthy_wallet_id,
        )
    };

    let cold_dir = tempfile::tempdir().expect("cold tempdir");
    copy_dir_recursive(source_dir.path(), cold_dir.path());
    let persister_path = cold_dir
        .path()
        .join("spv")
        .join("testnet")
        .join("platform-wallet.sqlite");
    let connection =
        rusqlite::Connection::open(&persister_path).expect("open cold-boot persister fixture");
    assert_eq!(
        connection
            .execute(
                "UPDATE shielded_viewing_keys SET viewing_key = X'00' WHERE wallet_id = ?1",
                [corrupt_wallet_id.as_slice()],
            )
            .expect("corrupt one persisted viewing key"),
        1,
        "the corruption fixture must update exactly one FVK row"
    );
    drop(connection);

    let (ctx, sender) = offline_testnet_context_at(cold_dir.path());
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("one corrupt FVK must not prevent cold boot");
    let backend = ctx.wallet_backend().expect("cold-boot backend");

    assert!(
        ctx.wallets.read_recover().contains_key(&corrupt_hash),
        "the corrupt-FVK DET wallet must remain registered"
    );
    assert!(
        ctx.wallets.read_recover().contains_key(&healthy_hash),
        "the healthy DET wallet must remain registered"
    );
    assert!(
        backend.is_wallet_registered(&corrupt_hash),
        "the corrupt-FVK wallet must remain registered upstream"
    );
    assert!(
        backend.is_wallet_registered(&healthy_hash),
        "the healthy wallet must remain registered upstream"
    );
    assert!(
        !backend
            .bind_shielded_from_persisted_for_test(&corrupt_hash)
            .await
            .expect("probe skipped corrupt viewing key"),
        "the corrupt wallet must not restore a shielded binding"
    );
    assert!(
        backend
            .bind_shielded_from_persisted_for_test(&healthy_hash)
            .await
            .expect("restore healthy viewing key"),
        "the healthy wallet must restore its shielded binding"
    );
    assert_ne!(
        corrupt_wallet_id, healthy_wallet_id,
        "fixture wallets must have distinct upstream ids"
    );

    backend.shutdown().await;
    let _ = ctx.subtasks.shutdown_async().await;
}

/// A corrupt persisted Core transaction must surface the dedicated fatal
/// local-data load error during seedless cold boot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_boot_surfaces_typed_error_when_persisted_transaction_txid_is_corrupt() {
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::{BlockHash, Transaction};
    use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
    use dash_sdk::dpp::key_wallet::managed_account::transaction_record::{
        TransactionDirection, TransactionRecord,
    };
    use dash_sdk::dpp::key_wallet::transaction_checking::BlockInfo;
    use dash_sdk::dpp::key_wallet::transaction_checking::transaction_router::TransactionType;
    use platform_wallet::changeset::{
        CoreChangeSet, PlatformWalletChangeSet, PlatformWalletPersistence,
    };
    use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig};

    let _guard = backend_reopen_lock().await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let seed = [0xD4u8; 64];

    let (_seed_hash, wallet_id) = {
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        let (ctx, sender) = offline_testnet_context_at(source_dir.path());

        ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("persist DET wallet sidecars");
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire boot-1 backend");
        let backend = ctx.wallet_backend().expect("boot-1 backend");
        backend
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("persist upstream wallet");
        let wallet_id = backend
            .registered_wallet_id(&seed_hash)
            .expect("registered upstream wallet id");
        backend.shutdown().await;
        let _ = ctx.subtasks.shutdown_async().await;
        (seed_hash, wallet_id)
    };

    let timestamp = 1_720_000_123u32;
    let transaction = Transaction {
        version: 1,
        lock_time: 17,
        input: Vec::new(),
        output: Vec::new(),
        special_transaction_payload: None,
    };
    let record = TransactionRecord::new(
        transaction,
        AccountType::Standard {
            index: 0,
            standard_account_type: StandardAccountType::BIP44Account,
        },
        dash_sdk::dpp::key_wallet::transaction_checking::TransactionContext::InBlock(
            BlockInfo::new(42, BlockHash::from_byte_array([0x42; 32]), timestamp),
        ),
        TransactionType::Standard,
        TransactionDirection::Incoming,
        Vec::new(),
        Vec::new(),
        250_000,
    );
    let cold_dir = tempfile::tempdir().expect("cold tempdir");
    copy_dir_recursive(source_dir.path(), cold_dir.path());

    let persister_path = cold_dir
        .path()
        .join("spv")
        .join("testnet")
        .join("platform-wallet.sqlite");
    let persister = SqlitePersister::open(SqlitePersisterConfig::new(&persister_path))
        .expect("reopen upstream persister");
    persister
        .store(
            wallet_id,
            PlatformWalletChangeSet {
                core: Some(CoreChangeSet {
                    records: vec![record],
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .expect("persist transaction record");
    persister
        .flush(wallet_id)
        .expect("flush transaction record");
    drop(persister);

    let connection = rusqlite::Connection::open(&persister_path)
        .expect("open upstream persister for corruption fixture");
    connection
        .execute(
            "INSERT INTO core_transactions \
                (wallet_id, txid, height, block_hash, block_time, finalized, record_blob) \
             SELECT wallet_id, X'A1', height, block_hash, block_time, finalized, record_blob \
             FROM core_transactions WHERE wallet_id = ?1 LIMIT 1",
            [wallet_id.as_slice()],
        )
        .expect("insert invalid-width transaction id");
    drop(connection);

    let (ctx, sender) = offline_testnet_context_at(cold_dir.path());
    let error = ctx
        .ensure_wallet_backend(sender)
        .await
        .expect_err("corrupt persisted transaction id must fail cold-boot loading");
    assert!(
        matches!(error, TaskError::WalletLocalDataLoadFailed { .. }),
        "fatal persisted-wallet corruption must use the dedicated error, got: {error:?}"
    );
}

/// Build a minimal basic identity for manager-reconcile tests — only its
/// id() and (empty) public_keys() are read by `add_identity`.
fn basic_test_identity() -> dash_sdk::dpp::identity::Identity {
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;
    Identity::create_basic_identity(Identifier::random(), PlatformVersion::latest())
        .expect("basic identity")
}

/// Wrap a basic identity in a minimal wallet-owned `QualifiedIdentity` for
/// sidecar-reconcile tests.
fn wallet_owned_qualified_identity(
    wallet_index: Option<u32>,
) -> crate::model::qualified_identity::QualifiedIdentity {
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
    QualifiedIdentity {
        identity: basic_test_identity(),
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: None,
        private_keys: KeyStorage::default(),
        dpns_names: vec![],
        associated_wallets: std::collections::BTreeMap::new(),
        secret_access: None,
        wallet_index,
        top_ups: std::collections::BTreeMap::new(),
        status: IdentityStatus::Active,
        network: Network::Testnet,
    }
}

/// `ensure_identity_managed` on a wallet that is not upstream-registered
/// fails with `WalletNotLoaded` (and the reconcile driver logs-and-skips it
/// rather than aborting).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ensure_identity_managed_unregistered_wallet_is_wallet_not_loaded() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    let unknown_seed: WalletSeedHash = [0x5Au8; 32];
    let identity = basic_test_identity();
    let err = backend
        .ensure_identity_managed(&unknown_seed, &identity, 0)
        .await
        .expect_err("an unregistered wallet must not resolve");
    assert!(
        matches!(err, TaskError::WalletNotLoaded { .. }),
        "expected WalletNotLoaded, got: {err:?}"
    );

    backend.shutdown().await;
}

/// Both `WalletNotLoaded` sites must name the wallet they are about: the
/// typed field carries the label and the user-facing message repeats it.
fn assert_wallet_not_loaded_named(err: &TaskError, expected_label: &str) {
    match err {
        TaskError::WalletNotLoaded { wallet_label } => {
            assert_eq!(wallet_label, expected_label, "wrong wallet named")
        }
        other => panic!("expected WalletNotLoaded, got {other:?}"),
    }
    assert!(
        err.to_string().contains(expected_label),
        "message must name the wallet ({expected_label}), got: {err}"
    );
}

/// With several wallets loaded, a `WalletNotLoaded` from either construction
/// site (`resolve_wallet` async, `monitored_receive_addresses` sync) names the
/// affected wallet by its alias — the user can tell which wallet to wait for.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wallet_not_loaded_names_the_wallet_by_alias() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    // A loaded sibling wallet, so the error cannot name "the only wallet".
    let loaded_seed = [0x11u8; 64];
    let loaded_hash = Wallet::new_from_seed(loaded_seed, Network::Testnet, None, None)
        .expect("build wallet")
        .seed_hash();
    backend
        .register_wallet_from_seed(&loaded_hash, &loaded_seed, None)
        .await
        .expect("register the sibling wallet");

    // The wallet under test: known by its meta sidecar, absent from `id_map`.
    let pending_hash: WalletSeedHash = [0x7Bu8; 32];
    backend
        .wallet_meta()
        .set(
            Network::Testnet,
            &pending_hash,
            &WalletMeta {
                alias: "paycheque".into(),
                ..Default::default()
            },
        )
        .expect("persist wallet meta");

    let err = backend
        .ensure_identity_managed(&pending_hash, &basic_test_identity(), 0)
        .await
        .expect_err("a wallet missing from id_map must not resolve");
    assert_wallet_not_loaded_named(&err, "paycheque");

    let err = backend
        .monitored_receive_addresses(&pending_hash)
        .expect_err("a wallet missing from id_map has no monitored addresses");
    assert_wallet_not_loaded_named(&err, "paycheque");

    backend.shutdown().await;
}

/// An unnamed wallet still gets identified: the label falls back to the same
/// truncated seed-hash hex `SeedLengthInvalid` uses (12 hex chars + ellipsis).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wallet_not_loaded_falls_back_to_truncated_seed_hash_when_unnamed() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    // No alias in the sidecar (and, for the second case below, no sidecar row
    // at all) — both degrade to the hex label, never to an unnamed error.
    let unnamed_hash: WalletSeedHash = [0x9Eu8; 32];
    backend
        .wallet_meta()
        .set(Network::Testnet, &unnamed_hash, &WalletMeta::default())
        .expect("persist wallet meta");

    let err = backend
        .ensure_identity_managed(&unnamed_hash, &basic_test_identity(), 0)
        .await
        .expect_err("a wallet missing from id_map must not resolve");
    assert_wallet_not_loaded_named(&err, "9e9e9e9e9e9e…");

    let no_meta_hash: WalletSeedHash = [0xC4u8; 32];
    let err = backend
        .monitored_receive_addresses(&no_meta_hash)
        .expect_err("a wallet missing from id_map has no monitored addresses");
    assert_wallet_not_loaded_named(&err, "c4c4c4c4c4c4…");

    backend.shutdown().await;
}

/// `ensure_identity_managed` registers a previously-unknown identity (→
/// `true`), then a second call is a no-op (→ `false`). Runs with no secret
/// session promoted, proving the reconcile is seed-free / locked-safe.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ensure_identity_managed_registers_then_noops_while_locked() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    let seed = [0x2Cu8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    backend
        .register_wallet_from_seed(&seed_hash, &seed, None)
        .await
        .expect("register wallet with upstream manager");

    let identity = basic_test_identity();

    // No secret session is open here — the wallet is effectively locked.
    let first = backend
        .ensure_identity_managed(&seed_hash, &identity, 0)
        .await
        .expect("registering a new identity must succeed while locked");
    assert!(first, "first call newly registers the identity");

    let second = backend
        .ensure_identity_managed(&seed_hash, &identity, 0)
        .await
        .expect("second call must be idempotent");
    assert!(!second, "second call is a no-op (already managed)");

    backend.shutdown().await;
}

/// `reconcile_managed_identities` registers exactly the wallet-owned
/// identities (`wallet_index.is_some()` and matching `seed_hash`) and leaves
/// index-less sidecar entries alone — proven via the idempotent
/// `ensure_identity_managed` (already-managed → `false`, never-managed →
/// `true`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconcile_managed_identities_registers_only_wallet_owned() {
    let (ctx, sender, _tmp) = offline_testnet_context();
    ctx.ensure_wallet_backend(sender)
        .await
        .expect("ensure_wallet_backend should succeed offline");
    let backend = ctx.wallet_backend().expect("backend wired");

    let seed = [0x3Du8; 64];
    let wallet = crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
        .expect("build wallet");
    let seed_hash = wallet.seed_hash();
    backend
        .register_wallet_from_seed(&seed_hash, &seed, None)
        .await
        .expect("register wallet with upstream manager");

    // Two wallet-owned identities (should be reconciled) and one index-less
    // identity (should be skipped by the `wallet_index.is_some()` filter).
    let owned_a = wallet_owned_qualified_identity(Some(0));
    let owned_b = wallet_owned_qualified_identity(Some(1));
    let detached = wallet_owned_qualified_identity(None);
    ctx.insert_local_qualified_identity(&owned_a, &Some((seed_hash, 0)))
        .expect("insert owned_a");
    ctx.insert_local_qualified_identity(&owned_b, &Some((seed_hash, 1)))
        .expect("insert owned_b");
    ctx.insert_local_qualified_identity(&detached, &None)
        .expect("insert detached");

    ctx.reconcile_managed_identities(&backend, &seed_hash).await;

    // The two wallet-owned identities are now managed → ensure is a no-op.
    assert!(
        !backend
            .ensure_identity_managed(&seed_hash, &owned_a.identity, 0)
            .await
            .expect("owned_a"),
        "wallet-owned identity A must already be managed after reconcile"
    );
    assert!(
        !backend
            .ensure_identity_managed(&seed_hash, &owned_b.identity, 1)
            .await
            .expect("owned_b"),
        "wallet-owned identity B must already be managed after reconcile"
    );
    // The index-less identity was skipped → ensure newly registers it.
    assert!(
        backend
            .ensure_identity_managed(&seed_hash, &detached.identity, 0)
            .await
            .expect("detached"),
        "index-less identity must have been skipped by the reconcile filter"
    );

    backend.shutdown().await;
}
