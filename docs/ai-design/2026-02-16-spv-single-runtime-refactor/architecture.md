# SPV Single-Runtime Architecture

## 1. Overview

This document describes the refactoring of `SpvManager` to eliminate the dedicated OS thread and
secondary 4-worker tokio runtime, moving all SPV operations onto the application's existing
12-worker multi-thread tokio runtime created in `main.rs`.

### Current Architecture

```
main() ──► 12-worker tokio runtime ──► block_on(start()) ──► eframe UI loop
                                        ├── AppContext backend tasks (tokio::spawn)
                                        ├── reconcile / finality listeners
                                        └── spv_wallet_load / unload

SpvManager::start() ──► std::thread("spv") ──► NEW 4-worker tokio runtime
                                                 └── block_on(run_spv_loop)
                                                      ├── build_client / start / monitor_network
                                                      └── event handlers (spawn_sync → tokio::spawn on SPV runtime)
```

### Target Architecture

```
main() ──► 12-worker tokio runtime ──► block_on(start()) ──► eframe UI loop
                                        ├── AppContext backend tasks (tokio::spawn)
                                        ├── reconcile / finality listeners
                                        ├── spv_wallet_load / unload
                                        └── SpvManager::start() ──► tokio::spawn(run_spv_loop)
                                             ├── build_client / start / monitor_network
                                             └── event handlers (spawn_sync → tokio::spawn on SAME runtime)
```

### Key Insight

The separate OS thread + runtime was originally added as a resource-isolation measure ("ensures SPV
sync doesn't compete with UI thread resources"). However, this isolation provides minimal benefit
because:

1. The UI event loop itself runs on the main tokio runtime via `block_on(start())`, which means
   the main runtime already handles significant I/O work from backend tasks.
2. The 12-worker pool has ample capacity for SPV's network I/O (peer connections, block filter
   downloads, header sync).
3. CPU-intensive operations (if any) in the SPV client should use `spawn_blocking`, not a separate
   runtime.
4. Having two runtimes complicates shutdown coordination, cross-runtime channel communication,
   and debugging.

## 2. Detailed Design

### 2.1 SpvManager::start() — Replace Thread + Runtime with tokio::spawn

**Current** (`src/spv/manager.rs:376-442`):

```rust
pub fn start(self: &Arc<Self>, expected_wallet_count: usize) -> Result<(), String> {
    // ... status checks, token setup ...
    std::thread::Builder::new()
        .name("spv".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .thread_name("spv-rt")
                .build()
                .expect("Failed to create SPV runtime");
            rt.block_on(async move {
                // ... run_spv_loop ...
            });
        })
        .map_err(|e| format!("Failed to spawn SPV thread: {e}"))?;
    Ok(())
}
```

**Proposed**:

```rust
pub fn start(self: &Arc<Self>, expected_wallet_count: usize) -> Result<(), String> {
    // ... status checks, token setup (unchanged) ...

    let manager = Arc::clone(self);
    let global_cancel = self.subtasks.cancellation_token.clone();

    // Spawn on the existing main tokio runtime instead of creating a new one
    self.subtasks.spawn_sync("spv_main_loop", async move {
        let manager_for_loop = Arc::clone(&manager);
        if let Err(err) = manager_for_loop
            .run_spv_loop(stop_token, global_cancel, expected_wallet_count)
            .await
        {
            tracing::error!(error = %err, network = ?manager.network, "SPV runtime failed");
            if let Err(e) = manager.write_last_error(Some(err.clone())) {
                tracing::error!("Failed to write SPV error: {}", e);
            }
            if let Err(e) = manager.write_status(SpvStatus::Error) {
                tracing::error!("Failed to write SPV status: {}", e);
            }
        }

        // Clean up on exit
        if let Ok(mut guard) = manager.stop_token.lock() {
            *guard = None;
        }
    });

    Ok(())
}
```

**Rationale**: `run_spv_loop` is an async function. `DashSpvClient::monitor_network` is async
and uses `tokio::select!` internally. All I/O operations (peer connections, message send/recv)
use tokio's networking primitives, so they naturally work on any tokio multi-thread runtime.
There is no requirement for a dedicated runtime.

### 2.2 Event Handler Spawning — No Changes Required

