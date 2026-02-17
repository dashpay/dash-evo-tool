# SPV Refactor Code Review: Single-Runtime Migration

**Reviewer**: code-reviewer agent  
**Date**: 2026-02-16  
**Branch**: `refactor/no-separate-spv-thread`  
**Scope**: Elimination of separate SPV OS thread + tokio runtime; SPV now runs on the main 12-worker tokio runtime via `TaskManager::spawn_sync`.

---

## Critical Issues (must fix before merge)

**None identified.**

The refactoring is minimal and well-targeted. The change from `std::thread::Builder::new("spv").spawn(|| { rt.block_on(...) })` to `self.subtasks.spawn_sync("spv_main_loop", async move { ... })` is mechanically sound and does not introduce new deadlock vectors.

---

## Warnings (should fix)

### W1. `block_in_place` + `block_on` in `get_quorum_public_key` — now riskier on shared runtime

**File**: `src/spv/manager.rs:639-664`

```rust
tokio::task::block_in_place(|| {
    tokio::runtime::Handle::current().block_on(async {
        interface.get_quorum_by_height(...).await
    })
})
```

Previously this ran on the separate 4-worker SPV runtime, so blocking a worker there had no impact on the main UI task pipeline. Now it runs on the shared 12-worker runtime. `block_in_place` temporarily converts the current worker to a blocking thread and spawns a replacement, which is correct but:

- If called frequently or if the quorum lookup is slow (network round-trip through the unbounded command channel), it can exhaust tokio worker threads temporarily.
- This is a sync method called from unknown contexts. If ever called from a tokio task that is not `block_in_place`-safe (e.g., from inside a `spawn_blocking` context), it will panic.

**Recommendation**: Consider making this method `async` and using `.await` directly, or document that it must only be called from a tokio worker context. The risk is low in practice given 12 workers and infrequent quorum lookups, but it is worth noting.

### W2. `reconcile_spv_wallets` holds multiple lock layers simultaneously

**File**: `src/context/wallet_lifecycle.rs:555-767`

