# Backend Architecture

**Purpose:** New `src/wallet_backend/` module design — `AppContext` placement, `WalletBackend` boundary, threading/async model, event flow replacing `reconcile_spv_wallets`, error model.

[← back to README](README.md)

---

> **Deltas from this spec (as shipped).** This document is a dated design record; three points below were superseded during implementation and are NOT repealed here — they're flagged so a reader absorbs current behavior, not a withdrawn rule:
>
> - **Type opacity mandate** (below, and again under "Read Accessors"): "no `WalletManager<PlatformWalletInfo>` / `PlatformWallet` / `IdentityManager` type ever escapes it" is **superseded** by **M-PLATFORM-WALLET-FIRST-PARTY** — `wallet_backend` is NOT a type-translation layer; upstream types appear freely on its public surface by design. See `src/wallet_backend/mod.rs` module header and [2026-06-02-jit-secret-access/design.md § 8.3](../2026-06-02-jit-secret-access/design.md).
> - **Module diagram — `loader`**: `SeedReregistrationLoader` is gone; the G2 swap this diagram anticipated shipped as `UpstreamFromPersisted`. See [g2-mock-boundary.md](g2-mock-boundary.md).
> - **Module diagram — `seed_store` / `single_key_stub`**: replaced by the JIT `SecretAccess`/`secret_store` chokepoint and a real `SingleKeyView`, respectively. See [2026-06-02-jit-secret-access/design.md](../2026-06-02-jit-secret-access/design.md) and [single-key-mock.md](single-key-mock.md).

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
        ├── loader: Arc<dyn PersistedWalletLoader>
        │         // = SeedReregistrationLoader now; UpstreamFromPersisted when G2 closes
        │         // see g2-mock-boundary.md
        ├── seed_store: encrypted-seed access (DET-retained, secret boundary)
        └── single_key_stub: SingleKeyBackend (mock — see single-key-mock.md)
```

### Placement in `AppContext`

`AppContext` holds `wallet_backend: Arc<WalletBackend>` replacing today's:
- `spv_manager: Arc<SpvManager>` (`src/context/mod.rs:97`)
- `wallets: RwLock<BTreeMap<..>>` and associated wallet fields
- `core_client` / `core_backend_mode` wallet fields

`WalletBackend` is the single seam (rust-best-practices M-DONT-LEAK-TYPES, C-NEWTYPE-HIDE): no `WalletManager<PlatformWalletInfo>` / `PlatformWallet` / `IdentityManager` type ever escapes it. It exposes DET-shaped methods and DET-shaped result types only.

**Migration marker fields in `AppContext`/settings inventory:**

Two settings keys are added by the two-stage migration (see [data-model-and-migration.md — Migration execution model](data-model-and-migration.md#migration-execution-model--two-stage-marker-gated-ratified)):

| Key | Type | Meaning |
|---|---|---|
| `platform_wallet_migration_pending` | bool (0/1) | Set by Stage-A v35 tx; cleared by Stage B only when every wallet re-registered AND every identity added AND all contacts re-established AND legacy tables dropped. Authoritative "pending" signal — the backup file's existence is NOT the signal. |
| `dashpay_dip14_quarantine_active` | bool (0/1) | INERT/RESERVED — column added in Stage A (commit `6d348566`) but the quarantine apparatus was WITHDRAWN (user decision 2026-05-18). Never set to 1 by the simplified Stage-B engine. Removal deferred to P4's batched dead-column cleanup. |

**`ensure_wallet_backend` as the Stage-B seam (`src/context/mod.rs:634`):**

`AppContext::ensure_wallet_backend` is the post-unlock async entry point and the sole invocation site for Stage-B migration. It is called after seed unlock, when `seed + SDK + persister + WalletBackend` are all available. Stage B runs here, behind an `AppContext`-owned `tokio::sync::Mutex` acquired BEFORE the `platform_wallet_migration_pending` marker check, guaranteeing exactly one Stage-B execution process-wide even under reentrant or concurrent callers. Stage B completes before `WalletBackend` is published to its `ArcSwapOption`, so no task ever observes a partially-migrated backend. If the user never unlocks, Stage B never runs; the marker persists and the app operates in `WalletBackendNotYetWired`-degraded state.

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

**`PersistedWalletLoader` step (between construction and `start()`).**
After constructing `PlatformWalletManager` (which does not auto-start), `WalletBackend::new()` calls `loader.wallets_to_register(ctx)` to obtain `Vec<WalletRegistration>`. For each registration it calls `create_wallet_from_seed_bytes` then `load_persisted()` (rehydrates identity/contact/address deltas from the persister), then calls `PlatformWalletManager.start()`. This is the G2 seam: today `loader` is `SeedReregistrationLoader`; the one-line swap to `UpstreamFromPersisted` requires zero other changes. See [g2-mock-boundary.md](g2-mock-boundary.md).

**Prose sequence (end to end):**

```
WalletBackend::new()
  → PlatformWalletManager constructed (not yet started)
    → loader.wallets_to_register() → Vec<WalletRegistration>
      → for each: create_wallet_from_seed_bytes + load_persisted()
        → PlatformWalletManager.start()
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

### WalletBackend Read-Accessor Surface + WalletSnapshot Push Model

#### Read Accessors

`WalletBackend` exposes four DET-typed read accessors. No upstream types (`PlatformWallet`, `WalletManager`, `IdentityManager`, etc.) cross the boundary — only DET view models come out (M-DONT-LEAK-TYPES, C-NEWTYPE-HIDE):