The event handlers (`spawn_sync_event_handler`, `spawn_wallet_event_handler`,
`spawn_network_event_handler`, `spawn_progress_watcher`, `spawn_request_handler`) all use
`self.subtasks.spawn_sync(name, future)` which calls `tokio::spawn`.

Currently, because these are called from within `run_spv_loop` which runs inside the SPV
runtime's `block_on`, `tokio::spawn` targets the SPV runtime. After the refactoring,
`run_spv_loop` will run as a task on the main runtime, so `tokio::spawn` will correctly
target the main runtime. **No code changes needed** in these methods.

### 2.3 get_quorum_public_key — No Changes Required

This method at `src/spv/manager.rs:616-675` uses:

```rust
tokio::task::block_in_place(|| {
    tokio::runtime::Handle::current().block_on(async { ... })
})
```

This pattern works correctly on any multi-thread tokio runtime. `block_in_place` temporarily
converts the current worker thread into a blocking thread, and `Handle::current()` gets the
handle of whatever runtime is active. Since this method is called from contexts that already
have a tokio runtime handle (either from the SPV context provider or from backend tasks),
**no changes needed**.

Important: `block_in_place` requires a multi-thread runtime. The main runtime is multi-thread
(12 workers), so this is safe.

### 2.4 Shutdown — Simplified

**Current shutdown flow**:
1. `AppContext::stop_spv()` → `SpvManager::stop()` → cancels `stop_token`
2. `stop_token` cancellation triggers the `tokio::select!` in `run_sync_and_monitor`
3. `monitor_cancel.cancel()` stops the network monitor
4. `client.stop().await` cleans up
5. The SPV runtime's `block_on` returns, the OS thread exits
6. App `on_exit` → `subtasks.shutdown()` → cancels global token, joins tasks

**Problem with current**: `subtasks.shutdown()` only joins tasks on the main runtime's
`JoinSet`. The SPV thread + runtime are completely independent and not tracked. If SPV
shutdown hangs, the app has no visibility or control.

**After refactoring**:
1. `SpvManager::stop()` → cancels `stop_token` (unchanged)
2. `run_spv_loop` is now a task in the main `TaskManager`'s `JoinSet`
3. `subtasks.shutdown()` cancels the global token AND waits for all tasks including SPV
4. Single unified shutdown path with timeout and diagnostics

This is a **correctness improvement**: the SPV loop is now properly tracked by the
`TaskManager` and will appear in shutdown diagnostics if it hangs.

### 2.5 DashSpvClient Ownership and Send Safety

`DashSpvClient` contains a `SyncManager` that is not behind `Arc<Mutex<_>>` (by design —
see the doc comment at `client/core.rs:110-133`). The client uses `&mut self` for
`monitor_network`, `start`, and `stop`.

The client is created, started, monitored, and stopped all within a single `async` block
in `run_spv_loop` → `run_sync_and_monitor`. It is never shared across tasks. This pattern
is compatible with `tokio::spawn` as long as `DashSpvClient` is `Send`.

**Verification**: `DashSpvClient<WalletManager<ManagedWalletInfo>, PeerNetworkManager, DiskStorageManager>`
must be `Send` for the future returned by `run_spv_loop` to be `Send` (required by
`tokio::spawn`). The struct fields are:
- `config: ClientConfig` — owned data, `Send`
- `state: Arc<RwLock<ChainState>>` — `Send + Sync`
- `network: PeerNetworkManager` — must be `Send`
- `storage: Arc<Mutex<DiskStorageManager>>` — `Send + Sync`
- `wallet: Arc<RwLock<WalletManager<...>>>` — `Send + Sync` (already shared via `Arc`)
- `sync_manager: SyncManager<S, N, W>` — must be `Send`
- Various channels (mpsc senders/receivers) — `Send`

If this compiles currently (which it does, since the SPV thread creates and uses the client
within `block_on`), the types are `Send`. The compiler will verify this at the call site
when we change to `tokio::spawn`.

### 2.6 TaskManager Interaction

The `SpvManager` receives a `subtasks: Arc<TaskManager>` in its constructor. This `TaskManager`
is the **same instance** shared by `AppContext` and `AppState`. Currently:

- `SpvManager` uses `self.subtasks.spawn_sync()` for event handlers → spawns on SPV runtime
- `AppContext` uses `self.subtasks.spawn_sync()` for reconcile/finality listeners → spawns on main runtime