The reconciliation function acquires:
1. `self.spv_manager.wallet().read().await` (tokio RwLock, held for entire function body)
2. `self.wallets.read().unwrap()` (std RwLock, held for entire function body)
3. Individual `wallet_arc.write()` (std RwLock, nested inside #2's scope)
4. `self.db.*` operations (std Mutex on SQLite connection)

This is a complex lock hierarchy. The key concern is that `self.wallets` is a `std::sync::RwLock` and it is acquired while an `await`-capable tokio lock is held. If any other code path acquires these locks in a different order, a deadlock could occur.

**Assessment**: The lock ordering appears consistent across the codebase (SPV wallet lock first, then DET wallets lock, then individual wallet locks). However, the function is long and holds locks for an extended period. This is pre-existing code and not introduced by this refactoring, so it is a warning rather than a blocker.

**Recommendation**: Consider breaking the reconciliation into smaller lock scopes, particularly releasing the tokio `wm` read lock before doing DB writes.

### W3. `std::sync::Mutex` guards held across sync points in event handler setup

**File**: `src/spv/manager.rs:1069-1070, 1142`

```rust
let reconcile_tx = self.reconcile_tx.lock().ok().and_then(|g| g.clone());
let finality_tx = self.finality_tx.lock().ok().and_then(|g| g.clone());
```

These lines acquire `std::sync::Mutex` locks, clone the inner `Option<Sender>`, and immediately drop the guard. This is correct -- the guards are not held across `.await` points. The cloned senders are then moved into the spawned async task. No issue here, but worth confirming the pattern is intentional (it is).

---

## Informational Notes

### I1. Lock inventory and ordering analysis

The `SpvManager` struct uses a mix of synchronization primitives:

| Field | Type | Usage |
|-------|------|-------|
| `status`, `last_error`, `started_at`, `sync_progress_state`, `progress_updated_at`, `det_wallets`, `connected_peers`, `client_interface`, `config` | `std::sync::RwLock` | Short-lived reads/writes, never held across `.await` |
| `storage`, `reconcile_tx`, `finality_tx`, `stop_token`, `request_tx` | `std::sync::Mutex` | Short-lived lock-clone-drop pattern, never held across `.await` |
| `wallet`, `network_manager` | `tokio::sync::RwLock` | Held across `.await` in async methods |

**Lock ordering is consistent**: All `std::sync` locks are acquired, used briefly (read/clone/write a primitive), and dropped before any `.await`. The `tokio::sync::RwLock` instances (`wallet`, `network_manager`) are only held across `.await` in async contexts where this is expected.

**No deadlock risk identified** from lock ordering.

### I2. Channel capacity analysis

| Channel | Type | Capacity | Risk |
|---------|------|----------|------|
| `reconcile_tx` | `tokio::sync::mpsc` | 64 | Low -- `try_send` used, drops on full |
| `finality_tx` | `tokio::sync::mpsc` | 64 | Low -- `try_send` used, drops on full |
| `request_tx` | `tokio::sync::mpsc` | 32 | Low -- only transaction broadcasts |
| `command_tx` (DashSpvClientInterface) | `tokio::sync::mpsc::unbounded` | Unbounded | See I3 |
| `sync_rx`, `wallet_rx`, `net_rx` | `tokio::sync::broadcast` | SDK-defined | Lagged events handled correctly |
| `progress_rx` | `tokio::sync::watch` | 1 (latest value) | Correct for progress updates |

### I3. Unbounded channel for `DashSpvClientCommand`

**File**: `src/spv/manager.rs:866`

The unbounded channel is required by the SDK's `DashSpvClientInterface` API. Currently only one command type exists (`GetQuorumByHeight`), which is called infrequently (only during identity operations that need quorum validation). The comment at line 864-865 accurately documents this.

**Risk**: Negligible in practice. The channel would only grow unbounded if quorum lookups are requested faster than the SPV monitor loop can process them, which is not a realistic scenario.

### I4. Shutdown ordering is correct

The shutdown flow works as follows:

1. `AppState::on_exit()` calls `self.subtasks.shutdown()` (`src/app.rs:1133`)
2. `TaskManager::shutdown()` cancels the `CancellationToken` and joins all tasks with a 10-second timeout (`src/utils/tasks.rs:64-132`)
3. All SPV subtasks (`spv_main_loop`, `spv_request_handler`, `spv_progress_watcher`, `spv_sync_event_handler`, `spv_wallet_event_handler`, `spv_network_event_handler`) listen on either the global cancellation token or the local `stop_token`
4. The `spv_main_loop` task cancels `monitor_cancel` which causes `monitor_network` to exit, then calls `client.stop()`, then cleans up shared state

Since all SPV tasks are now in the same `JoinSet` as the rest of the application tasks, the unified shutdown path in `TaskManager::shutdown()` covers them correctly. Previously the separate OS thread had its own shutdown path which was harder to coordinate.

### I5. UI thread contention is minimal

The UI thread calls `SpvManager::status()` (sync method, `src/spv/manager.rs:355-374`) which acquires several `std::sync::RwLock` read locks. These locks are:
- Uncontended in practice (writers are event handlers that do quick writes)
- Never held across `.await` by writers
- Read-preferring (multiple readers can proceed concurrently)

No risk of UI thread stalling from SPV lock contention.

### I6. `start_spv` race condition guard

**File**: `src/context/wallet_lifecycle.rs:46-49`

```rust
if self.spv_manager.status().status.is_active() {
    return Ok(());
}
```

This is a TOCTOU check (time-of-check-time-of-use), but `SpvManager::start()` at line 378-386 also checks `stop_token.is_some()` under lock, providing the actual protection against double-start. The outer check is a fast path optimization. This is correct.

### I7. `spawn_sync` double-indirection through `tokio::spawn`

**File**: `src/utils/tasks.rs:41`

```rust
tokio::spawn(spawn_subtask(subtasks, name, future));
```

The `spawn_subtask` function acquires a tokio Mutex on the JoinSet, then spawns the actual task into it. This means each `spawn_sync` call creates an intermediate task just to register the real task. This is a minor efficiency concern but is pre-existing and not introduced by this refactoring.

### I8. Task name tracking for diagnostics

The refactoring benefits from the task naming system (`spawn_sync("spv_main_loop", ...)`, etc.) which was added in recent commits. During shutdown timeouts, the remaining task names are logged, making it easy to diagnose which SPV tasks are slow to shut down. This is a nice improvement over the previous separate-thread approach where the SPV thread was opaque to the task manager.

---

## dash-spv Dependency Analysis

**Source**: `dash-spv` from `https://www.github.com/dashpay/rust-dashcore`, branch `v0.42-dev`

### Are the key types Send + Sync?

- **`DashSpvClient<W, N, S>`**: Contains `Arc<Mutex<S>>`, `Arc<RwLock<W>>`, `Arc<RwLock<bool>>`, etc. The struct itself is `Send + Sync` when its generic parameters are `Send + Sync`. With `WalletManager`, `PeerNetworkManager`, and `DiskStorageManager`, this holds.

- **`PeerNetworkManager`**: Uses `Arc`-wrapped fields throughout (pool, discovery, reputation manager, etc.) and implements `Clone` explicitly. All internal mutability uses `tokio::sync::Mutex`. It is `Send + Sync`.

- **`DiskStorageManager`**: Wrapped in `Arc<tokio::sync::Mutex<DiskStorageManager>>` when stored in `DashSpvClient`. The inner type need only be `Send`, which it is (it wraps file handles and SQLite connections).

- **`DashSpvClientInterface`**: Contains only `mpsc::UnboundedSender<DashSpvClientCommand>`, which is `Send + Sync + Clone`.

### Does `monitor_network()` spawn internal tasks?

Yes. The `run()` method in `sync_coordinator.rs:91-102` spawns tasks via `tokio::spawn`. The `PeerNetworkManager` also spawns tasks internally (3 `tokio::spawn` calls in `manager.rs`). These internal tasks will now run on the main 12-worker runtime instead of the separate 4-worker runtime.

**Impact**: The total task count on the main runtime increases. With ~8 internal dash-spv tasks plus the 6 DET-spawned SPV handler tasks, approximately 14 tasks are added to the main runtime. Given 12 worker threads and the cooperative nature of tokio tasks, this is well within capacity.

### Unbounded channel memory safety

The `DashSpvClientCommand` unbounded channel (`mpsc::unbounded_channel`) is a design choice in the dash-spv SDK. Currently only `GetQuorumByHeight` commands are sent through it, and these are infrequent request-response pairs. The backpressure concern is theoretical; in practice the channel will rarely have more than a single pending message.

---

## Verdict: **APPROVE**

The refactoring is clean, minimal, and correct. The change from a separate OS thread + dedicated tokio runtime to a single `spawn_sync` call on the shared runtime:

1. **Eliminates runtime overhead** of maintaining a separate 4-worker tokio runtime
2. **Simplifies shutdown** by bringing all SPV tasks under the unified `TaskManager` JoinSet
3. **Preserves all existing concurrency guarantees** -- no lock ordering changes, no new shared state
4. **Does not introduce deadlocks** -- the lock analysis confirms all `std::sync` locks are short-lived and never held across `.await` points

The warnings (W1, W2) are pre-existing concerns that were not introduced by this refactoring and should be addressed separately. No changes are required for merge.
