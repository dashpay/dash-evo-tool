# BackendTask Contract

**Purpose:** Full mapping of every DET `BackendTask` variant to its disposition in the rewrite — kept, modified, removed, or new — and the net frontend impact.

[← back to README](README.md)

---

## Transitional State: P0.5 → P2

Between P0.5 and P2, the `BackendTask` enum shape is preserved but wallet/identity/DashPay dispatch arms are inert. Specifically:

- **`TaskError::WalletBackendNotYetWired`** — new typed fieldless variant introduced at P0.5 for all wallet/identity/DashPay task arms that have been delete-and-stub'd (Cluster C in the [P0.5 Compile-Floor Task List](phasing.md#p05-compile-floor-task-list)). These arms return this error from P0.5 until P2 replaces them with real `WalletBackend` calls. User-facing message: *"This action is being upgraded and is temporarily unavailable. Please use the previous version of the app to transact, or wait for the next update."* UI renders a calm `MessageBanner`; no action required from the user.

- **`TaskError::SingleKeyWalletsUnsupported`** — typed fieldless variant for all single-key task arms, introduced at P0.5. Permanent from P0.5 onward (not replaced in P2; swap happens only when upstream ships a non-HD wallet type). See [single-key-mock.md](single-key-mock.md).

Both variants follow the error taxonomy rules (CLAUDE.md): dedicated fieldless variants, message via `#[error("…")]`, no `String` fields, no raw technical details in the user-facing message, `Debug` repr via `BannerHandle::with_details`.

---

## C. BackendTask Contract Mapping

The `BackendTask` enum (`src/backend_task/mod.rs:92-100`) and the action/channel/`TaskResult`/`display_task_result` loop are preserved. Signatures are modified only where the wallet model type changes. Verified DET task surface: `WalletTask` (`src/backend_task/wallet/mod.rs`), `CoreTask` (`src/backend_task/core/mod.rs:44`), `IdentityTask` (`src/backend_task/identity/mod.rs`), `DashPayTask` (`src/backend_task/dashpay.rs`).

### Task Table