After refactoring, **all** `spawn_sync` calls target the main runtime. The `TaskManager`'s
`JoinSet` will contain all SPV-related tasks in one place. This simplifies shutdown and
gives consistent task tracking.

## 3. File-by-File Change List

### `src/spv/manager.rs`

**Change 1: `SpvManager::start()` (lines 376-442)**

Remove the `std::thread::Builder` + `tokio::runtime::Builder` block. Replace with
`self.subtasks.spawn_sync("spv_main_loop", async move { ... })`.

The async block body is identical to what was inside `rt.block_on(async move { ... })`.

**Change 2: Remove unused imports**

After removing the thread/runtime code, the following imports become unused and should
be removed:
- No specific runtime-related imports are used directly (the runtime builder was inline).

**No other changes needed in this file.** The `run_spv_loop`, `run_sync_and_monitor`,
event handler spawning methods, `build_client`, `stop`, `get_quorum_public_key`,
wallet management methods, and all helper methods remain unchanged.

### `src/main.rs`

**No changes.** The 12-worker runtime stays as-is.

### `src/utils/tasks.rs`

**No changes.** `TaskManager` and `spawn_sync` work correctly. The SPV main loop
task will now appear in the `JoinSet` alongside other tasks.

### `src/context/wallet_lifecycle.rs`

**No changes.** `start_spv()` calls `self.spv_manager.start(expected_wallets)` which
now spawns a task instead of a thread. The calling code doesn't need to know.

### `src/app.rs`

**No changes.** Shutdown via `self.subtasks.shutdown()` now automatically covers
the SPV loop since it's in the same `JoinSet`.

### `src/spv/mod.rs`

**No changes.**

### `src/spv/error.rs`

**No changes.**

## 4. Deadlock and Concurrency Risk Analysis

### 4.1 Risk: TaskManager JoinSet Lock Contention

**Concern**: `TaskManager::spawn_sync` acquires a `tokio::sync::Mutex` on the `JoinSet`
to add tasks. The SPV event handlers spawn tasks frequently. Could this cause contention?

**Assessment**: LOW RISK. The lock is held only for the duration of inserting into the
`JoinSet`, which is O(1). The `tokio::sync::Mutex` is fair and async-aware, so it won't
block worker threads. This is the same pattern used today for all other background tasks.

### 4.2 Risk: SPV Competing for Worker Threads

**Concern**: SPV's `monitor_network` and event handlers now share worker threads with
UI backend tasks. Could SPV monopolize workers?

**Assessment**: LOW RISK. SPV operations are predominantly I/O-bound (network reads/writes,
disk storage). Tokio's work-stealing scheduler distributes I/O tasks efficiently across
12 workers. The SPV client uses standard tokio I/O primitives (`TcpStream`, channels,
`select!`), so it cooperatively yields. If CPU-intensive processing occurs (e.g., header
validation), it happens in small bursts between awaits.

**Mitigation** (if needed later, not recommended initially): Use `tokio::task::spawn_blocking`
for any CPU-heavy SPV operations. This is a targeted fix, not a blanket approach.

### 4.3 Risk: Shutdown Ordering

**Concern**: `TaskManager::shutdown()` cancels the global `CancellationToken` and then
joins all tasks. SPV event handlers check `cancel.cancelled()` in their `tokio::select!`
loops. Could the SPV main loop and its child handlers deadlock during shutdown?

**Assessment**: LOW RISK. The shutdown sequence is:
1. Global cancel token fires
2. All event handlers break out of their loops (they all check `cancel.cancelled()`)
3. `run_sync_and_monitor` detects `global_cancel.cancelled()`, cancels monitor, calls `client.stop()`
4. `run_spv_loop` cleans up shared state (interface, storage, etc.)
5. All tasks complete and are joined

The event handlers are independent tokio tasks. They don't hold any locks that the main
loop needs during cleanup. The main loop clears shared state (`client_interface`,
`network_manager`, etc.) after the client stops, which is safe because the handlers have
already exited.

### 4.4 Risk: `block_in_place` in `get_quorum_public_key`

**Concern**: `block_in_place` temporarily converts a worker thread to blocking mode.
With a shared runtime, this reduces available workers from 12 to 11 temporarily.

**Assessment**: LOW RISK. This operation is infrequent (only during quorum key lookups)
and short-lived (a single async channel send + receive). Losing one worker temporarily
out of 12 has negligible impact. This is actually the intended use case for `block_in_place`.

