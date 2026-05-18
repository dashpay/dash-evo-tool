# Backend Architecture

**Purpose:** New `src/wallet_backend/` module design — `AppContext` placement, `WalletBackend` boundary, threading/async model, event flow replacing `reconcile_spv_wallets`, error model.

[← back to README](README.md)

---

## B. Target Backend Architecture

### New Module: `src/wallet_backend/`

One owner, one boundary.

```
AppContext
  └── wallet_backend: Arc<WalletBackend>          // the ONLY wallet entry point
        ├── pwm: PlatformWalletManager<DetPersister>
        │         // upstream — owns SpvRuntime+sync+identity+dashpay+assetlocks
        ├── DetPersister: Arc<dyn PlatformWalletPersistence>
        │         // = upstream SqliteWalletPersister (PR #3625); no DET-authored persister
        ├── EventBridge: Arc<dyn PlatformEventHandler>
        │         // DET-authored; receives upstream events → TaskResult channel
        ├── seed_store: encrypted-seed access (DET-retained, decision #3 secret boundary)
        └── single_key_stub: SingleKeyBackend (mock — see single-key-mock.md)
```

### Placement in `AppContext`

`AppContext` holds `wallet_backend: Arc<WalletBackend>` replacing today's:
- `spv_manager: Arc<SpvManager>` (`src/context/mod.rs:97`)
- `wallets: RwLock<BTreeMap<..>>` and associated wallet fields
- `core_client` / `core_backend_mode` wallet fields

`WalletBackend` is the single seam (rust-best-practices M-DONT-LEAK-TYPES, C-NEWTYPE-HIDE): no `WalletManager<PlatformWalletInfo>` / `PlatformWallet` / `IdentityManager` type ever escapes it. It exposes DET-shaped methods and DET-shaped result types only.

### `BackendTask` Dispatch — Unchanged Shape

`AppContext::run_backend_task()` (`src/backend_task/mod.rs:409`) still matches the `BackendTask` enum and dispatches. Wallet/identity/DashPay task arms now call `self.wallet_backend.<method>()` instead of `spv_manager` / `run_wallet_task` / `reconcile`. The action→channel→`TaskResult`→`display_task_result` loop (CLAUDE.md "App Task System") is preserved verbatim — that is the frozen frontend contract. See [backendtask-contract.md](backendtask-contract.md) for the full task-by-task mapping.

### Threading / Async Model

`PlatformWalletManager` runs its own tokio tasks:
- `SpvRuntime` sync loop via `spawn_in_background`
- Identity-sync (per interval)
- Platform-address sync (~15s)
- Optional shielded sync (~60s)
- Internal event-adapter task

DET no longer spawns sync or reconcile tasks. `WalletBackend` is `Clone` via `Arc<Inner>` (M-SERVICES-CLONE), `Send + Sync`.

### Event Flow Back to Frontend

This replaces `reconcile_spv_wallets` and the SPV handlers.

**1. EventBridge construction.**
`WalletBackend` constructs a DET `EventBridge` implementing `platform_wallet::PlatformEventHandler` (sync trait, object-safe — confirmed in `packages/rs-platform-wallet/src/events.rs`). Registered via `PlatformEventManager::add_handler` (lock-free `ArcSwap`).

**2. Upstream events emitted.**
- `on_platform_address_sync_completed(&PlatformAddressSyncSummary)`
- `on_shielded_sync_completed(&ShieldedSyncPassSummary)`
- Plus `EventHandler` supertrait callbacks: SPV sync/network/wallet/progress/error — the same `dash-spv` event surface DET's `SpvEventHandler` consumes today.

**3. EventBridge callbacks.**
Callbacks are sync and must not block. Each maps the upstream event into a DET `TaskResult::{Success(Refresh)/...}` and sends it on the existing MPSC `task_result_sender` (the same channel `AppState::update()` polls each frame). For state queries it does a quick read off `WalletBackend` accessors.

**4. Frame loop — unchanged.**
`AppState::update()` continues to poll `task_result_receiver.try_recv()` and route to `display_task_result()`. `ConnectionStatus` gains a thin adapter fed by the sync/progress callbacks + `SpvRuntime::sync_progress()` / `tip_block_time()` instead of DET-owned SPV atomics.

**Prose sequence (end to end):**

```
PlatformWalletManager.start()
  → SpvRuntime spawns sync loop
    → blocks/filters processed internally
      → wallet/identity/dashpay state mutates inside PlatformWalletInfo
        → upstream changeset pipeline persists via DetPersister (upstream SQLite persister)
          → PlatformEventHandler fires
            → DET EventBridge
              → TaskResult MPSC
                → AppState::update()
                  → Screen::display_task_result()
                    → repaint
```

The 230-line `reconcile_spv_wallets` is deleted; its job is now upstream's changeset + event pipeline.

### Error Model

`PlatformWalletError` and `PersistenceError` are wrapped into dedicated typed `TaskError` variants with `#[source]` (rust-best-practices error rules; CLAUDE.md "Never store user-facing strings in error variants"). No catch-all `String` variant. Every error gets a dedicated variant enabling structural matching, clean `Display`/`Debug` separation, and testable user-facing text.