| Task variant | Disposition | Rationale / frontend impact |
|---|---|---|
| `WalletTask::GenerateReceiveAddress` | **modified** | `wallet_backend.next_receive_address(wallet_id)` → upstream `PlatformWallet`. `seed_hash` → `WalletId` (DET-opaque newtype). UI address-display screens unchanged. |
| `WalletTask::FetchPlatformAddressBalances` | **kept** | Platform addresses are upstream "Outside scope" (persister excludes them); DET keeps `fetch_platform_address_balances.rs` via DAPI. |
| `WalletTask::TransferPlatformCredits` | **kept** | Platform L2, DAPI/SDK. |
| `WalletTask::FundPlatformAddressFromAssetLock` | **modified** | Asset-lock state from upstream `AssetLockManager`/`TrackedAssetLock`; DET orchestrates funding. Result variant unchanged. |
| `WalletTask::WithdrawFromPlatformAddress` | **kept** | DAPI/SDK. |
| `WalletTask::FundPlatformAddressFromWalletUtxos` | **modified** | UTXO selection via upstream wallet; `CoreBackendMode` branch removed. |
| `CoreTask::SendWalletPayment` | **modified** | Single arm: `wallet_backend.send_payment(...)` → upstream build/sign + `SpvRuntime::broadcast_transaction`. RPC arm removed. Result `WalletPayment{txid,...}` unchanged. |
| `CoreTask::SendSingleKeyWalletPayment` | **modified → stub** | Single-key mock (see [single-key-mock.md](single-key-mock.md)): returns `TaskError::SingleKeyWalletsUnsupported`. UI shows not-supported banner. |
| `CoreTask::MineBlocks` | **kept (separate utility)** | No `platform-wallet` equivalent (full-node `generatetoaddress`). Moves to thin `Core-RPC` utility outside `WalletBackend` (see [removal-inventory.md § RPC Fate](removal-inventory.md#rpc-backend-mode--fate)). Regtest/Devnet only. Contract unchanged. |
| `CoreTask::RefreshWalletInfo` | **modified** | SPV arm (`reconcile_spv_wallets`) deleted; becomes no-op-or-light query — upstream syncs continuously and pushes events. RPC arm removed. May be demoted to UI "request refresh." Refresh button still works; returns faster. |
| `CoreTask::RefreshSingleKeyWalletInfo` | **modified → stub** | Single-key mock. Already errors under SPV today (`src/backend_task/core/refresh_single_key_wallet_info.rs:23`). |
| `CoreTask::CreateAssetLock` | **modified** | Build via upstream; broadcast via `SpvRuntime`. Result unchanged. |
| `CoreTask::ListCoreWallets` | **hard-removed** | Named Core wallets are RPC-only; meaningless without RPC mode. Hard-removed immediately; UI entry point (Core-wallet picker) deleted same release (Decision #8). |
| `CoreTask::RecoverAssetLocks` | **hard-removed** | Upstream `AssetLockManager` tracks continuously; explicit recovery is obsolete. Hard-removed immediately; UI entry point deleted same release (Decision #8 — no one-release grace). |
| `IdentityTask::RegisterIdentity` / `IdentityTask::TopUpIdentity` | **modified — `FundWithUtxo` variants removed** | DAPI/SDK state-transition flows. `RegisterIdentityFundingMethod::FundWithUtxo` and `TopUpIdentityFundingMethod::FundWithUtxo` are removed in P4a.5 (no upstream funding-outpoint API exists at #3625 head; cannot be preserved). Accepted user-facing behavior change: identity registration and top-up are funded only from wallet-managed balance via `WalletBackend::create_asset_lock_proof`. External scanned-outpoint direct funding is removed and disclosed via the one-time post-migration notice. All other identity task variants (transfer, withdraw, add_key, load, discover, refresh) are internally rewired with signatures stable and UI unaffected. |
| `IdentityTask::*` (transfer/withdraw/add_key/load/discover/refresh) | **mostly kept, internally rewired** | DAPI/SDK state-transition flows — zero `CoreBackendMode` branches. Identity state read via upstream `IdentityManager`/`IdentityWallet`; `QualifiedIdentity` blob retained. Signatures stable; UI unaffected. `discover_identities` keeps DET Devnet path (see [open-questions.md #4](open-questions.md)). |
| `IdentityTask::RegisterDpnsName`, DPNS load/refresh | **kept** | No upstream DPNS register flow (`DpnsNameInfo` is read-only). DET-owned permanently. |
| `DashPayTask::*` (contact request/accept, profile, avatar, auto-accept, incoming payments) | **modified, hybrid** | Contact-request/established-contact/profile state + crypto via upstream; DET keeps orchestration, avatar I/O, auto-accept proof, incoming-payment detection (Decision #5 hybrid split). DIP-14/15 derivation delegated upstream (`dip14_derivation.rs`/`hd_derivation.rs` deleted, subject to migration execution). Contacts are re-established on upstream derivation unconditionally — no quarantine error path (Decision #6, 2026-05-18 re-resolution; see [data-model-and-migration.md](data-model-and-migration.md) — "Accepted fund-accessibility trade-off"). `TaskError::DashPayContactDerivationIrreconcilable` is unused by the migration path — candidate for P4 removal if no other caller. Result variants stable; UI unchanged. |
| `BackendTask::SwitchNetwork{start_spv}` | **modified** | `start_spv` semantics → `PlatformWalletManager.start()`/`shutdown()`. `set_core_backend_mode_volatile` removed. |
| `BackendTask::ReinitCoreClientAndSdk` | **modified** | Core client only relevant to thin RPC mining utility; SDK reinit stays. |
| Token / contested-voting / document / contract / shielded / grovestark tasks | **kept as-is** | Out of `platform-wallet` scope. Zero change. |

### UI Display Data-Path — No BackendTask Changes

The wallets UI tx/balance/UTXO display data-path rewire (P4a) introduces **no `BackendTask` variant changes**. It is a read-path relocation behind the `WalletBackend` seam: display is fed by the `WalletSnapshot` (updated via the existing `EventBridge` `TaskResult::Refresh`), not by task results. `WalletTransaction`-row helpers continue to take `&WalletTransaction` unchanged. The action/channel contract is frozen; no frontend plumbing changes for this gap.

See [backend-architecture.md § WalletBackend Read-Accessor Surface + WalletSnapshot Push Model](backend-architecture.md#walletbackend-read-accessor-surface--walletsnapshot-push-model) for the snapshot design.

### Net Frontend Impact

Result variants and the action/channel contract are preserved — UI screens are largely unchanged. Concrete UI changes:

1. Remove RPC-mode toggle, Core-wallet picker, and "Local Dash Core node" settings (`network_chooser_screen`).
2. Single-key screens show a not-supported banner (read-only view of preserved data).
3. SPV sync-progress UI fed from upstream `sync_progress()` via thin `ConnectionStatus` adapter — visual parity, different source.
4. `RefreshWalletInfo` returns near-instantly (upstream is already syncing).
5. **`FundWithUtxo` removed (P4a.5):** The option to fund an identity directly from a scanned external outpoint (QR-direct-fund UI) is no longer available. Identity registration and top-up accept only wallet-managed balance as the funding source. This change is disclosed via the one-time post-migration informational notice shown to all migrated users.