### 4.5 Risk: Cross-Task Data Races on Shared State

**Concern**: `SpvManager` uses `Arc<RwLock<_>>` (std) and `Arc<AsyncRwLock<_>>` (tokio)
for shared state. Moving to a single runtime doesn't change the sharing pattern.

**Assessment**: NO RISK. The synchronization primitives are the same. The data flows
through the same channels. The only difference is which runtime executes the tasks, not
how they synchronize.

### 4.6 Risk: `DashSpvClient` and `!Send` / `!Sync` Concerns

**Concern**: If `DashSpvClient` or the future returned by `run_spv_loop` is not `Send`,
`tokio::spawn` will fail to compile.

**Assessment**: COMPILE-TIME CHECK. The Rust compiler will catch this immediately. If
the types are `Send` (which they must be since they currently work in `block_on` on a
multi-thread runtime), they will work with `tokio::spawn`. If for some reason a type
is `!Send`, the compiler error will pinpoint the exact field/type.

### 4.7 Risk: Reconcile/Finality Listeners Running on Wrong Runtime

**Concern**: Currently, reconcile and finality listeners are spawned by `AppContext`
on the main runtime. SPV event handlers are spawned on the SPV runtime. After the
change, both are on the main runtime.

**Assessment**: POSITIVE CHANGE. This eliminates a subtle cross-runtime timing issue.
Previously, reconcile signals sent from the SPV runtime's event handlers to the main
runtime's listeners had to cross runtime boundaries via tokio mpsc channels. Now they're
all on the same runtime, which is slightly more efficient and easier to reason about.

## 5. Migration Path

This refactoring is a single atomic change because the modifications are minimal:

### Step 1: Modify `SpvManager::start()` (the only code change)

Replace the thread + runtime block with `self.subtasks.spawn_sync(...)`. This is a
~15 line change in a single method.

### Step 2: Compile and verify `Send` bounds

Run `cargo build`. If any type is `!Send`, the compiler will report it. This is the
primary risk gate.

### Step 3: Test SPV functionality

1. Start the app with SPV enabled
2. Verify SPV sync starts and progresses (status transitions: Starting → Syncing → Running)
3. Verify wallet reconciliation works (balances update after sync)
4. Verify clean shutdown (no hanging tasks in shutdown logs)
5. Verify stop/restart SPV from UI works correctly
6. Verify quorum key lookups work (identity operations)

### Step 4: Verify shutdown diagnostics

1. Start app with SPV running
2. Close the app
3. Check logs for clean shutdown — the `spv_main_loop` task should appear in the
   `TaskManager` shutdown trace alongside other tasks

### Step 5: Run existing tests

```bash
cargo test --all-features --workspace
cargo clippy --all-features --all-targets -- -D warnings
```

## 6. Open Questions

### Q1: Should we remove the `subtasks` field from `SpvManager`?

Currently, `SpvManager` stores its own `Arc<TaskManager>` reference, which is the same
instance as `AppContext.subtasks`. This is correct and useful — the SPV manager's event
handlers are logically "subtasks" of the SPV lifecycle but need to be tracked for shutdown.

**Recommendation**: Keep as-is. The `TaskManager` is shared, not duplicated.

### Q2: Should `DashSpvClient` operations use `spawn_blocking` for CPU work?

The SPV client does header validation, merkle proof verification, and compact block
filter matching. These are CPU operations but typically fast (microseconds to low
milliseconds per item).

**Recommendation**: No. Keep as-is unless profiling shows that SPV CPU work causes
UI frame drops (> 16ms blocking on a worker thread). The 12-worker pool provides
sufficient headroom.

### Q3: Should we adjust the worker thread count?

Currently 12 workers. With SPV now sharing the pool, should we increase it?

**Recommendation**: No. 12 workers is already generous for an egui desktop app.
SPV adds a handful of I/O-bound tasks (peer connections, event handlers). The
work-stealing scheduler handles this efficiently. Monitor and adjust only if
performance issues are observed.

### Q4: What about the ZMQ listener (`CoreZMQListener`)?

The `CoreZMQListener` in `src/components/core_zmq_listener.rs` also creates a
separate OS thread + runtime (line 308-310). This is a separate concern and should
be addressed in a follow-up refactoring if desired.

**Recommendation**: Out of scope for this change. Address separately if unified
runtime is a broader goal.