```rust
fn wallet_balance(wallet_id: WalletId) -> DetWalletBalance
// DetWalletBalance { confirmed: u64, unconfirmed: u64, total: u64 }

fn transaction_history(wallet_id: WalletId) -> Vec<WalletTransaction>
// WalletTransaction — existing DET view model, retained (detached from Wallet/DB)

fn utxos(wallet_id: WalletId) -> Vec<DetUtxo>
// DetUtxo { outpoint: OutPoint, value: u64, script_pubkey: ScriptBuf, address: Address }

fn address_balances(wallet_id: WalletId) -> BTreeMap<Address, u64>
```

Sources inside `WalletBackend`: `PlatformWalletManager` / `CoreWallet` / `WalletBalance` / `SpvRuntime` / the upstream `DetPersister`.

The `TransactionRecord`→`WalletTransaction` mapping (the surviving piece of the deleted `reconcile_spv_wallets`) lives in `WalletBackend`, not in the UI or in any DB query layer.

#### WalletSnapshot Push Model

`WalletBackend` holds one `WalletSnapshot` per wallet behind an `ArcSwap`, consistent with the existing `wallet_backend: ArcSwapOption` at `src/context/mod.rs:130` and the `ConnectionStatus` push model:

```rust
struct WalletSnapshot {
    balance:      DetWalletBalance,
    transactions: Vec<WalletTransaction>,
    utxos:        Vec<DetUtxo>,
}
// Held inside WalletBackend as: ArcSwap<HashMap<WalletId, WalletSnapshot>>
```

**Update flow:** The `EventBridge` (`src/wallet_backend/event_bridge.rs`), on the `PlatformEventHandler` callbacks it already handles (`on_platform_address_sync_completed`, `on_shielded_sync_completed`, SPV supertrait callbacks), recomputes the affected wallet's snapshot off the four read accessors above and atomically swaps it in via `ArcSwap::store`. It then emits the existing `TaskResult::Refresh` on the MPSC channel.

**Read flow:** UI reads the snapshot synchronously via `app_context.wallet_backend()`. The load is lock-free and infallible — the egui frame thread never awaits, never calls upstream directly, and never blocks. A pre-first-sync snapshot is empty, which maps to the existing "syncing" state, not an error.

#### FUND-SAFETY MANDATE — Display-Only Snapshot

> **A04 — Reintroducing snapshot-based coin selection recreates the double-spend exposure the architecture eliminated. This is a P4a reviewer gate.**

The `WalletSnapshot` is **DISPLAY-ONLY**. It exists to drive the wallets screen (balance, transaction list, UTXO list) without blocking the UI thread.

Coin selection and transaction construction **MUST** go through:
- `WalletBackend::send_payment` — uses the upstream-authoritative live UTXO set at send time
- `WalletBackend::create_asset_lock_proof` — same; covers all asset-lock kinds including `AssetLockKind::Shielded` (added in P4a.5)

Both are already implemented in P2 (`src/wallet_backend/mod.rs:362,390`). No code path may select spendable inputs from `WalletSnapshot`. Any PR that routes coin selection through the snapshot must be rejected at review.

**`AssetLockKind::Shielded` (added P4a.5):** The `AssetLockKind` enum gains a `Shielded` variant, wiring `src/backend_task/shielded/bundle.rs:463,478` through `WalletBackend::create_asset_lock_proof` instead of the legacy `generic_asset_lock_transaction` + `select_unspent_utxos_for` path. This closes the last spend path that could select inputs from a legacy `Wallet.utxos` snapshot.

**No funding-outpoint API at #3625 head — `FundWithUtxo` removed, not emulated.** `platform-wallet` at PR #3625 head provides no API to fund an identity from a caller-supplied external outpoint. All asset-lock funding is upstream-authoritative wallet-managed selection via `WalletBackend::create_asset_lock_proof`. The `RegisterIdentityFundingMethod::FundWithUtxo` and `TopUpIdentityFundingMethod::FundWithUtxo` variants are removed in P4a.5 with disclosure via the one-time post-migration notice. They are not emulated, stubbed, or preserved behind a feature flag.

**`received_transaction_finality` — asset-lock-finality-only (P4a.5).** `context/transaction_processing.rs::received_transaction_finality` is slimmed in P4a.5 to handle only asset-lock finality. The `Wallet.utxos` / `address_balances` / legacy-`utxos`-table write branches are deleted; upstream / `WalletSnapshot` owns wallet-UTXO bookkeeping. The asset-lock detection and registration branch (`store_asset_lock_transaction` + the finality-wait channel that `broadcast_and_commit_asset_lock` / `wait_for_asset_lock_proof` depend on) is retained. ZMQ call sites at `app.rs:1267,1285` stay — ZMQ is still needed for asset-lock detection.

---

### Seed / Secret Boundary

The seed/secret boundary is enforced at source: `SeedReregistrationLoader` uses in-memory `Zeroizing` seed material (`src/wallet_backend/loader.rs`) and never writes seeds or private keys to the persister. Only public material (contact xpub, established-contact mapping, P2PKH addresses, identity ids) is written through the upstream `SqliteWalletPersister`. No automated `secrets_scan` test exists in the repository (add as future hardening). See `SECRETS.md` and `data-model-and-migration.md` conversion table (`WalletSeed`/`ClosedKeyItem` row).

### Error Model

`PlatformWalletError` and `PersistenceError` are wrapped into dedicated typed `TaskError` variants with `#[source]` (rust-best-practices error rules; CLAUDE.md "Never store user-facing strings in error variants"). No catch-all `String` variant. Every error gets a dedicated variant enabling structural matching, clean `Display`/`Debug` separation, and testable user-facing text.
