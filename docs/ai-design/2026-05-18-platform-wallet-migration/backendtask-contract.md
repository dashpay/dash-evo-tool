# BackendTask Contract

**Purpose:** Full mapping of every DET `BackendTask` variant to its disposition in the rewrite — kept, modified, removed, or new — and the net frontend impact.

[← back to README](README.md)

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
| `IdentityTask::*` (register/topup/transfer/withdraw/add_key/load/discover/refresh) | **mostly kept, internally rewired** | DAPI/SDK state-transition flows — zero `CoreBackendMode` branches. Identity state read via upstream `IdentityManager`/`IdentityWallet`; `QualifiedIdentity` blob retained. Signatures stable; UI unaffected. `discover_identities` keeps DET Devnet path (see [open-questions.md #4](open-questions.md)). |
| `IdentityTask::RegisterDpnsName`, DPNS load/refresh | **kept** | No upstream DPNS register flow (`DpnsNameInfo` is read-only). DET-owned permanently. |
| `DashPayTask::*` (contact request/accept, profile, avatar, auto-accept, incoming payments) | **modified, hybrid** | Contact-request/established-contact/profile state + crypto via upstream; DET keeps orchestration, avatar I/O, auto-accept proof, incoming-payment detection (Decision #5 hybrid split). DIP-14/15 derivation delegated upstream (`dip14_derivation.rs`/`hd_derivation.rs` deleted, subject to migration execution + hard-stop path proven). Contact tasks targeting a quarantined contact return `TaskError::DashPayContactDerivationIrreconcilable { contact: Identifier }` + blocking banner; non-quarantined contacts are unaffected (Decision #6 — see [dip14-migration-hardstop.md](dip14-migration-hardstop.md)). Result variants stable; UI unchanged. |
| `BackendTask::SwitchNetwork{start_spv}` | **modified** | `start_spv` semantics → `PlatformWalletManager.start()`/`shutdown()`. `set_core_backend_mode_volatile` removed. |
| `BackendTask::ReinitCoreClientAndSdk` | **modified** | Core client only relevant to thin RPC mining utility; SDK reinit stays. |
| Token / contested-voting / document / contract / shielded / grovestark tasks | **kept as-is** | Out of `platform-wallet` scope. Zero change. |

### Net Frontend Impact

Result variants and the action/channel contract are preserved — UI screens are largely unchanged. Concrete UI changes:

1. Remove RPC-mode toggle, Core-wallet picker, and "Local Dash Core node" settings (`network_chooser_screen`).
2. Single-key screens show a not-supported banner (read-only view of preserved data).
3. SPV sync-progress UI fed from upstream `sync_progress()` via thin `ConnectionStatus` adapter — visual parity, different source.
4. `RefreshWalletInfo` returns near-instantly (upstream is already syncing).
