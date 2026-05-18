# Upstream Reality

**Purpose:** Verified facts about what `platform-wallet` owns at PR #3625 head — the definitive answer on DET `src/spv/` deletion, the complete upstream export surface, and the G2 caveat.

[← back to README](README.md)

---

All facts verified at PR #3625 head `738091f734e05c7a1b822bb1ebff336c93b67891`. Sources listed at end.

## A. Decision #1 — `platform-wallet` Owns SPV Internally: CONFIRMED

Evidence chain:

**1. Direct `dash-spv` dependency.**
`packages/rs-platform-wallet/Cargo.toml` declares `dash-spv = { workspace = true }` as a direct dependency. The crate links the SPV engine itself.

**2. `SpvRuntime` owns and drives sync.**
`packages/rs-platform-wallet/src/spv/runtime.rs` — `SpvRuntime { event_manager, wallet_manager, client: RwLock<Option<SpvClient>>, background_cancel }`. It constructs `DashSpvClient::new()` internally, owns `PeerNetworkManager` + `DiskStorageManager`. Exposes:
- `run(config, cancel_token)` — its own sync loop
- `spawn_in_background(config)`
- `start`, `stop`, `sync_progress`, `tip_block_time`
- `clear_storage`, `update_config`
- `broadcast_transaction`, `get_quorum_public_key`

There is no host-feed API. The host does not push blocks, filters, mempool, or reorgs. The crate drives all of it.

**3. `PlatformWalletManager` owns `SpvRuntime`.**
`packages/rs-platform-wallet/src/manager/mod.rs` — `PlatformWalletManager<P: PlatformWalletPersistence>` owns:
- `spv_manager: Arc<SpvRuntime>`
- `platform_address_sync_manager`
- `identity_sync_manager: IdentitySyncManager<P>`
- `optional shielded_sync_manager`
- `persister: Arc<P>`
- SDK handle
- Internal event-adapter task (`event_adapter_cancel`/`event_adapter_join`)

Constructor signature: `new(sdk: Arc<Sdk>, persister: Arc<P>, app_handler: Arc<dyn PlatformEventHandler>)`. Constructor does not auto-start sync — "call start after wallets are registered."

## Definitive Answer: DET `src/spv/` Is Deletable

The following DET code is deletable and fully delegated to `platform-wallet`:

- `src/spv/` — `manager.rs` (1528L), `error.rs`, `mod.rs`, `tests.rs`
- `reconcile_spv_wallets` + `sync_spv_account_addresses` + `spv_setup_finality_listener` + `spv_setup_reconcile_listener` + `handle_spv_finality_event` (`src/context/wallet_lifecycle.rs:619-985`)
- SPV-specific `ConnectionStatus` push plumbing: `dash_spv::sync::*` imports, `set_spv_status`, SPV atomics (`src/context/connection_status.rs:8`)

**Residue that must stay (not chain sync):** A thin `ConnectionStatus`-style projection that subscribes to `platform-wallet`'s `PlatformEventHandler` callbacks and maps `sync_progress()` into the existing UI connection-state model. DET owns displaying status; `platform-wallet` owns producing it. See [backend-architecture.md § Event Flow](backend-architecture.md#event-flow-back-to-frontend).

## What `platform-wallet` Owns at PR Head

Confirmed from `packages/rs-platform-wallet/src/lib.rs` exports:

- **Chain sync** — `SpvRuntime`
- **HD wallet** — `PlatformWallet`, `WalletManager<PlatformWalletInfo>`
- **Identity lifecycle + DashPay** — `IdentityWallet<B>`, `IdentityManager`, `ManagedIdentity`, `ContactRequest`, `EstablishedContact`, `DashPayProfile`
- **DashPay derivation** — `derive_contact_xpub`, `calculate_account_reference`, `calculate_avatar_hash`, `derive_auto_accept_private_key`, `derive_contact_payment_address(_es)`, `calculate_dhash_fingerprint`
- **Asset locks** — `AssetLockManager`, `TrackedAssetLock`
- **Platform-address sync** — `PlatformAddressSyncManager`
- **Identity/token-balance sync** — `IdentitySyncManager`, `TokenBalanceChangeSet`
- **Persistence** — `PlatformWalletPersistence` trait (`store`/`flush`/`load`/`get_core_tx_record`)
- **Top-level handle** — `PlatformWalletManager`
- **Broadcast** — `broadcaster`
- **Event model** — `PlatformEventHandler` / `PlatformEventManager`
- **DPNS** — `DpnsNameInfo` (read-only data type only; no register flow)
- **Identity funding** — `IdentityFunding`, `TopUpFundingMethod`

For the reverse gap (what DET keeps permanently), see [feature-coverage.md § Section 2](feature-coverage.md#section-2--reverse-gap-det-features-absent-from-platform-wallet) and [removal-inventory.md § RETAIN](removal-inventory.md#retain).

## G2 Caveat — `Wallet::from_persisted` / `load()` Gap

Confirmed at PR head in `packages/rs-platform-wallet-storage/src/sqlite/persister.rs`:

```
LOAD_UNIMPLEMENTED = ["ClientStartState::wallets"]
```

Rustdoc: "Partial reconstruction caveat — leaves `ClientStartState::wallets` empty — the latter requires an upstream `Wallet::from_persisted` constructor that doesn't exist yet." `load()` populates only `platform_addresses`.

**Upstream prescribed workaround:** `manager/wallet_lifecycle.rs` — `create_wallet_from_seed_bytes → load_persisted()` re-initializes platform/identity state from the persister around a freshly seed-derived wallet.

**DET consequence:** DET must retain the encrypted seed and re-register each wallet from seed on every launch. The persister supplies identity/contact/UTXO/asset-lock deltas around the freshly derived wallet. This is the frozen Phase-2↔Phase-3 contract and shapes the one-time migration design in [data-model-and-migration.md](data-model-and-migration.md). See [phasing.md](phasing.md) for gate timing.

## Provenance

Upstream @ `738091f734…`:
- `packages/rs-platform-wallet/Cargo.toml` — direct `dash-spv` dep
- `packages/rs-platform-wallet/src/spv/runtime.rs` — `SpvRuntime` owns `DashSpvClient`, own sync loop
- `packages/rs-platform-wallet/src/spv/mod.rs`
- `packages/rs-platform-wallet/src/manager/mod.rs` — `PlatformWalletManager` owns `SpvRuntime` + persister + sync managers
- `packages/rs-platform-wallet/src/manager/wallet_lifecycle.rs` — `create_wallet_from_seed_bytes` / `load_persisted`
- `packages/rs-platform-wallet/src/events.rs` — `PlatformEventHandler` sync object-safe trait, `PlatformEventManager::add_handler`
- `packages/rs-platform-wallet/src/lib.rs` — export surface
- `packages/rs-platform-wallet/src/changeset/traits.rs` — `PlatformWalletPersistence`, "Outside scope"
- `packages/rs-platform-wallet-storage/src/sqlite/persister.rs` — `LOAD_UNIMPLEMENTED wallets` gap
- PR #3625 metadata: open, draft, not merged, base `v3.1-dev` (`54322f7a…`), head `738091f734…`, 17 commits, +6630/-24, milestone v3.1.0

DET @ `v1.0-dev`:
- `src/spv/manager.rs` — 1528L, owns `WalletManager<ManagedWalletInfo>`
- `src/context/wallet_lifecycle.rs:619-985` — reconcile + SPV listeners
- `src/context/mod.rs:97,427-686`
- `src/context/connection_status.rs:8` — SPV atomics
